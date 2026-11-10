use crate::app::DunlinApp;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

pub(crate) const DEFAULT_WIDTH: f32 = 278.;
pub(crate) const UTILITY_INSET: f32 = 8.;
pub(crate) const UTILITY_SIZE: f32 = 32.;
const RESIZE_HIT_WIDTH: f32 = 10.;
const RESIZE_LINE_WIDTH: f32 = 1.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Tab {
    #[default]
    Projects,
    Sessions,
    Explorer,
}

impl Tab {
    pub(crate) fn section(self) -> dunlin_domain::SidebarSection {
        use dunlin_domain::SidebarSection;
        match self {
            Self::Projects => SidebarSection::Projects,
            Self::Sessions => SidebarSection::Sessions,
            Self::Explorer => SidebarSection::Files,
        }
    }

    pub(crate) fn from_section(section: dunlin_domain::SidebarSection) -> Self {
        match section {
            dunlin_domain::SidebarSection::Sessions => Self::Sessions,
            dunlin_domain::SidebarSection::Files => Self::Explorer,
            _ => Self::Projects,
        }
    }
}

pub(crate) fn project_row_options(width: f32, compact: bool, wrap_names: bool) -> (bool, bool) {
    (width >= 320., !compact && wrap_names && width >= 260.)
}
pub(crate) fn width(saved: f32, viewport: f32) -> f32 {
    let saved = if saved.is_finite() {
        saved
    } else {
        DEFAULT_WIDTH
    };
    saved.clamp(220., (viewport - 470.).clamp(220., 560.))
}
impl DunlinApp {
    pub(crate) fn sidebar_section(&self) -> dunlin_domain::SidebarSection {
        use dunlin_domain::SidebarSection;
        match self.section {
            SidebarSection::SourceControl | SidebarSection::Search | SidebarSection::Knowledge => {
                self.section
            }
            _ => self.sidebar_tab.section(),
        }
    }

    pub(crate) fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_resize.take().is_some() {
            self.persist_settings(cx);
            cx.notify();
        }
    }
}
fn resize_indicator(p: Palette, dragging: bool) -> Div {
    div()
        .absolute()
        .right(px((RESIZE_HIT_WIDTH - RESIZE_LINE_WIDTH) / 2.))
        .top_0()
        .h_full()
        .w(px(RESIZE_LINE_WIDTH))
        .bg(transparent_black())
        .when(dragging, |line| line.bg(p.border_strong))
        .group_hover("sidebar-resize", move |line| line.bg(p.border_strong))
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("sidebar-resize-line")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
}

pub(crate) fn handle(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    div()
        .id("sidebar-resize-handle")
        .group("sidebar-resize")
        .aria_label("Drag to resize sidebar; double-click to reset")
        .absolute()
        .right_0()
        .top_0()
        .h_full()
        .w(px(RESIZE_HIT_WIDTH))
        .occlude()
        .cursor(CursorStyle::ResizeLeftRight)
        .children({
            #[cfg(debug_assertions)]
            {
                crate::workspace_ui_smoke::probe("sidebar-resize-handle")
            }
            #[cfg(not(debug_assertions))]
            {
                None::<AnyElement>
            }
        })
        .child(resize_indicator(p, this.sidebar_resize.is_some()))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, event: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                if event.click_count == 2 {
                    this.sidebar_width = DEFAULT_WIDTH;
                    this.sidebar_resize = None;
                    this.persist_settings(cx);
                } else {
                    this.sidebar_resize = Some((
                        f32::from(event.position.x),
                        width(this.sidebar_width, f32::from(window.viewport_size().width)),
                    ));
                }
                cx.notify();
            }),
        )
        .into_any_element()
}
#[cfg(test)]
mod tests {
    #[test]
    fn resize_indicator_is_invisible_at_rest_in_every_skin_and_appearance() {
        use dunlin_ui::*;
        for skin in Skin::ALL {
            for dark in [false, true] {
                let p = theme::for_skin(skin, dark);
                let mut idle = super::resize_indicator(p, false);
                assert_eq!(idle.style().background, Some(transparent_black().into()));
                assert_eq!(idle.style().size.width, Some(px(1.).into()));
                let mut dragging = super::resize_indicator(p, true);
                assert_eq!(dragging.style().background, Some(p.border_strong.into()));
            }
        }
    }

    #[test]
    fn sidebar_tab_is_independent_and_old_preferences_migrate() {
        use dunlin_domain::SidebarSection;
        for (section, tab) in [
            (SidebarSection::Files, super::Tab::Explorer),
            (SidebarSection::Sessions, super::Tab::Sessions),
            (SidebarSection::Knowledge, super::Tab::Projects),
        ] {
            assert_eq!(super::Tab::from_section(section), tab);
        }
        let legacy: crate::app::StoredSettings =
            serde_json::from_str(r#"{"section":"Files","sidebar_width":320}"#).unwrap();
        assert!(legacy.sidebar_tab.is_none());
        assert!(
            !legacy.workspace_navigation_collapsed,
            "older settings keep workspace navigation expanded"
        );
        let mut saved = legacy;
        saved.sidebar_tab = Some(super::Tab::Sessions);
        saved.workspace_navigation_collapsed = true;
        let restored: crate::app::StoredSettings =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.sidebar_tab, Some(super::Tab::Sessions));
        assert_eq!(restored.sidebar_width, 320.);
        assert!(restored.workspace_navigation_collapsed);
    }

    #[test]
    fn narrow_rows_reserve_space_for_names_instead_of_badges_or_letter_towers() {
        for width in [100., 180., 220., 250.] {
            assert_eq!(
                super::project_row_options(width, false, true),
                (false, false)
            );
        }
        assert_eq!(super::project_row_options(280., false, true), (false, true));
        assert_eq!(super::project_row_options(340., false, true), (true, true));
        assert_eq!(super::project_row_options(340., true, true), (true, false));
        assert_eq!(
            super::project_row_options(340., false, false),
            (true, false)
        );
    }
    #[test]
    fn widths_are_finite_and_leave_space_for_the_main_panel() {
        assert_eq!(super::width(f32::NAN, 1360.), 278.);
        assert_eq!(super::width(100., 1360.), 220.);
        assert_eq!(super::width(900., 1360.), 560.);
        assert_eq!(super::width(500., 800.), 330.);
    }
}
