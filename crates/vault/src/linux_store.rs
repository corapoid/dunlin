//! Secret Service via its official CLI. Never fall back to cleartext storage.
use crate::store::SecretStore;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub struct ServiceKeychainStore {
    namespace: &'static str,
}
impl ServiceKeychainStore {
    pub const fn desktop() -> Self {
        Self {
            namespace: "dev.dunlin",
        }
    }
    pub const fn relay() -> Self {
        Self {
            namespace: "dev.dunlin.relay",
        }
    }
    fn operation(
        &self,
        action: &str,
        kind: &str,
        id: &str,
        value: Option<&str>,
    ) -> Result<Zeroizing<String>, String> {
        if id.is_empty()
            || id.len() > 200
            || id.chars().any(char::is_control)
            || value.is_some_and(|s| s.len() > 65536)
        {
            return Err("Invalid credential identifier or oversized value".into());
        }
        let mut command = Command::new("secret-tool");
        command.arg(action);
        if action == "store" {
            command.arg("--label=Dunlin relay credential");
        }
        command
            .args(["service", self.namespace, "kind", kind, "id", id])
            .stdin(if value.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| "Install libsecret's secret-tool and unlock a Secret Service keyring for the relay service user".to_string())?;
        let mut stdout = child.stdout.take().ok_or("Missing keyring output")?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut bytes = Zeroizing::new(Vec::new());
            let result = stdout
                .by_ref()
                .take(65537)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = tx.send(result);
        });
        let input = if let Some(value) = value {
            let mut pipe = child.stdin.take().ok_or("Missing keyring input")?;
            let bytes = Zeroizing::new(value.as_bytes().to_vec());
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let _ = tx.send(pipe.write_all(&bytes));
            });
            Some(rx)
        } else {
            None
        };
        let start = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if start.elapsed() < Duration::from_secs(15) => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("Secret Service keyring unavailable or locked".into());
                }
            }
        };
        if !status.success() {
            return Err("Credential missing or Secret Service keyring unavailable/locked".into());
        }
        if let Some(input) = input {
            input
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| "Keyring input timed out")?
                .map_err(|_| "Could not send credential to keyring")?;
        }
        let bytes = rx
            .recv_timeout(Duration::from_secs(1))
            .map_err(|_| "Keyring output timed out")?
            .map_err(|_| "Could not read keyring")?;
        if bytes.len() > 65536 {
            return Err("Oversized keyring value".into());
        }
        let value = std::str::from_utf8(&bytes).map_err(|_| "Invalid keyring text")?;
        Ok(Zeroizing::new(
            value.strip_suffix('\n').unwrap_or(value).to_owned(),
        ))
    }
}
impl SecretStore for ServiceKeychainStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.operation("lookup", "value", id, None)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.operation("store", "value", id, Some(value))
            .map(|_| ())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.operation("clear", "value", id, None).map(|_| ())
    }
    fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.operation("lookup", "policy", id, None)
    }
    fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
        self.operation("store", "policy", id, Some(value))
            .map(|_| ())
    }
    fn delete_policy(&self, id: &str) -> Result<(), String> {
        self.operation("clear", "policy", id, None).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyring_namespaces_are_separate_and_invalid_ids_never_start_a_process() {
        let desktop = ServiceKeychainStore::desktop();
        let relay = ServiceKeychainStore::relay();
        assert_ne!(desktop.namespace, relay.namespace);
        assert!(relay.get("").is_err());
        assert!(relay.set("bad\nidentifier", "secret").is_err());
        assert!(relay.set("valid", &"x".repeat(65537)).is_err());
    }
}
