//! Left sidebar: project switcher, explorer, search, source control,
//! session history and the subscription usage footer.

use std::path::Path;

use crate::effects::menu_item;
use dunlin_domain::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::ThemeStyled as _;
use dunlin_ui::component::input::Input;
use dunlin_ui::component::menu::{ContextMenuExt, PopupMenu, DropdownMenu, PopupMenuItem};
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::Sizable;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{
    ContextMenuState, ContextTarget, DunlinApp, SettingsTab, status_palette, tone_color, tone_soft,
};
use dunlin_workspace::tree::FileEntry;

pub(crate) fn render(
    this: &mut DunlinApp,
    _window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let section = this.section;
    let body = match section {
        SidebarSection::Dashboard => projects_body(this, cx),
        SidebarSection::Projects => projects_body(this, cx),
        SidebarSection::Files => files_body(this, cx),
        SidebarSection::Search => search_body(this, cx),
        SidebarSection::SourceControl => source_control_body(this, cx),
        SidebarSection::Security if this.security_enabled => {
            crate::security::sidebar_body(this, cx)
        }
        SidebarSection::Security => projects_body(this, cx),
        SidebarSection::Sessions => sessions_body(this, cx),
    };
    let header = section_header(this, section, cx);
    let footer = Some(usage_footer(this, cx));

    let width =
        crate::sidebar_layout::width(this.sidebar_width, f32::from(_window.viewport_size().width));
    let scroll = div()
        .id("sidebar-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .when(section == SidebarSection::Files, |view| {
            crate::explorer::drop_target(view, this.project().path.clone(), cx)
        })
        .child(body);
    let scroll = if matches!(
        section,
        SidebarSection::Projects | SidebarSection::Dashboard
    ) {
        projects_context(scroll, None, this.project_groups.clone(), cx)
    } else {
        scroll.into_any_element()
    };
    let root = div()
        .relative()
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(p.sidebar)
        .border_r_1()
        .border_color(p.border)
        .child(header)
        .child(scroll)
        .children(footer)
        .child(crate::sidebar_layout::handle(this, cx));

    #[cfg(debug_assertions)]
    let root = root.children(crate::workspace_ui_smoke::probe("sidebar"));
    root.into_any_element()
}

pub(crate) fn project_switcher(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let project = this.project();
    let missing = !project.is_remote() && !Path::new(&project.path).is_dir();

    div()
        .id("project-switcher")
        .aria_label("Active project · switch project")
        .h(px(30.))
        .max_w(px(310.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(|this, _, _, cx| {
            this.project_menu_open = !this.project_menu_open;
            cx.notify();
        }))
        .child(crate::project_icons::render(this, project, 20., &p))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(p.fg)
                                .truncate()
                                .child(project.name.clone()),
                        )
                        .when(missing, |this| {
                            this.child(
                                Icon::new(IconName::TriangleAlert)
                                    .size(px(10.))
                                    .text_color(p.amber),
                            )
                        }),
                ),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .size(px(13.))
                .text_color(p.muted_fg),
        )
        .into_any_element()
}

fn section_header(
    this: &DunlinApp,
    section: SidebarSection,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let (title, count) = match section {
        SidebarSection::Dashboard => (
            "Projects",
            Some(this.projects.iter().filter(|p| !p.is_chat_space()).count()),
        ),
        SidebarSection::Projects => (
            "Projects",
            Some(this.projects.iter().filter(|p| !p.is_chat_space()).count()),
        ),
        SidebarSection::Files => ("Explorer", None),
        SidebarSection::Search => ("Search", Some(this.search_hits.len())),
        SidebarSection::SourceControl => ("Source control", Some(this.git.changes.len())),
        SidebarSection::Security => ("Security", None),
        SidebarSection::Sessions => ("Sessions", Some(this.visible_session_count())),
    };

    let right: AnyElement = match section {
        SidebarSection::Files => div()
            .id("explorer-actions")
            .flex()
            .items_center()
            .gap(px(2.))
            .child(crate::inline_blame::toggle(this, cx))
            .child(
                div()
                    .id("refresh-explorer")
                    .aria_label("Refresh explorer")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.refresh_file_tree(cx);
                        this.set_notice("Refreshed the explorer", cx);
                    }))
                    .child(
                        Icon::new(IconName::RefreshCw)
                            .size(px(11.5))
                            .text_color(p.muted_fg),
                    ),
            )
            .child(
                div()
                    .id("collapse-all")
                    .aria_label("Collapse all folders")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.file_tree.collapse_all();
                        cx.notify();
                    }))
                    .child(
                        Icon::new(IconName::FoldVertical)
                            .size(px(12.5))
                            .text_color(p.muted_fg),
                    ),
            )
            .child(
                div()
                    .id("new-file")
                    .aria_label("Create a new file")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_file(window, cx);
                    }))
                    .child(
                        Icon::new(IconName::FilePlus)
                            .size(px(13.))
                            .text_color(p.muted_fg),
                    ),
            )
            .child(
                div()
                    .id("new-folder")
                    .aria_label("Create a new folder")
                    .size(px(22.))
                    .rounded(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let parent = this.explorer_parent();
                        this.new_explorer_entry(parent, true, window, cx);
                    }))
                    .child(
                        Icon::new(IconName::FolderPlus)
                            .size(px(13.))
                            .text_color(p.muted_fg),
                    ),
            )
            .into_any_element(),
        SidebarSection::Projects => div()
            .id("add-project")
            .size(px(22.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.open_folder(cx);
            }))
            .child(
                Icon::new(IconName::Plus)
                    .size(px(13.))
                    .text_color(p.muted_fg),
            )
            .into_any_element(),
        SidebarSection::Sessions => div()
            .id("new-session")
            .h(px(22.))
            .px(px(7.))
            .rounded(px(5.))
            .flex()
            .items_center()
            .gap(px(4.))
            .cursor_pointer()
            .hover(move |style| style.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.new_session(cx);
            }))
            .child(
                Icon::new(IconName::Plus)
                    .size(px(11.5))
                    .text_color(p.muted_fg),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .text_color(p.muted_fg)
                    .child("New"),
            )
            .into_any_element(),
        _ => div().into_any_element(),
    };

    let header = div()
        .id("sidebar-section-header")
        .h(px(34.))
        .flex_shrink_0()
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child(title.to_uppercase()),
        )
        .when_some(count, |this, count| {
            this.child(
                div()
                    .text_size(px(11.0))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg.opacity(0.7))
                    .child(count.to_string()),
            )
        })
        .child(div().flex_1())
        .child(right);
    if matches!(
        section,
        SidebarSection::Projects | SidebarSection::Dashboard
    ) {
        projects_context(header, None, this.project_groups.clone(), cx)
    } else {
        header.into_any_element()
    }
}

fn projects_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.refresh_project_summaries(cx);
    this.refresh_project_catalog(cx);
    let p = palette(cx);
    let mut list = div().flex().flex_col().gap(px(2.)).px(px(6.)).pb(px(10.));
    let dragging = cx.has_active_drag();
    if dragging {
        list = list.child(div().p(px(9.)).mb(px(6.)).rounded(px(7.)).bg(p.blue_soft).text_size(px(11.)).text_color(p.blue).child("Move a project: drop anywhere inside its destination group. Files stay in place."));
    }
    let groups = this.project_groups.clone();
    let modified: std::collections::HashMap<_, _> = this
        .repo_summaries
        .values
        .iter()
        .map(|(id, summary)| (id.clone(), summary.activity.modified_ms))
        .collect();
    let chat_spaces: Vec<usize> = this
        .projects
        .iter()
        .enumerate()
        .filter(|(_, project)| project.is_chat_space())
        .map(|(index, _)| index)
        .collect();
    if !chat_spaces.is_empty() {
        list = list.child(
            div()
                .px(px(9.))
                .pt(px(4.))
                .pb(px(2.))
                .text_size(px(9.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child("CHATS · SCRATCH WORKSPACES"),
        );
        for index in chat_spaces {
            list = list.child(chat_space_row(this, index, cx, &p));
        }
        list = list.child(div().h(px(1.)).my(px(6.)).bg(p.border));
    }
    list = list.child(project_view_controls(this, cx, &p));
    list = list.child(project_filters(this, cx, &p));
    {
        for (index, depth) in crate::project_tree::visible_groups(
            &groups,
            &this.projects,
            &this.collapsed_project_groups,
            this.project_sort,
            &modified,
        ) {
            let group = &groups[index];
            let mut indices: Vec<_> = this
                .projects
                .iter()
                .enumerate()
                .filter(|(_, p)| !p.is_chat_space() && p.group_id.as_deref() == Some(&group.id) && crate::project_catalog::matches(this, p))
                .map(|(i, _)| i)
                .collect();
            crate::project_tree::sort_projects(
                &this.projects,
                &mut indices,
                this.project_sort,
                &modified,
            );
            let descendants = crate::project_tree::descendants(&groups, &group.id);
            let count = this
                .projects
                .iter()
                .filter(|p| {
                    !p.is_chat_space()
                        && p.group_id
                            .as_ref()
                            .is_some_and(|id| descendants.contains(id))
                })
                .count();
            let any_match = this.projects.iter().any(|p| !p.is_chat_space() && p.group_id.as_ref().is_some_and(|id| descendants.contains(id)) && crate::project_catalog::matches(this, p));
            if !any_match && ((this.project_list.show_language && this.project_catalog.language.is_some()) || (this.project_list.show_topics && this.project_catalog.topic.is_some())) { continue; }
            let collapsed = this.collapsed_project_groups.contains(&group.id);
            let mut section =
                project_group_drop_zone(Some(group.id.clone()), &group.name, dragging, cx, &p)
                    .ml(px(depth.min(8) as f32 * 12.))
                    .child(group_header(this, group, &groups, count, collapsed, cx, &p));
            for index in indices.into_iter().filter(|_| !collapsed) {
                section = section.child(div().ml(px(24.)).child(project_row(this, index, cx, &p)));
                section = section.child(div().ml(px(24.)).child(project_sessions(this, index, cx, &p)));
                if this.project_group_editor == Some(index) {
                    section = section.child(div().ml(px(24.)).child(project_group_picker(this, index, cx, &p)));
                }
            }
            list = list.child(section);
        }
    }
    let mut indices: Vec<_> = this
        .projects
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            !p.is_chat_space()
                && crate::project_catalog::matches(this, p)
                && (p.group_id
                        .as_ref()
                        .is_none_or(|id| !groups.iter().any(|g| &g.id == id)))
        })
        .map(|(i, _)| i)
        .collect();
    crate::project_tree::sort_projects(&this.projects, &mut indices, this.project_sort, &modified);
    if !indices.is_empty() || dragging {
        let mut section = project_group_drop_zone(None, "Top level", dragging, cx, &p);
        for index in indices {
            section = section.child(project_row(this, index, cx, &p));
            section = section.child(project_sessions(this, index, cx, &p));
            if this.project_group_editor == Some(index) {
                section = section.child(project_group_picker(this, index, cx, &p));
            }
        }
        list = list.child(section);
    }
    list.into_any_element()
}

fn project_filters(this: &DunlinApp, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let mut row = div().flex().flex_wrap().items_center().gap(px(5.)).pb(px(5.));
    if this.project_list.show_language {
        let owner = cx.entity().downgrade();
        row = row.child(Button::new("project-language-filter").ghost().small()
            .label(this.project_catalog.language.clone().unwrap_or_else(|| "All languages".into())).dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
                let Some(app) = owner.upgrade() else { return menu; };
                let this = app.read(cx);
                for language in std::iter::once(None).chain(crate::project_catalog::languages(this).into_iter().map(Some)) {
                    let app = owner.clone();
                    menu = menu.item(PopupMenuItem::new(language.clone().unwrap_or_else(|| "All languages".into()))
                        .checked(this.project_catalog.language == language)
                        .on_click(move |_, _, cx| { let _ = app.update(cx, |this, cx| { this.project_catalog.language = language.clone(); cx.notify(); }); }));
                }
                menu.scrollable(true).max_h(px(320.))
            }));
    }
    if this.project_list.show_topics && let Some(topic) = &this.project_catalog.topic {
        row = row.child(Button::new("project-topic-clear").small().ghost().label(format!("#{topic} ×"))
            .on_click(cx.listener(|this, _, _, cx| { this.project_catalog.topic = None; cx.notify(); })));
    }
    row.text_color(p.muted_fg).into_any_element()
}

fn project_sessions(this: &DunlinApp, index: usize, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let project = &this.projects[index];
    let live = this.live_agents();
    let now = chrono::Utc::now().timestamp_millis();
    let mut sessions: Vec<_> = this.all_sessions.iter().filter(|session| {
        let running = live.iter().any(|(owner, id, _)| owner == &project.id && id == &session.id);
        session.project_id == project.id && (running || this.session_visible(session)) && crate::project_tree::show_session(
            &session.updated_at, running,
            this.project_sessions_running_only, now,
        )
    }).collect();
    if sessions.is_empty() {
        return div().into_any_element();
    }
    sessions.sort_by(|a, b| {
        let running = |id: &str| live.iter().any(|(_, session, _)| session == id);
        let time = |s: &SessionSummary| chrono::DateTime::parse_from_rfc3339(&s.updated_at).map(|d| d.timestamp_millis()).unwrap_or(0);
        running(&b.id).cmp(&running(&a.id)).then_with(|| time(b).cmp(&time(a)))
    });
    let mut rows = div().ml(px(25.)).pl(px(8.)).border_l_1().border_color(p.border).mb(px(6.));
    #[cfg(debug_assertions)]
    { rows = rows.relative().children(crate::workspace_ui_smoke::probe("project-sessions")); }
    for session in sessions {
        let id = session.id.clone(); let owner = project.id.clone();
        let status = live.iter().find(|(_, id, _)| id == &session.id).map(|(_, _, label)| *label);
        let root = this.parked_runtimes.get(&session.id).map(|r| r.root.as_str())
            .or(session.workspace.as_deref());
        let workspace = root.map(|path| std::path::Path::new(path).file_name().unwrap_or_default().to_string_lossy().into_owned())
            .unwrap_or_else(|| "workspace not recorded".into());
        rows = rows.child(div().id(format!("project-session:{}", session.id))
            .aria_label(format!("{} · {} · {}", session.title, session.branch.as_deref().unwrap_or("branch not recorded"), workspace))
            .px(px(6.)).py(px(dunlin_ui::scale::space(5.))).rounded(px(6.)).cursor_pointer().hover(move |s| s.bg(p.hover))
            .when(this.project().id == project.id && this.session().id == session.id, |r| r.bg(p.accent))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.all_sessions.iter().any(|s| s.id == id && s.project_id == owner)
                    && this.open_session_id(&id, cx) { this.show_chat_tab(window, cx); }
            }))
            .child(div().flex().items_center().gap(px(5.))
                .child(div().size(px(5.)).rounded_full().bg(if status.is_some() { p.green } else { p.muted_fg }))
                .child(div().flex_1().min_w(px(0.)).truncate().text_size(px(11.)).child(session.title.clone())))
            .child(div().text_size(px(9.5)).text_color(p.muted_fg).truncate()
                .child(format!("{} · {} · {}", session.harness, status.unwrap_or("Recent"), session.branch.as_deref().unwrap_or("branch not recorded"))))
            .child(div().text_size(px(9.5)).text_color(p.muted_fg).truncate().child(workspace)));
    }
    rows.into_any_element()
}

fn project_view_controls(this: &DunlinApp, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let row = div().flex().items_center().gap(px(3.)).pb(px(6.));
    row.child(
        div().id("project-sessions-filter").aria_label("Toggle running sessions only")
            .px(px(6.)).py(px(4.)).rounded(px(5.)).text_size(px(10.))
            .text_color(if this.project_sessions_running_only { p.blue } else { p.muted_fg })
            .cursor_pointer().hover(move |s| s.bg(p.hover))
            .on_click(cx.listener(|this, _, _, cx| {
                this.project_sessions_running_only = !this.project_sessions_running_only; cx.notify();
            }))
            .child(if this.project_sessions_running_only { "Running only" } else { "Running + 8h" })
    ).child(div().flex_1())
        .child(
            div()
                .id("project-sort")
                .aria_label("Switch project sorting: name ascending or modification date descending; pinned projects stay first")
                .px(px(6.))
                .py(px(4.))
                .rounded(px(5.))
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_sort = if this.project_sort == crate::project_tree::Sort::Name {
                        crate::project_tree::Sort::Modified
                    } else {
                        crate::project_tree::Sort::Name
                    };
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(if this.project_sort == crate::project_tree::Sort::Name {
                    "Name ↑"
                } else {
                    "Modified ↓"
                }),
        )
        .into_any_element()
}

fn projects_context(
    view: Stateful<Div>,
    group: Option<String>,
    groups: Vec<ProjectGroup>,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let app = cx.entity().downgrade();
    view.context_menu(move |menu, window, cx| {
        let mut menu = menu;
        let create = app.clone();
        let parent = group.clone();
        menu = menu.item(
            menu_item(move |_, _| {
                div().px(px(4.)).py(px(3.)).child(if parent.is_some() {
                    "New subgroup…"
                } else {
                    "New group…"
                })
            })
            .on_click({
                let parent = group.clone();
                move |_, window, cx| {
                    let _ = create.update(cx, |this, cx| {
                        this.open_subgroup_prompt(parent.clone(), window, cx)
                    });
                }
            }),
        );
        let open = app.clone();
        menu = menu.item(
            menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Add projects…")).on_click(
                move |_, _, cx| {
                    let _ = open.update(cx, |this, cx| this.open_folder(cx));
                },
            ),
        );
        if let Some(id) = &group {
            let rename = app.clone();
            let rename_id = id.clone();
            menu = menu.separator().item(
                menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Group settings…")).on_click(
                    move |_, window, cx| {
                        let _ = rename.update(cx, |this, cx| {
                            this.open_group_settings(&rename_id, window, cx)
                        });
                    },
                ),
            );
            let rename = app.clone();
            let rename_id = id.clone();
            menu = menu.item(
                menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Rename group…")).on_click(
                    move |_, window, cx| {
                        let _ = rename.update(cx, |this, cx| {
                            this.open_rename_group_prompt(&rename_id, window, cx)
                        });
                    },
                ),
            );
            let app = app.clone();
            let id = id.clone();
            let groups = groups.clone();
            menu = menu.submenu("Move group to…", window, cx, move |mut menu, _, _| {
                let root = app.clone();
                let root_id = id.clone();
                menu = menu.item(
                    menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Top level")).on_click(
                        move |_, _, cx| {
                            let _ = root.update(cx, |this, cx| this.move_group(&root_id, None, cx));
                        },
                    ),
                );
                let excluded = crate::project_tree::descendants(&groups, &id);
                for target in groups.iter().filter(|g| !excluded.contains(&g.id)) {
                    let app = app.clone();
                    let id = id.clone();
                    let target_id = target.id.clone();
                    let path = crate::project_tree::group_path(&groups, &target.id);
                    menu = menu.item(
                        menu_item(move |_, _| div().px(px(4.)).py(px(3.)).child(path.clone()))
                            .on_click(move |_, _, cx| {
                                let _ = app.update(cx, |this, cx| {
                                    this.move_group(&id, Some(target_id.clone()), cx)
                                });
                            }),
                    );
                }
                menu
            });
        }
        menu
    })
    .into_any_element()
}

fn project_group_drop_zone(
    group: Option<String>,
    name: &str,
    dragging: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> Stateful<Div> {
    let key = group.as_deref().unwrap_or("ungrouped");
    let project_group = group.clone();
    div()
        .id(format!("group-drop-zone:{key}"))
        .rounded(px(8.))
        .border_2()
        .border_color(if dragging {
            p.border_strong
        } else {
            p.border.opacity(0.)
        })
        .p(px(2.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .when(dragging, |zone| {
            zone.border_dashed().child(
                div()
                    .px(px(7.))
                    .py(px(4.))
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.blue)
                    .child(format!("Drop project → {name}")),
            )
        })
        .drag_over::<crate::project_groups::ProjectDrag>(|style, _, _, cx| {
            let p = palette(cx);
            style.bg(p.blue_soft).border_color(p.blue).shadow(vec![
                BoxShadow::new(px(0.), px(0.), p.blue.opacity(0.25)).blur_radius(px(8.)),
            ])
        })
        .on_drop(cx.listener(
            move |this, drag: &crate::project_groups::ProjectDrag, _, cx| {
                cx.stop_propagation();
                this.move_project_to_group(&drag.id, project_group.clone(), cx);
            },
        ))
        .drag_over::<crate::project_groups::GroupDrag>(|style, _, _, cx| {
            style
                .bg(palette(cx).blue_soft)
                .border_color(palette(cx).blue)
        })
        .on_drop(cx.listener(
            move |this, drag: &crate::project_groups::GroupDrag, _, cx| {
                cx.stop_propagation();
                this.move_group(&drag.id, group.clone(), cx);
            },
        ))
}

fn group_header(
    this: &DunlinApp,
    group: &ProjectGroup,
    groups: &[ProjectGroup],
    count: usize,
    collapsed: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let id = group.id.clone();
    let settings_id = group.id.clone();
    let agent_id = group.id.clone();
    let appearance_id = group.id.clone();
    let name = group.name.clone();
    let hover_group = format!("project-group-{}", group.id);

    let menu_id = group.id.clone();
    let drag = crate::project_groups::GroupDrag {
        id: group.id.clone(),
        name: group.name.clone(),
    };
    let header = div()
        .id(format!("project-group-{}", group.id))
        .group(hover_group.clone())
        .mt(px(5.))
        .flex_col()
        .px(px(7.))
        .rounded(px(6.))
        .flex()
        .gap(px(5.))
        .child(div().id(format!("group-collapse-{}", group.id)).h(px(dunlin_ui::scale::control(29.))).flex().items_center().gap(px(5.)).cursor_pointer().rounded(px(5.)).hover(move |style| style.bg(p.hover)).on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.toggle_project_group(&id, cx);
        }))
        .child(
            Icon::new(if collapsed {
                IconName::ChevronRight
            } else {
                IconName::ChevronDown
            })
            .size(px(11.))
            .text_color(p.muted_fg),
        )
        .child(div().id(format!("group-appearance:{}", group.id)).aria_label("Change group color and icon").size(px(24.)).flex().items_center().justify_center().cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| { cx.stop_propagation(); this.open_group_settings(&appearance_id, window, cx); }))
            .child(crate::project_icons::render_group(this, group, 15., p)))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .truncate()
                .child(name.to_uppercase()),
        )
        .child(
            div()
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.6))
                .child(count.to_string()),
        )
        .child(
            div().id(format!("group-agent-{}", group.id)).aria_label(format!("Start agent for all {count} projects in this group and its subgroups"))
                .size(px(18.)).rounded(px(4.)).flex().items_center().justify_center().cursor_pointer().text_color(p.muted_fg)
                .when(count == 0, |button| button.opacity(0.35))
                .hover(move |style| style.bg(p.hover).text_color(p.fg))
                .on_click(cx.listener(move |this, _, _, cx| { cx.stop_propagation(); if count > 0 { this.start_group_session(&agent_id, cx); } }))
                .tooltip(move |window, cx| dunlin_ui::component::tooltip::Tooltip::new("Start one group conversation, including subgroups. No request is sent until you write a message.").build(window, cx))
                .child(Icon::new(IconName::Bot).size(px(12.))),
        )
        .child(
            div()
                .id(format!("project-group-settings-{}", group.id))
                .aria_label("Group settings · color and image")
                .size(px(24.))
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    { crate::workspace_ui_smoke::probe("group-settings-button") }
                    #[cfg(not(debug_assertions))]
                    { None::<AnyElement> }
                })
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .opacity(0.85)
                .group_hover(hover_group, move |style| style.opacity(1.0))
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_group_settings(&settings_id, window, cx);
                }))
                .tooltip(|window, cx| component::tooltip::Tooltip::new("Group settings · color and image").build(window, cx))
                .child(
                    Icon::new(IconName::Settings2)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                ),
        ));
    projects_context(header, Some(menu_id), groups.to_vec(), cx).into_any_element()
}

fn chat_space_row(
    this: &DunlinApp,
    index: usize,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let Some(project) = this.projects.get(index) else {
        return div().into_any_element();
    };
    let active = index == this.selected_project;
    let repo_context = project
        .settings
        .environment
        .get("DUNLIN_CHAT_CONTEXT")
        .cloned()
        .unwrap_or_else(|| {
            if project.path.starts_with('/') && !project.path.contains("/chat-spaces/") {
                project.path.clone()
            } else {
                String::new()
            }
        });
    let sessions = this
        .all_sessions
        .iter()
        .filter(|session| session.project_id == project.id)
        .count();
    div()
        .id(("chat-space-row", index))
        .h(px(36.))
        .px(px(8.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(8.))
        .cursor_pointer()
        .when(active, |row| {
            row.bg(p.card).border_1().border_color(p.border)
        })
        .when(!active, |row| row.hover(move |style| style.bg(p.hover)))
        .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        .child(
            Icon::new(if repo_context.is_empty() {
                IconName::MessageSquare
            } else {
                IconName::FolderOpen
            })
            .size(px(13.))
            .text_color(p.muted_fg),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.fg)
                        .truncate()
                        .child(project.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(9.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(if repo_context.is_empty() {
                            format!("scratch · {sessions} conversations")
                        } else {
                            repo_context
                        }),
                ),
        )
        .into_any_element()
}

fn project_row(
    this: &DunlinApp,
    index: usize,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let Some(project) = this.projects.get(index) else {
        return div().into_any_element();
    };
    let active = index == this.selected_project;
    let tone = tone_color(p, project.tone);
    let soft = tone_soft(p, project.tone);
    let name = project.name.clone();
    let summary = this
        .repo_summaries
        .values
        .get(&project.id)
        .filter(|row| row.path == project.path);
    let branch =
        if active && !this.git_loading && this.git.error.is_none() && !this.git.branch.is_empty() {
            if this.git.not_repository {
                "No Git repository".into()
            } else {
                this.git.branch.clone()
            }
        } else if let Some(summary) = summary {
            summary.branch.clone().unwrap_or_else(|| {
                if summary.changes.is_some() {
                    "No Git repository"
                } else {
                    "Git unavailable"
                }
                .into()
            })
        } else {
            project.branch.clone().unwrap_or_else(|| "…".into())
        };
    let changes = if active && !this.git_loading && this.git.error.is_none() {
        Some(this.git.changes.len())
    } else {
        summary.and_then(|row| row.changes)
    };
    let drag = crate::project_groups::ProjectDrag {
        id: project.id.clone(),
        name: name.clone(),
    };
    let running = this.running_sessions_for(&project.id);
    let pinned = project.pinned;
    let missing = !project.is_remote() && !Path::new(&project.path).is_dir();
    let hover_group = format!("project-row-hover-{index}");
    let editing = this.project_group_editor == Some(index);
    let meta = {
        let monorepo = crate::local_projects::monorepo_badge(this, project, p);
        let languages = if this.project_list.show_language {
            crate::local_projects::language_badge(this, project, p).or_else(|| crate::project_catalog::primary_language(this, project)
                .map(|name| div().text_size(px(9.5)).text_color(p.muted_fg).child(name).into_any_element()))
        } else { None };
        if monorepo.is_none() && languages.is_none() {
            None
        } else {
            Some(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_end()
                    .justify_center()
                    .gap(px(3.))
                    .children(monorepo)
                    .children(languages),
            )
        }
    };

    let row = div()
        .id(("project-row", index))
        .group(hover_group.clone())
        .min_h(px(dunlin_ui::scale::control(46.)))
        .py(px(dunlin_ui::scale::space(5.)))
        .px(px(8.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(dunlin_ui::scale::space(8.)))
        .cursor_pointer()
        .on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
        .when(active, |this| {
            this.bg(p.card).border_1().border_color(p.border)
        })
        .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
        .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                this.context_menu = Some(ContextMenuState {
                    target: ContextTarget::Project(index),
                    position: event.position,
                });
                cx.notify();
            }),
        )
        .child(crate::project_icons::render(this, project, 26., p))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(
                            div()
                                .flex_1().min_w(px(0.))
                                .text_size(px(12.5))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(p.fg)
                                .when(!this.project_list.wrap_names, |name| name.truncate())
                                .when(this.project_list.wrap_names, |name| name.whitespace_normal())
                                .child(name),
                        )
                        .when(pinned, |this| {
                            this.child(Icon::new(IconName::Pin).size(px(9.)).text_color(p.muted_fg))
                        })
                        .when(missing, |this| {
                            this.child(
                                Icon::new(IconName::TriangleAlert)
                                    .size(px(10.))
                                    .text_color(p.amber),
                            )
                        }),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .when(this.show_project_branches, |line| {
                            line.child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(branch),
                            )
                        })
                        .when(changes.is_some_and(|count| count > 0), |view| {
                            view.child(div().text_size(px(10.5)).text_color(p.amber).child(
                                format!(
                                    "{}{} uncommitted",
                                    if this.show_project_branches {
                                        "· "
                                    } else {
                                        ""
                                    },
                                    changes.unwrap_or(0)
                                ),
                            ))
                        }),
                ),
        )
        .when_some(meta, |row, meta| row.child(meta))
        .when(running > 0, |this| {
            this.child(
                div()
                    .h(px(17.))
                    .min_w(px(17.))
                    .px(px(4.))
                    .rounded_full()
                    .bg(soft)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(tone)
                            .child(running.to_string()),
                    ),
            )
        })
        .child(
            div()
                .id(("project-row-more", index))
                .size(px(20.))
                .flex_shrink_0()
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .when(editing, |this| this.bg(p.accent))
                .when(!editing, |this| {
                    this.opacity(0.0)
                        .group_hover(hover_group, move |style| style.opacity(1.0))
                })
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.project_group_editor = if this.project_group_editor == Some(index) {
                        None
                    } else {
                        Some(index)
                    };
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::Ellipsis)
                        .size(px(12.5))
                        .text_color(p.muted_fg),
                ),
        );
    #[cfg(debug_assertions)]
    let row = row.relative().when(active, |row| row.children(crate::workspace_ui_smoke::probe("active-project-row")));
    let row = crate::project_catalog::attach_preview(row, this, project, cx);
    let mut entry = div().flex().flex_col().child(row);
    if this.project_list.show_topics {
        let topics = crate::project_catalog::topics(this, project);
        if !topics.is_empty() {
            let mut tags = div().pl(px(42.)).pb(px(dunlin_ui::scale::space(5.))).flex().flex_wrap().gap(px(4.));
            for topic in topics {
                let selected = this.project_catalog.topic.as_ref() == Some(&topic);
                tags = tags.child(Button::new(format!("project-topic:{}:{topic}", project.id)).small()
                    .bg(if selected { p.blue_soft } else { p.card }).border_1()
                    .border_color(if selected { p.blue } else { p.border_strong })
                    .text_color(if selected { p.blue } else { p.fg })
                    .label(format!("#{topic}"))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.project_catalog.topic = if this.project_catalog.topic.as_ref() == Some(&topic) { None } else { Some(topic.clone()) };
                        cx.notify();
                    })));
            }
            entry = entry.child(tags);
        }
    }
    entry.into_any_element()
}

fn project_group_picker(
    this: &DunlinApp,
    index: usize,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let current = this
        .projects
        .get(index)
        .and_then(|project| project.group_id.clone());
    let mut chips = div().flex().flex_wrap().gap(px(4.));

    for group in &this.project_groups {
        let selected = current.as_deref() == Some(group.id.as_str());
        let id = group.id.clone();
        let tone = tone_color(p, group.tone);
        let soft = tone_soft(p, group.tone);
        chips = chips.child(
            div()
                .id(format!("project-group-chip-{}", group.id))
                .h(px(22.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(if selected { tone } else { p.border })
                .bg(if selected { soft } else { p.card })
                .flex()
                .items_center()
                .gap(px(4.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.assign_project_group(index, Some(id.clone()), cx);
                }))
                .when(selected, |this| {
                    this.child(Icon::new(IconName::Check).size(px(9.5)).text_color(tone))
                })
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(if selected { tone } else { p.fg })
                        .child(crate::project_tree::group_path(
                            &this.project_groups,
                            &group.id,
                        )),
                ),
        );
    }

    chips = chips
        .child(
            div()
                .id("project-group-chip-none")
                .h(px(22.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(if current.is_none() {
                    p.border_strong
                } else {
                    p.border
                })
                .bg(p.card)
                .flex()
                .items_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.assign_project_group(index, None, cx);
                }))
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child("No group"),
                ),
        )
        .child(
            div()
                .id("project-group-chip-new")
                .h(px(22.))
                .px(px(6.))
                .rounded(px(5.))
                .border_1()
                .border_color(p.border)
                .border_dashed()
                .flex()
                .items_center()
                .gap(px(3.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_new_group_prompt(Some(index), window, cx);
                }))
                .child(
                    Icon::new(IconName::Plus)
                        .size(px(9.5))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child("New"),
                ),
        )
        .child(
            div()
                .id(("project-picker-remove", index))
                .ml_auto()
                .h(px(22.))
                .px(px(6.))
                .flex_shrink_0()
                .rounded(px(5.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .flex()
                .items_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.red_soft))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_group_editor = None;
                    this.request_delete_project(index, cx);
                }))
                .child(div().text_size(px(10.5)).text_color(p.red).child("Remove")),
        );

    div()
        .mx(px(4.))
        .mb(px(3.))
        .p(px(7.))
        .rounded(px(7.))
        .border_1()
        .border_color(p.border)
        .bg(p.subtle)
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child("MOVE TO GROUP"),
        )
        .child(chips)
        .into_any_element()
}

fn files_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let project_name = this.project().name.clone();
    let branch = this
        .project()
        .branch
        .clone()
        .unwrap_or_else(|| "main".into());
    let selected = this
        .explorer
        .selected
        .as_ref()
        .filter(|(project, _, _)| project == &this.project().id)
        .map(|(_, path, _)| path.clone())
        .or_else(|| {
            this.inspector
                .as_ref()
                .map(|inspector| inspector.path.clone())
        });
    let root_path = this.project().path.clone();
    let root_select = root_path.clone();
    let project = this.project().id.clone();
    let app = cx.entity().downgrade();
    let rows = file_rows(
        &this.file_tree.entries,
        selected.as_deref(),
        &project,
        &root_path,
        this.show_file_sizes,
        cx,
    );

    let mut root = div().flex().flex_col().pb(px(10.)).child(
        div()
            .id("explorer-project-root")
            .mx(px(6.))
            .px(px(7.))
            .h(px(dunlin_ui::scale::control(26.)))
            .flex()
            .items_center()
            .gap(px(5.))
            .rounded(px(6.))
            .bg(p.muted)
            .on_click(cx.listener(move |this, _, _, cx| {
                this.select_explorer_entry(&root_select, true);
                cx.notify();
            }))
            .child(
                Icon::new(IconName::ChevronDown)
                    .size(px(11.))
                    .text_color(p.muted_fg),
            )
            .child(
                div()
                    .text_size(px(11.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(p.fg)
                    .truncate()
                    .child(project_name.to_uppercase()),
            )
            .child(div().flex_1())
            .when(this.show_file_sizes, |row| {
                row.child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(crate::app::format_size(this.file_tree.total_size)),
                )
            })
            .child(
                div()
                    .text_size(px(11.0))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg)
                    .child(branch),
            )
            .context_menu(move |menu, _, _| creation_menu(menu, &app, &root_path)),
    );

    if let Some(error) = this.file_tree.error.clone() {
        root = root.child(
            div()
                .mx(px(8.))
                .mt(px(8.))
                .p(px(9.))
                .rounded(px(7.))
                .bg(p.amber_soft)
                .text_size(px(11.5))
                .text_color(p.amber)
                .child(error),
        );
    } else if rows.is_empty() {
        root = root.child(
            div()
                .mx(px(8.))
                .mt(px(8.))
                .p(px(9.))
                .text_size(px(11.0))
                .text_color(p.muted_fg)
                .child(if this.explorer.tree_loading {
                    "Loading files…"
                } else {
                    "No files to show"
                }),
        );
    } else {
        root = root.children(rows);
    }
    root.into_any_element()
}

fn file_rows(
    entries: &[FileEntry],
    selected: Option<&str>,
    project: &str,
    root: &str,
    show_sizes: bool,
    cx: &mut Context<DunlinApp>,
) -> Vec<AnyElement> {
    let p = palette(cx);
    let mut rows = Vec::new();

    for entry in entries {
        let path = entry.path.clone();
        let select_path = path.clone();
        let name = entry.name.clone();
        let is_dir = entry.is_dir;
        let expanded = entry.expanded;
        let depth = entry.depth;
        let active = selected == Some(path.as_str());
        let (icon_name, icon_color) = if is_dir {
            (
                if expanded {
                    IconName::FolderOpen
                } else {
                    IconName::Folder
                },
                p.blue,
            )
        } else {
            crate::file_icons::appearance(&name, &p)
        };

        let mut row = div()
            .id(format!("file-row:{path}"))
            .h(px(dunlin_ui::scale::control(24.)))
            .pl(px(7. + depth as f32 * 13.))
            .pr(px(8.))
            .flex()
            .items_center()
            .gap(px(5.))
            .cursor_pointer()
            .when(active, |this| this.bg(p.accent))
            .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.select_explorer_entry(&select_path, is_dir);
                if is_dir {
                    this.toggle_folder(&path, cx);
                } else {
                    this.open_file(&path, window, cx);
                    cx.notify();
                }
            }));
        if show_sizes {
            let kind = if is_dir { "Folder" } else { "File" };
            let size = crate::app::format_size(entry.size);
            row = row.tooltip(move |window, cx| {
                component::tooltip::Tooltip::new(format!("{kind} · {size}")).build(window, cx)
            });
        }

        if is_dir {
            row = row.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size(px(10.))
                .text_color(p.muted_fg.opacity(0.7)),
            );
        } else {
            row = row.child(div().w(px(10.)));
        }

        row = row
            .child(
                Icon::new(icon_name)
                    .size(px(14.))
                    .flex_shrink_0()
                    .text_color(icon_color),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(if active { p.fg } else { p.fg.opacity(0.86) })
                    .truncate()
                    .child(name),
            );

        let parent = if is_dir {
            entry.path.clone()
        } else {
            Path::new(&entry.path)
                .parent()
                .unwrap_or(Path::new(root))
                .to_string_lossy()
                .to_string()
        };
        let app = cx.entity().downgrade();
        let drag = crate::explorer::EntryDrag {
            project: project.into(),
            root: root.into(),
            path: entry.path.clone(),
            directory: is_dir,
        };
        let menu_path = entry.path.clone();
        let row = crate::explorer::drop_target(row, parent.clone(), cx)
            .on_drag(drag, |entry, _, _, cx| cx.new(|_| entry.clone()))
            .context_menu(move |menu, _, _| {
                entry_menu(creation_menu(menu, &app, &parent), &app, &menu_path, is_dir)
            });
        rows.push(row.into_any_element());

        if is_dir && expanded {
            rows.extend(file_rows(
                &entry.children,
                selected,
                project,
                root,
                show_sizes,
                cx,
            ));
        }
    }

    rows
}

fn creation_menu(mut menu: PopupMenu, app: &WeakEntity<DunlinApp>, parent: &str) -> PopupMenu {
    for (label, directory) in [("New file…", false), ("New folder…", true)] {
        let app = app.clone();
        let parent = parent.to_string();
        menu = menu.item(
            menu_item(move |_, _| div().px(px(4.)).py(px(3.)).child(label)).on_click(
                move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| {
                        this.new_explorer_entry(parent.clone(), directory, window, cx)
                    });
                },
            ),
        );
    }
    menu
}

fn entry_menu(
    mut menu: PopupMenu,
    app: &WeakEntity<DunlinApp>,
    path: &str,
    directory: bool,
) -> PopupMenu {
    for (label, action) in [("Rename…", 0), ("Delete…", 1), ("Reveal in Finder", 2)] {
        let app = app.clone();
        let path = path.to_string();
        menu = menu.item(
            menu_item(move |_, _| div().px(px(4.)).py(px(3.)).child(label)).on_click(
                move |_, window, cx| {
                    let _ = app.update(cx, |this, cx| match action {
                        0 => this.rename_explorer_entry(path.clone(), directory, window, cx),
                        1 => this.delete_explorer_entry(path.clone(), window, cx),
                        _ => this.reveal_explorer_entry(&path, cx),
                    });
                },
            ),
        );
    }
    menu
}

fn search_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let root = this.project().path.clone();
    let query = this.search_query.clone();
    let hits = this.search_hits.clone();
    let loading = this.search_loading;

    let input = Input::new(&this.search_input)
        .appearance(false)
        .bordered(false)
        .cleanable(true)
        .h(px(28.))
        .prefix(
            Icon::new(IconName::Search)
                .size(px(12.5))
                .text_color(p.muted_fg),
        )
        .suffix(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.65))
                .child(if this.show_shortcut_hints { "↵" } else { "" }),
        );

    let mut root_el = div()
        .flex()
        .flex_col()
        .pb(px(10.))
        .child(
            div()
                .mx(px(6.))
                .px(px(6.))
                .h(px(30.))
                .rounded(px(7.))
                .border_1()
                .border_color(p.border)
                .bg(p.card)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(div().flex_1().min_w(px(0.)).child(input))
                .child(
                    div()
                        .id("run-search")
                        .h(px(21.))
                        .px(px(6.))
                        .rounded(px(5.))
                        .bg(p.fg)
                        .flex()
                        .items_center()
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| this.run_search(cx)))
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.bg)
                                .child("Run"),
                        ),
                ),
        )
        .child(
            div()
                .px(px(12.))
                .h(px(26.))
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(if query.is_empty() {
                            "RECENT MATCHES".to_string()
                        } else {
                            format!("{} RESULTS", hits.len())
                        }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(p.muted_fg.opacity(0.6))
                        .child("project"),
                ),
        );

    for hit in &hits {
        let path = &hit.path;
        let line = hit.line;
        let preview = &hit.preview;
        let display = Path::new(path)
            .strip_prefix(&root)
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|_| path.clone());
        let file = Path::new(&display)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&display)
            .to_string();
        let directory = Path::new(&display)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default();
        let hit_path = path.clone();
        let hit_line = line;
        let (file_icon, file_color) = crate::file_icons::appearance(&file, &p);

        root_el = root_el.child(
            div()
                .id(format!("search-hit:{path}:{line}"))
                .px(px(9.))
                .py(px(6.))
                .mx(px(6.))
                .rounded(px(6.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_search_hit(&hit_path, hit_line, window, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.))
                        .child(Icon::new(file_icon).size(px(11.5)).text_color(file_color))
                        .child(
                            div()
                                .text_size(px(12.0))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(file),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.7))
                                .truncate()
                                .child(directory),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child(format!(":{line}")),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .line_height(relative(1.45))
                        .truncate()
                        .child(preview.clone()),
                ),
        );
    }

    if loading && hits.is_empty() {
        root_el = root_el.child(empty_state(
            IconName::LoaderCircle,
            "Searching…",
            "Scanning the project in the background.",
            &p,
        ));
    } else if hits.is_empty() {
        root_el = root_el.child(empty_state(
            IconName::SearchX,
            "No matches yet",
            "Type a query and press Enter to search the whole project.",
            &p,
        ));
    }

    root_el.into_any_element()
}

fn source_control_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.ensure_ai_commit_auth(cx);
    let ai = this.ai_commit_available();
    let preparing = this.ai_commit_preparing();
    let p = palette(cx);
    if this.git.not_repository
        || this
            .git
            .error
            .as_ref()
            .is_some_and(|error| error.contains("not a git repository"))
    {
        return div().p(px(14.)).flex().flex_col().gap(px(10.))
            .child(Icon::new(IconName::GitBranch).size(px(24.)).text_color(p.muted_fg))
            .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(p.fg).child("No Git repository"))
            .child(div().text_size(px(11.5)).line_height(relative(1.5)).text_color(p.muted_fg).child("You can edit files normally. Initialize Git here to track changes, or open an existing repository."))
            .child(div().id("initialize-repository").px(px(10.)).py(px(7.)).rounded(px(6.)).bg(p.fg).text_color(p.bg).text_size(px(11.5)).cursor_pointer()
                .on_click(cx.listener(|this, _, _, cx| this.initialize_repository(cx)))
                .child(if this.git_loading { "Initializing…" } else { "Initialize repository" }))
            .into_any_element();
    }
    let branch = if this.git.branch.is_empty() {
        this.project()
            .branch
            .clone()
            .unwrap_or_else(|| "main".into())
    } else {
        this.git.branch.clone()
    };
    let mut changes = this.git.changes.clone();
    changes.sort_by_key(|change| !change.staged);
    let staged_count = changes.iter().filter(|change| change.staged).count();
    let worktree_count = changes.len().saturating_sub(staged_count);
    let loading = this.git_loading;
    let error = this.git.error.clone();
    let change_count = changes.len();
    let commit_enabled = preparing || change_count > 0;

    let header = div()
        .mx(px(6.))
        .p(px(9.))
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .flex()
        .flex_col()
        .gap(px(7.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(crate::branch_picker::badge(
                    this,
                    &this.project().id,
                    &branch,
                    "source-control-branch-picker",
                    cx,
                ))
                .child(div().flex_1())
                .child(
                    div()
                        .id("git-refresh")
                        .aria_label("Refresh source control")
                        .size(px(18.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .hover(move |style| style.bg(p.hover))
                        .on_click(cx.listener(|this, _, _, cx| this.refresh_git_status(cx)))
                        .child(
                            Icon::new(if loading {
                                IconName::LoaderCircle
                            } else {
                                IconName::RefreshCw
                            })
                            .size(px(11.))
                            .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!(
                            "{change_count} {}",
                            if change_count == 1 {
                                "change"
                            } else {
                                "changes"
                            }
                        )),
                ),
        )
        .when(!ai, |header| {
            header.child(
                div()
                    .h(px(26.))
                    .px(px(6.))
                    .rounded(px(5.))
                    .border_1()
                    .border_color(p.border)
                    .bg(p.sidebar)
                    .flex()
                    .items_center()
                    .child(
                        Input::new(&this.commit_input)
                            .appearance(false)
                            .bordered(false)
                            .h(px(24.)),
                    ),
            )
        })
        .child(
            div()
                .id("commit-button")
                .h(px(25.))
                .rounded(px(5.))
                .bg(p.fg)
                .flex()
                .items_center()
                .justify_center()
                .gap(px(5.))
                .when(!commit_enabled, |button| button.opacity(0.4))
                .when(commit_enabled, |button| {
                    button
                        .cursor_pointer()
                        .hover(move |style| style.opacity(0.9))
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.ai_commit_preparing() {
                                this.cancel_ai_commit_preparation();
                                cx.notify();
                            } else if this.ai_commit_available() {
                                this.request_ai_commit(window, cx);
                            } else {
                                this.commit_changes(window, cx);
                            }
                        }))
                })
                .child(
                    Icon::new(IconName::GitCommitHorizontal)
                        .size(px(11.5))
                        .text_color(p.bg),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.bg)
                        .child(if preparing {
                            "Cancel preparation"
                        } else if ai {
                            "Commit on AI"
                        } else {
                            "Commit staged"
                        }),
                ),
        )
        .child(
            div()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .truncate()
                .child(this.ai_commit_hint()),
        )
        .when(this.ai_commit_has_auth(), |header| {
            header.child(
                div()
                    .id("toggle-manual-commit")
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ai_commit.manual = !this.ai_commit.manual;
                        cx.notify();
                    }))
                    .child(if ai { "Manual commit" } else { "Use AI commit" }),
            )
        });

    let mut list = div().flex().flex_col().pb(px(10.)).child(header).child(
        div()
            .px(px(12.))
            .h(px(28.))
            .flex()
            .items_center()
            .gap(px(5.))
            .child(
                div()
                    .text_size(px(11.0))
                    .font_family(dunlin_ui::theme::code_font())
                    .text_color(p.muted_fg)
                    .child("CHANGES"),
            )
            .child(div().flex_1())
            .child(
                div()
                    .id("stage-all")
                    .text_size(px(11.5))
                    .text_color(if changes.is_empty() {
                        p.muted_fg.opacity(0.5)
                    } else {
                        p.blue
                    })
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.stage_all_changes(cx)))
                    .child("Stage all"),
            ),
    );

    if !changes.is_empty() {
        let mut actions = div().px(px(12.)).py(px(5.)).flex().flex_wrap().gap(px(12.));
        for (label, action) in [
            ("Unstage all", dunlin_workspace::git::operations::Action::UnstageAll),
            ("Discard all changes", dunlin_workspace::git::operations::Action::Discard {
                paths: changes.iter().map(|c| c.path.clone()).collect::<std::collections::BTreeSet<_>>().into_iter().collect(),
            }),
        ] {
            let owner = this.project().id.clone();
            actions = actions.child(div().id(format!("source-control:{label}")).text_size(px(11.))
                .text_color(if label.starts_with("Discard") { p.red } else { p.blue }).cursor_pointer().child(label)
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.project().id == owner { this.request_git_action(action.clone(), window, cx); }
                })));
        }
        list = list.child(actions);
    }

    if let Some(error) = error {
        list = list.child(
            div()
                .mx(px(6.))
                .p(px(9.))
                .rounded(px(7.))
                .bg(p.red_soft)
                .flex()
                .items_start()
                .gap(px(7.))
                .child(
                    Icon::new(IconName::CircleAlert)
                        .size(px(12.))
                        .text_color(p.red),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .line_height(relative(1.5))
                        .text_color(p.red)
                        .child(error),
                ),
        );
    } else if changes.is_empty() {
        list = list.child(
            div()
                .mx(px(6.))
                .py(px(22.))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .child(
                    Icon::new(if loading {
                        IconName::LoaderCircle
                    } else {
                        IconName::CircleCheck
                    })
                    .size(px(18.))
                    .text_color(if loading { p.muted_fg } else { p.green }),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.muted_fg)
                        .child(if loading {
                            "Reading the working tree…"
                        } else {
                            "Working tree clean"
                        }),
                ),
        );
    }

    let mut section = None;
    for change in &changes {
        if section != Some(change.staged) {
            section = Some(change.staged);
            let label = if change.staged {
                format!("STAGED · {staged_count}")
            } else {
                format!("CHANGES · {worktree_count}")
            };
            list = list.child(group_label(&label, &p));
        }
        let path = change.path.clone();
        let file_name = Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&path)
            .to_string();
        let status = change.status;
        let (tone, soft) = match status {
            'A' | 'U' => (p.green, p.green_soft),
            'D' => (p.red, p.red_soft),
            'R' => (p.violet, p.violet_soft),
            _ => (p.amber, p.amber_soft),
        };
        let label = status.to_string();
        let additions = change.additions;
        let deletions = change.deletions;
        let staged = change.staged;
        let side = if staged { "staged" } else { "worktree" };
        let group = format!("change-row:{side}:{path}");
        let action_path = path.clone();
        let diff_path = path.clone();
        let discard_path = path.clone();
        let menu_path = path.clone();
        let menu_project = this.project().id.clone();
        let app = cx.entity().downgrade();
        let discard_button = (!staged && status != 'U').then(|| {
            div()
                .id(format!("change-discard:{side}:{discard_path}"))
                .aria_label("Discard working tree changes")
                .size(px(18.))
                .flex_shrink_0()
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .opacity(0.35)
                .group_hover(group.clone(), move |style| style.opacity(1.))
                .hover(move |style| style.bg(p.red_soft))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.request_discard_file(&discard_path, window, cx);
                }))
                .child(Icon::new(IconName::Undo2).size(px(10.5)).text_color(p.red))
                .into_any_element()
        });

        list = list.child(
            div()
                .id(format!("change:{side}:{path}"))
                .group(group.clone())
                .mx(px(6.))
                .px(px(7.))
                .h(px(dunlin_ui::scale::control(29.)))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(6.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.open_diff(&path, staged, cx);
                    cx.notify();
                }))
                .child(
                    div()
                        .size(px(15.))
                        .rounded(px(4.))
                        .bg(soft)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .font_weight(FontWeight::BOLD)
                                .text_color(tone)
                                .child(label),
                        ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(px(11.5))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.fg.opacity(0.85))
                        .truncate()
                        .child(file_name),
                )
                .when(staged, |this| {
                    this.child(div().size(px(4.)).rounded_full().bg(p.green.opacity(0.8)))
                })
                .when(additions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.green)
                            .child(format!("+{additions}")),
                    )
                })
                .when(deletions > 0, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.red)
                            .child(format!("−{deletions}")),
                    )
                })
                .children(discard_button)
                .child(
                    div()
                        .id(format!("change-diff:{side}:{diff_path}"))
                        .aria_label(if staged {
                            "Show staged diff"
                        } else {
                            "Show working tree diff"
                        })
                        .size(px(18.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .opacity(0.35)
                        .group_hover(group.clone(), move |style| style.opacity(1.))
                        .hover(move |style| style.bg(p.blue_soft))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.open_diff(&diff_path, staged, cx);
                        }))
                        .child(
                            Icon::new(IconName::Diff)
                                .size(px(10.5))
                                .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id(format!("change-stage:{side}:{action_path}"))
                        .aria_label(if staged { "Unstage file" } else { "Stage file" })
                        .size(px(18.))
                        .flex_shrink_0()
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .opacity(0.35)
                        .group_hover(group, move |style| style.opacity(1.))
                        .hover(move |style| {
                            style.bg(if staged { p.amber_soft } else { p.green_soft })
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            if staged {
                                this.unstage_file(&action_path, cx);
                            } else {
                                this.stage_file(&action_path, cx);
                            }
                        }))
                        .child(
                            Icon::new(if staged {
                                IconName::Minus
                            } else {
                                IconName::Plus
                            })
                            .size(px(10.5))
                            .text_color(if staged {
                                p.amber
                            } else {
                                p.green
                            }),
                        ),
                )
                .context_menu(move |mut menu, _, _| {
                    for (label, action) in [
                        ("Discard changes…", dunlin_workspace::git::operations::Action::Discard { paths: vec![menu_path.clone()] }),
                        ("Add to .gitignore…", dunlin_workspace::git::operations::Action::Ignore { path: menu_path.clone() }),
                    ] {
                        let app = app.clone(); let owner = menu_project.clone();
                        menu = menu.item(menu_item(move |_, _| div().px(px(4.)).py(px(3.)).child(label))
                            .on_click(move |_, window, cx| {
                                if let Some(app) = app.upgrade() { app.update(cx, |this, cx| {
                                    if this.project().id == owner { this.request_git_action(action.clone(), window, cx); }
                                }); }
                            }));
                    }
                    menu
                }),
        );
    }

    list.into_any_element()
}

fn sessions_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    use crate::session_list::Scope;

    this.refresh_session_list(cx);
    div()
        .flex()
        .flex_col()
        .pb(px(10.))
        .child(sessions_section(this, Scope::General, cx))
        .when(!this.project().is_chat_space(), |list| {
            list.child(sessions_section(this, Scope::Project, cx))
        })
        .into_any_element()
}

fn sessions_section(
    this: &DunlinApp,
    scope: crate::session_list::Scope,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    use crate::session_list::Scope;

    let p = palette(cx);
    let mut sessions: Vec<_> = this
        .all_sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| {
            this.session_scope(session) == Some(scope) && this.session_visible(session)
        })
        .collect();
    sessions.sort_by_cached_key(|(_, session)| {
        std::cmp::Reverse(chrono::DateTime::parse_from_rfc3339(&session.updated_at).ok())
    });

    let count = sessions.len();
    let title = match scope {
        Scope::General => format!("General sessions · {count}"),
        Scope::Project => format!("Project · {} · {count}", this.project().name),
    };
    let mut list = div().flex().flex_col().child(
        div()
            .px(px(12.))
            .py(px(8.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(p.fg)
            .truncate()
            .child(title),
    );
    #[cfg(debug_assertions)]
    {
        list = list.relative().children(crate::workspace_ui_smoke::probe(match scope {
            Scope::General => "general-sessions",
            Scope::Project => "current-project-sessions",
        }));
    }
    if count == 0 {
        list = list.child(
            div()
                .px(px(12.))
                .py(px(8.))
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child(if scope == Scope::General {
                    "No general conversations yet. Start a standalone chat or a group session."
                } else {
                    "No conversations in this project yet. Send a message to start one."
                }),
        );
    }

    let mut last_group = None;
    for (index, session) in sessions {
        let active = session.id == this.session().id;
        let open_id = session.id.clone();
        let context_id = session.id.clone();
        let delete_id = session.id.clone();
        let context = this
            .group_contexts
            .get(&session.id)
            .map(|context| format!("Group · {}", context.name))
            .or_else(|| {
                this.projects.iter().find(|project| project.id == session.project_id)
                    .filter(|project| project.is_chat_space())
                    .map(|project| project.name.clone())
            });
        let (status_color, status_soft) = status_palette(&p, session.status);
        let title = session.title.clone();
        let harness = session.harness.clone();
        let model = session.model.clone();
        let updated = crate::app::format_session_date(&session.updated_at);
        let group_key = updated.to_uppercase();
        if last_group.as_deref() != Some(group_key.as_str()) {
            list = list.child(group_label(&group_key, &p));
            last_group = Some(group_key);
        }
        let group = format!("session-row-{index}");

        list = list.child(
            div()
                .id(("session-row", index))
                .group(group.clone())
                .mx(px(6.))
                .px(px(8.))
                .py(px(7.))
                .rounded(px(7.))
                .flex()
                .flex_col()
                .gap(px(4.))
                .cursor_pointer()
                .when(active, |this| {
                    this.bg(p.card).border_1().border_color(p.border)
                })
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.open_session_id(&open_id, cx) {
                        this.show_chat_tab(window, cx);
                    }
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.context_menu = Some(ContextMenuState {
                            target: ContextTarget::Session(context_id.clone()),
                            position: event.position,
                        });
                        cx.notify();
                    }),
                )
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap(px(6.))
                        .child(
                            div()
                                .mt(px(4.))
                                .size(px(5.))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(status_color),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .text_size(px(12.0))
                                .font_weight(if active {
                                    FontWeight::MEDIUM
                                } else {
                                    FontWeight::NORMAL
                                })
                                .line_height(relative(1.35))
                                .text_color(p.fg)
                                .truncate()
                                .child(title),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_size(px(10.5))
                                .text_color(p.muted_fg.opacity(0.75))
                                .child(updated),
                        ),
                )
                .when_some(context, |row, context| {
                    row.child(div().pl(px(11.)).text_size(px(10.5)).text_color(p.muted_fg).truncate().child(context))
                })
                .child(
                    div()
                        .pl(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .child(
                            div()
                                .h(px(16.))
                                .px(px(5.))
                                .rounded_full()
                                .bg(status_soft)
                                .flex()
                                .items_center()
                                .child(
                                    div()
                                        .text_size(px(10.5))
                                        .font_family(dunlin_ui::theme::code_font())
                                        .text_color(status_color)
                                        .child(session.status.label().to_uppercase()),
                                ),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(px(11.0))
                                .text_color(p.muted_fg)
                                .child(format!("{harness} · {model}")),
                        )
                        .child(div().flex_1())
                        .child(
                            div()
                                .id(("session-delete", index))
                                .size(px(18.))
                                .flex_shrink_0()
                                .rounded(px(4.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .opacity(0.35)
                                .group_hover(group, move |style| style.opacity(1.))
                                .hover(move |style| style.bg(p.red_soft))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.request_delete_session_id(&delete_id, cx);
                                }))
                                .child(
                                    Icon::new(IconName::Trash)
                                        .size(px(10.5))
                                        .text_color(p.muted_fg),
                                ),
                        ),
                ),
        );
    }

    list.into_any_element()
}

fn usage_footer(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let usages = this.active_usages();
    let refreshing = this.usage_request.as_ref() == Some(&this.usage_target());

    let mut footer = div()
        .h(px(if this.account_usage_collapsed {
            36.
        } else {
            184.
        }))
        .overflow_hidden()
        .flex_shrink_0()
        .border_t_1()
        .border_color(p.border)
        .bg(p.sidebar)
        .px(px(10.))
        .py(px(8.))
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.muted_fg)
                        .child(if this.project().is_remote() {
                            "ACCOUNT USAGE · RELAY"
                        } else {
                            "ACCOUNT USAGE"
                        }),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("toggle-account-usage")
                        .aria_label("Collapse or expand account usage")
                        .size(px(18.))
                        .cursor_pointer()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.account_usage_collapsed = !this.account_usage_collapsed;
                            this.persist_settings(cx);
                            cx.notify();
                        }))
                        .child(
                            Icon::new(if this.account_usage_collapsed {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(12.))
                            .text_color(p.muted_fg),
                        ),
                )
                .child(
                    div()
                        .id("usage-details")
                        .text_size(px(11.0))
                        .text_color(p.muted_fg)
                        .cursor_pointer()
                        .hover(move |style| style.text_color(p.fg))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings_open = true;
                            this.settings_tab = SettingsTab::Usage;
                            cx.notify();
                        }))
                        .child("Details"),
                ),
        );

    if this.account_usage_collapsed {
        return footer.into_any_element();
    }

    let mut content = div()
        .id("account-usage-rows")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(6.));

    for usage in usages {
        let tone = tone_color(&p, usage.accent);
        content = content.child(
            div()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(dot(tone, 4.5))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(px(11.5))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(p.fg.opacity(0.88))
                        .child(usage.provider.clone()),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .text_size(px(10.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(usage.plan.clone()),
                ),
        );
        if usage.windows.is_empty() {
            let detail = usage.detail.clone();
            content = content.child(
                div()
                    .id("usage-unavailable-detail")
                    .flex_shrink_0()
                    .max_h(px(44.))
                    .overflow_hidden()
                    .tooltip(move |window, cx| {
                        dunlin_ui::component::tooltip::Tooltip::new(detail.clone())
                            .build(window, cx)
                    })
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child(usage.detail.clone()),
            );
        }
        for window in &usage.windows {
            let percent = window.used_percent.clamp(0.0, 100.0);

            content = content.child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(5.))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_1()
                                    .truncate()
                                    .text_size(px(10.5))
                                    .text_color(p.muted_fg)
                                    .child(window.label.clone()),
                            )
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_size(px(10.0))
                                    .text_color(p.muted_fg)
                                    .child(window.resets_in.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .font_family(dunlin_ui::theme::code_font())
                                    .text_color(p.muted_fg)
                                    .child(format!("{percent:.0}%")),
                            ),
                    )
                    .child(
                        div()
                            .h(px(3.))
                            .w_full()
                            .rounded_full()
                            .bg(p.muted)
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .w(relative(percent / 100.0))
                                    .rounded_full()
                                    .bg(if percent > 85. { p.red } else { tone }),
                            ),
                    ),
            );
        }
    }

    footer = footer.child(content).child(
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .id("usage-refresh-inline")
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .cursor_pointer()
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_usage(cx)))
                    .child(if refreshing {
                        "Refreshing…"
                    } else {
                        "Refresh"
                    }),
            )
            .when(this.local_opencode(), |row| {
                row.child(
                    div()
                        .id("usage-open-console")
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .cursor_pointer()
                        .on_click(|_, _, cx| cx.open_url("https://opencode.ai/console"))
                        .child("Open Console ↗"),
                )
            }),
    );

    footer.into_any_element()
}

fn project_menu_row(
    this: &DunlinApp,
    index: usize,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let Some(project) = this.projects.get(index) else {
        return div().into_any_element();
    };
    let active = index == this.selected_project;
    let name = project.name.clone();

    let row = div()
        .id(("project-menu-row", index))
        .h(px(36.))
        .flex_shrink_0()
        .px(px(6.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .when(active, |this| this.bg(p.accent))
        .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.select_project(index, cx);
            if this.selected_project == index {
                this.project_menu_open = false;
                cx.notify();
            }
        }))
        .child(div().w(px(12.)).flex_shrink_0())
        .child(crate::project_icons::render(this, project, 20., p))
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.0))
                .font_weight(FontWeight::NORMAL)
                .text_color(p.fg)
                .truncate()
                .child(name),
        )
        .when(active, |this| {
            this.child(Icon::new(IconName::Check).size(px(11.)).text_color(p.fg))
        });
    #[cfg(debug_assertions)]
    let row = row.relative()
        .when(active, |r| r.children(crate::workspace_ui_smoke::probe("project-menu-active-row")))
        .when(project.id == "picker-top-level", |r| r.children(crate::workspace_ui_smoke::probe("project-menu-ungrouped-row")));
    row.into_any_element()
}

pub(crate) fn project_menu(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let trusted = this.project().trusted;
    let pinned = this.project().pinned;
    let missing = !this.project().is_remote() && !Path::new(&this.project().path).is_dir();
    let mut menu = div()
        .id("project-menu")
        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
            this.project_menu_open = false;
            cx.stop_propagation();
            cx.notify();
        }))
        .popover_style(cx)
        .occlude()
        .absolute()
        .top(px(44.))
        .left(px(80.))
        .w(px(360.))
        .max_w_full()
        .max_h(relative(0.85))
        .overflow_y_scroll()
        .p(px(5.))
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(
            div()
                .px(px(8.))
                .py(px(5.))
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg)
                .child("SWITCH PROJECT"),
        );

    let groups = this.project_groups.clone();
    for (group_index, depth) in crate::project_tree::visible_groups(
        &groups, &this.projects, &this.collapsed_project_groups,
        crate::project_tree::Sort::Name, &Default::default(),
    ) {
        let group = &groups[group_index];
        let collapsed = this.collapsed_project_groups.contains(&group.id);
        let id = group.id.clone();
        let header = div()
            .id(format!("project-menu-group:{}", group.id))
            .aria_label(format!("{} group · {}", group.name, if collapsed { "expand" } else { "collapse" }))
            .h(px(32.)).flex_shrink_0().ml(px(depth.min(8) as f32 * 14.))
            .px(px(6.)).rounded(px(6.)).flex().items_center().gap(px(7.))
            .cursor_pointer().hover(move |s| s.bg(p.hover))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_project_group(&id, cx)))
            .child(Icon::new(if collapsed { IconName::ChevronRight } else { IconName::ChevronDown }).size(px(12.)).text_color(p.muted_fg))
            .child(crate::project_icons::render_group(this, group, 18., &p))
            .child(div().flex_1().min_w(px(0.)).text_size(px(12.)).text_color(p.fg).truncate().child(group.name.clone()));
        #[cfg(debug_assertions)]
        let header = header.relative().children(crate::workspace_ui_smoke::probe(if group.id == "picker-root" { "project-menu-root" } else { "project-menu-child" }));
        menu = menu.child(header);
        if collapsed { continue; }
        let mut indices: Vec<usize> = this
            .projects
            .iter()
            .enumerate()
            .filter(|(_, project)| project.group_id.as_deref() == Some(group.id.as_str()))
            .map(|(index, _)| index)
            .collect();
        indices.sort_by_key(|index| {
            (
                !this.projects[*index].pinned,
                this.projects[*index].name.to_lowercase(),
            )
        });
        for index in indices {
            menu = menu.child(div().flex_shrink_0().ml(px((depth + 1).min(9) as f32 * 14.)).child(project_menu_row(this, index, cx, &p)));
        }
    }

    let ungrouped: Vec<usize> = this
        .projects
        .iter()
        .enumerate()
        .filter(|(_, project)| project.group_id.as_ref().is_none_or(|id| !groups.iter().any(|g| &g.id == id)))
        .map(|(index, _)| index)
        .collect();
    let ungrouped = {
        let mut indices = ungrouped;
        indices.sort_by_key(|index| {
            (
                !this.projects[*index].pinned,
                this.projects[*index].name.to_lowercase(),
            )
        });
        indices
    };
    if !groups.is_empty() && !ungrouped.is_empty() {
        menu = menu.child(
            div()
                .px(px(8.))
                .pt(px(6.))
                .pb(px(2.))
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.75))
                .child("UNGROUPED"),
        );
    }
    for index in ungrouped {
        menu = menu.child(project_menu_row(this, index, cx, &p));
    }

    let menu = menu.child(div().h(px(1.)).my(px(3.)).bg(p.border))
        .child(
            div()
                .id("menu-new-group")
                .h(px(30.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.project_menu_open = false;
                    this.open_new_group_prompt(None, window, cx);
                }))
                .child(
                    Icon::new(IconName::FolderTree)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(p.fg)
                        .child("New project group…"),
                ),
        )
        .child(
            div()
                .id("menu-open-folder")
                .h(px(32.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.open_folder(cx);
                }))
                .child(
                    Icon::new(IconName::FolderPlus)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(p.fg)
                        .child("Add projects…"),
                )
                .when(this.show_shortcut_hints, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(p.muted_fg)
                            .child("⌘O"),
                    )
                }),
        )
        .child(
            div()
                .id("project-add-remote")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.open_remote_picker(cx);
                }))
                .child(
                    Icon::new(IconName::Cloud)
                        .size(px(13.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.0))
                        .text_color(p.fg)
                        .child("Add remote project…"),
                ),
        )
        .child(
            div()
                .id("project-move-group")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.section = SidebarSection::Projects;
                    this.project_group_editor = Some(this.selected_project);
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(
                    Icon::new(IconName::FolderTree)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.0))
                        .text_color(p.fg)
                        .child("Move to group…"),
                ),
        )
        .child(
            div()
                .id("project-pin")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.toggle_project_pin(cx);
                }))
                .child(
                    Icon::new(IconName::Pin)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.0))
                        .text_color(p.fg)
                        .child(if pinned {
                            "Unpin project"
                        } else {
                            "Pin project"
                        }),
                ),
        )
        .child(
            div()
                .id("project-settings")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    let index = this.selected_project;
                    this.open_project_settings(index, cx);
                }))
                .child(
                    Icon::new(IconName::Settings)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.fg)
                        .child("Project settings"),
                ),
        )
        .child(
            div()
                .id("project-trust")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.toggle_project_trust(cx);
                }))
                .child(
                    Icon::new(if trusted {
                        IconName::ShieldCheck
                    } else {
                        IconName::ShieldAlert
                    })
                    .size(px(12.))
                    .text_color(if trusted { p.green } else { p.amber }),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(p.fg)
                        .child(if trusted {
                            "Untrust project"
                        } else {
                            "Trust project"
                        }),
                ),
        )
        .when(missing, |this| {
            this.child(
                div()
                    .id("project-locate")
                    .h(px(28.))
                    .px(px(7.))
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.project_menu_open = false;
                        this.locate_project(cx);
                    }))
                    .child(
                        Icon::new(IconName::FolderSearch)
                            .size(px(12.))
                            .text_color(p.amber),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.0))
                            .text_color(p.fg)
                            .child("Locate folder…"),
                    ),
            )
        })
        .child(
            div()
                .id("project-reveal")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    if this.project().is_remote() {
                        this.set_notice("Remote project · files live on the relay", cx);
                        return;
                    }
                    let path = this.project().path.clone();
                    cx.reveal_path(Path::new(&path));
                    this.set_notice("Revealed the project in Finder", cx);
                }))
                .child(
                    Icon::new(IconName::FolderOpen)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(p.fg)
                        .child("Reveal in Finder"),
                ),
        )
        .child(
            div()
                .id("project-copy-path")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    let path = this.project().path.clone();
                    cx.write_to_clipboard(ClipboardItem::new_string(path));
                    this.set_notice("Copied the project path", cx);
                }))
                .child(
                    Icon::new(IconName::Copy)
                        .size(px(12.))
                        .text_color(p.muted_fg),
                )
                .child(
                    div()
                        .flex_1()
                        .text_size(px(11.5))
                        .text_color(p.fg)
                        .child("Copy path"),
                ),
        )
        .child(div().h(px(1.)).my(px(3.)).bg(p.border))
        .child(
            div()
                .id("project-remove")
                .h(px(28.))
                .px(px(7.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(7.))
                .cursor_pointer()
                .hover(move |style| style.bg(p.red_soft))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.project_menu_open = false;
                    this.request_delete_project(this.selected_project, cx);
                }))
                .child(Icon::new(IconName::Trash).size(px(12.)).text_color(p.red))
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(p.red)
                            .child("Remove project…"),
                ),
        );
    #[cfg(debug_assertions)]
    let menu = menu.children(crate::workspace_ui_smoke::probe("title-project-menu"));
    menu.into_any_element()
}

fn group_label(label: &str, p: &Palette) -> AnyElement {
    div()
        .px(px(12.))
        .h(px(24.))
        .flex()
        .items_center()
        .child(
            div()
                .text_size(px(11.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.75))
                .child(label.to_string()),
        )
        .into_any_element()
}

fn empty_state(icon_name: IconName, title: &str, body: &str, p: &Palette) -> AnyElement {
    div()
        .px(px(16.))
        .py(px(28.))
        .flex()
        .flex_col()
        .items_center()
        .gap(px(6.))
        .child(
            div()
                .size(px(30.))
                .rounded(px(9.))
                .bg(p.muted)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(icon_name).size(px(15.)).text_color(p.muted_fg)),
        )
        .child(
            div()
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.fg.opacity(0.8))
                .child(title.to_string()),
        )
        .child(
            div()
                .max_w(px(180.))
                .text_center()
                .text_size(px(11.5))
                .line_height(relative(1.45))
                .text_color(p.muted_fg)
                .child(body.to_string()),
        )
        .into_any_element()
}
