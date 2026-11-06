//! Dunlin's use-only credential broker and MCP stdio proxy.

pub mod broker;
pub mod client;
pub mod http;
#[cfg(any(target_os = "linux", test))]
mod linux_store;
pub mod mcp;
pub mod protocol;
pub mod recipe;
pub mod store;
