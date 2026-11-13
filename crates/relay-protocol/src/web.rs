//! Read-only browser bootstrap. This is deliberately not an AHP implementation.
use serde::{Deserialize, Serialize};

use crate::User;

pub const CATALOG_VERSION: u8 = 1;
pub const MAX_CLIENT_FRAME: usize = 4096;
pub const MAX_SNAPSHOT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CATALOG_PROJECTS: usize = 500;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CatalogCommand {
    Subscribe {
        protocol_version: u8,
        project_id: Option<String>,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CatalogProject {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CatalogSession {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct CatalogSnapshot {
    pub protocol_version: u8,
    pub stream_id: String,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub workspace_revision: u64,
    pub server_name: String,
    pub user: User,
    pub projects: Vec<CatalogProject>,
    pub project_id: Option<String>,
    pub sessions: Vec<CatalogSession>,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CatalogEvent {
    Snapshot { snapshot: Box<CatalogSnapshot> },
    ResyncRequired,
    Error { code: String },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subscription_is_versioned_and_rejects_undeclared_authority() {
        let command = json!({"type":"subscribe", "protocol_version":1, "project_id":null});
        assert_eq!(
            serde_json::from_value::<CatalogCommand>(command).unwrap(),
            CatalogCommand::Subscribe {
                protocol_version: CATALOG_VERSION,
                project_id: None,
            }
        );
        assert!(
            serde_json::from_value::<CatalogCommand>(json!({
                "type":"subscribe", "protocol_version":1, "project_id":null,
                "user_id":"someone-else"
            }))
            .is_err()
        );
    }

    #[test]
    fn snapshots_round_trip_without_paths_or_credentials() {
        let event = CatalogEvent::Snapshot {
            snapshot: Box::new(CatalogSnapshot {
                protocol_version: CATALOG_VERSION,
                stream_id: "stream-fixture".into(),
                sequence: 1,
                workspace_revision: 0,
                server_name: "fixture".into(),
                user: User {
                    id: "u".into(),
                    name: "member".into(),
                    role: "member".into(),
                },
                projects: vec![CatalogProject {
                    id: "p".into(),
                    name: "project".into(),
                }],
                project_id: None,
                sessions: vec![],
            }),
        };
        let encoded = serde_json::to_value(&event).unwrap();
        assert_eq!(encoded["type"], "snapshot");
        assert!(encoded["snapshot"]["projects"][0].get("path").is_none());
        assert!(encoded["snapshot"].get("token").is_none());
        assert_eq!(
            serde_json::from_value::<CatalogEvent>(encoded).unwrap(),
            event
        );
    }

    #[test]
    fn shared_browser_fixture_matches_rust_contract() {
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/web-catalog/snapshot.json"
        ))
        .unwrap();
        let event: CatalogEvent = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(event).unwrap(), fixture);
    }
}
