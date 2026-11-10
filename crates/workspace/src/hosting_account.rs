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

/// Public identity metadata only. gh keeps tokens and account credentials private.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct GitHubAccount {
    pub login: String,
    pub active: bool,
}

fn valid_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 100
        && login.as_bytes()[0].is_ascii_alphanumeric()
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

fn parse_accounts(bytes: &[u8]) -> Result<Vec<GitHubAccount>, String> {
    let accounts: Vec<GitHubAccount> =
        serde_json::from_slice(bytes).map_err(|_| "Invalid GitHub account list".to_string())?;
    if accounts.iter().any(|account| !valid_login(&account.login)) {
        return Err("Invalid GitHub account identity".into());
    }
    Ok(accounts)
}

pub fn github_accounts() -> Result<Vec<GitHubAccount>, String> {
    let mut command = crate::github::cli_command();
    // Filter inside gh: no token, scopes, credential storage paths or raw auth
    // diagnostics ever cross this boundary, including on non-zero status.
    command.args([
        "auth",
        "status",
        "--hostname",
        "github.com",
        "--json",
        "hosts",
        "--jq",
        "[.hosts[\"github.com\"][]? | select(.state == \"success\") | {login, active}]",
    ]);
    let (status, bytes) =
        crate::github::process::run(command, None, std::time::Duration::from_secs(15), 65536)
            .map_err(|_| "GitHub account list unavailable".to_string())?;
    if !status.success() {
        return Err("GitHub account list unavailable".into());
    }
    parse_accounts(&bytes)
}

pub fn switch_github_account(login: &str) -> Result<(), String> {
    if !valid_login(login) {
        return Err("Invalid GitHub account identity".into());
    }
    let mut command = crate::github::cli_command();
    command.args([
        "auth",
        "switch",
        "--hostname",
        "github.com",
        "--user",
        login,
    ]);
    let (status, _) =
        crate::github::process::run(command, None, std::time::Duration::from_secs(15), 65536)
            .map_err(|_| "GitHub account switch unavailable".to_string())?;
    if !status.success() {
        return Err("Could not switch GitHub account. Check gh auth status; environment tokens may override the active account.".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn github_accounts_are_validated_without_credentials() {
        let accounts = parse_accounts(
            br#"[{"login":"first-user","active":true},{"login":"second","active":false}]"#,
        )
        .unwrap();
        assert_eq!(accounts.len(), 2);
        assert!(accounts[0].active && !accounts[1].active);
        for login in ["", "--user", "a/b", "user\nname", "a b"] {
            assert!(!valid_login(login));
        }
        assert!(parse_accounts(br#"[{"login":"user/name","active":false}]"#).is_err());
    }
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
