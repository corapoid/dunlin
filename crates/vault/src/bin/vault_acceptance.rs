//! Explicit OS credential-store acceptance; never part of default cargo tests.
use dunlin_vault::store::{SecretStore, ServiceKeychainStore};
use serde::Deserialize;
use std::io::Read;

#[derive(Deserialize)]
struct Request {
    id: String,
    secret: String,
    #[serde(default)]
    expect_unavailable: bool,
}

fn run() -> Result<(), String> {
    let mut input = String::new();
    std::io::stdin()
        .take(65537)
        .read_to_string(&mut input)
        .map_err(|_| "Cannot read fixture")?;
    if input.len() > 65536 {
        return Err("Fixture exceeds size limit".into());
    }
    let request: Request = serde_json::from_str(&input).map_err(|_| "Invalid private fixture")?;
    if !request.id.starts_with("dunlin-remote-e2e-") || request.secret.is_empty() {
        return Err("Invalid fixture identity".into());
    }
    let store = ServiceKeychainStore::relay();
    let write = store.set(&request.id, &request.secret);
    if request.expect_unavailable {
        if write.is_ok() {
            let _ = store.delete(&request.id);
            return Err("Unavailable keyring unexpectedly accepted a secret".into());
        }
        if store.get(&request.id).is_ok() {
            return Err("Unexpected plaintext/read fallback".into());
        }
        println!("{{\"failed_closed\":true}}");
        return Ok(());
    }
    write.map_err(|_| "OS keyring write failed")?;
    let result: Result<(), String> = (|| {
        if store.get(&request.id)?.as_str() != request.secret {
            return Err("Credential roundtrip mismatch".into());
        }
        store.set_policy(&request.id, "{\"fixture\":true}")?;
        if store.get_policy(&request.id)?.as_str() != "{\"fixture\":true}" {
            return Err("Policy roundtrip mismatch".into());
        }
        Ok(())
    })();
    let deleted = store.delete(&request.id);
    let policy_deleted = store.delete_policy(&request.id);
    result?;
    deleted?;
    policy_deleted?;
    if store.get(&request.id).is_ok() || store.get_policy(&request.id).is_ok() {
        return Err("Fixture credentials were not removed".into());
    }
    println!("{{\"roundtrip\":true,\"deleted\":true}}");
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("OS credential-store acceptance failed: {error}");
        std::process::exit(1);
    }
}
