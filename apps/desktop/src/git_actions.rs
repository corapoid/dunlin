//! Source-control and branch confirmations reuse the repository operation engine.
use crate::{
    app::DunlinApp,
    remote::{WorkspaceBackend, operations::Prepared},
};
use dunlin_domain::Project;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{Sizable, button::ButtonVariants},
    *,
};
use dunlin_workspace::git::operations::Action;

fn needs_confirmation(action: &Action, status: &str) -> bool {
    !matches!(action, Action::Switch { discard: false, .. }) || !status.is_empty()
}

pub(crate) struct Pending {
    id: uuid::Uuid,
    project: Project,
    backend: WorkspaceBackend,
    navigation: u64,
    prepared: Prepared,
    discard: Option<Prepared>,
    discard_error: Option<String>,
}

fn backend_current(backend: &WorkspaceBackend, app: &DunlinApp) -> bool {
    match backend {
        WorkspaceBackend::Local { .. } => true,
        WorkspaceBackend::Remote(remote) => app
            .remote_host(&remote.host.id)
            .is_some_and(|h| h.url == remote.host.url && h.device_id == remote.host.device_id),
    }
}

impl DunlinApp {
    pub(crate) fn request_git_action(
        &mut self,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.project().clone();
        let backend = match self.backend() {
            Ok(b) => b,
            Err(e) => {
                self.file_notice(e, cx);
                return;
            }
        };
        self.prepare_git_action(project, backend, action, window, cx);
    }
    pub(crate) fn prepare_git_action(
        &mut self,
        project: Project,
        backend: WorkspaceBackend,
        action: Action,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stash_editor_tab();
        if self.pending_git_action.is_some() {
            return;
        }
        if let Err(e) = self.git_operation_guard(&project, &action) {
            self.file_notice(e, cx);
            return;
        }
        let navigation = self.navigation_epoch();
        let id = uuid::Uuid::now_v7();
        self.git_history.running = true;
        self.git_history.mutation_global = true;
        self.git_history.mutation_project = Some(project.id.clone());
        self.begin_git_mutation(&project.id);
        let worker = backend.clone();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let prepared = worker.prepare_git_operation(action)?;
                let mut discard_error = None;
                let discard = if !prepared.plan.status.is_empty() && let Action::Switch { branch, discard: false } = &prepared.plan.action {
                    match worker.prepare_git_operation(Action::Switch { branch: branch.clone(), discard: true }) {
                        Ok(other) => {
                            if other.plan.fingerprint != prepared.plan.fingerprint || other.plan.root != prepared.plan.root || other.plan.target != prepared.plan.target {
                                return Err("Repository changed during preparation; reopen confirmation".into());
                            }
                            Some(other)
                        }
                        Err(e) => { discard_error = Some(e); None }
                    }
                } else { None };
                Ok::<_, String>((prepared, discard, discard_error))
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.git_history.running = false;
                this.git_history.mutation_global = false;
                this.git_history.mutation_project = None;
                this.finish_git_mutation(&project.id);
                if this.navigation_epoch() != navigation || !backend_current(&backend, this) { cx.notify(); return; }
                match result {
                    Ok((prepared, discard, discard_error)) => {
                        if let Err(e) = this.git_operation_guard(&project, &prepared.plan.action) { this.file_notice(e, cx); return; }
                        if !needs_confirmation(&prepared.plan.action, &prepared.plan.status) {
                            this.run_prepared_git_action(id, project, backend, prepared, cx);
                            return;
                        }
                        this.pending_git_action = Some(Pending { id, project, backend, navigation, prepared, discard, discard_error });
                        this.focus_handle.focus(window, cx);
                    }
                    Err(e) => this.file_notice(e, cx),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    pub(crate) fn cancel_git_action(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pending_git_action = None;
        self.focus_handle.focus(window, cx);
        cx.notify();
    }
    fn confirm_git_action(
        &mut self,
        id: uuid::Uuid,
        discard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.pending_git_action.as_ref().is_some_and(|p| p.id == id) {
            return;
        }
        let pending = self.pending_git_action.take().unwrap();
        let prepared = if discard {
            let Some(plan) = pending.discard else {
                return;
            };
            plan
        } else {
            pending.prepared
        };
        self.stash_editor_tab();
        if self.navigation_epoch() != pending.navigation || !backend_current(&pending.backend, self)
        {
            self.file_notice("Project or connection changed; reopen confirmation", cx);
            return;
        }
        if let Err(e) = self.git_operation_guard(&pending.project, &prepared.plan.action) {
            self.file_notice(e, cx);
            return;
        }
        self.focus_handle.focus(window, cx);
        self.run_prepared_git_action(pending.id, pending.project, pending.backend, prepared, cx);
    }
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_confirm_git_action(
        &mut self,
        discard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = self
            .pending_git_action
            .as_ref()
            .expect("prepared Git confirmation");
        if discard {
            assert!(pending.discard.is_some());
        }
        self.confirm_git_action(pending.id, discard, window, cx);
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let pending = this.pending_git_action.as_ref().unwrap();
    let p = palette(cx);
    let id = pending.id;
    let plan = &pending.prepared.plan;
    let description = match &plan.action {
        Action::Switch { branch, .. } => format!(
            "{} → {branch}",
            plan.overview
                .worktrees
                .iter()
                .find(|w| w.current)
                .and_then(|w| w.branch.as_deref())
                .unwrap_or("HEAD")
                .trim_start_matches("refs/heads/")
        ),
        Action::Discard { paths } => format!(
            "{} file(s): {}",
            paths.len(),
            paths.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
        ),
        Action::Ignore { path } => path.clone(),
        _ => String::new(),
    };
    let destructive = matches!(plan.action, Action::Discard { .. });
    let mut buttons = div().flex().flex_wrap().justify_end().gap(px(7.)).child(
        dunlin_ui::button("git-action-cancel")
            .small()
            .label("Cancel")
            .on_click(cx.listener(|this, _, window, cx| this.cancel_git_action(window, cx))),
    );
    if let Some(discard) = &pending.discard {
        buttons = buttons.child(
            dunlin_ui::button("git-action-discard-switch")
                .small()
                .danger()
                .label(discard.plan.action.label())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.confirm_git_action(id, true, window, cx)
                })),
        );
    }
    buttons = buttons.child(
        dunlin_ui::button("git-action-confirm")
            .small()
            .when(destructive, |b| b.danger())
            .label(plan.action.label())
            .on_click(cx.listener(move |this, _, window, cx| {
                this.confirm_git_action(id, false, window, cx)
            })),
    );
    div().id("git-action-overlay").absolute().inset_0().size_full().occlude().flex().items_center().justify_center()
        .on_click(cx.listener(|this, _, window, cx| this.cancel_git_action(window, cx)))
        .child(SurfaceGeometry::DIALOG.paint(p.card, p.border_strong).id("git-action-dialog").role(Role::Dialog).tab_group().aria_label("Confirm Git operation")
            .w(px(560.)).max_w_full()
            .p(px(18.)).flex().flex_col().gap(px(12.)).on_click(|_, _, cx| cx.stop_propagation())
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!("{} · {}", plan.action.label(), pending.project.name)))
            .child(div().text_size(px(12.)).child(description))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child(format!("Repository: {}", plan.root)))
            .child(div().text_size(px(12.)).text_color(p.muted_fg).child(plan.warning.clone()))
            .when(pending.discard.is_some(), |panel| panel.child(div().text_size(px(12.)).text_color(p.red).child("Discard and switch permanently deletes all uncommitted changes, including untracked files. No Undo or automatic stash.")))
            .when_some(pending.discard_error.as_ref(), |panel, error| panel.child(div().text_size(px(12.)).text_color(p.red).child(format!("Discard and switch unavailable: {error}"))))
            .child(buttons)).into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_clean_non_destructive_branch_switches_skip_confirmation() {
        let switch = Action::Switch {
            branch: "main".into(),
            discard: false,
        };
        assert!(!needs_confirmation(&switch, ""));
        assert!(needs_confirmation(&switch, " M file\0"));
        assert!(needs_confirmation(&switch, "?? file\0"));
        assert!(needs_confirmation(
            &Action::Switch {
                branch: "main".into(),
                discard: true
            },
            ""
        ));
        assert!(needs_confirmation(&Action::WorktreePrune, ""));
    }
}
