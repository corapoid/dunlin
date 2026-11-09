//! Shared workspace operations used by the desktop app and the relay.
//!
//! Keeping tree scanning, search and git in one crate guarantees that a remote
//! project behaves the same as a local one.

pub mod catalog_backup;
pub mod checks;
pub mod cleanup;
pub mod comparison;
pub mod fs;
pub mod git;
pub mod github;
pub mod gitlab;
pub mod hosting_account;
pub mod impact;
pub mod languages;
pub mod preview;
pub mod project_copy;
pub mod repositories;
pub mod run;
pub mod search;
pub mod server;
pub mod subtask;
pub mod syntax;
pub mod task_review;
pub mod transfer;
pub mod tree;
