//! Preview-first session import. No harness commands; OpenCode's local database
//! is opened read-only and only session/message/part rows are read. Re-scanning
//! can enrich an imported copy with missing token and diff metadata.
mod adapters;
mod opencode_db;
#[cfg(debug_assertions)]
pub(crate) mod smoke;
use crate::app::{DunlinApp, blank_session};
use crate::harness::ChatItem;
use adapters::Candidate;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dunlin_ui::component::Disableable;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{
        Sizable,
        button::ButtonVariants,
        menu::{DropdownMenu, PopupMenuItem},
    },
    *,
};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

#[derive(Default)]
pub(crate) struct State {
    pub loading: bool,
    pub scanning: Option<String>,
    pub applying: bool,
    progress: Option<Arc<Mutex<Progress>>>,
    preview: Option<Preview>,
    error: Option<String>,
    generation: u64,
    backfill_started: bool,
}
impl State {
    pub(crate) fn progress_label(&self) -> Option<String> {
        self.progress
            .as_ref()
            .map(|progress| progress.lock().unwrap().label())
    }
}
struct Preview {
    harness: String,
    rows: Vec<Row>,
    warnings: Vec<String>,
    staging: Arc<tempfile::TempDir>,
}
struct Row {
    candidate: Candidate,
    project: Option<String>,
    duplicate: bool,
    update: bool,
    selected: bool,
    entries: usize,
    transcript: PathBuf,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Progress {
    phase: &'static str,
    completed: usize,
    total: Option<usize>,
}
impl Progress {
    fn label(&self) -> String {
        match self.total {
            Some(total) => format!("{} · {} / {total}", self.phase, self.completed),
            None => format!("{} · {} files found", self.phase, self.completed),
        }
    }
}

impl Preview {
    fn new(harness: String) -> Result<Self, String> {
        Ok(Self {
            harness,
            rows: Vec::new(),
            warnings: Vec::new(),
            staging: Arc::new(
                tempfile::tempdir().map_err(|e| format!("Could not prepare import cache: {e}"))?,
            ),
        })
    }

    fn push(
        &mut self,
        mut candidate: Candidate,
        projects: &[dunlin_domain::Project],
    ) -> Result<(), String> {
        let project = adapters::match_project(candidate.directory.as_deref(), projects);
        let entries = candidate.transcript.len();
        let transcript = self
            .staging
            .path()
            .join(format!("{}.json", self.rows.len()));
        let mut file =
            BufWriter::new(std::fs::File::create(&transcript).map_err(|e| e.to_string())?);
        serde_json::to_writer(&mut file, &candidate.transcript).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        candidate.transcript = Vec::new();
        self.rows.push(Row {
            candidate,
            project,
            duplicate: false,
            update: false,
            selected: true,
            entries,
            transcript,
        });
        Ok(())
    }
}

fn read_transcript(path: &Path) -> dunlin_storage::Result<Vec<ChatItem>> {
    Ok(serde_json::from_reader(BufReader::new(
        std::fs::File::open(path)?,
    ))?)
}
fn imported_id(harness: &str, source: &str) -> String {
    format!(
        "import-{}-{}",
        URL_SAFE_NO_PAD.encode(harness),
        URL_SAFE_NO_PAD.encode(source)
    )
}

pub(crate) fn imported_source(id: &str, harness: &str) -> Option<String> {
    let prefix = format!("import-{}-", URL_SAFE_NO_PAD.encode(harness));
    let encoded = id.strip_prefix(&prefix)?;
    String::from_utf8(URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()
}
fn read_files(
    harness: &str,
    paths: Vec<PathBuf>,
    candidate: &mut dyn FnMut(Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
) -> Result<Vec<String>, String> {
    let mut warnings = Vec::new();
    let total = paths.len();
    progress(Progress {
        phase: "Reading session files",
        total: Some(total),
        completed: 0,
    });
    for (index, path) in paths.into_iter().enumerate() {
        let result = (|| {
            let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Choose a regular session file".into());
            }
            let mut bytes = Vec::new();
            file.take(adapters::MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            adapters::parse(harness, &bytes)
        })();
        match result {
            Ok(candidates) => {
                for row in candidates {
                    candidate(row)?;
                }
            }
            Err(error) => warnings.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
        progress(Progress {
            phase: "Reading session files",
            total: Some(total),
            completed: index + 1,
        });
    }
    Ok(warnings)
}
fn read_harness_sessions(
    harness: &str,
    candidate: &mut dyn FnMut(Candidate) -> Result<(), String>,
    progress: &mut dyn FnMut(Progress),
) -> Result<Vec<String>, String> {
    if harness == "OpenCode" {
        let Some(path) = dunlin_usage::billing::default_opencode_db() else {
            return Ok(vec![
                "No opencode.db was found in the standard OpenCode data folders".into(),
            ]);
        };
        return opencode_db::visit(&path, None, candidate, &mut |completed, total| {
            progress(Progress {
                phase: "Reading OpenCode sessions",
                completed,
                total: Some(total),
            });
        });
    }
    let (paths, mut warnings) = discover_session_files(harness, progress);
    if paths.is_empty() {
        return Ok(warnings);
    }
    let read_warnings = read_files(harness, paths, candidate, progress)?;
    warnings.extend(read_warnings);
    Ok(warnings)
}

fn discover_session_files(
    harness: &str,
    progress: &mut dyn FnMut(Progress),
) -> (Vec<PathBuf>, Vec<String>) {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return (
            Vec::new(),
            vec!["Home directory is unavailable; cannot scan harness folders".into()],
        );
    };
    let (root, extension) = match harness {
        "Claude Code" => {
            let base = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"));
            (base.join("projects"), "jsonl")
        }
        "OpenAI Codex" => {
            let base = std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex"));
            (base.join("sessions"), "jsonl")
        }
        _ => {
            return (
                Vec::new(),
                vec![format!(
                    "No automatic session folder is known for {harness}. Use Import a portable file…"
                )],
            );
        }
    };
    if !root.is_dir() {
        return (
            Vec::new(),
            vec![format!("No session folder found at {}", root.display())],
        );
    }
    let mut files = Vec::new();
    let mut warnings = Vec::new();
    collect_session_files(&root, extension, &mut files, &mut warnings, progress);
    files.sort_by_cached_key(|path| Reverse(modified_ms(path)));
    if files.is_empty() {
        warnings.push(format!(
            "No *.{extension} session files found in {}",
            root.display()
        ));
    }
    (files, warnings)
}

fn collect_session_files(
    dir: &Path,
    extension: &str,
    files: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
    progress: &mut dyn FnMut(Progress),
) {
    let mut directories = vec![dir.to_path_buf()];
    while let Some(dir) = directories.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            warnings.push(format!("Could not read {}", dir.display()));
            continue;
        };
        for entry in read {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    warnings.push(format!(
                        "Could not read an entry in {}: {error}",
                        dir.display()
                    ));
                    continue;
                }
            };
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                directories.push(path);
            } else if file_type.is_file()
                && path.extension().and_then(|value| value.to_str()) == Some(extension)
            {
                files.push(path);
                progress(Progress {
                    phase: "Discovering sessions",
                    completed: files.len(),
                    total: None,
                });
            }
        }
    }
}

fn modified_ms(path: &Path) -> u128 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn prepare_preview(
    harness: String,
    paths: Option<Vec<PathBuf>>,
    projects: Vec<dunlin_domain::Project>,
    progress: Arc<Mutex<Progress>>,
) -> Result<Preview, String> {
    let mut preview = Preview::new(harness.clone())?;
    let mut report = |value| *progress.lock().unwrap() = value;
    let mut candidate = |candidate| preview.push(candidate, &projects);
    let warnings = match paths {
        Some(paths) => read_files(&harness, paths, &mut candidate, &mut report)?,
        None => read_harness_sessions(&harness, &mut candidate, &mut report)?,
    };
    preview.warnings = warnings;
    Ok(preview)
}

struct Applied {
    sessions: Vec<dunlin_domain::SessionSummary>,
    updated: Vec<String>,
    error: Option<String>,
}

fn apply_histories(
    database: &Path,
    rows: &[(dunlin_domain::SessionSummary, PathBuf)],
    updates: &[(String, PathBuf)],
    progress: &Mutex<Progress>,
) -> Result<Applied, String> {
    let store = dunlin_storage::Store::open(database).map_err(|e| e.to_string())?;
    let total = rows.len() + updates.len();
    let mut report = |completed| {
        *progress.lock().unwrap() = Progress {
            phase: "Importing sessions",
            completed,
            total: Some(total),
        };
    };
    report(0);
    let ids = store
        .import_session_stream(
            rows.iter()
                .map(|(summary, path)| Ok((summary.clone(), read_transcript(path)?))),
            &mut report,
        )
        .map_err(|e| format!("Import rolled back: {e}"))?;
    let ids: HashSet<_> = ids.into_iter().collect();
    let sessions = rows
        .iter()
        .filter(|(summary, _)| ids.contains(&summary.id))
        .map(|(summary, _)| summary.clone())
        .collect();
    let mut updated = Vec::new();
    let mut error = None;
    for (index, (id, path)) in updates.iter().enumerate() {
        let result = (|| -> dunlin_storage::Result<bool> {
            let incoming = read_transcript(path)?;
            let Some(mut transcript) = store.load_transcript::<Vec<ChatItem>>(id)? else {
                return Ok(false);
            };
            crate::chat_presentation::normalize_legacy(&mut transcript);
            if !merge_imported_metrics(&mut transcript, &incoming) {
                return Ok(false);
            }
            store.save_transcript(id, &transcript)?;
            Ok(true)
        })();
        match result {
            Ok(true) => updated.push(id.clone()),
            Ok(false) => {}
            Err(e) => error = Some(format!("Could not update {id}: {e}")),
        }
        report(rows.len() + index + 1);
    }
    Ok(Applied {
        sessions,
        updated,
        error,
    })
}

fn finalize_import(
    this: &mut DunlinApp,
    result: Result<Preview, String>,
    cx: &mut Context<DunlinApp>,
) {
    this.session_import.loading = false;
    this.session_import.scanning = None;
    this.session_import.progress = None;
    let mut preview = match result {
        Ok(preview) => preview,
        Err(error) => {
            this.session_import.error = Some(error);
            cx.notify();
            return;
        }
    };
    let found = preview.rows.len();
    let detail = preview
        .warnings
        .first()
        .cloned()
        .unwrap_or_else(|| "No session files matched this harness".into());
    let mut seen = HashSet::new();
    let existing: HashSet<_> = this
        .all_sessions
        .iter()
        .map(|session| session.id.as_str())
        .collect();
    let source_ids: HashSet<_> = this
        .all_sessions
        .iter()
        .filter(|session| session.harness == preview.harness)
        .filter_map(|session| session.acp_session_id.as_deref())
        .collect();
    let projects: HashSet<_> = this
        .projects
        .iter()
        .filter(|p| !p.is_remote())
        .map(|p| p.id.as_str())
        .collect();
    for row in &mut preview.rows {
        let id = imported_id(&preview.harness, &row.candidate.source_id);
        let repeated = !seen.insert(id.clone());
        row.update = existing.contains(id.as_str()) && !repeated;
        row.duplicate =
            repeated || row.update || source_ids.contains(row.candidate.source_id.as_str());
        if row.duplicate && !row.update {
            row.selected = false;
        }
        if !row
            .project
            .as_deref()
            .is_some_and(|id| projects.contains(id))
        {
            row.project = None;
        }
    }
    this.session_import.preview = Some(preview);
    if found == 0 {
        this.file_notice(format!("No sessions found: {detail}"), cx);
    } else {
        this.file_notice(
            format!("Found {found} sessions · review the import preview"),
            cx,
        );
    }
    cx.notify();
}

impl DunlinApp {
    fn clear_session_import_preview(&mut self, cx: &Context<Self>) {
        if let Some(preview) = self.session_import.preview.take() {
            cx.background_executor()
                .spawn(async move {
                    drop(preview);
                })
                .detach();
        }
    }

    fn session_import_progress(
        &mut self,
        phase: &'static str,
        cx: &mut Context<Self>,
    ) -> Arc<Mutex<Progress>> {
        let progress = Arc::new(Mutex::new(Progress {
            phase,
            ..Default::default()
        }));
        self.session_import.progress = Some(progress.clone());
        let generation = self.session_import.generation;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(100))
                    .await;
                let running = this
                    .update(cx, |this, cx| {
                        let running = this.session_import.loading
                            && this.session_import.generation == generation;
                        if running {
                            cx.notify();
                        }
                        running
                    })
                    .unwrap_or(false);
                if !running {
                    break;
                }
            }
        })
        .detach();
        progress
    }

    /// One-time background repair for imported sessions created before token
    /// and diff metadata existed. Runs when the Dashboard is opened.
    pub(crate) fn backfill_imported_metrics(&mut self, cx: &mut Context<Self>) {
        if self.session_import.backfill_started {
            return;
        }
        self.session_import.backfill_started = true;
        let sources: Vec<(String, String)> = self
            .all_sessions
            .iter()
            .filter_map(|session| {
                imported_source(&session.id, "OpenCode").map(|source| (session.id.clone(), source))
            })
            .collect();
        if sources.is_empty() {
            return;
        }
        let Some(database) = self.store.as_ref().and_then(|store| store.database_path()) else {
            return;
        };
        let Some(opencode) = dunlin_usage::billing::default_opencode_db() else {
            return;
        };
        let skip = (self.harness.running || self.harness.status.is_busy())
            .then(|| self.session().id.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let sources: HashMap<_, _> = sources
                        .into_iter()
                        .map(|(session, source)| (source, session))
                        .collect();
                    let ids: HashSet<_> = sources.keys().cloned().collect();
                    let store =
                        dunlin_storage::Store::open(&database).map_err(|e| e.to_string())?;
                    let mut updated = Vec::new();
                    opencode_db::visit(
                        &opencode,
                        Some(&ids),
                        &mut |candidate| {
                            let session_id = &sources[&candidate.source_id];
                            if skip.as_ref().is_some_and(|skip| skip == session_id) {
                                return Ok(());
                            }
                            let Ok(Some(mut transcript)) =
                                store.load_transcript::<Vec<ChatItem>>(session_id)
                            else {
                                return Ok(());
                            };
                            if transcript.iter().any(|item| {
                                matches!(
                                    item,
                                    ChatItem::User {
                                        timing: Some(_),
                                        ..
                                    }
                                )
                            }) {
                                return Ok(());
                            }
                            crate::chat_presentation::normalize_legacy(&mut transcript);
                            if merge_imported_metrics(&mut transcript, &candidate.transcript)
                                && store.save_transcript(session_id, &transcript).is_ok()
                            {
                                updated.push(session_id.clone());
                            }
                            Ok(())
                        },
                        &mut |_, _| {},
                    )?;
                    Ok::<_, String>(updated)
                })
                .await;
            let _ = this.update(cx, |this, cx| match result {
                Ok(updated) => {
                    if updated.is_empty() {
                        return;
                    }
                    let current = this.session().id.clone();
                    let reload_current = updated.iter().any(|id| id == &current);
                    for id in &updated {
                        if id != &current {
                            this.forget_transcript(id);
                        }
                    }
                    if reload_current {
                        this.load_transcript(&current);
                    }
                    this.refresh_dashboard_now(cx);
                    this.refresh_billing(cx);
                    cx.notify();
                }
                Err(error) => {
                    crate::logging::warn(&format!("imported metrics backfill unavailable: {error}"))
                }
            });
        })
        .detach();
    }

    pub(crate) fn scan_session_import(&mut self, harness: &str, cx: &mut Context<Self>) {
        if self.session_import.loading {
            return;
        }
        let harness = harness.to_string();
        self.session_import.generation = self.session_import.generation.wrapping_add(1);
        let generation = self.session_import.generation;
        self.session_import.loading = true;
        self.session_import.scanning = Some(harness.clone());
        self.session_import.error = None;
        self.clear_session_import_preview(cx);
        let progress = self.session_import_progress("Discovering sessions", cx);
        self.file_notice(format!("Scanning {harness} session folders…"), cx);
        cx.notify();
        let source = harness.clone();
        cx.spawn(async move |this, cx| {
            let projects = this
                .read_with(cx, |this, _| this.projects.clone())
                .unwrap_or_default();
            let result = cx
                .background_executor()
                .spawn(async move { prepare_preview(source, None, projects, progress) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_import.generation == generation {
                    finalize_import(this, result, cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn choose_session_import(&mut self, harness: &str, cx: &mut Context<Self>) {
        if self.session_import.loading {
            return;
        }
        let harness = harness.to_string();
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(format!("Import {harness} session files (JSON / JSONL)").into()),
        });
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        self.session_import.generation = self.session_import.generation.wrapping_add(1);
        let generation = self.session_import.generation;
        self.session_import.loading = true;
        self.session_import.scanning = Some(harness.clone());
        self.session_import.error = None;
        self.clear_session_import_preview(cx);
        let progress = self.session_import_progress("Choosing session files", cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                let _ = this.update(cx, |this, cx| {
                    if this.session_import.generation == generation {
                        this.session_import.loading = false;
                        this.session_import.scanning = None;
                        this.session_import.progress = None;
                        cx.notify();
                    }
                });
                return;
            };
            let projects = this
                .read_with(cx, |this, _| this.projects.clone())
                .unwrap_or_default();
            let source = harness.clone();
            let result = cx
                .background_executor()
                .spawn(async move { prepare_preview(source, Some(paths), projects, progress) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_import.generation == generation {
                    finalize_import(this, result, cx);
                }
            });
        })
        .detach();
    }
    fn apply_session_import(&mut self, cx: &mut Context<Self>) {
        if self.session_import.loading {
            return;
        }
        let Some(preview) = &self.session_import.preview else {
            return;
        };
        let mut rows = Vec::new();
        let mut updates = Vec::new();
        for row in &preview.rows {
            if !row.selected {
                continue;
            }
            if row.update {
                updates.push((
                    imported_id(&preview.harness, &row.candidate.source_id),
                    row.transcript.clone(),
                ));
                continue;
            }
            if row.duplicate {
                continue;
            }
            let Some(project) = self
                .projects
                .iter()
                .find(|p| Some(&p.id) == row.project.as_ref() && !p.is_remote())
            else {
                continue;
            };
            let mut summary = blank_session(&project.id, &preview.harness, None);
            summary.id = imported_id(&preview.harness, &row.candidate.source_id);
            summary.title = row.candidate.title.clone();
            if summary.title == "New session" {
                summary.title = "Imported conversation".into();
            }
            summary.model = row.candidate.model.clone();
            summary.status = dunlin_domain::SessionStatus::Done;
            summary.updated_at = chrono::DateTime::from_timestamp_millis(
                row.candidate.at_ms.min(i64::MAX as u64) as i64,
            )
            .filter(|_| row.candidate.at_ms > 0)
            .map(|d| d.to_rfc3339())
            .unwrap_or_else(|| "Imported".into());
            summary.summary = "Imported history copy. Tool records are inert; a new turn does not resume the source session.".into();
            rows.push((summary, row.transcript.clone()));
        }
        let Some(database) = self.store.as_ref().and_then(|store| store.database_path()) else {
            self.session_import.error =
                Some("History storage unavailable; nothing imported".into());
            cx.notify();
            return;
        };
        let staging = preview.staging.clone();
        self.session_import.scanning = Some(preview.harness.clone());
        self.session_import.generation = self.session_import.generation.wrapping_add(1);
        self.session_import.loading = true;
        self.session_import.applying = true;
        self.session_import.error = None;
        let progress = self.session_import_progress("Importing sessions", cx);
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let _staging = staging;
                apply_histories(&database, &rows, &updates, &progress)
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.session_import.loading = false;
                this.session_import.applying = false;
                this.session_import.scanning = None;
                this.session_import.progress = None;
                match result {
            Ok(applied) => {
                let imported = applied.sessions.len();
                let projects: HashSet<_> = this.projects.iter().map(|project| project.id.clone()).collect();
                for summary in applied.sessions {
                    if !projects.contains(&summary.project_id) { continue; }
                    if summary.project_id == this.project().id {
                        this.sessions.push(summary.clone());
                    }
                    this.all_sessions.push(summary);
                }
                for id in &applied.updated {
                    if this.session().id == *id {
                        if let Some(incoming) = this.store.as_ref()
                            .and_then(|store| store.load_transcript::<Vec<ChatItem>>(id).ok().flatten()) {
                            // A follow-up may have arrived while the worker was running.
                            merge_imported_metrics(&mut this.harness.transcript, &incoming);
                        }
                    } else {
                        this.forget_transcript(id);
                    }
                }
                        this.clear_session_import_preview(cx);
                this.session_import.error = applied.error;
                let message = match (imported, applied.updated.len()) {
                    (0, 0) => "No new sessions to import".into(),
                    (0, updated) => format!("Updated {updated} imported session histories"),
                    (imported, 0) => format!(
                        "Imported {imported} session histories; existing conversations unchanged"
                    ),
                    (imported, updated) => {
                        format!("Imported {imported} and updated {updated} session histories")
                    }
                };
                this.file_notice(message, cx);
                // Imported histories must feed both the recorded dashboard
                // counts and the local billing evidence without a manual scan.
                this.refresh_dashboard_now(cx);
                this.refresh_billing(cx);
            }
            Err(error) => {
                this.session_import.error = Some(error);
            }
                }
                cx.notify();
            });
        }).detach();
    }
}

/// Copy token timings and diff counts from a fresh source transcript onto an
/// existing imported copy. Conversation text and local follow-up turns are kept.
fn merge_imported_metrics(existing: &mut [ChatItem], incoming: &[ChatItem]) -> bool {
    use std::collections::HashMap;
    let mut timings = HashMap::new();
    let mut tools = Vec::new();
    for item in incoming {
        match item {
            ChatItem::User {
                at_ms,
                timing: Some(timing),
                ..
            } if *at_ms > 0 => {
                timings.insert(*at_ms, timing.clone());
            }
            ChatItem::Tool { view, at_ms, .. } => tools.push((*at_ms, view)),
            _ => {}
        }
    }
    let mut changed = false;
    let mut cursor = 0;
    for item in existing {
        match item {
            ChatItem::User { at_ms, timing, .. } => {
                if timing.is_none()
                    && let Some(found) = timings.get(at_ms)
                {
                    *timing = Some(found.clone());
                    changed = true;
                }
            }
            ChatItem::Tool { view, at_ms, .. } => {
                let by_id = tools[cursor..]
                    .iter()
                    .position(|(_, known)| known.id == view.id)
                    .map(|offset| cursor + offset);
                let position = by_id.or_else(|| {
                    tools
                        .get(cursor)
                        .filter(|(known_at, known)| known_at == at_ms || known.title == view.title)
                        .map(|_| cursor)
                });
                let Some(position) = position else {
                    continue;
                };
                cursor = position + 1;
                let (_, known) = tools[position];
                if view.additions == 0 && known.additions > 0 {
                    view.additions = known.additions;
                    changed = true;
                }
                if view.deletions == 0 && known.deletions > 0 {
                    view.deletions = known.deletions;
                    changed = true;
                }
                if view.diff.is_none() && known.diff.is_some() {
                    view.diff = known.diff.clone();
                    changed = true;
                }
                if view.locations.is_empty() && !known.locations.is_empty() {
                    view.locations = known.locations.clone();
                    changed = true;
                }
            }
            _ => {}
        }
    }
    changed
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = div().flex().flex_col().gap(px(8.));
    if this.session_import.loading {
        let label = this
            .session_import
            .progress_label()
            .unwrap_or_else(|| "Preparing session import…".into());
        return body
            .child(
                div()
                    .id("session-import-progress")
                    .aria_label("Session import progress")
                    .text_color(p.muted_fg)
                    .child(label),
            )
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child(
                if this.session_import.applying {
                    "Saving in the background. Please wait for the import to finish."
                } else {
                    "Reading histories in the background. Source files stay unchanged."
                },
            ))
            .into_any_element();
    }
    if let Some(preview) = &this.session_import.preview {
        body = body.child(format!("Import preview · {} · {} sessions", preview.harness, preview.rows.len()))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Copies histories only. Source files stay unchanged. Assign unmatched sessions or leave them skipped. Updating an imported copy adds recorded token and diff data without replacing your local follow-up turns."));
        let list = uniform_list(
            "session-import-preview",
            preview.rows.len(),
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                let Some(preview) = &this.session_import.preview else {
                    return Vec::new();
                };
                let p = palette(cx);
                let mut items = Vec::with_capacity(range.len());
                for i in range {
                    let row = &preview.rows[i];
                    let app = cx.entity().downgrade();
                    let project = this
                        .projects
                        .iter()
                        .find(|p| Some(&p.id) == row.project.as_ref());
                    let label = if row.update {
                        "Update imported copy".to_string()
                    } else if row.duplicate {
                        "Already imported".to_string()
                    } else {
                        project
                            .map(|p| p.name.clone())
                            .unwrap_or_else(|| "Assign project…".into())
                    };
                    let choose = dunlin_ui::button(format!("import-project-{i}"))
                        .small()
                        .label(label)
                        .disabled(row.duplicate)
                        .dropdown_caret(true)
                        .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, cx| {
                            let Some(entity) = app.upgrade() else {
                                return menu;
                            };
                            let this = entity.read(cx);
                            for project in this.projects.iter().filter(|p| !p.is_remote()) {
                                let name = format!("{} · {}", project.name, project.path);
                                let id = project.id.clone();
                                let app = app.clone();
                                menu = menu.item(
                                    PopupMenuItem::element(move |window, cx| {
                                        crate::effects::menu_surface(
                                            div().text_size(px(12.)).child(name.clone()),
                                            window,
                                            cx,
                                        )
                                    })
                                    .on_click(
                                        move |_, _, cx| {
                                            let _ = app.update(cx, |this, cx| {
                                                if let Some(row) = this
                                                    .session_import
                                                    .preview
                                                    .as_mut()
                                                    .and_then(|v| v.rows.get_mut(i))
                                                {
                                                    row.project = Some(id.clone());
                                                }
                                                cx.notify();
                                            });
                                        },
                                    ),
                                );
                            }
                            menu.scrollable(true).max_h(px(280.))
                        });
                    items.push(
                        div()
                            .h(px(56.))
                            .p(px(7.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border)
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                dunlin_ui::button(format!("import-select-{i}"))
                                    .small()
                                    .label(if row.selected { "✓" } else { "–" })
                                    .disabled(row.duplicate && !row.update)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(row) = this
                                            .session_import
                                            .preview
                                            .as_mut()
                                            .and_then(|v| v.rows.get_mut(i))
                                        {
                                            row.selected = !row.selected;
                                        }
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(12.))
                                            .child(row.candidate.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(10.))
                                            .text_color(p.muted_fg)
                                            .child(format!(
                                                "{} · {} entries",
                                                row.candidate
                                                    .directory
                                                    .as_deref()
                                                    .unwrap_or("No source directory"),
                                                row.entries
                                            )),
                                    ),
                            )
                            .child(choose)
                            .into_any_element(),
                    );
                }
                items
            }),
        )
        .h(px((preview.rows.len().min(5) * 56) as f32))
        .w_full();
        let ready = preview
            .rows
            .iter()
            .filter(|r| {
                r.selected && (!r.duplicate || r.update) && (r.project.is_some() || r.update)
            })
            .count();
        body = body
            .child(list)
            .when(!preview.warnings.is_empty(), |body| {
                body.child(
                    div()
                        .id("session-import-warnings")
                        .max_h(px(120.))
                        .overflow_y_scroll()
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(format!("{} file warning(s)", preview.warnings.len()))
                        .children(preview.warnings.iter().map(|w| {
                            div()
                                .text_size(px(11.))
                                .text_color(p.amber)
                                .child(w.clone())
                        })),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        dunlin_ui::button("confirm-session-import")
                            .small()
                            .primary()
                            .label(format!("Apply {ready} selected sessions"))
                            .disabled(ready == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_session_import(cx))),
                    )
                    .child(
                        dunlin_ui::button("cancel-session-import")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_session_import_preview(cx);
                                cx.notify();
                            })),
                    ),
            );
    }
    body.children(
        this.session_import
            .error
            .as_ref()
            .map(|e| div().text_size(px(12.)).text_color(p.red).child(e.clone())),
    )
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::harness::{ChatItem, TurnTiming};
    use dunlin_agent::{TokenUsage, ToolCallView};
    use dunlin_domain::{ToolKind, ToolStatus};

    fn tool(id: &str, additions: u32, deletions: u32) -> ChatItem {
        ChatItem::Tool {
            view: ToolCallView {
                id: id.into(),
                title: "edit".into(),
                kind: ToolKind::Edit,
                status: ToolStatus::Done,
                detail: "changed".into(),
                locations: Vec::new(),
                additions,
                deletions,
                diff: (additions > 0).then(|| "+line".into()),
            },
            duration_ms: 0,
            at_ms: 10,
        }
    }

    #[test]
    fn updating_an_import_adds_metrics_without_rewriting_local_text() {
        let timing = TurnTiming {
            started_at_ms: 10,
            finished_at_ms: Some(20),
            elapsed_ms: Some(10),
            usage: Some(TokenUsage {
                total: 120,
                input: 100,
                output: 20,
                reasoning: Some(0),
            }),
            usage_before: Some(TokenUsage::default()),
            speaker: None,
        };
        let mut existing = vec![
            ChatItem::User {
                text: "keep me".into(),
                timing: None,
                at_ms: 10,
            },
            tool("tool-1", 0, 0),
            ChatItem::User {
                text: "local follow-up".into(),
                timing: None,
                at_ms: 30,
            },
        ];
        let incoming = vec![
            ChatItem::User {
                text: "source copy".into(),
                timing: Some(timing),
                at_ms: 10,
            },
            tool("tool-1", 3, 2),
        ];
        assert!(super::merge_imported_metrics(&mut existing, &incoming));
        let ChatItem::User {
            text,
            timing: Some(copied),
            ..
        } = &existing[0]
        else {
            panic!("user timing was not copied");
        };
        assert_eq!(text, "keep me");
        assert_eq!(copied.usage.unwrap().total, 120);
        let ChatItem::Tool { view, .. } = &existing[1] else {
            panic!("tool view");
        };
        assert_eq!(
            (view.additions, view.deletions, view.diff.as_deref()),
            (3, 2, Some("+line"))
        );
        assert!(matches!(
            &existing[2],
            ChatItem::User { text, timing: None, .. } if text == "local follow-up"
        ));
        assert!(!super::merge_imported_metrics(&mut existing, &incoming));
    }

    #[test]
    fn source_identity_is_stable_scoped_to_harness_and_has_no_path_characters() {
        let one = super::imported_id("Claude Code", "source/id");
        assert_eq!(one, super::imported_id("Claude Code", "source/id"));
        assert_ne!(one, super::imported_id("OpenAI Codex", "source/id"));
        assert!(!one.contains('/'));
        assert_eq!(
            super::imported_source(&super::imported_id("OpenCode", "ses_123"), "OpenCode")
                .as_deref(),
            Some("ses_123")
        );
        assert!(super::imported_source(&one, "OpenCode").is_none());
    }
    #[test]
    fn session_scan_finds_nested_jsonl_and_ignores_other_files() {
        let root =
            std::env::temp_dir().join(format!("dunlin-session-scan-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(root.join("project-a")).unwrap();
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join("project-a/session.jsonl"), b"{}\n").unwrap();
        std::fs::write(root.join("project-a/notes.txt"), b"not a session").unwrap();
        std::fs::write(root.join(".hidden/hidden.jsonl"), b"{}\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&root, root.join("project-a/loop")).unwrap();

        let mut files = Vec::new();
        let mut warnings = Vec::new();
        super::collect_session_files(&root, "jsonl", &mut files, &mut warnings, &mut |_| {});
        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("project-a/session.jsonl"));
        assert!(warnings.is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_preview_reports_bad_inputs_without_touching_valid_sources() {
        let root =
            std::env::temp_dir().join(format!("dunlin-import-preview-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("session.json");
        let content = br#"{"format":"dunlin.session.v1","id":"preview","messages":[{"role":"user","content":"Hello"}]}"#;
        std::fs::write(&path, content).unwrap();
        let mut rows = Vec::new();
        let warnings = super::read_files(
            "Custom ACP",
            vec![path.clone(), root.join("missing")],
            &mut |candidate| {
                rows.push(candidate);
                Ok(())
            },
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(std::fs::read(path).unwrap(), content);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn project(path: &std::path::Path) -> dunlin_domain::Project {
        dunlin_domain::Project {
            id: "import-project".into(),
            name: "Import test".into(),
            path: path.to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: false,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        }
    }

    #[test]
    fn large_import_has_no_file_session_or_aggregate_byte_cap_and_reports_progress() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("sessions/a/b/c/d/e/f/g/h");
        std::fs::create_dir_all(&source).unwrap();
        let project = project(root.path());
        let padding = " ".repeat(120_000);
        for index in 0..600 {
            let content = serde_json::json!({
                "format": "dunlin.session.v1", "id": format!("source-{index}"),
                "directory": project.path, "title": format!("Session {index}"),
                "messages": [{"role":"user","content":"Preserve this conversation"}],
                "padding": padding,
            });
            std::fs::write(
                source.join(format!("{index}.jsonl")),
                serde_json::to_vec(&content).unwrap(),
            )
            .unwrap();
        }
        let mut files = Vec::new();
        let mut warnings = Vec::new();
        let mut discoveries = Vec::new();
        collect_session_files(
            &root.path().join("sessions"),
            "jsonl",
            &mut files,
            &mut warnings,
            &mut |progress| discoveries.push(progress.completed),
        );
        assert_eq!(files.len(), 600);
        assert_eq!(discoveries.last(), Some(&600));
        assert!(warnings.is_empty());
        let progress = Arc::new(Mutex::new(Progress::default()));
        let preview = prepare_preview(
            "Custom ACP".into(),
            Some(files),
            vec![project.clone()],
            progress.clone(),
        )
        .unwrap();
        assert_eq!(preview.rows.len(), 600);
        assert!(preview.warnings.is_empty());
        assert!(
            preview
                .rows
                .iter()
                .all(|row| row.entries == 1 && row.candidate.transcript.is_empty())
        );
        assert_eq!(
            progress.lock().unwrap().label(),
            "Reading session files · 600 / 600"
        );
        let staging = preview.staging.path().to_path_buf();
        let database = root.path().join("dunlin.db");
        let store = dunlin_storage::Store::open(&database).unwrap();
        store.upsert_project(&project).unwrap();
        let rows: Vec<_> = preview
            .rows
            .iter()
            .map(|row| {
                let mut summary = blank_session(&project.id, "Custom ACP", None);
                summary.id = imported_id("Custom ACP", &row.candidate.source_id);
                summary.title = row.candidate.title.clone();
                (summary, row.transcript.clone())
            })
            .collect();
        let applied = apply_histories(&database, &rows, &[], &progress).unwrap();
        assert_eq!(applied.sessions.len(), 600);
        assert_eq!(store.sessions(None).unwrap().len(), 600);
        assert_eq!(
            progress.lock().unwrap().label(),
            "Importing sessions · 600 / 600"
        );
        assert!(
            apply_histories(&database, &rows, &[], &progress)
                .unwrap()
                .sessions
                .is_empty()
        );
        let transcript = store
            .load_transcript::<Vec<ChatItem>>(&rows[599].0.id)
            .unwrap()
            .unwrap();
        assert!(
            matches!(&transcript[0], ChatItem::User { text, .. } if text == "Preserve this conversation")
        );
        assert!(std::fs::metadata(source.join("599.jsonl")).unwrap().len() > 120_000);
        drop(preview);
        assert!(!staging.exists(), "temporary histories must be cleaned up");
    }

    #[test]
    fn staged_read_failure_rolls_back_inserts_and_leaves_preview_available_for_retry() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let project = project(root.path());
        let database = root.path().join("dunlin.db");
        let store = dunlin_storage::Store::open(&database).unwrap();
        store.upsert_project(&project).unwrap();
        let transcript = root.path().join("history.json");
        std::fs::write(
            &transcript,
            serde_json::to_vec(&vec![ChatItem::User {
                text: "hello".into(),
                timing: None,
                at_ms: 0,
            }])
            .unwrap(),
        )
        .unwrap();
        let mut first = blank_session(&project.id, "Custom ACP", None);
        first.id = "import-first".into();
        let mut second = first.clone();
        second.id = "import-second".into();
        let rows = vec![
            (first, transcript.clone()),
            (second, root.path().join("missing.json")),
        ];
        let progress = Mutex::new(Progress::default());
        assert!(
            apply_histories(&database, &rows, &[], &progress)
                .err()
                .unwrap()
                .contains("rolled back")
        );
        assert!(store.sessions(None).unwrap().is_empty());
        assert!(transcript.exists());
    }
}
