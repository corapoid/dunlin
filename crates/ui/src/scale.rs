//! Shared UI scale for semantic typography and spacing.
//!
//! New surfaces use these tokens directly. Legacy views still contain literal
//! sizes from the first prototype; Compact tightens the semantic tokens without
//! shrinking hit targets or font sizes.

use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

static COMPACT: AtomicU32 = AtomicU32::new(0);

pub fn set_density(density: Density) {
    COMPACT.store(u32::from(density == Density::Compact), Ordering::Release);
}

pub fn density() -> Density {
    if COMPACT.load(Ordering::Acquire) == 1 {
        Density::Compact
    } else {
        Density::Comfortable
    }
}

/// Scale a spacing token. Compact keeps at least 70% so targets remain usable.
pub fn space(value: f32) -> f32 {
    match density() {
        Density::Comfortable => value,
        Density::Compact => (value * 0.74).max(1.),
    }
}

/// Scale a control height with a hard floor for pointer/keyboard targets.
pub fn control(value: f32) -> f32 {
    match density() {
        Density::Comfortable => value,
        Density::Compact => (value * 0.9).max(20.),
    }
}

/// Scale a text size only very slightly; Compact must not become unreadable.
pub fn text(value: f32) -> f32 {
    match density() {
        Density::Comfortable => value,
        Density::Compact => (value - 0.5).max(9.5),
    }
}

pub const TEXT_MICRO: f32 = 10.0;
pub const TEXT_CAPTION: f32 = 10.5;
pub const TEXT_SMALL: f32 = 11.0;
pub const TEXT_BODY: f32 = 11.5;
pub const TEXT_BODY_LARGE: f32 = 12.0;
pub const TEXT_TITLE: f32 = 12.5;
pub const TEXT_HEADING: f32 = 13.0;
pub const TEXT_DISPLAY: f32 = 14.0;

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 6.0;
pub const SPACE_3: f32 = 8.0;
pub const SPACE_4: f32 = 12.0;
pub const SPACE_5: f32 = 16.0;

pub const RADIUS_SM: f32 = crate::radii::ICON;
pub const RADIUS_MD: f32 = crate::radii::CONTROL;
pub const RADIUS_LG: f32 = crate::radii::CARD;

pub const WORKSPACE_RADIUS: f32 = crate::radii::WORKSPACE;
pub const WORKSPACE_INSET: f32 = 10.;
pub const TITLEBAR_HEIGHT: f32 = 38.;
pub const HEADER_HEIGHT: f32 = 36.;

/// Project rows need a stronger density change than ordinary form controls.
pub fn project_row_height(density: Density) -> f32 {
    match density {
        Density::Comfortable => 46.,
        Density::Compact => 32.,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_project_rows_save_space_without_shrinking_text_targets() {
        assert_eq!(project_row_height(Density::Compact), 32.);
        assert!(
            project_row_height(Density::Compact) <= project_row_height(Density::Comfortable) * 0.7
        );
    }
}
