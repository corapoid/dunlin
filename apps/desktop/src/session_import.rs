//! Preview-first session import. No harness commands; OpenCode's local database
//! is opened read-only and only session/message/part rows are read. Re-scanning
//! can enrich an imported copy with missing token and diff metadata.
mod adapters;
mod opencode_db;
use crate::app::{DunlinApp, blank_session};
use crate::harness::ChatItem;
use adapters::Candidate;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dunlin_ui::component::Disableable;
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        menu::{DropdownMenu, PopupMenuItem},
    },
    *,
};
use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
    io::Read,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Default)]
pub(crate) struct State {
    pub loading: bool,
    pub scanning: Option<String>,
    preview: Option<Preview>,
    error: Option<String>,
    generation: u64,
    backfill_started: bool,
}
struct Preview {
    harness: String,
    rows: Vec<Row>,
    warnings: Vec<String>,
}
struct Row {
    candidate: Candidate,
    project: Option<String>,
    duplicate: bool,
    update: bool,
    selected: bool,
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
fn read_files(harness: &str, paths: Vec<PathBuf>) -> (Vec<Candidate>, Vec<String>) {
    let mut rows = Vec::new();
    let mut warnings = Vec::new();
    let mut total = 0;
    for path in paths {
        let result = (|| {
            let file = std::fs::File::open(&path).map_err(|e| e.to_string())?;
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Choose a regular session file".into());
            }
            let mut bytes = Vec::new();
            file.take(adapters::MAX_FILE + 1)
                .read_to_end(&mut bytes)
                .map_err(|e| e.to_string())?;
            total += bytes.len();
            if total > 64 * 1024 * 1024 {
                return Err("Preview exceeds 64 MiB; choose fewer files".into());
            }
            adapters::parse(harness, &bytes)
        })();
        match result {
            Ok(candidates) => {
                if rows.len() + candidates.len() > adapters::MAX_SESSIONS {
                    warnings.push(
                        "Preview is limited to 256 sessions; choose fewer files for the rest"
                            .into(),
                    );
                    break;
                }
                rows.extend(candidates);
            }
            Err(error) => warnings.push(format!(
                "{}: {error}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
        }
        if total > 64 * 1024 * 1024 {
            break;
        }
    }
    (rows, warnings)
}
fn read_harness_sessions(harness: &str) -> (Vec<Candidate>, Vec<String>) {
    if harness == "OpenCode" {
        let Some(path) = dunlin_usage::billing::default_opencode_db() else {
            return (
                Vec::new(),
                vec!["No opencode.db was found in the standard OpenCode data folders".into()],
            );
        };
        return match opencode_db::load(&path) {
            Ok(result) => result,
            Err(error) => (Vec::new(), vec![error]),
        };
    }
    let (paths, mut warnings) = discover_session_files(harness);
    if paths.is_empty() {
        return (Vec::new(), warnings);
    }
    let (candidates, read_warnings) = read_files(harness, paths);
    warnings.extend(read_warnings);
    (candidates, warnings)
}

fn discover_session_files(harness: &str) -> (Vec<PathBuf>, Vec<String>) {
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
    collect_session_files(&root, extension, 0, &mut files, &mut warnings);
    files.sort_by_key(|path| Reverse(modified_ms(path)));
    if files.len() > 256 {
        files.truncate(256);
        warnings.push("Showing the 256 most recently modified session files".into());
    }
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
    depth: usize,
    files: &mut Vec<PathBuf>,
    warnings: &mut Vec<String>,
) {
    if depth > 6 || files.len() >= 512 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        warnings.push(format!("Could not read {}", dir.display()));
        return;
    };
    for entry in read.flatten() {
        if files.len() >= 512 {
            warnings.push("Session scan limited to 512 files".into());
            break;
        }
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
            collect_session_files(&path, extension, depth + 1, files, warnings);
        } else if file_type.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some(extension)
        {
            files.push(path);
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

fn finalize_import(
    this: &mut DunlinApp,
    harness: String,
    rows: Vec<Row>,
    warnings: Vec<String>,
    cx: &mut Context<DunlinApp>,
) {
    this.session_import.loading = false;
    this.session_import.scanning = None;
    let found = rows.len();
    let detail = warnings
        .first()
        .cloned()
        .unwrap_or_else(|| "No session files matched this harness".into());
    let mut seen = HashSet::new();
    let rows = rows
        .into_iter()
        .map(|mut row| {
            let id = imported_id(&harness, &row.candidate.source_id);
            let existing = this.all_sessions.iter().find(|session| session.id == id);
            row.update = existing.is_some_and(|session| session.id.starts_with("import-"));
            row.duplicate = !seen.insert(id.clone())
                || existing.is_some()
                || this.all_sessions.iter().any(|session| {
                    session.harness == harness
                        && session.acp_session_id.as_deref()
                            == Some(row.candidate.source_id.as_str())
                });
            if row.duplicate && !row.update {
                row.selected = false;
            }
            if !this
                .projects
                .iter()
                .any(|p| Some(&p.id) == row.project.as_ref() && !p.is_remote())
            {
                row.project = None;
            }
            row
        })
        .collect();
    this.session_import.preview = Some(Preview {
        harness,
        rows,
        warnings,
    });
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
                    let ids: HashSet<_> =
                        sources.iter().map(|(_, source)| source.clone()).collect();
                    let (candidates, _) = opencode_db::load_selected(&opencode, &ids)?;
                    let candidates: HashMap<_, _> = candidates
                        .into_iter()
                        .map(|candidate| (candidate.source_id.clone(), candidate))
                        .collect();
                    let store =
                        dunlin_storage::Store::open(&database).map_err(|e| e.to_string())?;
                    let mut updated = Vec::new();
                    for (session_id, source_id) in sources {
                        if skip.as_ref().is_some_and(|skip| skip == &session_id) {
                            continue;
                        }
                        let Some(candidate) = candidates.get(&source_id) else {
                            continue;
                        };
                        let Ok(Some(mut transcript)) =
                            store.load_transcript::<Vec<ChatItem>>(&session_id)
                        else {
                            continue;
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
                            continue;
                        }
                        crate::chat_presentation::normalize_legacy(&mut transcript);
                        if merge_imported_metrics(&mut transcript, &candidate.transcript)
                            && store.save_transcript(&session_id, &transcript).is_ok()
                        {
                            updated.push(session_id);
                        }
                    }
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
        self.session_import.preview = None;
        self.file_notice(format!("Scanning {harness} session folders…"), cx);
        cx.notify();
        let source = harness.clone();
        cx.spawn(async move |this, cx| {
            let projects = this
                .read_with(cx, |this, _| this.projects.clone())
                .unwrap_or_default();
            let (rows, warnings) = cx
                .background_executor()
                .spawn(async move {
                    let (candidates, warnings) = read_harness_sessions(&source);
                    let rows = candidates
                        .into_iter()
                        .map(|candidate| {
                            let project =
                                adapters::match_project(candidate.directory.as_deref(), &projects);
                            Row {
                                candidate,
                                project,
                                duplicate: false,
                                update: false,
                                selected: true,
                            }
                        })
                        .collect::<Vec<_>>();
                    (rows, warnings)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_import.generation == generation {
                    finalize_import(this, harness, rows, warnings, cx);
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
        self.session_import.preview = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                let _ = this.update(cx, |this, cx| {
                    if this.session_import.generation == generation {
                        this.session_import.loading = false;
                        this.session_import.scanning = None;
                        cx.notify();
                    }
                });
                return;
            };
            let projects = this
                .read_with(cx, |this, _| this.projects.clone())
                .unwrap_or_default();
            let source = harness.clone();
            let (rows, warnings) = cx
                .background_executor()
                .spawn(async move {
                    let (candidates, warnings) = read_files(&source, paths);
                    let rows = candidates
                        .into_iter()
                        .map(|candidate| {
                            let project =
                                adapters::match_project(candidate.directory.as_deref(), &projects);
                            Row {
                                candidate,
                                project,
                                duplicate: false,
                                update: false,
                                selected: true,
                            }
                        })
                        .collect::<Vec<_>>();
                    (rows, warnings)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_import.generation == generation {
                    finalize_import(this, harness, rows, warnings, cx);
                }
            });
        })
        .detach();
    }
    fn apply_session_import(&mut self, cx: &mut Context<Self>) {
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
                    row.candidate.transcript.clone(),
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
            rows.push((summary, row.candidate.transcript.clone()));
        }
        let mut updated = 0usize;
        let mut update_error = None;
        let mut prepared = Vec::new();
        {
            let Some(store) = &self.store else {
                self.session_import.error =
                    Some("History storage unavailable; nothing imported".into());
                cx.notify();
                return;
            };
            for (id, incoming) in updates {
                match store.load_transcript::<Vec<ChatItem>>(&id) {
                    Ok(Some(mut transcript)) => {
                        crate::chat_presentation::normalize_legacy(&mut transcript);
                        if !merge_imported_metrics(&mut transcript, &incoming) {
                            continue;
                        }
                        match store.save_transcript(&id, &transcript) {
                            Ok(()) => prepared.push((id, transcript)),
                            Err(error) => {
                                update_error = Some(format!("Could not update {id}: {error}"));
                            }
                        }
                    }
                    Ok(None) => {}
                    Err(error) => update_error = Some(format!("Could not update {id}: {error}")),
                }
            }
        }
        for (id, transcript) in prepared {
            self.session_list.record(&id, &transcript);
            if self.session().id == id {
                self.harness.transcript = transcript;
                self.chat_visible_items = crate::app::CHAT_WINDOW;
            } else {
                self.forget_transcript(&id);
            }
            updated += 1;
        }
        let Some(store) = &self.store else {
            self.session_import.error =
                Some("History storage unavailable; nothing imported".into());
            cx.notify();
            return;
        };
        match store.import_sessions(&rows) {
            Ok(ids) => {
                for (summary, transcript) in rows {
                    if !ids.contains(&summary.id) {
                        continue;
                    }
                    self.session_list.record(&summary.id, &transcript);
                    if summary.project_id == self.project().id {
                        self.sessions.push(summary.clone());
                    }
                    self.all_sessions.push(summary);
                }
                self.session_import.preview = None;
                self.session_import.error = update_error;
                let message = match (ids.len(), updated) {
                    (0, 0) => "No new sessions to import".into(),
                    (0, updated) => format!("Updated {updated} imported session histories"),
                    (imported, 0) => format!(
                        "Imported {imported} session histories; existing conversations unchanged"
                    ),
                    (imported, updated) => {
                        format!("Imported {imported} and updated {updated} session histories")
                    }
                };
                self.file_notice(message, cx);
                // Imported histories must feed both the recorded dashboard
                // counts and the local billing evidence without a manual scan.
                self.refresh_dashboard_now(cx);
                self.refresh_billing(cx);
            }
            Err(error) => {
                self.session_import.error = Some(format!("Import rolled back: {error}"));
                cx.notify();
            }
        }
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
                timings.insert(*at_ms, *timing);
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
                    *timing = Some(*found);
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
            .scanning
            .as_deref()
            .map(|harness| format!("Scanning {harness} session folders…"))
            .unwrap_or_else(|| "Scanning session files…".into());
        body = body.child(label);
    }
    if let Some(preview) = &this.session_import.preview {
        body = body.child(format!("Import preview · {} · {} sessions", preview.harness, preview.rows.len()))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Copies histories only. Source files stay unchanged. Assign unmatched sessions or leave them skipped. Updating an imported copy adds recorded token and diff data without replacing your local follow-up turns."));
        let mut list = div()
            .id("session-import-preview")
            .max_h(px(280.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(5.));
        for (i, row) in preview.rows.iter().enumerate() {
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
            let choose = Button::new(format!("import-project-{i}"))
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
                            .on_click(move |_, _, cx| {
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
                            }),
                        );
                    }
                    menu.scrollable(true).max_h(px(280.))
                });
            list = list.child(
                div()
                    .p(px(7.))
                    .rounded(px(6.))
                    .border_1()
                    .border_color(p.border)
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Button::new(format!("import-select-{i}"))
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
                                        row.candidate.transcript.len()
                                    )),
                            ),
                    )
                    .child(choose),
            );
        }
        let ready = preview
            .rows
            .iter()
            .filter(|r| {
                r.selected && (!r.duplicate || r.update) && (r.project.is_some() || r.update)
            })
            .count();
        body = body
            .child(list)
            .children(preview.warnings.iter().map(|w| {
                div()
                    .text_size(px(11.))
                    .text_color(p.amber)
                    .child(w.clone())
            }))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        Button::new("confirm-session-import")
                            .small()
                            .primary()
                            .label(format!("Apply {ready} selected sessions"))
                            .disabled(ready == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.apply_session_import(cx))),
                    )
                    .child(
                        Button::new("cancel-session-import")
                            .small()
                            .label("Cancel")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.session_import.preview = None;
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

        let mut files = Vec::new();
        let mut warnings = Vec::new();
        super::collect_session_files(&root, "jsonl", 0, &mut files, &mut warnings);
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
        let (rows, warnings) =
            super::read_files("Custom ACP", vec![path.clone(), root.join("missing")]);
        assert_eq!(rows.len(), 1);
        assert_eq!(warnings.len(), 1);
        assert_eq!(std::fs::read(path).unwrap(), content);
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
