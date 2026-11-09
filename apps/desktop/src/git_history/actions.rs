//! Graph actions pin the view and commit, never a stale virtual row index.
use super::*;
use crate::effects::menu_item;
use dunlin_ui::component::menu::PopupMenu;
use dunlin_workspace::git::operations::Action;

#[derive(Clone, Copy)]
enum Command {
    Branch,
    Merge,
    Rebase,
    Pick,
    Worktree,
    Operations,
}

impl DunlinApp {
    fn history_command(
        &mut self,
        id: uuid::Uuid,
        oid: &str,
        command: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self
            .git_history
            .view
            .as_ref()
            .filter(|v| v.id == id && owns(&v.project, self.project()))
        else {
            return;
        };
        let Some(index) = view.rows.iter().position(|c| c.oid == oid) else {
            self.file_notice(
                "This commit is no longer in the displayed snapshot. Refresh history.",
                cx,
            );
            return;
        };
        if self.git_history.running || view.loading {
            return;
        }
        let merge = view.rows[index].parents.len() > 1;
        if view.selected != Some(index) {
            self.select_history_commit(index, 0, cx);
        }
        let view = self.git_history.view.as_mut().unwrap();
        view.inspector.tab = details::Tab::Operations;
        view.mutations.set_target(oid, window, cx);
        view.branch_form_open = matches!(command, Command::Branch);
        let action = match command {
            Command::Merge => Some(Action::Merge {
                revision: oid.into(),
            }),
            Command::Rebase => Some(Action::Rebase {
                revision: oid.into(),
            }),
            Command::Pick if !merge => Some(Action::CherryPick {
                revision: oid.into(),
                mainline: None,
            }),
            // A merge pick needs an explicitly entered mainline. Never silently
            // reuse a parent's comparison selection as mutation consent.
            _ => None,
        };
        if let Some(action) = action {
            self.prepare_repository_action(action, cx);
        }
        cx.notify();
    }
}

pub(super) fn menu(
    app: WeakEntity<DunlinApp>,
    id: uuid::Uuid,
    commit: Commit,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu {
    move |menu, _, cx| {
        let disabled = app.upgrade().is_none_or(|app| {
            let app = app.read(cx);
            app.git_history.running
                || app
                    .git_history
                    .view
                    .as_ref()
                    .is_none_or(|v| v.id != id || v.loading)
        });
        let copy = commit.oid.clone();
        let mut menu = menu
            .label(format!(
                "Commit {}",
                commit.oid.chars().take(8).collect::<String>()
            ))
            .item(
                menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Copy commit SHA")).on_click(
                    move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())),
                ),
            )
            .separator();
        for (label, command) in [
            ("Create branch here…", Command::Branch),
            ("Merge into current branch…", Command::Merge),
            ("Rebase current branch onto here…", Command::Rebase),
            ("Cherry-pick…", Command::Pick),
            ("Create worktree here…", Command::Worktree),
            ("More Git operations…", Command::Operations),
        ] {
            let app = app.clone();
            let oid = commit.oid.clone();
            menu = menu.item(
                menu_item(move |_, _| div().px(px(4.)).py(px(3.)).child(label))
                    .disabled(disabled)
                    .on_click(move |_, window, cx| {
                        let _ = app.update(cx, |this, cx| {
                            this.history_command(id, &oid, command, window, cx)
                        });
                    }),
            );
        }
        menu
    }
}

pub(super) fn status(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let view = this.git_history.view.as_ref().unwrap();
    let p = palette(cx);
    let mut bar = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap(px(6.))
        .text_size(px(11.))
        .text_color(p.muted_fg);
    if this.git_history.running {
        bar = bar.child(
            div()
                .text_color(p.amber)
                .child("Git operation in progress…"),
        );
    }
    if let Some(value) = &view.overview.value {
        for operation in &value.operations {
            bar = bar.child(
                div()
                    .text_color(p.amber)
                    .child(format!("{} in progress", operation.label())),
            );
            if matches!(
                operation,
                dunlin_workspace::git::overview::Operation::Merge
                    | dunlin_workspace::git::overview::Operation::Rebase
                    | dunlin_workspace::git::overview::Operation::CherryPick
            ) {
                bar = bar.child(mutations::operation_actions(
                    *operation,
                    this.git_history.running || view.overview.loading,
                    cx,
                ));
            }
        }
        if !value.conflicts.is_empty() {
            bar = bar.child(
                div()
                    .text_color(p.red)
                    .child(format!("{} conflicted paths", value.conflicts.len())),
            );
        }
    }
    if view.overview.loading {
        bar = bar.child("Reading repository state…");
    }
    if let Some(error) = &view.overview.error {
        bar = bar.child(div().text_color(p.red).child(error.clone()));
    }
    bar.into_any_element()
}
