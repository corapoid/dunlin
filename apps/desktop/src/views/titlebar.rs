//! Custom window title bar: brand, project breadcrumb, primary view switcher
//! and the quiet utility cluster on the right.

use std::time::Duration;

use dunlin_domain::{MainView, SidebarSection};
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{ActiveTheme, Icon, TitleBar};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::DunlinApp;

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let compact = window.viewport_size().width < px(1050.);
    let has_projects = !this.projects.is_empty();
    let dashboard = this.section == SidebarSection::Dashboard;
    let show_session_context = !dashboard && has_projects && this.view != MainView::Chat;
    let show_shortcut_hints = this.show_shortcut_hints;
    let project = this.project().clone();
    let session = this.session().clone();
    let running = if !dashboard && session.status == dunlin_domain::SessionStatus::Running {
        crate::projection::nodes(&this.harness.transcript, &project, &session)
            .iter()
            .filter(|node| node.status == dunlin_domain::AgentStatus::Running)
            .count()
    } else {
        0
    };
    let (badge_bg, badge_fg, badge_label) = match session.status {
        dunlin_domain::SessionStatus::Idle => (p.muted, p.muted_fg, "ready".to_string()),
        dunlin_domain::SessionStatus::Running if running > 0 => (
            p.green_soft,
            p.green,
            format!(
                "{running} {}",
                if running == 1 { "agent" } else { "agents" }
            ),
        ),
        dunlin_domain::SessionStatus::Running => (p.green_soft, p.green, "running".to_string()),
        dunlin_domain::SessionStatus::Waiting => (p.amber_soft, p.amber, "waiting".to_string()),
        dunlin_domain::SessionStatus::Failed => (p.red_soft, p.red, "failed".to_string()),
        _ => (p.muted, p.muted_fg, "idle".to_string()),
    };

    let left = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(px(8.))
        .child(
            div()
                .size(px(20.))
                .rounded(px(6.))
                .bg(p.fg)
                .text_color(p.bg)
                .flex()
                .items_center()
                .justify_center()
                .child(Icon::new(IconName::Bird).size(px(12.)).text_color(p.bg)),
        )
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(p.fg)
                .child("Dunlin"),
        )
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
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_fg)
                    .truncate()
                    .child(project.name.clone()),
            )
            .child(
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
    if has_projects && !dashboard {
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
    }
    let center = if dashboard {
        crate::dashboard::metric_switcher(this, compact, cx)
    } else if has_projects {
        center.into_any_element()
    } else {
        div().flex_shrink_0().into_any_element()
    };

    let sidebar_collapsed = this.sidebar_collapsed;
    let right = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.))
        .when(has_projects && !dashboard, |bar| {
            bar.when(
                this.view == MainView::Chat && !this.final_answer_only,
                |bar| bar.child(super::chat::reasoning_toggle(this, cx)),
            )
            .child(
                div()
                    .id("title-session-menu")
                    .aria_label("Session menu")
                    .size(px(26.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .when(this.session_menu_open, |button| button.bg(p.muted))
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.session_menu_open = !this.session_menu_open;
                        cx.notify();
                    }))
                    .child(
                        Icon::new(IconName::Ellipsis)
                            .size(px(15.))
                            .text_color(p.muted_fg),
                    ),
            )
        })
        .when(show_session_context, |this| {
            this.child(
                div()
                    .h(px(22.))
                    .px(px(7.))
                    .rounded_full()
                    .bg(badge_bg)
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(dot(badge_fg, 5.0))
                    .child(
                        div()
                            .text_size(px(11.5))
                            .font_family(dunlin_ui::theme::code_font())
                            .text_color(badge_fg)
                            .child(badge_label),
                    ),
            )
        })
        .when(has_projects, |this| {
            this.child(
                div()
                    .id("title-sidebar")
                    .aria_label("Toggle sidebar")
                    .size(px(26.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.sidebar_collapsed = !this.sidebar_collapsed;
                        this.persist_settings(cx);
                        cx.notify();
                    }))
                    .child(
                        Icon::new(if sidebar_collapsed {
                            IconName::PanelLeftOpen
                        } else {
                            IconName::PanelLeftClose
                        })
                        .size(px(13.5))
                        .text_color(p.muted_fg),
                    ),
            )
        })
        .when(has_projects, |this| {
            this.child(
                div()
                    .id("title-search")
                    .aria_label("Open command palette")
                    .h(px(26.))
                    .px(px(8.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .cursor_pointer()
                    .hover(move |style| style.bg(p.hover))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_palette(window, cx);
                    }))
                    .child(
                        Icon::new(IconName::Search)
                            .size(px(12.5))
                            .text_color(p.muted_fg),
                    )
                    .when(!compact, |this| {
                        this.child(
                            div()
                                .text_size(px(12.0))
                                .text_color(p.muted_fg)
                                .child("Search"),
                        )
                    })
                    .when(!compact && show_shortcut_hints, |this| {
                        this.child(kbd("⌘K", p))
                    }),
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
                    toggle_mode(window, cx);
                    this.persist_settings(cx);
                    cx.notify();
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
