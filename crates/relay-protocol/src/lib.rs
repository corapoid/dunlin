//! Wire contracts shared by the relay and its clients. No transport or storage.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct User {
    pub id: String,
    pub name: String,
    pub role: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize)]
pub struct PairRequest {
    pub code: String,
    pub device_name: String,
}

// Deliberately no Debug: pairing responses contain a bearer token.
#[derive(Serialize, Deserialize)]
pub struct PairResponse {
    pub token: String,
    pub user: User,
    pub device_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub last_device_id: Option<String>,
    pub active_device_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_value, json, to_value};

    #[test]
    fn pairing_preserves_the_existing_v1_shape() {
        let wire = json!({"token":"test-token", "user":{"id":"u", "name":"admin", "role":"admin"}, "device_id":"d"});
        let response: PairResponse = from_value(wire.clone()).unwrap();
        assert_eq!(to_value(response).unwrap(), wire);
        let request: PairRequest = from_value(json!({"code":"ABC", "device_name":"CLI"})).unwrap();
        assert_eq!(
            to_value(request).unwrap(),
            json!({"code":"ABC", "device_name":"CLI"})
        );
    }

    #[test]
    fn session_accepts_legacy_missing_optional_device_fields() {
        let session: SessionInfo =
            from_value(json!({"id":"s", "title":"Session", "updated_at":42})).unwrap();
        assert_eq!(session.last_device_id, None);
        assert_eq!(session.active_device_id, None);
        assert_eq!(
            to_value(session).unwrap(),
            json!({"id":"s", "title":"Session", "updated_at":42, "last_device_id":null, "active_device_id":null})
        );
    }

    #[test]
    fn project_preserves_the_existing_v1_shape() {
        let wire = json!({"id":"p", "name":"Project", "path":"/srv/project"});
        assert_eq!(
            to_value(from_value::<Project>(wire.clone()).unwrap()).unwrap(),
            wire
        );
    }
}
