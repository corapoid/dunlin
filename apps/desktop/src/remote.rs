//! Desktop adapter: relay services live in `dunlin-relay-client`; credentials
//! and transcript-upload diagnostics remain owned by the desktop application.
pub(crate) use dunlin_relay_client::*;
mod credentials;
pub(crate) use credentials::*;
pub(crate) mod transcripts;
