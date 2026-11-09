//! Read-only inventory shares the history tab's project/backend ownership, but
//! has independent request/error state so unborn repos and bad revisions work.
use super::*;
use dunlin_workspace::git::overview::Overview;

#[derive(Default)]
pub(super) struct State {
    pub visible: bool,
    pub(super) value: Option<Overview>,
    pub(super) error: Option<String>,
    pub(super) loading: bool,
    request: uuid::Uuid,
    scroll: ScrollHandle,
}

impl DunlinApp {
    pub(super) fn open_git_overview(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = &mut self.git_history.view {
            view.overview.visible = true;
        }
        self.load_git_overview(cx);
    }

    pub(super) fn load_git_overview(&mut self, cx: &mut Context<Self>) {
        self.git_history.prune(&self.projects, &self.remote_hosts);
        let Some(view) = self.git_history.view.as_mut() else {
            return;
        };
        if view.overview.loading {
            return;
        }
        view.overview.loading = true;
        view.overview.error = None;
        view.overview.request = uuid::Uuid::now_v7();
        let (id, request) = (view.id, view.overview.request);
        let (backend, expected) = (view.backend.clone(), view.scope.clone());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { backend.git_overview(expected.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.git_history.prune(&this.projects, &this.remote_hosts);
                let Some(view) = this
                    .git_history
                    .view
                    .as_mut()
                    .filter(|v| v.id == id && v.overview.request == request)
                else {
                    return;
                };
                view.overview.loading = false;
                match result {
                    Ok(response) => match pin_scope(&mut view.scope, response.scope) {
                        Ok(()) => {
                            view.overview.value = Some(response.value);
                            view.overview.scroll.set_offset(point(px(0.), px(0.)));
                        }
                        Err(error) => view.overview.error = Some(error),
                    },
                    Err(error) => view.overview.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

pub(super) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let view = this.git_history.view.as_ref().unwrap();
    let state = &view.overview;
    let mut content = div()
        .id("git-inventory-list")
        .flex_1()
        .min_h(px(0.))
        .overflow_scroll()
        .track_scroll(&state.scroll)
        .p(px(12.))
        .flex()
        .flex_col()
        .gap(px(8.))
        .text_size(px(12.));
    if state.loading {
        content = content.child("Reading repository metadata…");
    }
    if let Some(error) = &state.error {
        content = content.child(div().text_color(p.red).child(error.clone()));
    }
    if let Some(value) = &state.value {
        content = content.child(heading(format!("Operations · {}", value.operations.len())));
        if value.operations.is_empty() {
            content =
                content.child("No merge/rebase/cherry-pick/revert/sequencer markers observed.");
        }
        for operation in &value.operations {
            content = content.child(
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
                content = content.child(super::mutations::operation_actions(
                    *operation,
                    this.git_history.running,
                    cx,
                ));
            }
        }
        content = content.child(heading(format!(
            "Unmerged index paths · {}",
            value.conflicts.len()
        )));
        if value.conflicts.is_empty() {
            content = content.child(
                "No unmerged index entries observed. This does not mean the working tree is clean.",
            );
        }
        for conflict in &value.conflicts {
            content = content.child(row(format!(
                "{} · {} · stages {:?}",
                conflict.path.escape_debug(),
                conflict.label(),
                conflict.stages
            )));
        }
        content = content.child(heading(format!("Stash · {}", value.stashes.len())));
        if value.stashes.is_empty() {
            content = content.child("No stashes.");
        }
        for stash in &value.stashes {
            let date = chrono::DateTime::from_timestamp(stash.timestamp, 0)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            content = content.child(row(format!(
                "{} · {} · {date} · {}",
                stash.selector,
                stash.oid.chars().take(8).collect::<String>(),
                stash.subject.escape_debug()
            )));
            content = content.child(super::mutations::stash_actions(
                &stash.oid,
                this.git_history.running,
                cx,
            ));
        }
        content = content.child(heading(format!("Worktrees · {}", value.worktrees.len())));
        for tree in &value.worktrees {
            let mut flags = Vec::new();
            if tree.current {
                flags.push("selected project".to_string());
            }
            if tree.bare {
                flags.push("bare".into());
            }
            if tree.detached {
                flags.push("detached HEAD".into());
            }
            if let Some(reason) = &tree.locked {
                flags.push(format!("locked: {}", reason.escape_debug()));
            }
            if let Some(reason) = &tree.prunable {
                flags.push(format!("prunable: {}", reason.escape_debug()));
            }
            let reference = tree.branch.as_deref().unwrap_or("no branch");
            content = content.child(row(format!(
                "{} · {} · {} · {}",
                tree.path.escape_debug(),
                reference.escape_debug(),
                tree.head
                    .as_ref()
                    .map(|s| s.chars().take(8).collect::<String>())
                    .unwrap_or_default(),
                flags.join(" · ")
            )));
            content = content.child(super::mutations::worktree_action(
                &tree.path,
                this.git_history.running
                    || tree.current
                    || tree.bare
                    || tree.locked.is_some()
                    || tree.prunable.is_some(),
                cx,
            ));
            let path = tree.path.clone();
            let mut actions = div().flex().gap(px(4.)).child(
                dunlin_ui::button(format!("worktree-copy-{path}"))
                    .ghost()
                    .small()
                    .label(if view.project.is_remote() {
                        "Copy server path"
                    } else {
                        "Copy path"
                    })
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(path.clone()))
                    }),
            );
            if !view.project.is_remote() && !tree.bare && tree.prunable.is_none() {
                let path = tree.path.clone();
                actions = actions.child(
                    dunlin_ui::button(format!("worktree-open-{path}"))
                        .ghost()
                        .small()
                        .label("Open as project…")
                        .disabled(this.git_history.running)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_project(std::path::PathBuf::from(&path), cx)
                        })),
                );
            }
            content = content.child(actions);
        }
    }
    div().id("git-inventory-panel").track_focus(&view.focus).flex_1().min_w(px(0.)).min_h(px(0.))
        .bg(p.bg).text_color(p.fg).flex().flex_col().overflow_hidden()
        .child(div().p(px(10.)).flex().flex_wrap().items_center().gap(px(8.))
            .child(format!("Repository state · {}", view.project.name)).child(div().flex_1())
            .child(dunlin_ui::button("inventory-history").label("Commit history").on_click(cx.listener(|this, _, _, cx| {
                if let Some(view) = &mut this.git_history.view { view.overview.visible = false; }
                cx.notify();
            })))
            .child(dunlin_ui::button("inventory-refresh").label("Refresh state").disabled(state.loading || this.git_history.running)
                .on_click(cx.listener(|this, _, _, cx| this.load_git_overview(cx))))
            .child(dunlin_ui::button("inventory-close").label("Close tab").on_click(cx.listener(|this, _, window, cx| {
                this.close_git_history(cx); this.focus_handle.focus(window, cx);
            }))))
        .child(div().px(px(12.)).pb(px(8.)).text_size(px(11.)).text_color(p.muted_fg)
             .child(format!("{} snapshot · mutations require a fresh preflight and explicit confirmation", if view.project.is_remote() { "Relay" } else { "Local" })))
        .child(super::mutations::render(this,cx))
        .child(content)
        .child(div().p(px(10.)).text_size(px(11.)).text_color(p.muted_fg)
             .child("Snapshot alone never authorizes a mutation. New detached worktrees live in the common Git directory's dunlin-worktrees folder; existing worktree removal is explicit and requires clean state."))
        .into_any_element()
}

fn heading(label: String) -> impl IntoElement {
    div()
        .flex_shrink_0()
        .mt(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label)
}
fn row(label: String) -> impl IntoElement {
    div().flex_shrink_0().child(label)
}
