//! Shared workspace operations used by the desktop app and the relay.
//!
//! Keeping tree scanning, search and git in one crate guarantees that a remote
//! project behaves the same as a local one.

pub mod checks;
pub mod comparison;
pub mod fs;
pub mod git;
pub mod github;
pub mod impact;
pub mod preview;
pub mod repositories;
pub mod run;
pub mod search;
pub mod task_review;
pub mod transfer;
pub mod tree;
