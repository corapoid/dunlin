//! Dunlin design system.
//!
//! The visual language mirrors Kobra's calm, shadcn-neutral surfaces and uses
//! Geist for UI text and Geist Mono for code/metadata. All tokens are defined
//! here so the application never reaches for hard-coded colours.

use std::borrow::Cow;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme, Colorize, Theme, ThemeMode};
use gpui_kit::*;

/// Monospaced text size shared by editors, diffs and code chips.
static CODE_FONT_SIZE: AtomicU32 = AtomicU32::new(13.0f32.to_bits());
static TYPOGRAPHY: RwLock<Option<Typography>> = RwLock::new(None);

#[derive(Clone)]
struct Typography {
    ui: SharedString,
    code: SharedString,
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            ui: "Geist".into(),
            code: "Geist Mono".into(),
        }
    }
}

pub fn ui_font() -> SharedString {
    TYPOGRAPHY
        .read()
        .expect("typography lock")
        .clone()
        .unwrap_or_default()
        .ui
}

pub fn code_font() -> SharedString {
    TYPOGRAPHY
        .read()
        .expect("typography lock")
        .clone()
        .unwrap_or_default()
        .code
}

/// Keep selected families when switching between light and dark palettes.
pub fn set_font_families(ui: &str, code: &str, cx: &mut App) {
    *TYPOGRAPHY.write().expect("typography lock") = Some(Typography {
        ui: ui.to_owned().into(),
        code: code.to_owned().into(),
    });
    Theme::update(cx, |theme| {
        theme.font_family = ui.to_owned().into();
        theme.mono_font_family = code.to_owned().into();
    });
}

/// Semantic colours used across Dunlin, in addition to the GPUI Kit theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub skin: crate::Skin,
    pub bg: Hsla,
    pub panel: Hsla,
    pub chrome: Hsla,
    pub sidebar: Hsla,
    pub card: Hsla,
    pub titlebar: Hsla,
    pub statusbar: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub fg: Hsla,
    pub muted_fg: Hsla,
    pub muted: Hsla,
    pub subtle: Hsla,
    pub accent: Hsla,
    pub accent_fg: Hsla,
    pub primary: Hsla,
    pub primary_fg: Hsla,
    pub hover: Hsla,
    pub overlay: Hsla,
    pub selection: Hsla,
    pub blue: Hsla,
    pub blue_soft: Hsla,
    pub green: Hsla,
    pub green_soft: Hsla,
    pub amber: Hsla,
    pub amber_soft: Hsla,
    pub red: Hsla,
    pub red_soft: Hsla,
    pub violet: Hsla,
    pub violet_soft: Hsla,
    pub teal: Hsla,
    pub graph_grid: Hsla,
    pub graph_edge: Hsla,
    pub graph_ring: Hsla,
}

fn h(hex: u32) -> Hsla {
    rgb(hex).into()
}

fn ha(hex: u32) -> Hsla {
    rgba(hex).into()
}

fn luminance(color: Hsla) -> f32 {
    let color: Rgba = color.into();
    let linear = |v: f32| {
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
}

fn contrast_ink(fill: Hsla) -> Hsla {
    if luminance(fill) > 0.179 {
        h(0x000000)
    } else {
        h(0xffffff)
    }
}

fn semantic_button_state(fill: Hsla, factor: f32) -> Hsla {
    let away_from_ink = if luminance(fill) > 0.179 {
        h(0xffffff)
    } else {
        h(0x000000)
    };
    fill.mix_oklab(away_from_ink, factor)
}

pub fn light() -> Palette {
    Palette {
        skin: crate::Skin::Default,
        bg: h(0xe3e7ee),
        panel: h(0xeeeeee),
        chrome: h(0xeeeeee).opacity(0.84),
        sidebar: h(0xeeeeee),
        card: h(0xedf0f4),
        titlebar: h(0xeeeeee),
        statusbar: h(0xeeeeee),
        border: h(0xcdd3dd),
        border_strong: h(0x7a8494),
        fg: h(0x252b35),
        muted_fg: h(0x48525f),
        muted: h(0xdce2ea),
        subtle: h(0xe2e7ee),
        accent: h(0xd3dbe6),
        accent_fg: h(0x18181b),
        primary: h(0x18181b),
        primary_fg: h(0xfafafa),
        hover: h(0xd6dfeb),
        overlay: ha(0x0000002e),
        selection: h(0xdbeafe),
        blue: h(0x2563eb),
        blue_soft: h(0xeff6ff),
        green: h(0x16a34a),
        green_soft: h(0xecfdf3),
        amber: h(0xb45309),
        amber_soft: h(0xfffbeb),
        red: h(0xdc2626),
        red_soft: h(0xfef2f2),
        violet: h(0x7c3aed),
        violet_soft: h(0xf5f3ff),
        teal: h(0x0d9488),
        graph_grid: ha(0x0000000f),
        graph_edge: h(0xd4d4d8),
        graph_ring: h(0xe4e4e7),
    }
}

pub fn dark() -> Palette {
    Palette {
        skin: crate::Skin::Default,
        bg: h(0x191d25),
        panel: h(0x1c2025),
        chrome: h(0x1c2025).opacity(0.82),
        sidebar: h(0x1c2025),
        card: h(0x1d212a),
        titlebar: h(0x1c2025),
        statusbar: h(0x1c2025),
        border: h(0x30343d),
        border_strong: h(0x707b8e),
        fg: h(0xf4f4f5),
        muted_fg: h(0xa1a1aa),
        muted: h(0x292f39),
        subtle: h(0x242a34),
        accent: h(0x343e4c),
        accent_fg: h(0xf4f4f5),
        primary: h(0xfafafa),
        primary_fg: h(0x18181b),
        hover: h(0x303946),
        overlay: ha(0x0000002e),
        selection: h(0x1e3a5f),
        blue: h(0x60a5fa),
        blue_soft: h(0x17233a),
        green: h(0x4ade80),
        green_soft: h(0x11261a),
        amber: h(0xfbbf24),
        amber_soft: h(0x2a2110),
        red: h(0xf87171),
        red_soft: h(0x2b1214),
        violet: h(0xa78bfa),
        violet_soft: h(0x211933),
        teal: h(0x2dd4bf),
        graph_grid: ha(0xffffff12),
        graph_edge: h(0x3f3f46),
        graph_ring: h(0x2a2a2e),
    }
}

#[derive(Default)]
struct ActiveSkin(crate::Skin);
impl Global for ActiveSkin {}

/// Resolve all six skin/appearance combinations without changing the original palettes.
pub fn for_skin(skin: crate::Skin, dark_mode: bool) -> Palette {
    let mut p = if dark_mode { dark() } else { light() };
    p.skin = skin;
    match skin {
        crate::Skin::Default => {}
        crate::Skin::Solid => {
            if dark_mode {
                p.bg = h(0x11151c);
                p.panel = h(0x141820);
                p.card = h(0x2d3542);
                p.border = h(0x515f73);
                p.border_strong = h(0x8899b3);
                p.muted_fg = h(0xbdc7d6);
                p.muted = h(0x343d4b);
                p.subtle = h(0x3d4a5e);
                p.accent = h(0x43526b);
                p.hover = h(0x414f65);
            } else {
                p.bg = h(0xc7c7c7);
                p.panel = h(0xbdbdbd);
                p.card = h(0xe2e2e2);
                p.border = h(0xa2a2a2);
                p.border_strong = h(0x6c6c6c);
                p.fg = h(0x202020);
                p.muted_fg = h(0x444444);
                p.muted = h(0xc3c3c3);
                p.subtle = h(0xc9c9c9);
                p.accent = h(0xb2b2b2);
                p.hover = h(0xbcbcbc);
            }
            p.sidebar = p.panel;
            p.titlebar = p.panel;
            p.statusbar = p.panel;
            p.chrome = p.panel;
        }
        crate::Skin::Grain => {
            if dark_mode {
                p.bg = h(0x060b15);
                p.panel = h(0x091120);
                p.card = h(0x1b3152);
                p.border = h(0x3b557a);
                p.border_strong = h(0x8095b4);
                p.fg = h(0xe8eef9);
                p.muted_fg = h(0xb1c0d9);
                p.muted = h(0x233858);
                p.subtle = h(0x2a4163);
                p.accent = h(0x304e73);
                p.hover = h(0x2c4668);
            } else {
                p.bg = h(0xe1e3e6);
                p.panel = h(0xdbdee2);
                p.card = h(0xeceef1);
                p.border = h(0xc9cdd3);
                p.border_strong = h(0x7a828e);
                p.fg = h(0x292e36);
                p.muted_fg = h(0x4b5563);
                p.muted = h(0xdddfe4);
                p.subtle = h(0xe2e5e9);
                p.accent = h(0xcdd3dc);
                p.hover = h(0xd6dbe2);
            }
            p.chrome = p.panel.opacity(0.97);
            p.sidebar = p.panel;
            p.titlebar = p.panel;
            p.statusbar = p.panel;
            p.accent_fg = p.fg;
        }
    }
    p
}

pub fn set_skin(skin: crate::Skin, cx: &mut App) {
    cx.set_global(ActiveSkin(skin));
    let state = cx.default_global::<ThemeTransition>();
    state.generation += 1;
    state.palette = None;
    apply(cx);
    cx.refresh_windows();
}

pub fn palette(cx: &App) -> Palette {
    if let Some(palette) = cx
        .try_global::<ThemeTransition>()
        .and_then(|state| state.palette)
    {
        return palette;
    }
    for_skin(
        cx.try_global::<ActiveSkin>()
            .map(|s| s.0)
            .unwrap_or_default(),
        cx.theme().is_dark(),
    )
}

#[derive(Default)]
struct ThemeTransition {
    palette: Option<Palette>,
    generation: u64,
}
impl Global for ThemeTransition {}

impl Palette {
    /// Floating widgets preserve Default's tint; new skins keep content opaque.
    pub fn floating_surface(self, default_opacity: f32) -> Hsla {
        match self.skin {
            crate::Skin::Default => self.card.opacity(default_opacity),
            crate::Skin::Solid | crate::Skin::Grain => self.card,
        }
    }

    fn mix(self, target: Self, progress: f32) -> Self {
        if progress <= 0. {
            return self;
        }
        if progress >= 1. {
            return target;
        }
        macro_rules! colors {
            ($($field:ident),+ $(,)?) => {
                Self { skin: target.skin, $($field: self.$field.mix_oklab(target.$field, 1. - progress)),+ }
            };
        }
        colors!(
            bg,
            panel,
            chrome,
            sidebar,
            card,
            titlebar,
            statusbar,
            border,
            border_strong,
            fg,
            muted_fg,
            muted,
            subtle,
            accent,
            accent_fg,
            primary,
            primary_fg,
            hover,
            overlay,
            selection,
            blue,
            blue_soft,
            green,
            green_soft,
            amber,
            amber_soft,
            red,
            red_soft,
            violet,
            violet_soft,
            teal,
            graph_grid,
            graph_edge,
            graph_ring
        )
    }
}

pub fn is_transitioning(cx: &App) -> bool {
    cx.try_global::<ThemeTransition>()
        .is_some_and(|state| state.palette.is_some())
}

#[cfg(test)]
mod palette_contrast_tests {
    use super::*;
    fn luminance(color: Hsla) -> f32 {
        let color: Rgba = color.into();
        let linear = |v: f32| {
            if v <= 0.04045 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * linear(color.r) + 0.7152 * linear(color.g) + 0.0722 * linear(color.b)
    }
    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let a = luminance(a);
        let b = luminance(b);
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
    #[test]
    fn all_skins_keep_reading_surfaces_opaque_and_text_accessible() {
        for skin in crate::Skin::ALL {
            for dark in [false, true] {
                let p = for_skin(skin, dark);
                assert_eq!(p.skin, skin);
                for surface in [p.bg, p.panel, p.card, p.subtle, p.hover, p.accent] {
                    assert_eq!(surface.a, 1.);
                    assert!(contrast(p.fg, surface) >= 7., "{skin:?}, dark={dark}");
                }
                for surface in [p.bg, p.panel, p.card, p.subtle] {
                    assert!(
                        contrast(p.muted_fg, surface) >= 4.5,
                        "{skin:?}, dark={dark}"
                    );
                }
                assert!(contrast(p.border_strong, p.card) >= 3.);
                assert_eq!(crate::chrome_background(p, true).a, 1.);
                if skin == crate::Skin::Solid {
                    assert_eq!(p.chrome.a, 1.);
                }
                if skin == crate::Skin::Grain {
                    assert_eq!(p.chrome.a, 0.97);
                }
                if skin != crate::Skin::Default {
                    assert_eq!(p.floating_surface(0.88).a, 1.);
                }
            }
        }
    }

    #[test]
    fn default_skin_preserves_existing_palettes() {
        assert_eq!(for_skin(crate::Skin::Default, false).chrome, light().chrome);
        assert_eq!(for_skin(crate::Skin::Default, true).chrome, dark().chrome);
        assert_eq!(for_skin(crate::Skin::Default, false).card, light().card);
        assert_eq!(for_skin(crate::Skin::Default, true).card, dark().card);
    }

    #[test]
    fn solid_light_uses_darker_neutral_grays_with_separate_reading_surfaces() {
        let p = for_skin(crate::Skin::Solid, false);
        for color in [
            p.bg,
            p.panel,
            p.card,
            p.chrome,
            p.sidebar,
            p.titlebar,
            p.statusbar,
            p.subtle,
            p.muted,
            p.hover,
            p.accent,
            p.border,
            p.border_strong,
        ] {
            let color: Rgba = color.into();
            assert!((color.r - color.g).abs() < 0.001);
            assert!((color.g - color.b).abs() < 0.001);
            assert_eq!(color.a, 1.);
        }
        assert!((0.45..0.65).contains(&luminance(p.bg)));
        assert_eq!(p.panel, h(0xbdbdbd));
        assert!(luminance(p.card) > luminance(p.panel));
    }

    #[test]
    fn solid_and_dark_grain_keep_chrome_content_and_composer_visibly_separate() {
        for (skin, dark) in [
            (crate::Skin::Solid, false),
            (crate::Skin::Solid, true),
            (crate::Skin::Grain, true),
        ] {
            let p = for_skin(skin, dark);
            assert!(
                contrast(p.panel, p.card) >= 1.4,
                "{skin:?}, dark={dark}: chrome/content"
            );
            assert!(
                contrast(p.card, p.subtle) >= 1.25,
                "{skin:?}, dark={dark}: content/composer"
            );
            for surface in [p.panel, p.card, p.subtle, p.accent, p.hover] {
                assert!(contrast(p.fg, surface) >= 7.);
                assert!(contrast(p.muted_fg, surface) >= 4.5);
            }
        }
    }

    #[test]
    fn grain_uses_deep_midnight_navy_and_cool_gray_light_surfaces() {
        let p = for_skin(crate::Skin::Grain, true);
        for color in [p.bg, p.panel, p.card, p.subtle, p.muted, p.accent, p.hover] {
            let color: Rgba = color.into();
            assert!(color.b > color.g && color.g > color.r);
        }
        assert!(luminance(p.bg) < 0.004);
        assert!(luminance(p.card) > luminance(p.panel));
        assert_eq!(p.chrome.a, 0.97);
        let light = for_skin(crate::Skin::Grain, false);
        assert_eq!(light.bg, h(0xe1e3e6));
        assert_eq!(light.panel, h(0xdbdee2));
        assert_eq!(light.card, h(0xeceef1));
        for color in [light.bg, light.panel, light.card] {
            let color: Rgba = color.into();
            assert!(color.b >= color.g && color.g >= color.r);
            assert!(color.b - color.r < 0.04);
        }
    }

    #[test]
    fn grain_chrome_keeps_readable_text_over_extreme_backdrops_and_noise() {
        for dark in [false, true] {
            let p = for_skin(crate::Skin::Grain, dark);
            let tint: Rgba = p.chrome.into();
            for backdrop in [0., 1.] {
                for noise in [0., 1.] {
                    let channel = |c: f32| {
                        let base = c * tint.a + backdrop * (1. - tint.a);
                        base * (1. - 9. / 255.) + noise * (9. / 255.)
                    };
                    let background = Rgba {
                        r: channel(tint.r),
                        g: channel(tint.g),
                        b: channel(tint.b),
                        a: 1.,
                    }
                    .into();
                    assert!(contrast(p.fg, background) >= 7.);
                    assert!(contrast(p.muted_fg, background) >= 4.5);
                }
            }
        }
    }
    #[test]
    fn both_palettes_keep_quiet_dividers_and_accessible_inputs() {
        for p in [light(), dark()] {
            assert!(contrast(p.border, p.card) > 1. && contrast(p.border, p.card) < 2.);
            assert!(contrast(p.border_strong, p.card) >= 3.);
            assert!(contrast(p.fg, p.card) >= 7.);
            assert_ne!(p.panel, p.card);
        }
    }

    #[test]
    fn light_surfaces_are_softened_and_status_metadata_is_readable() {
        let p = light();
        assert!(luminance(p.card) < 0.9);
        assert!(luminance(p.card) > luminance(p.subtle));
        for p in [p, dark()] {
            assert_eq!(p.statusbar.a, 1.);
            assert!(contrast(p.muted_fg, p.statusbar) >= 4.5);
            assert!(contrast(p.fg, p.statusbar) >= 7.);
        }
    }

    #[test]
    fn shared_chrome_is_readable_over_any_backdrop_and_light_tint_is_translucent_gray() {
        let tint: Rgba = light().chrome.into();
        assert!((tint.r - tint.g).abs() < 0.001 && (tint.g - tint.b).abs() < 0.001);
        assert!((0.8..=0.85).contains(&tint.a));
        for p in [light(), dark()] {
            let tint: Rgba = p.chrome.into();
            for backdrop in [0., 1.] {
                let background: Hsla = Rgba {
                    r: tint.r * tint.a + backdrop * (1. - tint.a),
                    g: tint.g * tint.a + backdrop * (1. - tint.a),
                    b: tint.b * tint.a + backdrop * (1. - tint.a),
                    a: 1.,
                }
                .into();
                assert!(contrast(p.fg, background) >= 7.);
                if p.card.l > 0.5 {
                    assert!(contrast(p.muted_fg, background) >= 4.5);
                }
            }
        }
    }

    #[test]
    fn theme_transition_blends_surfaces_ink_and_glass_without_endpoint_jumps() {
        let light = light();
        let dark = dark();
        let middle = light.mix(dark, 0.5);
        assert_eq!(light.mix(dark, 0.).card, light.card);
        assert_eq!(light.mix(dark, 1.).card, dark.card);
        assert!(middle.card.l > dark.card.l && middle.card.l < light.card.l);
        assert!(light.mix(dark, 0.25).card.l > light.mix(dark, 0.75).card.l);
        assert!((light.mix(dark, 0.001).card.l - light.card.l).abs() < 0.01);
        assert!((light.mix(dark, 0.999).card.l - dark.card.l).abs() < 0.01);
        assert!(middle.fg.l > light.fg.l && middle.fg.l < dark.fg.l);
        assert!((middle.chrome.a - (light.chrome.a + dark.chrome.a) / 2.).abs() < 0.01);
        assert_eq!(middle.mix(light, 0.).chrome, middle.chrome);
        assert_eq!(middle.mix(light, 1.).chrome, light.chrome);
        assert!(dark.hover.l > dark.card.l && (dark.hover.h - dark.accent.h).abs() < 0.03);
    }
}

const FONTS: [&[u8]; 7] = [
    include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Medium.ttf"),
    include_bytes!("../../../assets/fonts/SymbolsNerdFontMono-Regular.ttf"),
];

/// Register bundled fonts and install Dunlin's tokens over GPUI Kit.
pub fn install(cx: &mut App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("bundled Geist fonts must be valid");

    apply(cx);
}

/// Re-apply Dunlin's palette after a light/dark switch.
pub fn apply(cx: &mut App) {
    let p = palette(cx);
    let ui = ui_font();
    let code = code_font();
    Theme::update(cx, |theme| {
        theme.font_family = ui;
        theme.mono_font_family = code;
        theme.font_size = px(13.);
        theme.mono_font_size = px(f32::from_bits(CODE_FONT_SIZE.load(Ordering::Relaxed)));
        theme.radius = px(crate::scale::RADIUS_MD);
        theme.radius_lg = px(crate::scale::RADIUS_LG);
        theme.focus_ring = false;
        // The fixed line-number gutter must remain opaque while source text
        // scrolls underneath it, including editors without form appearance.
        let highlight = std::sync::Arc::make_mut(&mut theme.highlight_theme);
        highlight.style.editor_background = Some(p.card);
        highlight.style.editor_gutter_background = Some(p.card);
        highlight.style.editor_foreground = Some(p.fg);

        let c = &mut theme.colors;
        c.background = p.bg;
        c.foreground = p.fg;
        c.border = p.border;
        c.input = p.border_strong;
        c.muted = p.muted;
        c.muted_foreground = p.muted_fg;
        c.secondary = p.muted;
        c.secondary_hover = p.hover;
        c.secondary_active = p.accent;
        c.secondary_foreground = p.fg;
        c.accent = p.accent;
        c.accent_foreground = p.accent_fg;
        c.primary = p.primary;
        c.primary_hover = p.primary.mix_oklab(p.primary_fg, 0.9);
        c.primary_active = p.primary.mix_oklab(p.primary_fg, 0.8);
        c.primary_foreground = p.primary_fg;
        c.button = p.subtle;
        c.button_hover = p.hover;
        c.button_active = p.accent;
        c.button_foreground = p.fg;
        c.button_primary = c.primary;
        c.button_primary_foreground = c.primary_foreground;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.button_secondary = c.secondary;
        c.button_secondary_foreground = c.secondary_foreground;
        c.button_secondary_hover = c.secondary_hover;
        c.button_secondary_active = c.secondary_active;
        c.sidebar = p.sidebar;
        c.sidebar_border = p.border;
        c.sidebar_foreground = p.fg;
        c.sidebar_accent = p.accent;
        c.sidebar_accent_foreground = p.accent_fg;
        c.sidebar_primary = p.primary;
        c.sidebar_primary_foreground = p.primary_fg;
        c.title_bar = p.titlebar;
        c.title_bar_border = p.border;
        c.status_bar = p.statusbar;
        c.status_bar_border = p.border;
        c.tab_bar = p.panel;
        c.tab_bar_segmented = p.muted;
        c.tab = p.panel;
        c.tab_active = p.card;
        c.tab_active_foreground = p.fg;
        c.tab_foreground = p.muted_fg;
        c.list = p.card;
        c.list_hover = p.hover;
        c.list_active = p.accent;
        c.list_active_border = p.border;
        c.list_even = p.subtle;
        c.list_head = p.panel;
        c.table = p.card;
        c.table_hover = p.hover;
        c.table_active = p.accent;
        c.table_active_border = p.border;
        c.table_head = p.panel;
        c.table_row_border = p.border;
        c.popover = p.card;
        c.popover_foreground = p.fg;
        c.ring = p.blue;
        c.switch = p.border_strong;
        c.selection = p.selection;
        c.caret = p.fg;
        c.progress_bar = p.muted;
        c.skeleton = p.muted;
        c.scrollbar = p.bg.opacity(0.0);
        c.scrollbar_thumb = p.border_strong.opacity(0.55);
        c.scrollbar_thumb_hover = p.muted_fg.opacity(0.55);
        c.overlay = p.overlay;
        c.info = p.blue;
        c.info_foreground = contrast_ink(p.blue);
        c.success = p.green;
        c.success_foreground = contrast_ink(p.green);
        c.warning = p.amber;
        c.warning_foreground = contrast_ink(p.amber);
        c.danger = p.red;
        c.danger_foreground = contrast_ink(p.red);
        c.danger_hover = semantic_button_state(p.red, 0.9);
        c.danger_active = semantic_button_state(p.red, 0.8);
        c.button_danger = c.danger;
        c.button_danger_foreground = c.danger_foreground;
        c.button_danger_hover = c.danger_hover;
        c.button_danger_active = c.danger_active;
        c.button_info = c.info;
        c.button_info_foreground = c.info_foreground;
        c.button_info_hover = semantic_button_state(p.blue, 0.9);
        c.button_info_active = semantic_button_state(p.blue, 0.8);
        c.button_success = c.success;
        c.button_success_foreground = c.success_foreground;
        c.button_success_hover = semantic_button_state(p.green, 0.9);
        c.button_success_active = semantic_button_state(p.green, 0.8);
        c.button_warning = c.warning;
        c.button_warning_foreground = c.warning_foreground;
        c.button_warning_hover = semantic_button_state(p.amber, 0.9);
        c.button_warning_active = semantic_button_state(p.amber, 0.8);
        c.blue = p.blue;
        c.green = p.green;
        c.yellow = p.amber;
        c.red = p.red;
        c.magenta = p.violet;
    });
}

/// Switch appearance, then restore Dunlin's tokens over the loaded theme.
pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    let state = cx.default_global::<ThemeTransition>();
    state.generation += 1;
    state.palette = None;
    Theme::change(mode, window, cx);
    apply(cx);
}

/// Interpolate shared surfaces and component tokens; interrupted switches start at the visible colors.
pub fn set_mode_animated(mode: ThemeMode, window: &mut Window, reduce_motion: bool, cx: &mut App) {
    if cx.theme().mode == mode {
        return;
    }
    let from = palette(cx);
    set_mode(mode, Some(window), cx);
    if reduce_motion {
        return;
    }
    let target = palette(cx);
    let state = cx.default_global::<ThemeTransition>();
    state.palette = Some(from);
    let generation = state.generation;
    apply(cx);
    let started = Instant::now();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let done = cx.update(|cx| {
                if cx.global::<ThemeTransition>().generation != generation {
                    return true;
                }
                let t = (started.elapsed().as_secs_f32() / 0.24).min(1.);
                cx.global_mut::<ThemeTransition>().palette =
                    (t < 1.).then(|| from.mix(target, t * t * (3. - 2. * t)));
                apply(cx);
                t >= 1.
            });
            if done {
                break;
            }
        }
    })
    .detach();
}

/// Set the monospaced text size used by editors, diffs and code metadata.
pub fn set_code_font_size(size: f32, cx: &mut App) {
    let size = size.clamp(10.0, 20.0);
    CODE_FONT_SIZE.store(size.to_bits(), Ordering::Relaxed);
    Theme::update(cx, |theme| {
        theme.mono_font_size = px(size);
    });
}

pub fn toggle_mode(window: &mut Window, cx: &mut App) {
    let next = if cx.theme().is_dark() {
        ThemeMode::Light
    } else {
        ThemeMode::Dark
    };
    set_mode(next, Some(window), cx);
}

/// Card surface shared by panels, tool calls and graph nodes.
pub fn card(cx: &App) -> Div {
    crate::card_surface(palette(cx))
}

#[cfg(test)]
mod contrast_tests {
    use super::*;

    fn contrast(a: Hsla, b: Hsla) -> f32 {
        let (a, b) = (luminance(a), luminance(b));
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    #[test]
    fn semantic_outline_actions_keep_readable_ink_in_every_pointer_state() {
        for (p, dark) in [(light(), false), (dark(), true)] {
            for color in [p.red, p.amber, p.green, p.blue] {
                let ink = if dark {
                    color
                } else {
                    color.mix_oklab(h(0x000000), 0.75)
                };
                for (factor, opacity) in [(1., 0.04), (0.9, 0.08), (0.8, 0.12)] {
                    let tint: Rgba = semantic_button_state(color, factor).into();
                    let base: Rgba = p.card.into();
                    let background: Hsla = Rgba {
                        r: tint.r * opacity + base.r * (1. - opacity),
                        g: tint.g * opacity + base.g * (1. - opacity),
                        b: tint.b * opacity + base.b * (1. - opacity),
                        a: 1.,
                    }
                    .into();
                    assert!(
                        contrast(ink, background) >= 4.5,
                        "outline action contrast, dark={dark}, state={factor}"
                    );
                }
            }
        }
    }

    #[test]
    fn reading_surfaces_inputs_and_all_filled_button_states_are_legible() {
        for p in [light(), dark()] {
            assert!(contrast(p.fg, p.card) >= 7.);
            assert!(contrast(p.muted_fg, p.card) >= 4.5);
            assert!(contrast(p.border_strong, p.card) >= 3.);
            for fill in [p.subtle, p.hover, p.accent] {
                assert!(contrast(p.fg, fill) >= 7., "neutral action contrast");
            }
            assert_ne!(p.subtle, p.hover);
            assert_ne!(p.hover, p.accent);
            for factor in [1., 0.9, 0.8] {
                assert!(contrast(p.primary.mix_oklab(p.primary_fg, factor), p.primary_fg) >= 4.5);
                for fill in [p.blue, p.green, p.amber, p.red] {
                    let ratio = contrast(semantic_button_state(fill, factor), contrast_ink(fill));
                    assert!(
                        ratio >= 4.5,
                        "filled button contrast: {ratio}, factor {factor}"
                    );
                }
            }
        }
    }
}
