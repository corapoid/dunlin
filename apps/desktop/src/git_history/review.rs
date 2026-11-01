//! Explicit baselines, observed changes and selective restore; no inferred authors.
use super::*;
use dunlin_workspace::task_review::{self as api, Change, Checkpoint, Restore};
use sha2::{Digest, Sha256};
use similar::TextDiff;

pub(super) struct State {
    pub visible: bool,
    checkpoints: Vec<Checkpoint>,
    selected: Option<String>,
    changes: Vec<Change>,
    diff: Option<(String, String)>,
    pending: Option<(uuid::Uuid, Restore)>,
    loading: bool,
    request: uuid::Uuid,
    error: Option<String>,
    label: Entity<InputState>,
    comment: Entity<InputState>,
    line: Entity<InputState>,
    scroll: ScrollHandle,
}
impl State {
    pub fn new(window: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            checkpoints: Vec::new(),
            selected: None,
            changes: Vec::new(),
            diff: None,
            pending: None,
            loading: false,
            request: uuid::Uuid::now_v7(),
            error: None,
            label: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Checkpoint label (capture before task)")
            }),
            comment: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Review comment for selected file")
            }),
            line: cx.new(|cx| InputState::new(window, cx).placeholder("Line number (optional)")),
            scroll: ScrollHandle::new(),
        }
    }
}
#[cfg(debug_assertions)]
pub(super) fn smoke_begin(app: &mut DunlinApp, w: &mut Window, cx: &mut Context<DunlinApp>) {
    app.open_task_review(w, cx);
    app.git_history
        .view
        .as_ref()
        .unwrap()
        .review
        .label
        .update(cx, |s, cx| s.set_value("Before fixture task", w, cx));
    app.capture_checkpoint(cx);
    assert!(app.git_history.running);
}
#[cfg(debug_assertions)]
pub(super) fn smoke_edit(app: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> bool {
    let s = &app.git_history.view.as_ref().unwrap().review;
    if s.loading {
        return false;
    }
    assert_eq!(s.checkpoints.len(), 1, "{:?}", s.error);
    let id = s.checkpoints[0].id.clone();
    std::fs::write(
        std::path::Path::new(&app.project().path).join("example.rs"),
        "fn after_fixture_task() {}\n// unchanged\n",
    )
    .unwrap();
    app.review_changes(id, cx);
    true
}
#[cfg(debug_assertions)]
pub(super) fn smoke_diff(app: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> bool {
    let s = &app.git_history.view.as_ref().unwrap().review;
    if s.loading {
        return false;
    }
    assert_eq!(s.changes.len(), 1, "{:?}", s.error);
    app.review_file("example.rs".into(), false, cx);
    true
}
#[cfg(debug_assertions)]
pub(super) fn smoke_finish(app: &DunlinApp, _cx: &App) -> bool {
    let s = &app.git_history.view.as_ref().unwrap().review;
    if s.loading {
        return false;
    }
    let (_, patch) = s.diff.as_ref().unwrap();
    assert!(
        patch.contains("changed_on_disk"),
        "preexisting user text must be the baseline"
    );
    assert!(patch.contains("after_fixture_task"));
    true
}
fn key(p: &Project) -> String {
    format!(
        "task-checkpoints:{:x}",
        Sha256::digest(format!("{}\0{}\0{:?}", p.id, p.path, p.remote).as_bytes())
    )
}
fn diff(c: &Change) -> String {
    let before = c.before.as_ref().and_then(|i| i.text());
    let after = c.after.as_ref().and_then(|i| i.text());
    if (c.before.is_some() && before.is_none()) || (c.after.is_some() && after.is_none()) {
        return "Binary/non-UTF-8 content changed; no text diff. Restore retains exact bytes."
            .into();
    }
    let d = TextDiff::configure()
        .timeout(std::time::Duration::from_millis(100))
        .diff_lines(
            before.as_deref().unwrap_or(""),
            after.as_deref().unwrap_or(""),
        );
    dunlin_workspace::github::display_text(
        &d.unified_diff()
            .context_radius(3)
            .header("checkpoint", "current saved file")
            .to_string(),
        80_000,
    )
}
impl DunlinApp {
    pub(crate) fn open_task_review(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(w, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let loaded = self
            .store
            .as_ref()
            .ok_or("Storage unavailable".to_string())
            .and_then(|s| {
                s.load_setting::<Vec<Checkpoint>>(&key(self.project()))
                    .map_err(|_| "Could not load checkpoints".into())
            })
            .map(|v| v.unwrap_or_default());
        let v = self.git_history.view.as_mut().unwrap();
        v.hide_tools();
        v.context.visible = false;
        v.github.visible = false;
        v.review.visible = true;
        v.checks.visible = false;
        match loaded {
            Ok(points) => v.review.checkpoints = points,
            Err(e) => v.review.error = Some(e),
        }
        cx.notify();
    }
    fn task_guard(&self, restore: bool) -> Result<(), String> {
        let v = self.git_history.view.as_ref().ok_or("Reopen task review")?;
        if !owns(&v.project, self.project()) || v.project.is_remote() {
            return Err("Checkpoints require the same trusted local project. Relay capability is not implemented; no local fallback.".into());
        }
        if self.git_history.running
            || self.branch_ui.running()
            || self.explorer.busy
            || !self.live_agents().is_empty()
            || self.projects.iter().any(|p| {
                self.git_mutation_in_flight(&p.id) || self.ai_commit_locks_project_id(&p.id)
            })
        {
            return Err(
                "Stop agents and finish repository/file operations before capture/restore".into(),
            );
        }
        if self.editor_dirty
            || self
                .workbench
                .tabs
                .iter()
                .any(|t| t.dirty || t.loading || t.git_changed || t.conflict.is_some())
        {
            return Err(
                "Save buffers and resolve file reviews first. Checkpoints cover saved files only."
                    .into(),
            );
        }
        if restore && !self.workbench.tabs.is_empty() {
            return Err("Close all file tabs before restore; no editor text/undo is discarded automatically.".into());
        }
        Ok(())
    }
    fn capture_checkpoint(&mut self, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        if let Err(e) = self.task_guard(false) {
            self.file_notice(e, cx);
            return;
        }
        let v = self.git_history.view.as_mut().unwrap();
        if v.review.loading {
            return;
        }
        if v.review.checkpoints.len() >= 3 {
            self.file_notice("Three-baseline retention limit reached. Explicitly delete an obsolete checkpoint first.",cx);
            return;
        }
        let label = v.review.label.read(cx).value().to_string();
        let project = v.project.clone();
        let root = project.path.clone();
        let id = v.id;
        v.review.loading = true;
        self.git_history.running = true;
        self.git_history.mutation_global = true;
        self.git_history.mutation_project = Some(project.id.clone());
        self.begin_git_mutation(&project.id);
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api::capture(&root, &label) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running = false;
                this.git_history.mutation_global = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                let Some(v) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|v| v.id == id && this.projects.iter().any(|p| owns(&project, p)))
                else {
                    return;
                };
                v.review.loading = false;
                match result {
                    Ok(point) => {
                        v.review.selected = Some(point.id.clone());
                        v.review.checkpoints.push(point);
                        match this
                            .store
                            .as_ref()
                            .ok_or("Storage unavailable")
                            .and_then(|s| {
                                s.save_setting(&key(&project), &v.review.checkpoints)
                                    .map_err(|_| "Checkpoint persistence failed")
                            }) {
                            Ok(()) => v.review.error = None,
                            Err(e) => v.review.error = Some(e.into()),
                        }
                    }
                    Err(e) => v.review.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn review_changes(&mut self, checkpoint_id: String, cx: &mut Context<Self>) {
        let Some(v) = &mut self.git_history.view else {
            return;
        };
        if !v.project.trusted || v.project.is_remote() || v.review.loading {
            return;
        }
        let Some(point) = v
            .review
            .checkpoints
            .iter()
            .find(|c| c.id == checkpoint_id)
            .cloned()
        else {
            return;
        };
        let id = v.id;
        let request = uuid::Uuid::now_v7();
        v.review.request = request;
        v.review.selected = Some(checkpoint_id);
        v.review.loading = true;
        v.review.diff = None;
        v.review.changes.clear();
        v.review.pending = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { api::changes(&point) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(v) = this.git_history.view.as_mut().filter(|v| {
                    v.id == id
                        && v.review.request == request
                        && this.projects.iter().any(|p| owns(&v.project, p))
                }) else {
                    return;
                };
                v.review.loading = false;
                match result {
                    Ok(changes) => {
                        v.review.changes = changes;
                        v.review.error = None;
                    }
                    Err(e) => v.review.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn review_file(&mut self, name: String, restore: bool, cx: &mut Context<Self>) {
        if restore {
            self.stash_editor_tab();
            if let Err(e) = self.task_guard(true) {
                self.file_notice(e, cx);
                return;
            }
        }
        let Some(v) = &mut self.git_history.view else {
            return;
        };
        if v.review.loading {
            return;
        }
        let Some(point) = v
            .review
            .checkpoints
            .iter()
            .find(|c| Some(&c.id) == v.review.selected.as_ref())
            .cloned()
        else {
            return;
        };
        let id = v.id;
        let request = uuid::Uuid::now_v7();
        v.review.request = request;
        v.review.loading = true;
        v.review.pending = None;
        v.review.diff = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    api::prepare_restore(&point, &name).map(|plan| {
                        let text = diff(&plan.change);
                        (plan, text)
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let Some(v) = this.git_history.view.as_mut().filter(|v| {
                    v.id == id
                        && v.review.request == request
                        && this.projects.iter().any(|p| owns(&v.project, p))
                }) else {
                    return;
                };
                v.review.loading = false;
                match result {
                    Ok((plan, text)) => {
                        v.review.diff = Some((plan.change.path.clone(), text));
                        if restore {
                            v.review.pending = Some((uuid::Uuid::now_v7(), plan));
                        }
                    }
                    Err(e) => v.review.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn restore_checkpoint_file(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        if let Err(e) = self.task_guard(true) {
            self.file_notice(e, cx);
            return;
        }
        let v = self.git_history.view.as_ref().unwrap();
        let Some((token, plan)) = &v.review.pending else {
            return;
        };
        let token = *token;
        let id = v.id;
        let message = format!(
            "{} ← {}\n{}\nThis replaces later saved changes to this ONE file, without restoring Git index/refs. Stop external editors/Git first: cross-process changes are not transactional.",
            plan.change.path,
            plan.checkpoint.label,
            if plan.change.before.is_none() {
                "The file was absent: this explicitly DELETES that one file."
            } else {
                "Restores checkpoint contents and permissions."
            }
        );
        let receiver = w.prompt(
            PromptLevel::Warning,
            "Restore selected checkpoint file?",
            Some(&message),
            &["Cancel", "Restore this file"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this.task_guard(true).is_err() {
                    return;
                }
                let Some(v) = this.git_history.view.as_mut().filter(|v| {
                    v.id == id && v.review.pending.as_ref().is_some_and(|(t, _)| *t == token)
                }) else {
                    return;
                };
                let (_, plan) = v.review.pending.take().unwrap();
                let project = v.project.clone();
                this.git_history.running = true;
                this.git_history.mutation_global = true;
                this.git_history.mutation_project = Some(project.id.clone());
                this.begin_git_mutation(&project.id);
                cx.spawn(async move |this, cx| {
                    let result = cx
                        .background_executor()
                        .spawn(async move { api::restore(plan) })
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.git_history.running = false;
                        this.git_history.mutation_global = false;
                        this.git_history.mutation_project = None;
                        this.finish_git_mutation(&project.id);
                        match result {
                            Ok(()) => this.file_notice(
                                "Selected file restored; Git index/refs unchanged",
                                cx,
                            ),
                            Err(e) => this.file_notice(e, cx),
                        }
                        this.refresh_git_status(cx);
                        this.refresh_file_tree(cx);
                        if let Some(v) = &mut this.git_history.view
                            && v.id == id
                        {
                            v.review.changes.clear();
                            v.review.diff = None;
                        }
                        cx.notify();
                    });
                })
                .detach();
                cx.notify();
            });
        })
        .detach();
    }
    fn review_to_draft(&mut self, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        if !owns(&v.project, self.project()) {
            return;
        }
        let Some((path, patch)) = &v.review.diff else {
            return;
        };
        let comment = v.review.comment.read(cx).value();
        if comment.trim().is_empty() {
            return;
        }
        let evidence = format!(
            "Task review {}:{}\nUser comment: {}\nObserved changes (author unknown; may include user/external edits):\n{}",
            path,
            v.review.line.read(cx).value(),
            comment,
            patch
        );
        let old = self.composer.read(cx).value();
        if old.len() + evidence.len() > 100_000 {
            self.file_notice("Review exceeds composer limit; trim it first", cx);
            return;
        }
        self.show_chat_tab(w, cx);
        self.composer.update(cx, |s, cx| {
            s.set_value(format!("{old}\n\n{evidence}"), w, cx)
        });
        self.file_notice("Review appended to draft; no agent started", cx);
    }
    fn delete_checkpoint(&mut self, target: String, w: &mut Window, cx: &mut Context<Self>) {
        let Some(v) = &self.git_history.view else {
            return;
        };
        let id = v.id;
        let receiver=w.prompt(PromptLevel::Warning,"Delete local baseline?",Some("Project files and Git objects are not deleted. The local checkpoint will no longer be available for restoration."),&["Cancel","Delete baseline"],cx);
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                let Some(v) = this.git_history.view.as_mut().filter(|v| v.id == id) else {
                    return;
                };
                let keep = v
                    .review
                    .checkpoints
                    .iter()
                    .filter(|c| c.id != target)
                    .cloned()
                    .collect::<Vec<_>>();
                if let Some(store) = &this.store {
                    match store.save_setting(&key(&v.project), &keep) {
                        Ok(()) => {
                            v.review.checkpoints = keep;
                            v.review.pending = None;
                            v.review.changes.clear();
                            v.review.diff = None;
                        }
                        Err(_) => {
                            v.review.error = Some(
                                "Could not persist checkpoint deletion; baseline retained".into(),
                            )
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let v = this.git_history.view.as_ref().unwrap();
    let s = &v.review;
    let p = palette(cx);
    let busy = s.loading || this.git_history.running;
    let mut body=div().p(px(12.)).flex().flex_col().gap(px(8.)).text_size(px(12.))
        .child("Capture BEFORE a task. Review is an observed diff, not agent authorship. Unsaved buffers, ignored files and Git index/refs are NOT backed up. Baseline HEAD objects must remain available. Local snapshots may contain secrets; never published automatically.")
        .child(Input::new(&s.label).small()).child(Button::new("capture-task-checkpoint").small().label("Capture saved-file baseline").disabled(busy).on_click(cx.listener(|this,_,_,cx|this.capture_checkpoint(cx))));
    for point in &s.checkpoints {
        let id = point.id.clone();
        let delete = id.clone();
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(format!(
                    "{} · {} · {}",
                    point.label, point.created_ms, point.head
                ))
                .child(
                    Button::new(format!("review-checkpoint-{id}"))
                        .ghost()
                        .small()
                        .label("Review changes")
                        .disabled(busy)
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.review_changes(id.clone(), cx)),
                        ),
                )
                .child(
                    Button::new(format!("delete-checkpoint-{delete}"))
                        .ghost()
                        .small()
                        .label("Delete baseline…")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, w, cx| {
                            this.delete_checkpoint(delete.clone(), w, cx)
                        })),
                ),
        );
    }
    body = body.child(format!(
        "{} changed files · verification unknown until independently tested",
        s.changes.len()
    ));
    for change in s.changes.iter().take(100) {
        let name = change.path.clone();
        let restore = name.clone();
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .child(
                    Button::new(format!("task-diff-{name}"))
                        .ghost()
                        .small()
                        .label(name.clone())
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review_file(name.clone(), false, cx)
                        })),
                )
                .child(
                    Button::new(format!("task-restore-{restore}"))
                        .ghost()
                        .small()
                        .label("Preview restore…")
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.review_file(restore.clone(), true, cx)
                        })),
                ),
        );
    }
    if s.changes.len() > 100 {
        body = body.child("Only the first 100 files are displayed; use Git for larger reviews.");
    }
    if let Some((name, patch)) = &s.diff {
        body = body
            .child(name.clone())
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .child(patch.clone()),
            )
            .child(Input::new(&s.line).small())
            .child(Input::new(&s.comment).small())
            .child(
                Button::new("task-review-comment")
                    .ghost()
                    .small()
                    .label("Add file/line review to agent draft")
                    .on_click(cx.listener(|this, _, w, cx| this.review_to_draft(w, cx))),
            );
    }
    if s.pending.is_some() {
        body = body.child(
            Button::new("task-restore-confirm")
                .small()
                .label("Confirm this one-file restore…")
                .disabled(busy)
                .on_click(cx.listener(|this, _, w, cx| this.restore_checkpoint_file(w, cx))),
        );
    }
    if let Some(e) = &s.error {
        body = body.child(div().text_color(p.red).child(e.clone()));
    }
    div()
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .bg(p.bg)
        .text_color(p.fg)
        .child(
            div()
                .flex_shrink_0()
                .p(px(10.))
                .flex()
                .gap(px(6.))
                .child("Task review & checkpoints")
                .child(
                    Button::new("task-review-back")
                        .ghost()
                        .small()
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(v) = &mut this.git_history.view {
                                v.review.visible = false;
                            }
                            cx.notify();
                        })),
                ),
        )
        .child(
            div()
                .id("task-review-scroll")
                .track_scroll(&s.scroll)
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .child(body),
        )
        .into_any_element()
}
