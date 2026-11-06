//! Authenticated hosting identity, never Git author configuration or repo ownership.
use crate::git::hosting::Provider;
use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct Profile {
    pub provider: Provider,
    pub login: String,
    pub name: String,
    pub avatar_url: Option<String>,
}
#[derive(Deserialize)]
struct User {
    id: u64,
    #[serde(alias = "username")]
    login: String,
    name: Option<String>,
    avatar_url: Option<String>,
}
fn profile(user: User, provider: Provider) -> Result<Profile, String> {
    if user.id == 0
        || user.login.is_empty()
        || user.login.len() > 100
        || !user
            .login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err("Invalid hosting account identity".into());
    }
    let name = user
        .name
        .filter(|s| !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(char::is_control))
        .unwrap_or_else(|| user.login.clone());
    Ok(Profile {
        provider,
        login: user.login,
        name,
        avatar_url: user.avatar_url,
    })
}
pub fn load(provider: Provider) -> Result<Profile, String> {
    let user = match provider {
        Provider::GitHub => crate::github::imports::api("user", &[])?,
        Provider::GitLab => crate::gitlab::api("user", &[])?,
    };
    profile(user, provider)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_account_contracts_use_the_authenticated_id_and_fall_back_to_login() {
        for (key, provider) in [("login", Provider::GitHub), ("username", Provider::GitLab)] {
            let user: User = serde_json::from_value(
                serde_json::json!({"id":42, key:"fixture", "name":null, "avatar_url":null}),
            )
            .unwrap();
            assert_eq!(profile(user, provider).unwrap().name, "fixture");
        }
        let user = User {
            id: 0,
            login: "author".into(),
            name: None,
            avatar_url: None,
        };
        assert!(profile(user, Provider::GitHub).is_err());
    }
}
