//! The sidebar owns all primary navigation; there is no separate activity rail.
use super::*;
use crate::app::Command;
use crate::sidebar_layout::Tab;

pub(super) fn tabs(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut tabs = div()
        .id("sidebar-navigation-tabs")
        .h(px(44.))
        .flex_shrink_0()
        .px(px(10.))
        .flex()
        .items_center()
        .gap(px(4.));
    for (tab, icon, label) in [
        (Tab::Projects, IconName::LayoutGrid, "Projects"),
        (Tab::Sessions, IconName::MessageSquare, "Sessions"),
        (Tab::Explorer, IconName::Files, "Explorer"),
    ] {
        let active = this.sidebar_tab == tab;
        let disabled = tab == Tab::Explorer && this.projects.is_empty();
        tabs = tabs.child(
            navigation_button(format!("sidebar-tab:{label}"), icon, label)
                .flex_1()
                .h(px(30.))
                .selected(active)
                .toggled(active)
                .disabled(disabled)
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe(match tab {
                            Tab::Projects => "sidebar-tab-projects",
                            Tab::Sessions => "sidebar-tab-sessions",
                            Tab::Explorer => "sidebar-tab-explorer",
                        })
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .on_click(cx.listener(move |this, _, window, cx| {
                    if !this.dismiss_history_for_menu(cx) {
                        return;
                    }
                    this.sidebar_tab = tab;
                    this.run_command(Command::Section(tab.section()), window, cx);
                    if tab == Tab::Projects {
                        this.focus_projects(window, cx);
                    }
                    this.persist_settings(cx);
                })),
        );
    }
    tabs.border_b_1()
        .border_color(p.border.opacity(0.5))
        .into_any_element()
}

fn entry(
    this: &DunlinApp,
    id: &'static str,
    label: &'static str,
    icon: IconName,
    command: Command,
    active: bool,
    count: usize,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    dunlin_ui::button(id)
        .w_full()
        .h(px(if this.density == crate::app::UiDensity::Compact {
            26.
        } else {
            32.
        }))
        .px(px(10.))
        .justify_start()
        .rounded(px(dunlin_ui::radii::CONTROL))
        .selected(active)
        .toggled(active)
        .accessibility_label(label)
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe(id)
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(10.))
                .child(Icon::new(icon).size(px(15.)).text_color(if active {
                    p.fg
                } else {
                    p.muted_fg
                }))
                .child(div().flex_1().text_size(px(12.)).child(label))
                .when(count > 0, |row| {
                    row.child(
                        div()
                            .px(px(5.))
                            .rounded(px(dunlin_ui::radii::BADGE))
                            .bg(p.subtle)
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .child(count.to_string()),
                    )
                }),
        )
        .on_click(cx.listener(move |this, _, window, cx| {
            if !this.dismiss_history_for_menu(cx) {
                return;
            }
            this.run_command(command, window, cx);
            cx.notify();
        }))
        .into_any_element()
}

pub(super) fn footer(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let tool = this.current_workspace_tool();
    div()
        .id("sidebar-footer")
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(if this.density == crate::app::UiDensity::Compact {
            1.
        } else {
            2.
        }))
        .px(px(crate::sidebar_layout::UTILITY_INSET))
        .pt(px(if this.density == crate::app::UiDensity::Compact {
            8.
        } else {
            12.
        }))
        .pb(px(8.))
        .border_t_1()
        .border_color(p.border.opacity(0.6))
        .relative()
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("sidebar-footer")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(
            dunlin_ui::button("sidebar-workspace-toggle")
                .w_full()
                .h(px(28.))
                .px(px(10.))
                .justify_start()
                .accessibility_label(if this.workspace_navigation_collapsed {
                    "Expand project workspace"
                } else {
                    "Collapse project workspace"
                })
                .toggled(!this.workspace_navigation_collapsed)
                .tooltip(if this.workspace_navigation_collapsed {
                    "Expand project workspace"
                } else {
                    "Collapse project workspace"
                })
                .text_size(px(11.))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child(sidebar_heading("PROJECT WORKSPACE", &p).flex_1())
                .child(
                    Icon::new(if this.workspace_navigation_collapsed {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    })
                    .size(px(12.)),
                )
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("sidebar-workspace-toggle")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.workspace_navigation_collapsed = !this.workspace_navigation_collapsed;
                    this.persist_settings(cx);
                    cx.notify();
                })),
        )
        .when(!this.workspace_navigation_collapsed, |footer| {
            footer
                .child(entry(
                    this,
                    "sidebar-source-control",
                    "Source Control",
                    IconName::GitBranch,
                    Command::Section(SidebarSection::SourceControl),
                    !this.settings_open && this.section == SidebarSection::SourceControl,
                    this.git.changes.len(),
                    cx,
                ))
                .child(entry(
                    this,
                    "sidebar-knowledge",
                    "Knowledge",
                    IconName::BookOpen,
                    Command::Section(SidebarSection::Knowledge),
                    tool == Some(crate::workspace_tools::Tool::Knowledge) && !this.settings_open,
                    this.knowledge.graph.objects.len(),
                    cx,
                ))
                .when(this.spec_tools_enabled(), |footer| {
                    footer.child(entry(
                        this,
                        "sidebar-intent",
                        "Intent",
                        IconName::ListTodo,
                        Command::Spec,
                        this.spec.is_active_for(this.project()) && !this.settings_open,
                        0,
                        cx,
                    ))
                })
                .child(entry(
                    this,
                    "sidebar-artifacts",
                    "Artifacts",
                    IconName::FileText,
                    Command::Artifacts,
                    tool == Some(crate::workspace_tools::Tool::Artifacts) && !this.settings_open,
                    this.knowledge.artifacts.len(),
                    cx,
                ))
                .when(this.security_workspace_enabled(), |footer| {
                    footer.child(entry(
                        this,
                        "sidebar-security",
                        "Security Audit",
                        IconName::ShieldCheck,
                        Command::Section(SidebarSection::Security),
                        tool == Some(crate::workspace_tools::Tool::Security) && !this.settings_open,
                        0,
                        cx,
                    ))
                })
        })
        .child(div().h(px(1.)).my(px(7.)).bg(p.border.opacity(0.6)))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    navigation_button(
                        "sidebar-settings",
                        IconName::Settings,
                        this.shortcut_label("Settings", "⌘,"),
                    )
                    .size(px(crate::sidebar_layout::UTILITY_SIZE))
                    .flex_shrink_0()
                    .selected(this.settings_open)
                    .toggled(this.settings_open)
                    .relative()
                    .children({
                        #[cfg(debug_assertions)]
                        {
                            crate::workspace_ui_smoke::probe("sidebar-settings")
                        }
                        #[cfg(not(debug_assertions))]
                        {
                            None::<AnyElement>
                        }
                    })
                    .on_click(cx.listener(|this, _, window, cx| {
                        if !this.dismiss_history_for_menu(cx) {
                            return;
                        }
                        this.request_settings_location(Some(SettingsTab::General), window, cx);
                    })),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .child(super::super::host_control::render(this, cx)),
                ),
        )
        .into_any_element()
}
