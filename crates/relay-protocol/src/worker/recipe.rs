//! Shared domain export of the secret-free semantic recipe pin.
//! This module does not enable recipe execution on any worker transport.
pub use dunlin_domain::vault::{RecipeTemplate, TemplateParameter};

/// Separate v6 metadata/negotiation codecs; no active socket accepts these yet.
pub mod private;
