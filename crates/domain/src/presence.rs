//! Relay-local presence. This deliberately carries no device identifiers,
//! projects, conversations, credentials or connections to other hosts.
use serde::{Deserialize, Serialize};

pub const HEARTBEAT_SECS: u64 = 5;
pub const PRESENCE_TTL_SECS: u64 = 30;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub devices: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub self_user_id: String,
    pub users: Vec<User>,
    pub ttl_secs: u64,
    /// Counts only jobs in projects explicitly granted to the caller.
    #[serde(default)]
    pub workload: Option<Workload>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workload {
    pub queued: usize,
    pub active: usize,
    pub interrupted: usize,
    pub paused: bool,
}
