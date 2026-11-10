//! Automatic session import. No harness commands; OpenCode's local database
//! is opened read-only and only session/message/part rows are read. Re-scanning
//! can enrich an imported copy with missing token and diff metadata.
mod adapters;
mod opencode_db;
mod opencode_files;
#[cfg(debug_assertions)]
pub(crate) mod smoke;
use crate::app::{DunlinApp, blank_session};
use crate::harness::ChatItem;
use adapters::Candidate;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dunlin_ui::*;
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    io::{BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};

pub(crate) fn supports_scan(harness: &str) -> bool {
    matches!(harness, "OpenCode" | "Claude Code" | "OpenAI Codex")
}

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
    pub(crate) pending: Vec<dunlin_domain::SessionSummary>,
    pub(crate) notices: HashMap<String, String>,
    pub(crate) last_import: HashMap<String, i64>,
}
impl State {
    pub(crate) fn load(store: Option<&dunlin_storage::Store>) -> Self {
        Self {
            last_import: store
                .and_then(|store| store.load_setting("session_import_times").ok().flatten())
                .unwrap_or_default(),
            ..Default::default()
        }
    }

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
    #[cfg(test)]
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
        #[cfg(test)]
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
            #[cfg(test)]
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
        let mut warnings = Vec::new();
        let mut seen = HashSet::new();
        let mut unique = |row: Candidate| {
            if seen.insert(row.source_id.clone()) {
                candidate(row)?;
            }
            Ok(())
        };
        let mut found = false;
        for root in opencode_files::roots() {
            for path in [root.join("opencode.db"), root.join("storage/opencode.db")] {
                if !path.is_file() {
                    continue;
                }
                found = true;
                // Source read errors must not prevent importing other locations.
                // Visitor/staging errors still abort rather than silently losing data.
                let mut visitor_error = None;
                let result = opencode_db::visit(
                    &path,
                    None,
                    &mut |row| {
                        unique(row)
                            .inspect_err(|error: &String| visitor_error = Some(error.clone()))
                    },
                    &mut |completed, total| {
                        progress(Progress {
                            phase: "Reading OpenCode sessions",
                            completed,
                            total: Some(total),
                        })
                    },
                );
                if let Some(error) = visitor_error {
                    return Err(error);
                }
                match result {
                    Ok(notes) => warnings.extend(notes),
                    Err(error) => warnings.push(error),
                }
            }
            let storage = root.join("storage");
            if storage.join("session").is_dir() {
                found = true;
                warnings.extend(opencode_files::visit(&storage, &mut unique, progress)?);
            }
        }
        if !found {
            warnings.push("No OpenCode database or legacy session storage found".into());
        }
        return Ok(warnings);
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
                    "Automatic session import is not supported for {harness}"
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
    let harness = this.session_import.scanning.take();
    this.session_import.progress = None;
    let mut preview = match result {
        Ok(preview) => preview,
        Err(error) => {
            if let Some(harness) = harness {
                this.session_import.notices.insert(harness, error.clone());
            }
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
        if row.update && row.project.is_none() {
            row.project = this
                .all_sessions
                .iter()
                .find(|session| session.id == id)
                .map(|session| session.project_id.clone());
        }
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
    for warning in &preview.warnings {
        crate::logging::warn(&format!("{} import: {warning}", preview.harness));
    }
    let warnings = preview
        .warnings
        .first()
        .map(|warning| format!("{} warning(s) · {warning}", preview.warnings.len()))
        .unwrap_or_default();
    this.session_import
        .notices
        .insert(preview.harness.clone(), warnings);
    this.session_import.preview = Some(preview);
    if found == 0 {
        this.file_notice(format!("No sessions found: {detail}"), cx);
    }
    this.apply_session_import(cx);
    cx.notify();
}

impl DunlinApp {
    /// Reconcile durable unassigned copies and older imported locations when
    /// projects are added. Never transfer an active runtime to another root.
    pub(crate) fn assign_imported_projects(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else {
            return;
        };
        let pending = match store.pending_session_imports() {
            Ok(pending) => pending,
            Err(error) => {
                self.file_notice(format!("Could not read pending imports: {error}"), cx);
                return;
            }
        };
        self.session_import.pending.clear();
        for mut session in pending {
            let Some(project) =
                adapters::match_project(session.workspace.as_deref(), &self.projects)
            else {
                self.session_import.pending.push(session);
                continue;
            };
            session.project_id = project;
            match store.assign_pending_session_import(&session) {
                Ok(()) => {
                    if !self.all_sessions.iter().any(|known| known.id == session.id) {
                        if session.project_id == self.project().id {
                            self.sessions.push(session.clone());
                        }
                        self.all_sessions.push(session);
                    }
                }
                Err(error) => {
                    crate::logging::warn(&format!("Could not assign imported session: {error}"));
                    self.session_import.pending.push(session);
                }
            }
        }
        let locations: Vec<_> = self
            .all_sessions
            .iter()
            .filter(|session| imported_source(&session.id, &session.harness).is_some())
            .filter_map(|session| {
                let project =
                    adapters::match_project(session.workspace.as_deref(), &self.projects)?;
                (project != session.project_id)
                    .then(|| (session.id.clone(), project, session.workspace.clone()))
            })
            .collect();
        for (id, project, directory) in locations {
            if let Err(error) = self.assign_imported_location(&id, &project, directory.as_deref()) {
                crate::logging::warn(&error);
            }
        }
        cx.notify();
    }

    fn assign_imported_location(
        &mut self,
        id: &str,
        project_id: &str,
        directory: Option<&str>,
    ) -> Result<bool, String> {
        let Some(session) = self.all_sessions.iter().find(|session| session.id == id) else {
            return Ok(false);
        };
        if imported_source(id, &session.harness).is_none()
            || !self
                .projects
                .iter()
                .any(|project| project.id == project_id && !project.is_remote())
        {
            return Ok(false);
        }
        let moved = session.project_id != project_id;
        let workspace = directory
            .map(str::to_owned)
            .or_else(|| session.workspace.clone());
        if !moved && workspace == session.workspace {
            return Ok(false);
        }
        // A live adapter must never inherit another project's permissions or root.
        if moved && (self.session().id == id || self.parked_runtimes.contains_key(id)) {
            return Err(format!(
                "Could not reassign {}: close its agent and switch to another session, then import again",
                session.title
            ));
        }
        self.store
            .as_ref()
            .ok_or("History storage unavailable")?
            .update_session_location(id, project_id, directory)
            .map_err(|error| error.to_string())?;
        let session = self
            .all_sessions
            .iter_mut()
            .find(|session| session.id == id)
            .unwrap();
        session.project_id = project_id.into();
        session.workspace = workspace;
        let summary = session.clone();
        let current = self.session().id.clone();
        let current_project = self.project().id.clone();
        self.sessions
            .retain(|session| session.id != id || project_id == current_project);
        if project_id == current_project {
            if let Some(session) = self.sessions.iter_mut().find(|session| session.id == id) {
                session.project_id = summary.project_id;
                session.workspace = summary.workspace;
            } else {
                self.sessions.push(summary);
            }
        }
        self.selected_session = self
            .sessions
            .iter()
            .position(|session| session.id == current)
            .unwrap_or(0);
        Ok(true)
    }

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
        if self.session_import.loading || !supports_scan(harness) {
            return;
        }
        let harness = harness.to_string();
        self.session_import.generation = self.session_import.generation.wrapping_add(1);
        let generation = self.session_import.generation;
        self.session_import.loading = true;
        self.session_import.scanning = Some(harness.clone());
        self.session_import.error = None;
        self.session_import.notices.remove(&harness);
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

    fn apply_session_import(&mut self, cx: &mut Context<Self>) {
        if self.session_import.loading {
            return;
        }
        let Some(preview) = &self.session_import.preview else {
            return;
        };
        let mut rows = Vec::new();
        let mut updates = Vec::new();
        let mut locations = Vec::new();
        for row in &preview.rows {
            if !row.selected {
                continue;
            }
            if row.update {
                if let Some(project) = &row.project {
                    locations.push((
                        imported_id(&preview.harness, &row.candidate.source_id),
                        project.clone(),
                        row.candidate.directory.clone(),
                    ));
                }
                updates.push((
                    imported_id(&preview.harness, &row.candidate.source_id),
                    row.transcript.clone(),
                ));
                continue;
            }
            if row.duplicate {
                continue;
            }
            let project =
                adapters::match_project(row.candidate.directory.as_deref(), &self.projects);
            let mut summary =
                blank_session(project.as_deref().unwrap_or(""), &preview.harness, None);
            summary.id = imported_id(&preview.harness, &row.candidate.source_id);
            summary.title = row.candidate.title.clone();
            if summary.title == "New session" {
                summary.title = "Imported conversation".into();
            }
            summary.model = row.candidate.model.clone();
            summary.workspace = row.candidate.directory.clone();
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
            self.session_import.notices.insert(
                preview.harness.clone(),
                "History storage unavailable; nothing imported".into(),
            );
            self.session_import.error =
                Some("History storage unavailable; nothing imported".into());
            cx.notify();
            return;
        };
        let staging = preview.staging.clone();
        let harness = preview.harness.clone();
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
            Ok(mut applied) => {
                for (id, project, directory) in locations {
                    match this.assign_imported_location(&id, &project, directory.as_deref()) {
                        Ok(true) => {
                            if !applied.updated.contains(&id) { applied.updated.push(id); }
                        }
                        Ok(false) => {}
                        Err(error) => applied.error = Some(error),
                    }
                }
                let imported = applied.sessions.len();
                 for summary in applied.sessions {
                     if summary.project_id.is_empty() {
                         this.session_import.pending.retain(|known| known.id != summary.id);
                         this.session_import.pending.push(summary);
                         continue;
                     }
                     this.session_import.pending.retain(|known| known.id != summary.id);
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
                 if this.session_import.error.is_none() {
                     this.session_import.last_import.insert(harness.clone(), chrono::Utc::now().timestamp_millis());
                     if let Some(store) = &this.store
                         && let Err(error) = store.save_setting("session_import_times", &this.session_import.last_import) {
                         crate::logging::warn(&format!("Could not save last import time: {error}"));
                     }
                 }
                if let Some(error) = &this.session_import.error {
                    this.session_import.notices.entry(harness.clone()).or_default().push_str(&format!(" · {error}"));
                }
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
                 this.assign_imported_projects(cx);
                // Imported histories must feed both the recorded dashboard
                // counts and the local billing evidence without a manual scan.
                this.refresh_dashboard_now(cx);
                this.refresh_billing(cx);
            }
             Err(error) => {
                 this.session_import.notices.insert(harness, error.clone());
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

#[cfg(test)]
mod tests {
    use crate::harness::{ChatItem, TurnTiming};
    use dunlin_agent::{TokenUsage, ToolCallView};
    use dunlin_domain::{ToolKind, ToolStatus};

    #[test]
    fn last_import_times_survive_restart_and_legacy_data_starts_unknown() {
        let store = dunlin_storage::Store::open_in_memory().unwrap();
        assert!(super::State::load(Some(&store)).last_import.is_empty());
        let times = std::collections::HashMap::from([("Claude Code".to_string(), 123456789_i64)]);
        store.save_setting("session_import_times", &times).unwrap();
        assert_eq!(super::State::load(Some(&store)).last_import, times);
    }

    #[test]
    fn session_import_only_offers_supported_automatic_sources() {
        for name in crate::harness_discovery::catalog_names().chain(["Custom ACP"]) {
            assert_eq!(
                super::supports_scan(name),
                matches!(name, "OpenCode" | "Claude Code" | "OpenAI Codex")
            );
        }
    }

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
                file_changes: Vec::new(),
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
            failure: None,
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
    fn unmatched_histories_survive_restart_and_are_promoted_when_a_project_is_added() {
        use super::*;
        let root = tempfile::tempdir().unwrap();
        let database = root.path().join("dunlin.db");
        let transcript = root.path().join("transcript.json");
        std::fs::write(
            &transcript,
            serde_json::to_vec(&vec![ChatItem::User {
                text: "Imported before the project exists".into(),
                timing: None,
                at_ms: 0,
            }])
            .unwrap(),
        )
        .unwrap();
        let mut summary = blank_session("", "OpenCode", None);
        summary.id = imported_id("OpenCode", "unmatched");
        summary.workspace = Some(root.path().to_string_lossy().into());
        let progress = Mutex::new(Progress::default());
        apply_histories(
            &database,
            &[(summary.clone(), transcript.clone())],
            &[],
            &progress,
        )
        .unwrap();
        // Reimport replaces only the unopened staged copy, never creates duplicates.
        apply_histories(&database, &[(summary.clone(), transcript)], &[], &progress).unwrap();
        let store = dunlin_storage::Store::open(&database).unwrap();
        assert!(store.sessions(None).unwrap().is_empty());
        let pending = store.pending_session_imports().unwrap();
        assert_eq!(pending.len(), 1);
        let project = project(root.path());
        store.upsert_project(&project).unwrap();
        summary.project_id =
            adapters::match_project(pending[0].workspace.as_deref(), &[project]).unwrap();
        store.assign_pending_session_import(&summary).unwrap();
        assert!(store.pending_session_imports().unwrap().is_empty());
        assert_eq!(store.sessions(None).unwrap().len(), 1);
        let history = store
            .load_transcript::<Vec<ChatItem>>(&summary.id)
            .unwrap()
            .unwrap();
        assert!(
            matches!(&history[0], ChatItem::User { text, .. } if text == "Imported before the project exists")
        );
        store.clear_sessions().unwrap();
        assert!(store.sessions(None).unwrap().is_empty());
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
