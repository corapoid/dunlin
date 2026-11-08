//! OpenSSH-backed execution fabric.
//!
//! Dunlin never implements SSH cryptography itself. Host configuration,
//! known_hosts verification, agent auth and ControlMaster are delegated to the
//! user's OpenSSH installation. This crate adds host profiles, Keychain
//! passphrase storage, relay bootstrap and a loopback-only tunnel.

pub mod keychain;
pub mod service;

pub use service::{
    BootstrapResult, ResolvedHost, Tunnel, bootstrap, check_master, control_path, ensure_master,
    exit_master, import_aliases, pair, resolve, start_tunnel,
};
