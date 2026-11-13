//! Relay configuration.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct Config {
    pub name: String,
    pub listen: String,
    pub db: PathBuf,
    pub harness: String,
    pub max_frame_bytes: usize,
    pub idle_timeout_secs: u64,
    pub lease_secs: u64,
    pub linger_secs: u64,
    pub sandbox: bool,
    pub store_transcripts: bool,
    /// Optional trusted build directory. Never point this at a project or home.
    pub web_root: Option<PathBuf>,
    pub oidc: Option<crate::oidc::Config>,
    pub mcp_oauth: Vec<crate::mcp_oauth::config::Provider>,
    pub experimental_interactive_runtime: bool,
    pub worker_controller: Option<crate::worker_channel::ControllerConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            name: "Dunlin server".into(),
            listen: "127.0.0.1:8787".into(),
            db: default_db(),
            harness: "opencode acp".into(),
            max_frame_bytes: 4 * 1024 * 1024,
            idle_timeout_secs: 15 * 60,
            lease_secs: 120,
            linger_secs: 15 * 60,
            sandbox: cfg!(any(target_os = "macos", target_os = "linux")),
            store_transcripts: true,
            web_root: None,
            oidc: None,
            mcp_oauth: Vec::new(),
            experimental_interactive_runtime: false,
            worker_controller: None,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    name: Option<String>,
    listen: Option<String>,
    db: Option<PathBuf>,
    harness: Option<String>,
    max_frame_bytes: Option<usize>,
    idle_timeout_secs: Option<u64>,
    lease_secs: Option<u64>,
    linger_secs: Option<u64>,
    sandbox: Option<bool>,
    store_transcripts: Option<bool>,
    web_root: Option<PathBuf>,
    oidc: Option<crate::oidc::Config>,
    mcp_oauth: Vec<crate::mcp_oauth::config::Provider>,
    experimental_interactive_runtime: bool,
    worker_controller: Option<crate::worker_channel::ControllerConfig>,
}

impl Config {
    /// Remote access is through private Tailscale Serve, never a public relay
    /// listener. Do not resolve arbitrary hostnames while validating the bind.
    pub fn listen_address(&self) -> Result<std::net::SocketAddr, String> {
        let address:std::net::SocketAddr=self.listen.parse().map_err(|_|"Use a numeric loopback listen address (127.0.0.1:8787 or [::1]:8787) behind private Tailscale Serve".to_string())?;
        if !address.ip().is_loopback() {
            return Err("Relay listeners must use loopback; expose remote access only through private Tailscale Serve HTTPS".into());
        }
        Ok(address)
    }
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let file: FileConfig =
            toml::from_str(&text).map_err(|error| format!("Could not parse config: {error}"))?;
        let mut config = Self::default();
        if let Some(name) = file.name {
            config.name = name;
        }
        if let Some(value) = file.listen {
            config.listen = value;
        }
        if let Some(value) = file.db {
            config.db = value;
        }
        if let Some(value) = file.harness {
            config.harness = value;
        }
        if let Some(value) = file.max_frame_bytes {
            config.max_frame_bytes = value.clamp(64 * 1024, 32 * 1024 * 1024);
        }
        if let Some(value) = file.idle_timeout_secs {
            config.idle_timeout_secs = value.clamp(30, 24 * 60 * 60);
        }
        if let Some(value) = file.lease_secs {
            config.lease_secs = value.clamp(15, 60 * 60);
        }
        if let Some(value) = file.linger_secs {
            config.linger_secs = value.clamp(30, 24 * 60 * 60);
        }
        if let Some(value) = file.sandbox {
            config.sandbox = value;
        }
        if let Some(value) = file.store_transcripts {
            config.store_transcripts = value;
        }
        config.web_root = file.web_root;
        if let Some(ref oidc) = file.oidc {
            oidc.validate()?;
        }
        config.oidc = file.oidc;
        crate::mcp_oauth::config::validate_registry(&file.mcp_oauth)?;
        config.mcp_oauth = file.mcp_oauth;
        config.experimental_interactive_runtime = file.experimental_interactive_runtime;
        config.worker_controller = file.worker_controller;
        Ok(config)
    }

    pub fn apply_env(&mut self) {
        if let Some(value) = std::env::var_os("DUNLIN_RELAY_WEB_ROOT") {
            self.web_root = Some(PathBuf::from(value));
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_LISTEN") {
            self.listen = value;
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_DB") {
            self.db = PathBuf::from(value);
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_HARNESS") {
            self.harness = value;
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_LINGER_SECS")
            && let Ok(seconds) = value.parse::<u64>()
        {
            self.linger_secs = seconds.clamp(30, 24 * 60 * 60);
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_SANDBOX") {
            self.sandbox = parse_bool(&value);
        }
        if let Ok(value) = std::env::var("DUNLIN_RELAY_STORE_TRANSCRIPTS") {
            self.store_transcripts = parse_bool(&value);
        }
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub fn default_db() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        let (current, legacy) = if cfg!(target_os = "macos") {
            (
                home.join("Library/Application Support/Dunlin/relay.db"),
                home.join("Library/Application Support/Kestrel/relay.db"),
            )
        } else {
            (
                home.join(".local/share/dunlin/relay.db"),
                home.join(".local/share/kestrel/relay.db"),
            )
        };
        if !current.exists() && legacy.exists() {
            let _ = std::fs::rename(&legacy, &current);
        }
        return current;
    }
    PathBuf::from("relay.db")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remote_access_cannot_bypass_private_serve_with_a_public_bind() {
        for (listen, allowed) in [
            ("127.0.0.1:8787", true),
            ("[::1]:8787", true),
            ("0.0.0.0:8787", false),
            ("[::]:8787", false),
            ("100.64.0.1:8787", false),
            ("relay.example.test:8787", false),
        ] {
            let config = Config {
                listen: listen.into(),
                ..Default::default()
            };
            assert_eq!(config.listen_address().is_ok(), allowed, "{listen}");
        }
    }
}
