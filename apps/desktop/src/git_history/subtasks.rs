//! One explicit worktree/branch per agent subtask. This is a bounded task-board
//! v1: it never decomposes work, starts agents, merges or pushes automatically.
use super::*;
use dunlin_workspace::git::operations::{Action, subtask_branch};
use dunlin_workspace::subtask::SubtaskWorktree;
use std::path::{Path, PathBuf};

pub(super) struct State {
    pub visible: bool,
    task: Entity<InputState>,
    task_id: Option<String>,
    name: Entity<InputState>,
    revision: Entity<InputState>,
    busy: bool,
    error: Option<String>,
}

impl State {
    pub fn new(window: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            visible: false,
            task: cx
                .new(|cx| InputState::new(window, cx).placeholder("Subtask / task-board label")),
            task_id: None,
            name: cx.new(|cx| InputState::new(window, cx).placeholder("review-api")),
            revision: cx.new(|cx| InputState::new(window, cx).default_value("HEAD")),
            busy: false,
            error: None,
        }
    }
}

impl DunlinApp {
    pub(crate) fn open_subtask_worktrees(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_git_history(window, cx);
        if !self.git_history.is_open_for(self.project()) {
            return;
        }
        let view = self.git_history.view.as_mut().unwrap();
        view.hide_tools();
        view.subtasks.visible = true;
        cx.notify();
    }

    pub(crate) fn prefill_subtask_worktree(
        &mut self,
        task: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_subtask_worktrees(window, cx);
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        view.subtasks.task.update(cx, |state, cx| {
            state.set_value(task.to_string(), window, cx)
        });
        view.subtasks.name.update(cx, |state, cx| {
            state.set_value(name.to_string(), window, cx)
        });
        view.subtasks.revision.update(cx, |state, cx| {
            state.set_value("HEAD".to_string(), window, cx)
        });
        cx.notify();
    }

    pub(crate) fn create_task_worktree(
        &mut self,
        task: &str,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prefill_subtask_worktree(task, name, window, cx);
        self.create_subtask_worktree(window, cx);
    }

    fn create_subtask_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        let task = view.subtasks.task.read(cx).value().trim().to_string();
        let name = view.subtasks.name.read(cx).value().trim().to_string();
        let revision = view.subtasks.revision.read(cx).value().trim().to_string();
        let task_id = view.subtasks.task_id.clone().unwrap_or_default();
        if task.is_empty() || task.len() > 256 || name.is_empty() {
            self.file_notice("Enter a subtask label and a simple worktree name", cx);
            return;
        }
        if revision.is_empty() {
            self.file_notice("Choose an explicit commit, branch or tag", cx);
            return;
        }
        let backend = view.backend.clone();
        let project = view.project.clone();
        let view_id = view.id;
        let action = Action::SubtaskAdd {
            name: name.clone(),
            revision,
        };
        let prepared = match backend.prepare_git_operation(action) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.file_notice(format!("Subtask worktree preflight refused: {error}"), cx);
                return;
            }
        };
        let path = prepared
            .plan
            .worktree_target
            .clone()
            .unwrap_or_else(|| "unavailable".into());
        let branch = subtask_branch(&name);
        let base = prepared.plan.target.clone().unwrap_or_default();
        let description = format!(
            "Task: {task}\nBranch: {branch}\nWorktree: {path}\nBase commit: {base}\n\nCreates one local branch and one worktree. It starts no agent, merges nothing, pushes nothing and is not a security sandbox. A new approval is required for every subtask."
        );
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Create a worktree for this subtask?",
            Some(&description),
            &["Cancel", "Create worktree"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        if let Some(view) = self.git_history.view.as_mut() {
            view.subtasks.busy = true;
            view.subtasks.error = None;
        }
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    if let Some(view) = this.git_history.view.as_mut().filter(|v| v.id == view_id) {
                        view.subtasks.busy = false;
                    }
                    cx.notify();
                });
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move {
                    let outcome = backend.execute_git_operation(&prepared)?;
                    if !outcome.success {
                        return Err(outcome.message);
                    }
                    Ok::<_, String>((
                        prepared.plan.root.clone(),
                        prepared.plan.target.clone().unwrap_or_default(),
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                let current =
                    this.git_history.view.as_ref().is_some_and(|view| {
                        view.id == view_id && owns(&view.project, this.project())
                    });
                if !current {
                    return;
                }
                if let Some(view) = this.git_history.view.as_mut() {
                    view.subtasks.busy = false;
                }
                match result {
                    Ok((root, base)) => {
                        let entry = SubtaskWorktree {
                            id: uuid::Uuid::now_v7().to_string(),
                            project_id: project.id.clone(),
                            root,
                            task: task.clone(),
                            task_id: task_id.clone(),
                            name: name.clone(),
                            branch: branch.clone(),
                            path: path.clone(),
                            base,
                            created_ms: crate::harness::now_ms(),
                        };
                        let mut next = this.subtask_worktrees.registry.clone();
                        let result = next
                            .add(entry)
                            .and_then(|_| this.persist_subtask_worktrees(next));
                        match result {
                            Ok(()) => this.file_notice(
                                format!("Created subtask worktree {branch}; no agent was started"),
                                cx,
                            ),
                            Err(error) => {
                                if let Some(view) = this.git_history.view.as_mut() {
                                    view.subtasks.error = Some(error);
                                }
                            }
                        }
                    }
                    Err(error) => {
                        if let Some(view) = this.git_history.view.as_mut() {
                            view.subtasks.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn remove_subtask_worktree(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self
            .subtask_worktrees
            .registry
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        let Some(view) = &self.git_history.view else {
            return;
        };
        if view.project.is_remote() {
            self.file_notice(
                "Removing a relay subtask worktree requires a fresh relay preflight from its host; this v1 keeps removal local",
                cx,
            );
            return;
        }
        let backend = view.backend.clone();
        let view_id = view.id;
        let prepared = match backend.prepare_git_operation(Action::WorktreeRemove {
            path: entry.path.clone(),
        }) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.file_notice(format!("Subtask worktree removal refused: {error}"), cx);
                return;
            }
        };
        let description = format!(
            "Remove the clean worktree {}?\n\nThe branch {} and its commits are retained. Unsaved buffers in the worktree must be closed first. This action has no application Undo.",
            entry.path, entry.branch
        );
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Remove this subtask worktree?",
            Some(&description),
            &["Cancel", "Remove worktree"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        if let Some(view) = self.git_history.view.as_mut() {
            view.subtasks.busy = true;
            view.subtasks.error = None;
        }
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    if let Some(view) = this.git_history.view.as_mut().filter(|v| v.id == view_id) {
                        view.subtasks.busy = false;
                    }
                    cx.notify();
                });
                return;
            }
            let result = cx
                .background_executor()
                .spawn(async move { backend.execute_git_operation(&prepared) })
                .await;
            let _ = this.update(cx, |this, cx| {
                let current =
                    this.git_history.view.as_ref().is_some_and(|view| {
                        view.id == view_id && owns(&view.project, this.project())
                    });
                if !current {
                    return;
                }
                if let Some(view) = this.git_history.view.as_mut() {
                    view.subtasks.busy = false;
                }
                match result {
                    Ok(outcome) if outcome.success => {
                        let mut next = this.subtask_worktrees.registry.clone();
                        next.remove(&id);
                        match this.persist_subtask_worktrees(next) {
                            Ok(()) => this.file_notice(
                                "Removed the subtask worktree; its branch was retained",
                                cx,
                            ),
                            Err(error) => {
                                if let Some(view) = this.git_history.view.as_mut() {
                                    view.subtasks.error = Some(error);
                                }
                            }
                        }
                    }
                    Ok(outcome) => {
                        if let Some(view) = this.git_history.view.as_mut() {
                            view.subtasks.error = Some(outcome.message);
                        }
                    }
                    Err(error) => {
                        if let Some(view) = this.git_history.view.as_mut() {
                            view.subtasks.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_subtask_project(&mut self, id: String, cx: &mut Context<Self>) {
        let Some(entry) = self
            .subtask_worktrees
            .registry
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .cloned()
        else {
            return;
        };
        if self.project().is_remote() {
            self.file_notice(
                "Opening a relay worktree as a local project is not supported by this v1",
                cx,
            );
            return;
        }
        if !Path::new(&entry.path).is_dir() {
            self.file_notice(
                "The subtask worktree no longer exists; prune it from Git",
                cx,
            );
            return;
        }
        self.add_project(PathBuf::from(entry.path), cx);
    }
}

pub(super) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(view) = this.git_history.view.as_ref() else {
        return div().into_any_element();
    };
    let s = &view.subtasks;
    let mut body = div()
        .id("subtask-worktrees")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(12.))
        .child(
            div()
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("One worktree per subtask"),
        )
        .child(
            div()
                .text_size(px(11.))
                .line_height(relative(1.4))
                .text_color(p.muted_fg)
                .child("Creates an isolated local branch and worktree from an explicit commit. This is a task-board v1: no automatic decomposition, agent start, merge, push or sandbox. Every creation and removal is a separate two-phase Git approval."),
        )
        .child(Input::new(&s.task).small())
        .child(Input::new(&s.name).small())
        .child(Input::new(&s.revision).small())
        .child(
            Button::new("subtask-create")
                .small()
                .primary()
                .label(if s.busy {
                    "Working…"
                } else {
                    "Review and create worktree…"
                })
                .disabled(s.busy || this.git_history.running)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.create_subtask_worktree(window, cx)
                })),
        );
    if let Some(error) = &this.subtask_worktrees.error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    if let Some(error) = &s.error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    let entries = this
        .subtask_worktrees
        .registry
        .for_project(&view.project.id)
        .cloned()
        .collect::<Vec<_>>();
    if entries.is_empty() {
        body = body.child(
            div()
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("No subtask worktrees recorded for this project."),
        );
    }
    for entry in entries {
        let remove = entry.id.clone();
        let open = entry.id.clone();
        let exists = Path::new(&entry.path).is_dir();
        body = body.child(
            div()
                .rounded(px(8.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .p(px(9.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(12.))
                                .font_weight(FontWeight::MEDIUM)
                                .child(entry.task.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(9.5))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(if exists { p.green } else { p.amber })
                                .child(if exists {
                                    "worktree present"
                                } else {
                                    "missing"
                                }),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!("{} · base {}", entry.branch, entry.base)),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(entry.path.clone()),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(
                            Button::new(format!("subtask-open-{open}"))
                                .ghost()
                                .small()
                                .label("Open as project")
                                .disabled(!exists || view.project.is_remote())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_subtask_project(open.clone(), cx)
                                })),
                        )
                        .child(
                            Button::new(format!("subtask-remove-{remove}"))
                                .ghost()
                                .small()
                                .label("Remove worktree…")
                                .disabled(s.busy || this.git_history.running)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.remove_subtask_worktree(remove.clone(), window, cx)
                                })),
                        ),
                ),
        );
    }
    body.into_any_element()
}
