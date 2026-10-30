//! Left sidebar: project switcher, explorer, search, source control,
//! session history and the subscription usage footer.

use std::path::Path;

use crate::effects::menu_item;
use dunlin_domain::*;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::Icon;
use dunlin_ui::component::input::Input;
use dunlin_ui::component::menu::{ContextMenuExt, PopupMenu};
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
    let body = match this.section {
        SidebarSection::Dashboard => projects_body(this, cx),
        SidebarSection::Projects => projects_body(this, cx),
        SidebarSection::Files => files_body(this, cx),
        SidebarSection::Search => search_body(this, cx),
        SidebarSection::SourceControl => source_control_body(this, cx),
        SidebarSection::Sessions => sessions_body(this, cx),
    };
    let header = section_header(this, cx);
    let footer = usage_footer(this, cx);
    let menu = this.project_menu_open.then(|| project_menu(this, cx));

    let width =
        crate::sidebar_layout::width(this.sidebar_width, f32::from(_window.viewport_size().width));
    let scroll = div()
        .id("sidebar-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .when(this.section == SidebarSection::Files, |view| {
            crate::explorer::drop_target(view, this.project().path.clone(), cx)
        })
        .child(body);
    let scroll = if matches!(
        this.section,
        SidebarSection::Projects | SidebarSection::Dashboard
    ) {
        projects_context(scroll, None, this.project_groups.clone(), cx)
    } else {
        scroll.into_any_element()
    };
    let mut root = div()
        .relative()
        .w(px(width))
        .h_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .bg(p.sidebar)
        .border_r_1()
        .border_color(p.border)
        .child(project_switcher(this, cx))
        .child(header)
        .child(scroll)
        .child(footer)
        .child(crate::sidebar_layout::handle(this, cx));

    if let Some(menu) = menu {
        root = root.child(menu);
    }
    root.into_any_element()
}

fn project_switcher(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let project = this.project();
    let tone = tone_color(&p, project.tone);
    let running = this.running_sessions_for(&project.id);
    let missing = !project.is_remote() && !Path::new(&project.path).is_dir();

    div()
        .id("project-switcher")
        .h(px(50.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(8.))
        .border_b_1()
        .border_color(p.border)
        .cursor_pointer()
        .hover(move |style| style.bg(p.hover))
        .on_click(cx.listener(|this, _, _, cx| {
            this.project_menu_open = !this.project_menu_open;
            cx.notify();
        }))
        .child(crate::project_icons::render(this, project, 28., &p))
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
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .truncate()
                        .child(project.path_tail().to_string()),
                ),
        )
        .child(
            div()
                .h(px(19.))
                .px(px(6.))
                .rounded(px(5.))
                .bg(p.muted)
                .flex()
                .items_center()
                .gap(px(4.))
                .child(dot(tone, 4.5))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_family(dunlin_ui::theme::code_font())
                        .text_color(p.muted_fg)
                        .child(format!("{running} live")),
                ),
        )
        .child(
            Icon::new(IconName::ChevronDown)
                .size(px(13.))
                .text_color(p.muted_fg),
        )
        .into_any_element()
}

fn section_header(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let (title, count) = match this.section {
        SidebarSection::Dashboard => ("Projects", Some(this.projects.len())),
        SidebarSection::Projects => ("Projects", Some(this.projects.len())),
        SidebarSection::Files => ("Explorer", None),
        SidebarSection::Search => ("Search", Some(this.search_hits.len())),
        SidebarSection::SourceControl => ("Source control", Some(this.git.changes.len())),
        SidebarSection::Sessions => ("Sessions", Some(this.visible_session_count())),
    };

    let right: AnyElement = match this.section {
        SidebarSection::Files => div()
            .id("explorer-actions")
            .flex()
            .items_center()
            .gap(px(2.))
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
        this.section,
        SidebarSection::Projects | SidebarSection::Dashboard
    ) {
        projects_context(header, None, this.project_groups.clone(), cx)
    } else {
        header.into_any_element()
    }
}

fn projects_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.refresh_project_summaries(cx);
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
    list = list.child(project_view_controls(this, cx, &p));
    if this.project_view == crate::project_tree::View::Tree {
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
                .filter(|(_, p)| p.group_id.as_deref() == Some(&group.id))
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
                    p.group_id
                        .as_ref()
                        .is_some_and(|id| descendants.contains(id))
                })
                .count();
            let collapsed = this.collapsed_project_groups.contains(&group.id);
            let mut section =
                project_group_drop_zone(Some(group.id.clone()), &group.name, dragging, cx, &p)
                    .ml(px(depth.min(8) as f32 * 12.))
                    .child(group_header(group, &groups, count, collapsed, cx, &p));
            for index in indices.into_iter().filter(|_| !collapsed) {
                section = section.child(project_row(this, index, cx, &p));
                if this.project_group_editor == Some(index) {
                    section = section.child(project_group_picker(this, index, cx, &p));
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
            this.project_view == crate::project_tree::View::List
                || p.group_id
                    .as_ref()
                    .is_none_or(|id| !groups.iter().any(|g| &g.id == id))
        })
        .map(|(i, _)| i)
        .collect();
    crate::project_tree::sort_projects(&this.projects, &mut indices, this.project_sort, &modified);
    if !indices.is_empty() || dragging {
        let mut section = project_group_drop_zone(None, "Top level", dragging, cx, &p);
        for index in indices {
            section = section.child(project_row(this, index, cx, &p));
            if this.project_view == crate::project_tree::View::List
                && let Some(id) = &this.projects[index].group_id
            {
                section = section.child(
                    div()
                        .pl(px(40.))
                        .pb(px(4.))
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(crate::project_tree::group_path(&groups, id)),
                );
            }
            if this.project_group_editor == Some(index) {
                section = section.child(project_group_picker(this, index, cx, &p));
            }
        }
        list = list.child(section);
    }
    list.into_any_element()
}

fn project_view_controls(this: &DunlinApp, cx: &mut Context<DunlinApp>, p: &Palette) -> AnyElement {
    let mut row = div().flex().items_center().gap(px(3.)).pb(px(6.));
    for (label, view) in [
        ("List", crate::project_tree::View::List),
        ("Tree", crate::project_tree::View::Tree),
    ] {
        row = row.child(
            div()
                .id(format!("project-view:{label}"))
                .aria_label(format!("Show projects as {label}"))
                .px(px(7.))
                .py(px(4.))
                .rounded(px(5.))
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .cursor_pointer()
                .when(this.project_view == view, |button| {
                    button.bg(p.muted).text_color(p.fg)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.project_view = view;
                    this.persist_settings(cx);
                    cx.notify();
                }))
                .child(label),
        );
    }
    row.child(div().flex_1())
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
            menu_item(|_, _| div().px(px(4.)).py(px(3.)).child("Open local folders…")).on_click(
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
    group: &ProjectGroup,
    groups: &[ProjectGroup],
    count: usize,
    collapsed: bool,
    cx: &mut Context<DunlinApp>,
    p: &Palette,
) -> AnyElement {
    let id = group.id.clone();
    let rename_id = group.id.clone();
    let agent_id = group.id.clone();
    let name = group.name.clone();
    let tone = tone_color(p, group.tone);
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
        .child(div().id(format!("group-collapse-{}", group.id)).h(px(29.)).flex().items_center().gap(px(5.)).cursor_pointer().rounded(px(5.)).hover(move |style| style.bg(p.hover)).on_drag(drag, |drag, _, _, cx| cx.new(|_| drag.clone()))
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
        .child(div().size(px(6.)).rounded_full().bg(tone))
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
                .id(format!("project-group-rename-{}", group.id))
                .size(px(18.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .opacity(0.55)
                .group_hover(hover_group, move |style| style.opacity(1.0))
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_rename_group_prompt(&rename_id, window, cx);
                }))
                .child(
                    Icon::new(IconName::Pencil)
                        .size(px(10.5))
                        .text_color(p.muted_fg),
                ),
        ));
    projects_context(header, Some(menu_id), groups.to_vec(), cx).into_any_element()
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

    div()
        .id(("project-row", index))
        .group(hover_group.clone())
        .h(px(46.))
        .px(px(8.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
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
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .truncate()
                                .child(name),
                        )
                        .when(pinned, |this| {
                            this.child(Icon::new(IconName::Pin).size(px(9.)).text_color(p.muted_fg))
                        })
                        .children(crate::local_projects::monorepo_badge(this, project, p))
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
        )
        .into_any_element()
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
        cx,
    );

    let mut root = div().flex().flex_col().pb(px(10.)).child(
        div()
            .id("explorer-project-root")
            .mx(px(6.))
            .px(px(7.))
            .h(px(26.))
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
            .h(px(24.))
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
            rows.extend(file_rows(&entry.children, selected, project, root, cx));
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
                .child(
                    div()
                        .id("open-git-history")
                        .aria_label("Open branch graph and history")
                        .px(px(6.))
                        .py(px(3.))
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(p.hover))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_git_history(window, cx)),
                        )
                        .child(
                            Icon::new(IconName::Waypoints)
                                .size(px(13.))
                                .text_color(p.muted_fg),
                        ),
                )
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.request_discard_file(&discard_path, cx);
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
                .h(px(29.))
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
                ),
        );
    }

    list.into_any_element()
}

fn sessions_body(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.refresh_session_list(cx);
    let p = palette(cx);
    let selected = this.selected_session;
    let sessions = this.sessions.clone();

    let mut list = div()
        .flex()
        .flex_col()
        .pb(px(10.))
        .when(this.visible_session_count() > 0, |list| {
            list.child(group_label("TODAY", &p))
        })
        .when(this.visible_session_count() == 0, |list| {
            list.child(
                div()
                    .p(px(12.))
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child("No conversations yet. Send a message to start one."),
            )
        });

    for (position, (index, session)) in sessions
        .iter()
        .enumerate()
        .filter(|(_, session)| this.session_visible(session))
        .enumerate()
    {
        if position == 2 {
            list = list.child(group_label("EARLIER", &p));
        }
        let active = index == selected;
        let (status_color, status_soft) = status_palette(&p, session.status);
        let title = session.title.clone();
        let harness = session.harness.clone();
        let model = session.model.clone();
        let updated = session.updated_at.clone();
        let cost = session.cost_usd;
        let duration = session.duration_secs;
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
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.select_session(index, cx);
                }))
                .on_mouse_down(
                    MouseButton::Right,
                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        this.context_menu = Some(ContextMenuState {
                            target: ContextTarget::Session(index),
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
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg.opacity(0.75))
                                .child(updated),
                        ),
                )
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
                                .flex_shrink_0()
                                .text_size(px(11.0))
                                .font_family(dunlin_ui::theme::code_font())
                                .text_color(p.muted_fg)
                                .child(format!(
                                    "${cost:.2} · {}",
                                    crate::app::format_duration(duration)
                                )),
                        )
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
                                    this.request_delete_session(index, cx);
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
    let tail = project.path_tail().to_string();
    let group = project
        .group_id
        .as_ref()
        .and_then(|id| this.project_groups.iter().find(|group| &group.id == id))
        .map(|group| crate::project_tree::group_path(&this.project_groups, &group.id));

    div()
        .id(("project-menu-row", index))
        .h(px(36.))
        .px(px(6.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .gap(px(7.))
        .cursor_pointer()
        .when(active, |this| this.bg(p.accent))
        .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
        .on_click(cx.listener(move |this, _, _, cx| this.select_project(index, cx)))
        .child(crate::project_icons::render(this, project, 20., p))
        .child(
            div()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(p.fg)
                        .truncate()
                        .child(name),
                )
                .when_some(group, |this, group| {
                    this.child(
                        div()
                            .text_size(px(10.0))
                            .text_color(p.muted_fg.opacity(0.65))
                            .truncate()
                            .child(group),
                    )
                }),
        )
        .child(div().flex_1())
        .child(
            div()
                .max_w(px(86.))
                .text_size(px(10.5))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.7))
                .truncate()
                .child(tail),
        )
        .when(active, |this| {
            this.child(Icon::new(IconName::Check).size(px(11.)).text_color(p.fg))
        })
        .into_any_element()
}

fn project_menu(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let trusted = this.project().trusted;
    let pinned = this.project().pinned;
    let missing = !this.project().is_remote() && !Path::new(&this.project().path).is_dir();
    let mut menu = div()
        .id("project-menu")
        .child(crate::effects::rounded_scroll_probe(
            &this.overlay_backdrop,
            10.,
        ))
        .absolute()
        .top(px(45.))
        .left(px(6.))
        .right(px(6.))
        .max_h(px(720.))
        .overflow_y_scroll()
        .p(px(5.))
        .rounded(px(10.))
        .border_1()
        .border_color(p.border_strong)
        .bg(p.card)
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
    for group in &groups {
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
        if indices.is_empty() {
            continue;
        }
        menu = menu.child(
            div()
                .px(px(8.))
                .pt(px(5.))
                .pb(px(2.))
                .text_size(px(10.0))
                .font_family(dunlin_ui::theme::code_font())
                .text_color(p.muted_fg.opacity(0.75))
                .truncate()
                .child(group.name.to_uppercase()),
        );
        for index in indices {
            menu = menu.child(project_menu_row(this, index, cx, &p));
        }
    }

    let ungrouped: Vec<usize> = this
        .projects
        .iter()
        .enumerate()
        .filter(|(_, project)| project.group_id.is_none())
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

    menu.child(div().h(px(1.)).my(px(3.)).bg(p.border))
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
                        .child("Open local folders…"),
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
        )
        .into_any_element()
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
