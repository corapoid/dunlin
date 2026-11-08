//! Custom window title bar: brand, project breadcrumb, primary view switcher
//! and the quiet utility cluster on the right.

use dunlin_domain::{MainView, SidebarSection};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{Disableable, Icon, TitleBar, button::ButtonVariants};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::DunlinApp;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Switcher {
    Agent,
    Git,
    None,
}

pub(crate) fn switcher(this: &DunlinApp) -> Switcher {
    if this.projects.is_empty()
        || this.settings_open
        || this.section == SidebarSection::Dashboard
        || (this.section == SidebarSection::Security && this.security_enabled)
        || this.section == SidebarSection::Knowledge
        || (this.spec_tools_enabled && this.spec.is_active_for(this.project()))
    {
        Switcher::None
    } else if this.section == SidebarSection::SourceControl
        || this.git_history.is_active_for(this.project())
    {
        Switcher::Git
    } else if this.workbench.show_file
        || matches!(
            this.current_workspace_tab(),
            None | Some(crate::workspace_panes::TabKey::Terminal(_))
        )
    {
        Switcher::None
    } else {
        Switcher::Agent
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let has_projects = !this.projects.is_empty();
    let dashboard = this.section == SidebarSection::Dashboard;
    let security = this.section == SidebarSection::Security && this.security_enabled;
    let switcher = switcher(this);
    let agent = switcher == Switcher::Agent;
    let show_shortcut_hints = this.show_shortcut_hints;
    let sidebar_collapsed = this.sidebar_collapsed;

    let left = div()
        .flex_1()
        .h_full()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(has_projects, |bar| {
            let sidebar_label = match (sidebar_collapsed, show_shortcut_hints) {
                (true, true) => "Show sidebar (⌘B)",
                (true, false) => "Show sidebar",
                (false, true) => "Hide sidebar (⌘B)",
                (false, false) => "Hide sidebar",
            };
            bar.child(
                navigation_button(
                    "title-sidebar",
                    if sidebar_collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    },
                    sidebar_label,
                )
                .size(px(28.))
                .relative()
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("title-sidebar")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                    this.persist_settings(cx);
                    cx.notify();
                })),
            )
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    navigation_button("navigation-back", IconName::ArrowLeft, "Back")
                        .size(px(28.))
                        .relative()
                        .children({
                            #[cfg(debug_assertions)]
                            {
                                crate::workspace_ui_smoke::probe("navigation-back")
                            }
                            #[cfg(not(debug_assertions))]
                            {
                                None::<AnyElement>
                            }
                        })
                        .tooltip(if show_shortcut_hints {
                            "Back (⌘[)"
                        } else {
                            "Back"
                        })
                        .disabled(!this.can_navigate_history(false))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate_history(false, window, cx);
                        })),
                )
                .child(
                    navigation_button("navigation-forward", IconName::ArrowRight, "Forward")
                        .size(px(28.))
                        .tooltip(if show_shortcut_hints {
                            "Forward (⌘])"
                        } else {
                            "Forward"
                        })
                        .disabled(!this.can_navigate_history(true))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.navigate_history(true, window, cx);
                        })),
                ),
        )
        .when(has_projects, |bar| {
            bar.child(super::sidebar::project_switcher(this, cx))
        })
        .when(!has_projects, |bar| {
            bar.child(div().text_size(px(13.)).child("Dunlin"))
        });

    let view_selector = super::view_switcher::render(this, agent, window, cx);
    let mut center = div().flex().flex_shrink_0();
    if agent {
        center = center.child(view_selector);
    } else if switcher == Switcher::Git {
        center = center.child(
            dunlin_ui::button("title-git-history")
                .ghost()
                .label("History")
                .on_click(cx.listener(|this, _, window, cx| this.show_git_history(window, cx))),
        );
    }
    let center = if dashboard {
        div().into_any_element()
    } else if security {
        div()
            .flex_shrink_0()
            .h(px(28.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(p.border)
            .bg(p.muted)
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                Icon::new(IconName::ShieldCheck)
                    .size(px(13.))
                    .text_color(p.fg),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.fg)
                    .child("Security"),
            )
            .into_any_element()
    } else if switcher != Switcher::None {
        center.into_any_element()
    } else {
        div().flex_shrink_0().into_any_element()
    };

    let right = div()
        .flex_1()
        .min_w(px(0.))
        .pr(px(12.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.))
        .child(super::host_control::render(this, cx))
        .when(agent, |bar| {
            bar.when(
                this.view == MainView::Chat && !this.final_answer_only,
                |bar| bar.child(super::chat::reasoning_toggle(this, cx)),
            )
        })
        .child(crate::user_profile::render(this, cx));

    TitleBar::new()
        .h(px(dunlin_ui::scale::TITLEBAR_HEIGHT))
        .border_b_0()
        .bg(transparent_black())
        .border_color(transparent_black())
        .child(
            div()
                .relative()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(left)
                .child(center)
                .child(right)
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("titlebar-content")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                }),
        )
        .into_any_element()
}
