//! Secret value and policy storage. Values never enter SQLite or the harness
//! process.

use std::collections::HashMap;
use std::sync::Mutex;

use zeroize::Zeroizing;

/// Keychain service name for credential values. The account is the internal id.
pub const KEYCHAIN_SERVICE: &str = "dev.dunlin.secrets";
/// Separate service for the security-critical policy. A harness that edits the
/// SQLite display cache cannot change the policy the broker enforces.
pub const KEYCHAIN_POLICY_SERVICE: &str = "dev.dunlin.secret-policies";

pub trait SecretStore: Send + Sync {
    /// Read a value. The caller must zeroize or drop it promptly.
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String>;
    fn set(&self, id: &str, value: &str) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
    /// Read the JSON policy. Contains no secret value.
    fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String>;
    fn set_policy(&self, id: &str, value: &str) -> Result<(), String>;
    fn delete_policy(&self, id: &str) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
pub struct KeychainStore;

#[cfg(target_os = "macos")]
fn keychain_read(service: &str, id: &str, what: &str) -> Result<Zeroizing<String>, String> {
    use security_framework::passwords::{PasswordOptions, generic_password};

    let bytes =
        generic_password(PasswordOptions::new_generic_password(service, id)).map_err(|error| {
            if error.code() == -25300 {
                format!("The {what} is not present in the macOS Keychain")
            } else {
                format!("Could not read the {what} from the macOS Keychain: {error}")
            }
        })?;
    let value = String::from_utf8(bytes)
        .map_err(|_| format!("The {what} in the macOS Keychain is not valid UTF-8"))?;
    Ok(Zeroizing::new(value))
}

#[cfg(target_os = "macos")]
fn keychain_write(service: &str, id: &str, value: &str, what: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(service, id, value.as_bytes())
        .map_err(|error| format!("Could not save the {what} in the macOS Keychain: {error}"))
}

#[cfg(target_os = "macos")]
fn keychain_delete(service: &str, id: &str, what: &str) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(service, id) {
        Ok(()) => Ok(()),
        Err(error) if error.code() == -25300 => Ok(()),
        Err(error) => Err(format!(
            "Could not remove the {what} from the macOS Keychain: {error}"
        )),
    }
}

/// Keychain store with an explicit service namespace. The desktop app uses
/// [`desktop`](Self::desktop); the relay binary uses [`relay`](Self::relay) so
/// the two identities never share credential items.
#[cfg(target_os = "macos")]
pub struct ServiceKeychainStore {
    value_service: &'static str,
    policy_service: &'static str,
}

#[cfg(target_os = "macos")]
impl ServiceKeychainStore {
    pub const fn desktop() -> Self {
        Self {
            value_service: KEYCHAIN_SERVICE,
            policy_service: KEYCHAIN_POLICY_SERVICE,
        }
    }
    pub const fn relay() -> Self {
        Self {
            value_service: "dev.dunlin.relay-secrets",
            policy_service: "dev.dunlin.relay-secret-policies",
        }
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for ServiceKeychainStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        keychain_read(self.value_service, id, "credential")
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        keychain_write(self.value_service, id, value, "credential")
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        keychain_delete(self.value_service, id, "credential")
    }
    fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
        keychain_read(self.policy_service, id, "credential policy")
    }
    fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
        keychain_write(self.policy_service, id, value, "credential policy")
    }
    fn delete_policy(&self, id: &str) -> Result<(), String> {
        keychain_delete(self.policy_service, id, "credential policy")
    }
}

#[cfg(target_os = "macos")]
impl SecretStore for KeychainStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        ServiceKeychainStore::desktop().get(id)
    }
    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        ServiceKeychainStore::desktop().set(id, value)
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        ServiceKeychainStore::desktop().delete(id)
    }
    fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
        ServiceKeychainStore::desktop().get_policy(id)
    }
    fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
        ServiceKeychainStore::desktop().set_policy(id, value)
    }
    fn delete_policy(&self, id: &str) -> Result<(), String> {
        ServiceKeychainStore::desktop().delete_policy(id)
    }
}

#[cfg(not(target_os = "macos"))]
pub struct KeychainStore;

#[cfg(not(target_os = "macos"))]
pub struct ServiceKeychainStore;

#[cfg(not(target_os = "macos"))]
impl ServiceKeychainStore {
    pub const fn desktop() -> Self {
        Self
    }
    pub const fn relay() -> Self {
        Self
    }
}

#[cfg(not(target_os = "macos"))]
impl SecretStore for ServiceKeychainStore {
    fn get(&self, _id: &str) -> Result<Zeroizing<String>, String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn set(&self, _id: &str, _value: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn delete(&self, _id: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn get_policy(&self, _id: &str) -> Result<Zeroizing<String>, String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn set_policy(&self, _id: &str, _value: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn delete_policy(&self, _id: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
}

#[cfg(not(target_os = "macos"))]
impl SecretStore for KeychainStore {
    fn get(&self, _id: &str) -> Result<Zeroizing<String>, String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn set(&self, _id: &str, _value: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn delete(&self, _id: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn get_policy(&self, _id: &str) -> Result<Zeroizing<String>, String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn set_policy(&self, _id: &str, _value: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
    fn delete_policy(&self, _id: &str) -> Result<(), String> {
        Err("Credential storage requires the macOS Keychain".into())
    }
}

/// In-memory store for tests and for session-only secrets.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, Zeroizing<String>>>,
    policies: Mutex<HashMap<String, Zeroizing<String>>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemoryStore {
    fn get(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.values
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .get(id)
            .map(|value| Zeroizing::new(value.to_string()))
            .ok_or_else(|| "The credential is no longer available".into())
    }

    fn set(&self, id: &str, value: &str) -> Result<(), String> {
        self.values
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .insert(id.to_string(), Zeroizing::new(value.to_string()));
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        self.values
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .remove(id);
        Ok(())
    }

    fn get_policy(&self, id: &str) -> Result<Zeroizing<String>, String> {
        self.policies
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .get(id)
            .map(|value| Zeroizing::new(value.to_string()))
            .ok_or_else(|| "The credential policy is no longer available".into())
    }

    fn set_policy(&self, id: &str, value: &str) -> Result<(), String> {
        self.policies
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .insert(id.to_string(), Zeroizing::new(value.to_string()));
        Ok(())
    }

    fn delete_policy(&self, id: &str) -> Result<(), String> {
        self.policies
            .lock()
            .map_err(|_| "credential store is unavailable".to_string())?
            .remove(id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_round_trips_values_and_policies() {
        let store = MemoryStore::new();
        store.set("dunlin/one", "secret-value").unwrap();
        store
            .set_policy("dunlin/one", r#"{"managed":true}"#)
            .unwrap();
        assert_eq!(&*store.get("dunlin/one").unwrap(), "secret-value");
        assert_eq!(
            &*store.get_policy("dunlin/one").unwrap(),
            r#"{"managed":true}"#
        );
        store.delete("dunlin/one").unwrap();
        store.delete_policy("dunlin/one").unwrap();
        assert!(store.get("dunlin/one").is_err());
        assert!(store.get_policy("dunlin/one").is_err());
    }
}
