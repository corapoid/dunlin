//! Read-only inventory shares the history tab's project/backend ownership, but
//! has independent request/error state so unborn repos and bad revisions work.
use super::*;
use dunlin_ui::{assets::IconName, component::Icon};
use dunlin_workspace::git::overview::Overview;

#[derive(Default)]
pub(super) struct State {
    pub visible: bool,
    pub(super) value: Option<Overview>,
    pub(super) error: Option<String>,
    pub(super) loading: bool,
    request: uuid::Uuid,
    pub(super) scroll: ScrollHandle,
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
        .w_full()
        .max_w(px(1100.))
        .self_center()
        .flex_shrink_0()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(px(16.))
        .text_size(px(12.));
    if state.loading {
        content = content.child(description("Reading repository metadata…", p));
    }
    if let Some(error) = &state.error {
        content = content.child(dunlin_ui::settings::notice(error.clone(), p).text_color(p.red));
    }
    if let Some(value) = &state.value {
        let mut operations = section(
            IconName::GitBranch,
            "In progress",
            value.operations.len(),
            p,
        );
        if value.operations.is_empty() {
            operations = operations.child(description("No Git operation in progress.", p));
        }
        for operation in &value.operations {
            operations = operations.child(
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
                operations = operations.child(super::mutations::operation_actions(
                    *operation,
                    this.git_history.running,
                    cx,
                ));
            }
        }
        let mut conflicts = section(
            IconName::TriangleAlert,
            "Conflicts",
            value.conflicts.len(),
            p,
        );
        if value.conflicts.is_empty() {
            conflicts = conflicts.child(description(
                "No unmerged paths. Working tree changes may still exist.",
                p,
            ));
        }
        for conflict in &value.conflicts {
            conflicts = conflicts.child(
                item()
                    .child(
                        div()
                            .font_family(dunlin_ui::theme::code_font())
                            .child(conflict.path.escape_debug().to_string()),
                    )
                    .child(description(
                        format!("{} · stages {:?}", conflict.label(), conflict.stages),
                        p,
                    )),
            );
        }
        content = content.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(16.))
                .child(operations.flex_1().min_w(px(260.)))
                .child(conflicts.flex_1().min_w(px(260.))),
        );
        let mut stashes = section(IconName::Archive, "Stashes", value.stashes.len(), p);
        if value.stashes.is_empty() {
            stashes = stashes.child(description("No saved stashes.", p));
        }
        for stash in &value.stashes {
            let date = chrono::DateTime::from_timestamp(stash.timestamp, 0)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%Y-%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default();
            stashes = stashes.child(
                item()
                    .child(
                        div()
                            .text_size(px(12.))
                            .child(stash.subject.escape_debug().to_string()),
                    )
                    .child(description(
                        format!(
                            "{} · {} · {date}",
                            stash.selector,
                            stash.oid.chars().take(8).collect::<String>()
                        ),
                        p,
                    ))
                    .child(super::mutations::stash_actions(
                        &stash.oid,
                        this.git_history.running,
                        cx,
                    )),
            );
        }
        content = content.child(stashes);
        let mut worktrees = section(IconName::FolderGit2, "Worktrees", value.worktrees.len(), p);
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
            let path = tree.path.clone();
            let mut actions = dunlin_ui::settings::actions().child(
                dunlin_ui::settings::button(format!("worktree-copy-{path}"))
                    .icon(IconName::Copy)
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
                    dunlin_ui::settings::button(format!("worktree-open-{path}"))
                        .icon(IconName::FolderOpen)
                        .label("Open as project…")
                        .disabled(this.git_history.running)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.add_project(std::path::PathBuf::from(&path), cx)
                        })),
                );
            }
            actions = actions.child(super::mutations::worktree_action(
                &tree.path,
                this.git_history.running
                    || tree.current
                    || tree.bare
                    || tree.locked.is_some()
                    || tree.prunable.is_some(),
                cx,
            ));
            worktrees = worktrees.child(
                item()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                Icon::new(IconName::GitBranch)
                                    .size(px(14.))
                                    .text_color(p.muted_fg),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(
                                        reference
                                            .trim_start_matches("refs/heads/")
                                            .escape_debug()
                                            .to_string(),
                                    ),
                            )
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(p.muted_fg)
                                    .font_family(dunlin_ui::theme::code_font())
                                    .child(
                                        tree.head
                                            .as_ref()
                                            .map(|s| s.chars().take(8).collect::<String>())
                                            .unwrap_or_default(),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(11.))
                            .text_color(p.muted_fg)
                            .child(tree.path.escape_debug().to_string()),
                    )
                    .when(!flags.is_empty(), |row| {
                        row.child(description(flags.join(" · "), p))
                    })
                    .child(actions),
            );
        }
        content = content.child(worktrees);
    }
    content = content.child(div().pt(px(8.)).flex().flex_col().gap(px(12.))
        .child(div().text_size(px(14.)).font_weight(FontWeight::MEDIUM).child("Git operations"))
        .child(super::mutations::render(this, cx)))
        .child(description("Every change requires a fresh preflight and explicit confirmation. New detached worktrees are created in the common Git directory's dunlin-worktrees folder; removal requires a clean, unlocked worktree.", p));
    div()
        .id("git-inventory-panel")
        .relative()
        .track_focus(&view.focus)
        .flex_1()
        .min_w(px(0.))
        .min_h(px(0.))
        .text_color(p.fg)
        .flex()
        .flex_col()
        .overflow_hidden()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("git-inventory-panel")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(
            div()
                .px(px(20.))
                .py(px(16.))
                .flex_shrink_0()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(180.))
                        .flex()
                        .flex_col()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(16.))
                                .font_weight(FontWeight::MEDIUM)
                                .child("Repository state"),
                        )
                        .child(description(
                            format!(
                                "{} · {} snapshot",
                                view.project.name,
                                if view.project.is_remote() {
                                    "Relay"
                                } else {
                                    "Local"
                                }
                            ),
                            p,
                        )),
                )
                .child(
                    dunlin_ui::toolbar_button("inventory-history")
                        .icon(IconName::GitBranch)
                        .label("Commit history")
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(view) = &mut this.git_history.view {
                                view.overview.visible = false;
                            }
                            cx.notify();
                        })),
                )
                .child(
                    dunlin_ui::toolbar_button("inventory-refresh")
                        .icon(IconName::RefreshCw)
                        .label("Refresh")
                        .disabled(state.loading || this.git_history.running)
                        .on_click(cx.listener(|this, _, _, cx| this.load_git_overview(cx))),
                )
                .child(
                    icon_button("inventory-close", IconName::X, "Close Git tab").on_click(
                        cx.listener(|this, _, window, cx| {
                            this.close_git_history(cx);
                            this.focus_handle.focus(window, cx);
                        }),
                    ),
                ),
        )
        .child(
            div()
                .id("git-inventory-list")
                .flex_1()
                .min_h(px(0.))
                .min_w(px(0.))
                .overflow_y_scroll()
                .track_scroll(&state.scroll)
                .px(px(20.))
                .pb(px(20.))
                .flex()
                .flex_col()
                .child(content),
        )
        .into_any_element()
}

fn section(icon: IconName, title: &str, count: usize, p: Palette) -> Div {
    SurfaceGeometry::SETTINGS_CARD
        .paint(transparent_black(), p.border)
        .min_w(px(0.))
        .flex_shrink_0()
        .p(px(16.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(Icon::new(icon).size(px(15.)).text_color(p.muted_fg))
                .child(
                    div()
                        .flex_1()
                        .font_weight(FontWeight::MEDIUM)
                        .child(title.to_owned()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child(count.to_string()),
                ),
        )
}

fn item() -> Div {
    div()
        .min_w(px(0.))
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(8.))
}

fn description(text: impl Into<SharedString>, p: Palette) -> Div {
    dunlin_ui::settings::description(text, p)
}
