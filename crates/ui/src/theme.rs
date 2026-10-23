//! Kestrel design system.
//!
//! The visual language mirrors Kobra's calm, shadcn-neutral surfaces and uses
//! Geist for UI text and Geist Mono for code/metadata. All tokens are defined
//! here so the application never reaches for hard-coded colours.

use std::borrow::Cow;
use std::sync::atomic::{AtomicU32, Ordering};

use gpui_kit::component::{ActiveTheme, Theme, ThemeMode};
use gpui_kit::*;

/// Monospaced text size shared by editors, diffs and code chips.
static CODE_FONT_SIZE: AtomicU32 = AtomicU32::new(13.0f32.to_bits());

/// Semantic colours used across Kestrel, in addition to the GPUI Kit theme.
#[derive(Clone, Copy)]
pub struct Palette {
    pub bg: Hsla,
    pub panel: Hsla,
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

pub fn light() -> Palette {
    Palette {
        bg: h(0xffffff),
        panel: h(0xfafafa),
        sidebar: h(0xfafafa),
        card: h(0xffffff),
        titlebar: h(0xfbfbfc),
        statusbar: h(0xfafafa),
        border: h(0xe6e6e9),
        border_strong: h(0xd4d4d8),
        fg: h(0x101012),
        muted_fg: h(0x71717a),
        muted: h(0xf4f4f5),
        subtle: h(0xf8f8f9),
        accent: h(0xf2f2f4),
        accent_fg: h(0x18181b),
        primary: h(0x18181b),
        primary_fg: h(0xfafafa),
        hover: h(0xf4f4f5),
        overlay: ha(0x09090b66),
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
        bg: h(0x0a0a0a),
        panel: h(0x111113),
        sidebar: h(0x111113),
        card: h(0x151517),
        titlebar: h(0x101012),
        statusbar: h(0x101012),
        border: ha(0xffffff14),
        border_strong: h(0x2a2a2e),
        fg: h(0xf4f4f5),
        muted_fg: h(0xa1a1aa),
        muted: h(0x1d1d20),
        subtle: h(0x141416),
        accent: h(0x26262a),
        accent_fg: h(0xf4f4f5),
        primary: h(0xfafafa),
        primary_fg: h(0x18181b),
        hover: h(0x202024),
        overlay: ha(0x00000099),
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

pub fn palette(cx: &App) -> Palette {
    if cx.theme().is_dark() {
        dark()
    } else {
        light()
    }
}

const FONTS: [&[u8]; 6] = [
    include_bytes!("../../../assets/fonts/Geist-Regular.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Medium.ttf"),
    include_bytes!("../../../assets/fonts/Geist-SemiBold.ttf"),
    include_bytes!("../../../assets/fonts/Geist-Bold.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Regular.ttf"),
    include_bytes!("../../../assets/fonts/GeistMono-Medium.ttf"),
];

/// Register bundled fonts and install Kestrel's tokens over GPUI Kit.
pub fn install(cx: &mut App) {
    cx.text_system()
        .add_fonts(FONTS.iter().map(|bytes| Cow::Borrowed(*bytes)).collect())
        .expect("bundled Geist fonts must be valid");

    apply(cx);
}

/// Re-apply Kestrel's palette after a light/dark switch.
pub fn apply(cx: &mut App) {
    let p = palette(cx);
    Theme::update(cx, |theme| {
        theme.font_family = "Geist".into();
        theme.mono_font_family = "Geist Mono".into();
        theme.font_size = px(13.);
        theme.mono_font_size = px(f32::from_bits(CODE_FONT_SIZE.load(Ordering::Relaxed)));
        theme.radius = px(10.);
        theme.radius_lg = px(14.);
        theme.focus_ring = false;

        let c = &mut theme.colors;
        c.background = p.bg;
        c.foreground = p.fg;
        c.border = p.border;
        c.input = p.border;
        c.muted = p.muted;
        c.muted_foreground = p.muted_fg;
        c.secondary = p.muted;
        c.secondary_hover = p.hover;
        c.secondary_active = p.accent;
        c.secondary_foreground = p.fg;
        c.accent = p.accent;
        c.accent_foreground = p.accent_fg;
        c.primary = p.primary;
        c.primary_hover = p.primary;
        c.primary_active = p.primary;
        c.primary_foreground = p.primary_fg;
        c.button = p.card;
        c.button_hover = p.hover;
        c.button_active = p.accent;
        c.button_foreground = p.fg;
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
        c.selection = p.selection;
        c.caret = p.fg;
        c.progress_bar = p.muted;
        c.skeleton = p.muted;
        c.scrollbar = p.bg.opacity(0.0);
        c.scrollbar_thumb = p.border_strong.opacity(0.55);
        c.scrollbar_thumb_hover = p.muted_fg.opacity(0.55);
        c.overlay = p.overlay;
        c.info = p.blue;
        c.info_foreground = h(0xffffff);
        c.success = p.green;
        c.success_foreground = h(0xffffff);
        c.warning = p.amber;
        c.warning_foreground = h(0xffffff);
        c.danger = p.red;
        c.danger_foreground = h(0xffffff);
        c.blue = p.blue;
        c.green = p.green;
        c.yellow = p.amber;
        c.red = p.red;
        c.magenta = p.violet;
    });
}

/// Switch appearance, then restore Kestrel's tokens over the loaded theme.
pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    Theme::change(mode, window, cx);
    apply(cx);
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
    let p = palette(cx);
    div()
        .rounded(px(12.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
}
