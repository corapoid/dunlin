//! Generic, file-first knowledge spine.
//!
//! The host owns storage, files, UI and permissions. Plugins contribute
//! schemas and pure transformations. This crate deliberately has no SQLite,
//! GPUI or process dependencies so the same model can be used locally and over
//! a relay.

mod context;
mod discover;
mod graph;
mod model;
mod parse;

pub use context::{
    EnvelopeOptions, EnvelopeSection, IntentEnvelope, build_envelope, build_envelope_from_parts,
};
pub use discover::{MAX_DEPTH, MAX_FILE_BYTES, MAX_FILES, MAX_TOTAL_BYTES, discover};
pub use graph::{GraphIssue, Severity, topological_order, validate_graph};
pub use model::{
    API_VERSION, FieldDef, FieldKind, KnowledgeEdge, KnowledgeGraph, KnowledgeKind,
    KnowledgeObject, KnowledgeSchema, MAX_EDGES, MAX_OBJECTS, RelationDef,
};
pub use parse::{ParsedDocument, parse_document, parse_frontmatter};
