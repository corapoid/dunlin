//! Shared UI scale for new surfaces.
//!
//! Existing views still contain a few literal sizes from the first prototype.
//! New code should use these tokens so a future Compact/Comfortable setting can
//! scale the whole interface without hunting through every view.

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

pub const RADIUS_SM: f32 = 5.0;
pub const RADIUS_MD: f32 = 8.0;
pub const RADIUS_LG: f32 = 12.0;
