//! All mutations originate in a fresh, explicit preflight and native confirmation.
//! Mutation ownership outlives a closed tab; canonical editor buffers are preserved.
use super::*;
use crate::remote::operations::Prepared;
use dunlin_workspace::git::{operations::Action, overview::Operation};
pub(super) struct Form {
    revision: Entity<InputState>,
    name: Entity<InputState>,
    message: Entity<InputState>,
    mainline: Entity<InputState>,
    pub(super) pending: Option<Prepared>,
    pub(super) error: Option<String>,
}
impl Form {
    pub(super) fn initialize_target(
        &mut self,
        oid: &str,
        window: &mut Window,
        cx: &mut Context<DunlinApp>,
    ) {
        if self.pending.is_none() && self.revision.read(cx).value().is_empty() {
            self.set_target(oid, window, cx);
        }
    }
    pub(super) fn set_target(
        &mut self,
        oid: &str,
        window: &mut Window,
        cx: &mut Context<DunlinApp>,
    ) {
        self.revision
            .update(cx, |state, cx| state.set_value(oid.to_owned(), window, cx));
        self.mainline
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.pending = None;
        self.error = None;
    }
    pub fn new(window: &mut Window, cx: &mut Context<DunlinApp>) -> Self {
        Self {
            revision: cx
                .new(|cx| InputState::new(window, cx).placeholder("Local commit / branch / tag")),
            name: cx.new(|cx| InputState::new(window, cx).placeholder("New worktree name")),
            message: cx.new(|cx| InputState::new(window, cx).placeholder("Optional stash message")),
            mainline: cx.new(|cx| {
                InputState::new(window, cx).placeholder("Merge parent (optional, 1-based)")
            }),
            pending: None,
            error: None,
        }
    }
}
impl DunlinApp {
    pub(crate) fn git_operation_guard(
        &self,
        project: &Project,
        action: &Action,
    ) -> Result<(), String> {
        if !self.projects.iter().any(|p| owns(project, p))
            || (self.project().id != project.id && !matches!(action, Action::Switch { .. }))
        {
            return Err("Project changed; reopen repository state".into());
        }
        if self.group_cleanup.busy
            || self.git_history.running
            || self.branch_ui.running()
            || self.projects.iter().any(|p| {
                self.git_mutation_in_flight(&p.id) || self.ai_commit_locks_project_id(&p.id)
            })
            || self.explorer.busy
            || self.git_loading
        {
            return Err("Wait for existing repository/file operations to finish".into());
        }
        if !self.live_agents().is_empty() {
            return Err("Finish connected agents before changing repository state (including linked worktrees/group contexts)".into());
        }
        if self
            .workbench
            .tabs
            .iter()
            .any(|t| t.dirty || t.loading || t.conflict.is_some() || t.git_changed)
            || self.editor_dirty
        {
            return Err("Save buffers and resolve file conflicts before Git mutations. Nothing is saved or discarded automatically.".into());
        }
        if matches!(action, Action::WorktreeRemove { .. }) && !self.workbench.tabs.is_empty() {
            return Err("Close all file tabs before removing a worktree; this also protects folder aliases and linked project views".into());
        }
        Ok(())
    }
    pub(super) fn prepare_repository_action(&mut self, action: Action, cx: &mut Context<Self>) {
        self.stash_editor_tab();
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self.git_history.view.as_ref() else {
            return;
        };
        let project = view.project.clone();
        let backend = view.backend.clone();
        let id = view.id;
        if !backend_current(&backend, &self.remote_hosts) {
            return;
        }
        if let Err(error) = self.git_operation_guard(&project, &action) {
            self.file_notice(error, cx);
            return;
        }
        if let Some(view) = &mut self.git_history.view {
            view.inspector.tab = details::Tab::Operations;
        }
        self.git_history.running = true;
        self.git_history.mutation_global = true;
        self.git_history.mutation_project = Some(project.id.clone());
        self.begin_git_mutation(&project.id);
        let form = &mut self.git_history.view.as_mut().unwrap().mutations;
        form.pending = None;
        form.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.prepare_git_operation(action) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running = false;
                this.git_history.mutation_global = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                this.git_history.prune(&this.projects, &this.remote_hosts);
                if let Some(view) = this.git_history.view.as_mut().filter(|v| v.id == id) {
                    match result {
                        Ok(plan) => view.mutations.pending = Some(plan),
                        Err(e) => view.mutations.error = Some(e),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn confirm_repository_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(view) = &self.git_history.view else {
            return;
        };
        let Some(prepared) = view.mutations.pending.clone() else {
            return;
        };
        let id = view.id;
        let project = view.project.clone();
        if let Err(e) = self.git_operation_guard(&project, &prepared.plan.action) {
            self.file_notice(e, cx);
            return;
        }
        let description = format!(
            "{}\nProject: {}\nRepository: {}\nTarget commit: {}\nWorktree: {}\nRequest: {}\n{}\n\nGit may run configured hooks, filters, merge drivers, signing tools and rebase todo commands. No application Undo. On error/conflict, inspect state before any new action. External Git processes must be stopped.",
            prepared.plan.action.label(),
            project.name,
            prepared.plan.root,
            prepared.plan.target.as_deref().unwrap_or("—"),
            prepared.plan.worktree_target.as_deref().unwrap_or("—"),
            serde_json::to_string(&prepared.plan.action).unwrap_or_default(),
            prepared.plan.warning
        );
        let receiver = window.prompt(
            PromptLevel::Warning,
            "Confirm Git mutation?",
            Some(&description),
            &["Cancel", "Run this operation"],
            cx,
        );
        let receiver = self.overlay_backdrop.track_native(receiver, cx);
        cx.spawn(async move |this, cx| {
            if receiver.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if !this.git_history.view.as_ref().is_some_and(|v| {
                    v.id == id
                        && v.mutations
                            .pending
                            .as_ref()
                            .is_some_and(|p| p.id == prepared.id)
                }) {
                    return;
                }
                this.execute_repository_action(id, project, prepared, cx);
            });
        })
        .detach();
    }
    pub(super) fn execute_repository_action(
        &mut self,
        id: uuid::Uuid,
        project: Project,
        prepared: Prepared,
        cx: &mut Context<Self>,
    ) {
        self.stash_editor_tab();
        if let Err(e) = self.git_operation_guard(&project, &prepared.plan.action) {
            self.file_notice(e, cx);
            return;
        }
        let Some(view) = self.git_history.view.as_mut().filter(|v| {
            v.id == id
                && backend_current(&v.backend, &self.remote_hosts)
                && v.mutations
                    .pending
                    .as_ref()
                    .is_some_and(|p| p.id == prepared.id)
        }) else {
            return;
        };
        let backend = view.backend.clone();
        view.mutations.pending = None;
        self.run_prepared_git_action(id, project, backend, prepared, cx);
    }
    pub(crate) fn run_prepared_git_action(
        &mut self,
        id: uuid::Uuid,
        project: Project,
        backend: WorkspaceBackend,
        prepared: Prepared,
        cx: &mut Context<Self>,
    ) {
        let switched_branch = match &prepared.plan.action {
            Action::Switch { branch, .. } => Some(branch.clone()),
            _ => None,
        };
        if switched_branch.is_some() {
            self.branch_ui.begin(id, project.id.clone());
        }
        let tabs = self
            .workbench
            .tabs
            .iter()
            .map(|t| {
                (
                    t.id.clone(),
                    t.project.clone(),
                    t.root.clone(),
                    t.backend.clone(),
                    t.inspector.path.clone(),
                    t.editor.read(cx).text().to_string(),
                )
            })
            .collect::<Vec<_>>();
        for tab in &mut self.workbench.tabs {
            tab.reload_generation = tab.reload_generation.wrapping_add(1);
        }
        self.git_history.running = true;
        self.git_history.mutation_global = true;
        self.git_history.mutation_project = Some(project.id.clone());
        self.begin_git_mutation(&project.id);
        cx.spawn(async move |this, cx| {
            let (result, files) = cx.background_executor().spawn(async move {
                let result = backend.execute_git_operation(&prepared);
                let mut files=Vec::new();
                let verification_started=std::time::Instant::now();
                for (id, owner, root, file_backend, path, old) in tabs {
                    let affected=match (&backend,&file_backend) {
                        (WorkspaceBackend::Local{..},WorkspaceBackend::Local{root})=>std::fs::canonicalize(root).ok().as_deref()==Some(std::path::Path::new(&prepared.plan.root)),
                        (WorkspaceBackend::Remote(a),WorkspaceBackend::Remote(b))=>a.host.url==b.host.url && a.host.device_id==b.host.device_id && a.project.project_id==b.project.project_id && a.project.path==b.project.path,
                        _=>false,
                    };
                    if affected {
                        let changed=verification_started.elapsed()>std::time::Duration::from_secs(10) || file_backend.read_file(&path).map_or(true,|f|f.contents!=old);
                        files.push((id,owner,root,path,old,changed));
                    }
                }
                (result,files)
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.running=false;
                this.git_history.mutation_global=false;
                this.git_history.mutation_project=None;
                this.finish_git_mutation(&project.id);
                this.branch_ui.finish(id);
                this.repo_summaries.invalidate();
                if result.as_ref().is_ok_and(|outcome| outcome.success) && let Some(branch) = switched_branch {
                    if let Some(current) = this.projects.iter_mut().find(|p| owns(&project, p)) {
                        current.branch = Some(branch);
                        let current = current.clone();
                        if let Some(store) = &this.store && let Err(error) = store.upsert_project(&current) {
                            this.report_storage_error("save branch selection", error);
                        }
                    }
                    this.repo_summaries.values.remove(&project.id);
                }
                for (id,owner,root,path,old,changed) in files {
                    if let Some(tab)=this.workbench.tabs.iter_mut().find(|t|t.id==id && t.project==owner && t.root==root && t.inspector.path==path) {
                        tab.blame.invalidate();tab.minimap.invalidate_baseline();tab.git_changed=changed;
                        if changed {
                            tab.conflict=Some(crate::app::EditorConflict {
                                path:tab.inspector.path.clone(),
                                message:if *tab.editor.read(cx).text()==old {"Git changed, removed or could not verify this file. The open buffer is preserved; reload explicitly."} else {"The buffer changed during Git. Its text is preserved; compare before saving."}.into(),
                            });
                        }
                    }
                }
                if this.projects.iter().any(|p|owns(&project,p)) && this.project().id==project.id {
                    this.diff_view = None;
                    this.refresh_file_tree(cx);
                    this.refresh_git_status(cx);
                    this.invalidate_project_icon();
                    this.invalidate_minimap_baselines();
                    if this.workbench.show_file && let Some(tab)=this.active_file_tab() {this.editor_conflict=tab.conflict.clone();}
                }
                match result {
                    Ok(value)=>this.file_notice(value.message,cx),
                    Err(e)=>this.file_notice(format!("Git stopped: {e} Refresh state; no automatic retry."),cx),
                }
                if this.git_history.view.as_ref().is_some_and(|v|v.id==id) {
                    this.load_git_overview(cx);
                    this.load_git_history(cx);
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
}

fn action_button(
    id: impl Into<ElementId>,
    label: &'static str,
    action: Action,
    disabled: bool,
    cx: &mut Context<DunlinApp>,
) -> Button {
    dunlin_ui::button(id)
        .ghost()
        .small()
        .label(label)
        .disabled(disabled)
        .on_click(
            cx.listener(move |this, _, _, cx| this.prepare_repository_action(action.clone(), cx)),
        )
}
pub(super) fn stash_actions(oid: &str, disabled: bool, cx: &mut Context<DunlinApp>) -> AnyElement {
    div()
        .flex()
        .gap(px(4.))
        .children([
            action_button(
                format!("stash-apply-{oid}"),
                "Apply…",
                Action::StashApply { oid: oid.into() },
                disabled,
                cx,
            ),
            action_button(
                format!("stash-pop-{oid}"),
                "Pop…",
                Action::StashPop { oid: oid.into() },
                disabled,
                cx,
            ),
            action_button(
                format!("stash-drop-{oid}"),
                "Drop…",
                Action::StashDrop { oid: oid.into() },
                disabled,
                cx,
            ),
        ])
        .into_any_element()
}
pub(super) fn worktree_action(path: &str, disabled: bool, cx: &mut Context<DunlinApp>) -> Button {
    action_button(
        format!("worktree-remove-{path}"),
        "Remove…",
        Action::WorktreeRemove { path: path.into() },
        disabled,
        cx,
    )
}
pub(super) fn operation_actions(
    operation: Operation,
    disabled: bool,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    div()
        .flex()
        .gap(px(4.))
        .children([
            action_button(
                format!("continue-{operation:?}"),
                "Continue…",
                Action::Continue { operation },
                disabled,
                cx,
            ),
            action_button(
                format!("abort-{operation:?}"),
                "Abort…",
                Action::Abort { operation },
                disabled,
                cx,
            ),
        ])
        .into_any_element()
}
pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let form = &view.mutations;
    let busy = this.git_history.running;
    let p = palette(cx);
    let revision = form.revision.read(cx).value().to_string();
    let name = form.name.read(cx).value().to_string();
    let message = form.message.read(cx).value().to_string();
    let mainline_text = form.mainline.read(cx).value();
    let mainline = if mainline_text.is_empty() {
        Ok(None)
    } else {
        mainline_text.parse::<usize>().map(Some)
    };
    div()
        .p(px(10.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .text_size(px(11.))
        .border_b_1()
        .border_color(p.border)
        .child("Git operations · every action prepares a fresh confirmation")
        .child(
            div()
                .flex()
                .gap(px(6.))
                .flex_wrap()
                .child(
                    compact_input(&form.message)
                        .aria_label("Stash message")
                        .min_w(px(140.))
                        .flex_1(),
                )
                .child(action_button(
                    "stash-tracked",
                    "Stash tracked…",
                    Action::StashPush {
                        message: message.clone(),
                        include_untracked: false,
                    },
                    busy,
                    cx,
                ))
                .child(action_button(
                    "stash-untracked",
                    "Stash + untracked…",
                    Action::StashPush {
                        message,
                        include_untracked: true,
                    },
                    busy,
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .flex_wrap()
                .child(
                    compact_input(&form.revision)
                        .aria_label("Target revision")
                        .min_w(px(150.))
                        .flex_1(),
                )
                .child(
                    compact_input(&form.mainline)
                        .aria_label("Merge parent number")
                        .min_w(px(150.))
                        .flex_1(),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(4.))
                .child(action_button(
                    "git-merge",
                    "Merge…",
                    Action::Merge {
                        revision: revision.clone(),
                    },
                    busy,
                    cx,
                ))
                .child(action_button(
                    "git-rebase",
                    "Rebase…",
                    Action::Rebase {
                        revision: revision.clone(),
                    },
                    busy,
                    cx,
                ))
                .child(action_button(
                    "git-cherry-pick",
                    "Cherry-pick…",
                    Action::CherryPick {
                        revision: revision.clone(),
                        mainline: mainline.as_ref().ok().copied().flatten(),
                    },
                    busy || mainline.is_err(),
                    cx,
                )),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .flex_wrap()
                .child(
                    compact_input(&form.name)
                        .aria_label("Worktree name")
                        .min_w(px(140.))
                        .flex_1(),
                )
                .child(action_button(
                    "git-worktree-add",
                    "Add worktree…",
                    Action::WorktreeAdd { name, revision },
                    busy,
                    cx,
                ))
                .child(action_button(
                    "git-worktree-prune",
                    "Prune stale…",
                    Action::WorktreePrune,
                    busy,
                    cx,
                )),
        )
        .when(busy, |r| {
            r.child(
                "Git is running; repository mutation locks remain held even if this tab closes.",
            )
        })
        .when_some(form.error.as_ref(), |r, e| {
            r.child(div().text_color(p.red).child(e.clone()))
        })
        .when_some(form.pending.as_ref(), |r, prepared| {
            r.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .p(px(6.))
                    .bg(p.muted)
                    .child(format!(
                        "{} · HEAD {} · target {}",
                        prepared.plan.action.label(),
                        prepared.plan.head.as_deref().unwrap_or("unborn"),
                        prepared.plan.target.as_deref().unwrap_or("—")
                    ))
                    .child(prepared.plan.warning.clone())
                    .child(format!(
                        "Request: {}",
                        serde_json::to_string(&prepared.plan.action).unwrap_or_default()
                    ))
                    .child(format!(
                        "Worktree: {} · changed status records: {} (first 80 below)",
                        prepared.plan.worktree_target.as_deref().unwrap_or("—"),
                        prepared.plan.status.split_terminator('\0').count()
                    ))
                    .child(
                        div()
                            .id("git-preflight-status")
                            .max_h(px(100.))
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .children(
                                prepared
                                    .plan
                                    .status
                                    .split_terminator('\0')
                                    .take(80)
                                    .map(|line| div().child(line.escape_debug().to_string())),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                dunlin_ui::button("git-operation-confirm")
                                    .small()
                                    .label("Review and confirm…")
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, w, cx| {
                                        this.confirm_repository_action(w, cx)
                                    })),
                            )
                            .child(
                                dunlin_ui::button("git-operation-cancel")
                                    .ghost()
                                    .small()
                                    .label("Cancel")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(v) = &mut this.git_history.view {
                                            v.mutations.pending = None;
                                        }
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
        })
        .into_any_element()
}
