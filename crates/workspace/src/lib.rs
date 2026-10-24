//! Shared workspace operations used by the desktop app and the relay.
//!
//! Keeping tree scanning, search and git in one crate guarantees that a remote
//! project behaves the same as a local one.

pub mod fs;
pub mod git;
pub mod search;
pub mod tree;
