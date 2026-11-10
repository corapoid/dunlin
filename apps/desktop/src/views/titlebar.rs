//! Unified chrome: window navigation, centered global search, view modes and avatar.
use crate::app::DunlinApp;
use dunlin_domain::SidebarSection;
use dunlin_ui::assets::IconName;
use dunlin_ui::component::{Disableable, Icon, TitleBar};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Switcher {
    Agent,
    Git,
    None,
}

pub(crate) fn switcher(this: &DunlinApp) -> Switcher {
    if this.current_workspace_tool().is_some()
        || this.projects.is_empty()
        || this.settings_open
        || matches!(
            this.section,
            SidebarSection::Dashboard | SidebarSection::Knowledge | SidebarSection::Security
        )
        || (this.spec_tools_enabled() && this.spec.is_active_for(this.project()))
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

fn sidebar_icon(collapsed: bool) -> Icon {
    Icon::default().data(if collapsed {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18m4-13 4 4-4 4"/></svg>"#
    } else {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="18" height="18" rx="2"/><path d="M9 3v18m7-13-4 4 4 4"/></svg>"#
    })
}

fn history_icon(forward: bool, ink: Hsla) -> Icon {
    Icon::default().data(if forward {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14m-6-6 6 6-6 6"/></svg>"#
    } else {
        br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.35" stroke-linecap="round" stroke-linejoin="round"><path d="M19 12H5m6-6-6 6 6 6"/></svg>"#
    }).size(px(15.)).text_color(ink)
}

pub(crate) fn search_geometry(viewport: f32) -> (f32, f32) {
    let width = (viewport - 720.)
        .clamp(200., 420.)
        .min((viewport - 120.).max(0.));
    ((viewport - width) / 2., width)
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let (search_left, search_width) = search_geometry(f32::from(window.viewport_size().width));
    let collapsed = this.sidebar_collapsed;
    let left = div()
        .h_full()
        .max_w(px((search_left - 12.).max(0.)))
        .min_w(px(0.))
        .pl(px(if cfg!(target_os = "macos") { 80. } else { 12. }))
        .flex()
        .items_center()
        .gap(px(4.))
        .child(
            navigation_button_with_icon(
                "title-sidebar",
                sidebar_icon(collapsed).size(px(15.)).text_color(p.muted_fg),
                this.shortcut_label(
                    if collapsed {
                        "Show sidebar"
                    } else {
                        "Hide sidebar"
                    },
                    "⌘B",
                ),
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
        .child(
            navigation_button_with_icon(
                "navigation-back",
                history_icon(false, p.muted_fg),
                this.shortcut_label("Back", "⌘["),
            )
            .size(px(28.))
            .disabled(!this.can_navigate_history(false))
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
            .on_click(cx.listener(|this, _, window, cx| this.navigate_history(false, window, cx))),
        )
        .child(
            navigation_button_with_icon(
                "navigation-forward",
                history_icon(true, p.muted_fg),
                this.shortcut_label("Forward", "⌘]"),
            )
            .size(px(28.))
            .disabled(!this.can_navigate_history(true))
            .on_click(cx.listener(|this, _, window, cx| this.navigate_history(true, window, cx))),
        )
        .when(!this.projects.is_empty(), |bar| {
            let leading = if cfg!(target_os = "macos") { 80. } else { 12. };
            bar.child(
                super::sidebar::project_switcher(this, cx)
                    .max_w(px((search_left - leading - 112.).max(48.)))
                    .min_w(px(0.)),
            )
        });

    let modes = super::view_switcher::render(this, switcher(this) == Switcher::Agent, window, cx);
    let right = div()
        .ml_auto()
        .pr(px(12.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap(px(10.))
        .child(modes)
        .child(crate::user_profile::render(this, cx));
    let search = titlebar_field("global-search-trigger", "Global search", p)
        .absolute()
        .left(px(search_left))
        .w(px(search_width))
        .tooltip(this.shortcut_label("Global search", "⌘K"))
        .child(
            Icon::new(IconName::Search)
                .size(px(14.))
                .text_color(p.muted_fg),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .text_size(px(12.))
                .text_color(p.muted_fg)
                .truncate()
                .child("Search…"),
        )
        .when(this.show_shortcut_hints, |button| {
            button.child(div().text_size(px(10.)).text_color(p.muted_fg).child("⌘K"))
        })
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("global-search-trigger")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx)));
    TitleBar::new()
        .h(px(dunlin_ui::scale::TITLEBAR_HEIGHT))
        .pl(px(0.))
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
                .when(!this.onboarding_visible(), |bar| {
                    bar.child(left).child(right).child(search)
                })
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

#[cfg(test)]
mod tests {
    #[test]
    fn global_search_is_centered_at_every_supported_window_size() {
        for viewport in [800., 1100., 1360., 1920.] {
            let (left, width) = super::search_geometry(viewport);
            assert_eq!(left + width / 2., viewport / 2.);
            assert!(width >= 200. && left >= 0. && left + width <= viewport);
        }
    }
}
