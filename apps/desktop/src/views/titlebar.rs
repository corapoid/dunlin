//! Custom window title bar: brand, project breadcrumb, primary view switcher
//! and the quiet utility cluster on the right.

use std::time::Duration;

use kestrel_domain::MainView;
use kestrel_ui::assets::IconName;
use kestrel_ui::component::{ActiveTheme, Icon, TitleBar};
use kestrel_ui::prelude::FluentBuilder as _;
use kestrel_ui::*;

use crate::app::KestrelApp;

pub(crate) fn render(
    this: &mut KestrelApp,
    window: &mut Window,
    cx: &mut Context<KestrelApp>,
) -> AnyElement {
    let p = palette(cx);
    let compact = window.viewport_size().width < px(1050.);
    let project = this.project().clone();
    let session = this.session().clone();
    let running = if session.status == kestrel_domain::SessionStatus::Running {
        crate::projection::nodes(&this.harness.transcript, &project, &session)
            .iter()
            .filter(|node| node.status == kestrel_domain::AgentStatus::Running)
            .count()
    } else {
        0
    };
    let (badge_bg, badge_fg, badge_label) = if this.harness.transcript.is_empty() {
        (p.amber_soft, p.amber, "preview".to_string())
    } else {
        match session.status {
            kestrel_domain::SessionStatus::Running if running > 0 => (
                p.green_soft,
                p.green,
                format!(
                    "{running} {}",
                    if running == 1 { "agent" } else { "agents" }
                ),
            ),
            kestrel_domain::SessionStatus::Running => {
                (p.green_soft, p.green, "running".to_string())
            }
            kestrel_domain::SessionStatus::Waiting => {
                (p.amber_soft, p.amber, "waiting".to_string())
            }
            kestrel_domain::SessionStatus::Failed => (p.red_soft, p.red, "failed".to_string()),
            _ => (p.muted, p.muted_fg, "idle".to_string()),
        }
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
                .child("Kestrel"),
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
        );

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
            .when(!compact, |this| {
                this.child(
                    div()
                        .text_size(px(11.0))
                        .font_family("Geist Mono")
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

    let right = div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .justify_end()
        .gap(px(6.))
        .child(
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
                        .font_family("Geist Mono")
                        .text_color(badge_fg)
                        .child(badge_label),
                ),
        )
        .child(
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
                    Icon::new(if this.sidebar_collapsed {
                        IconName::PanelLeftOpen
                    } else {
                        IconName::PanelLeftClose
                    })
                    .size(px(13.5))
                    .text_color(p.muted_fg),
                ),
        )
        .child(
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
                .when(!compact, |this| this.child(kbd("⌘K", p))),
        )
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
