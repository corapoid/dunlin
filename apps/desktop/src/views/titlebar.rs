//! Custom window title bar: brand, project breadcrumb, primary view switcher
//! and the quiet utility cluster on the right.

use std::time::Duration;

use dunlin_domain::{MainView, SidebarSection};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{
    ActiveTheme, Disableable, Icon, Sizable, TitleBar,
    button::{Button, ButtonVariants},
};
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
        || (this.spec_tools_enabled && this.spec.is_active_for(this.project()))
    {
        Switcher::None
    } else if this.section == SidebarSection::SourceControl
        || this.git_history.is_active_for(this.project())
    {
        Switcher::Git
    } else if this.workbench.show_file {
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
    let compact = window.viewport_size().width < px(1050.);
    let has_projects = !this.projects.is_empty();
    let dashboard = this.section == SidebarSection::Dashboard;
    let security = this.section == SidebarSection::Security && this.security_enabled;
    let switcher = switcher(this);
    let agent = switcher == Switcher::Agent;
    let show_session_context = agent && this.view != MainView::Chat;
    let show_shortcut_hints = this.show_shortcut_hints;
    let session = this.session().clone();
    let sidebar_collapsed = this.sidebar_collapsed;

    let left = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(px(8.))
        .when(has_projects, |bar| bar.child(
            Button::new("title-sidebar").ghost().small()
                .icon(if sidebar_collapsed { IconName::PanelLeftOpen } else { IconName::PanelLeftClose })
                .accessibility_label("Toggle sidebar")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sidebar_collapsed = !this.sidebar_collapsed;
                    this.persist_settings(cx); cx.notify();
                }))
        ))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .child(
                    Button::new("navigation-back")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowLeft)
                        .accessibility_label("Back")
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
                    Button::new("navigation-forward")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowRight)
                        .accessibility_label("Forward")
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
        .when(has_projects, |bar| bar.child(super::sidebar::project_switcher(this, cx)))
        .when(!has_projects, |bar| bar.child(div().text_size(px(13.)).child("Dunlin")))
        .when(show_session_context, |this| {
            this.child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.border_strong)
                    .child("/"),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .text_color(p.muted_fg.opacity(0.85))
                    .truncate()
                    .child(session.title),
            )
        });

    let mut center = div()
        .flex_shrink_0()
        .h(px(28.))
        .p(px(2.))
        .rounded(px(8.))
        .border_1()
        .border_color(p.border)
        .bg(p.muted)
        .flex()
        .items_center()
        .gap(px(2.));

    let epoch = this.view_epoch;
    if agent {
        for view in MainView::ALL {
            let active = this.view == view;
            let (icon_name, hint) = match view {
                MainView::Chat => (IconName::MessageSquare, "⌘1"),
                MainView::Graph => (IconName::Waypoints, "⌘2"),
                MainView::Trace => (IconName::ChartNoAxesGantt, "⌘3"),
            };
            let color = if active { p.fg } else { p.muted_fg };

            let tab = div()
                .id(("view-tab", view as usize))
                .aria_label(format!("Open {} view", view.label()))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .gap(px(5.))
                .cursor_pointer()
                .when(active, |this| {
                    this.bg(p.card).border_1().border_color(p.border)
                })
                .when(!active, |this| this.hover(move |style| style.bg(p.hover)))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_view(view, cx);
                }))
                .child(Icon::new(icon_name).size(px(12.5)).text_color(color))
                .child(
                    div()
                        .text_size(px(12.0))
                        .font_weight(if active {
                            FontWeight::MEDIUM
                        } else {
                            FontWeight::NORMAL
                        })
                        .text_color(color)
                        .child(view.label()),
                )
                .when(!compact && this.show_shortcut_hints, |this| {
                    this.child(
                        div()
                            .text_size(px(11.0))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(if active {
                                p.muted_fg
                            } else {
                                p.muted_fg.opacity(0.55)
                            })
                            .child(hint),
                    )
                });

            let tab = if active {
                let card = p.card;
                let border = p.border;
                tab.with_animation(
                    ElementId::NamedInteger("view-tab-active".into(), epoch),
                    Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                    move |this, t| {
                        this.bg(card.opacity(t))
                            .border_color(border.opacity(t))
                            .opacity(0.68 + 0.32 * t)
                    },
                )
                .into_any_element()
            } else {
                tab.into_any_element()
            };

            center = center.child(tab);
        }
    } else if switcher == Switcher::Git {
        center = center.child(Button::new("title-git-history").ghost().small().label("History")
            .on_click(cx.listener(|this, _, window, cx| this.show_git_history(window, cx))));
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
        .child(
            div()
                .id("title-theme")
                .aria_label("Toggle light or dark theme")
                .size(px(26.))
                .rounded(px(7.))
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(move |style| style.bg(p.hover))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_appearance(window, cx);
                }))
                .child(
                    Icon::new(if cx.theme().is_dark() {
                        IconName::Sun
                    } else {
                        IconName::Moon
                    })
                    .size(px(13.5))
                    .text_color(p.muted_fg),
                ),
        );

    TitleBar::new()
        .h(px(44.))
        .bg(p.titlebar)
        .border_color(p.border)
        .child(
            div()
                .w_full()
                .h_full()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(left)
                .child(center)
                .child(right),
        )
        .into_any_element()
}
