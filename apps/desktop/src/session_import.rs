//! Preview-first, insert-only import. No harness commands or database reads.
mod adapters;
use crate::app::{DunlinApp, blank_session};
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
use std::{collections::HashSet, io::Read, path::PathBuf};

#[derive(Default)]
pub(crate) struct State {
    pub loading: bool,
    preview: Option<Preview>,
    error: Option<String>,
    generation: u64,
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
    selected: bool,
}
fn imported_id(harness: &str, source: &str) -> String {
    format!(
        "import-{}-{}",
        URL_SAFE_NO_PAD.encode(harness),
        URL_SAFE_NO_PAD.encode(source)
    )
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
impl DunlinApp {
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
        self.session_import.error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = receiver.await else {
                let _ = this.update(cx, |this, cx| {
                    if this.session_import.generation == generation {
                        this.session_import.loading = false;
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
                                selected: true,
                            }
                        })
                        .collect::<Vec<_>>();
                    (rows, warnings)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.session_import.generation != generation {
                    return;
                }
                this.session_import.loading = false;
                let mut seen = HashSet::new();
                let rows = rows
                    .into_iter()
                    .map(|mut row| {
                        let id = imported_id(&harness, &row.candidate.source_id);
                        row.duplicate = !seen.insert(id.clone())
                            || this.all_sessions.iter().any(|s| {
                                s.id == id
                                    || (s.harness == harness
                                        && s.acp_session_id.as_deref()
                                            == Some(row.candidate.source_id.as_str()))
                            });
                        if row.duplicate {
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
                cx.notify();
            });
        })
        .detach();
    }
    fn apply_session_import(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = &self.session_import.preview else {
            return;
        };
        let mut rows = Vec::new();
        for row in &preview.rows {
            if !row.selected || row.duplicate {
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
                self.session_import.error = None;
                self.file_notice(
                    format!(
                        "Imported {} session histories; existing conversations unchanged",
                        ids.len()
                    ),
                    cx,
                );
            }
            Err(error) => {
                self.session_import.error = Some(format!("Import rolled back: {error}"));
                cx.notify();
            }
        }
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = div().flex().flex_col().gap(px(8.));
    if this.session_import.loading {
        body = body.child("Reading selected session files…");
    }
    if let Some(preview) = &this.session_import.preview {
        body = body.child(format!("Import preview · {} · {} sessions", preview.harness, preview.rows.len()))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Copies histories only. Source files stay unchanged. Assign unmatched sessions or leave them skipped. Tool records become inert text."));
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
            let label = if row.duplicate {
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
                            .disabled(row.duplicate)
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
            .filter(|r| r.selected && !r.duplicate && r.project.is_some())
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
                            .label(format!("Import {ready} assigned sessions"))
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
    #[test]
    fn source_identity_is_stable_scoped_to_harness_and_has_no_path_characters() {
        let one = super::imported_id("Claude Code", "source/id");
        assert_eq!(one, super::imported_id("Claude Code", "source/id"));
        assert_ne!(one, super::imported_id("OpenAI Codex", "source/id"));
        assert!(!one.contains('/'));
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
