//! Relay configuration.

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Clone, Debug)]
pub struct Config {
    pub listen: String,
    pub db: PathBuf,
    pub harness: String,
    pub max_frame_bytes: usize,
    pub idle_timeout_secs: u64,
    pub lease_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8787".into(),
            db: default_db(),
            harness: "opencode acp".into(),
            max_frame_bytes: 4 * 1024 * 1024,
            idle_timeout_secs: 15 * 60,
            lease_secs: 120,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct FileConfig {
    listen: Option<String>,
    db: Option<PathBuf>,
    harness: Option<String>,
    max_frame_bytes: Option<usize>,
    idle_timeout_secs: Option<u64>,
    lease_secs: Option<u64>,
}

impl Config {
    pub fn load(path: PathBuf) -> Result<Self, String> {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("Could not read {}: {error}", path.display()))?;
        let file: FileConfig =
            toml::from_str(&text).map_err(|error| format!("Could not parse config: {error}"))?;
        let mut config = Self::default();
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
        Ok(config)
    }

    pub fn apply_env(&mut self) {
        if let Ok(value) = std::env::var("KESTREL_RELAY_LISTEN") {
            self.listen = value;
        }
        if let Ok(value) = std::env::var("KESTREL_RELAY_DB") {
            self.db = PathBuf::from(value);
        }
        if let Ok(value) = std::env::var("KESTREL_RELAY_HARNESS") {
            self.harness = value;
        }
    }
}

pub fn default_db() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        if cfg!(target_os = "macos") {
            return home.join("Library/Application Support/Kestrel/relay.db");
        }
        return home.join(".local/share/kestrel/relay.db");
    }
    PathBuf::from("relay.db")
}
