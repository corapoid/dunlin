//! Desktop Keychain adapter. Service names remain stable across refactors.
const KEYCHAIN_SERVICE: &str = "dev.dunlin.remote";
const LEGACY_KEYCHAIN_SERVICE: &str = "dev.kestrel.remote";

#[cfg(target_os = "macos")]
pub(crate) fn save_token(host_id: &str, token: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(KEYCHAIN_SERVICE, host_id, token.as_bytes())
        .map_err(|error| format!("Could not save the relay token in the keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn save_token(_host_id: &str, _token: &str) -> Result<(), String> {
    Err("Relay token storage requires macOS Keychain".into())
}

pub(crate) fn load_token(host_id: &str) -> Result<Option<String>, String> {
    if let Ok(token) = std::env::var("DUNLIN_REMOTE_TOKEN") {
        return Ok(Some(token));
    }
    if let Some(token) = read_token(KEYCHAIN_SERVICE, host_id)? {
        return Ok(Some(token));
    }
    let Some(token) = read_token(LEGACY_KEYCHAIN_SERVICE, host_id)? else {
        return Ok(None);
    };
    let _ = save_token(host_id, &token);
    delete_token_for(LEGACY_KEYCHAIN_SERVICE, host_id);
    Ok(Some(token))
}

#[cfg(target_os = "macos")]
fn read_token(service: &str, host_id: &str) -> Result<Option<String>, String> {
    use security_framework::passwords::{PasswordOptions, generic_password};
    match generic_password(PasswordOptions::new_generic_password(service, host_id)) {
        Ok(bytes) => String::from_utf8(bytes)
            .map(Some)
            .map_err(|_| "The relay token in the keychain is not valid UTF-8".into()),
        Err(error) if error.code() == -25300 => Ok(None),
        Err(error) => Err(format!(
            "Could not read the relay token from the keychain: {error}"
        )),
    }
}

#[cfg(not(target_os = "macos"))]
fn read_token(_service: &str, _host_id: &str) -> Result<Option<String>, String> {
    Err("Relay token storage requires macOS Keychain".into())
}

pub(crate) fn delete_token(host_id: &str) {
    delete_token_for(KEYCHAIN_SERVICE, host_id);
    delete_token_for(LEGACY_KEYCHAIN_SERVICE, host_id);
}

/// Factory reset must report Keychain failures instead of forgetting their IDs.
pub(crate) fn delete_token_checked(host_id: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    for service in [KEYCHAIN_SERVICE, LEGACY_KEYCHAIN_SERVICE] {
        match security_framework::passwords::delete_generic_password(service, host_id) {
            Ok(()) => {}
            Err(error) if error.code() == -25300 => {}
            Err(_) => return Err("Could not remove a relay pairing from the Keychain. Unlock the Keychain and retry.".into()),
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = host_id;
    Ok(())
}

#[cfg(target_os = "macos")]
fn delete_token_for(service: &str, host_id: &str) {
    let _ = security_framework::passwords::delete_generic_password(service, host_id);
}

#[cfg(not(target_os = "macos"))]
fn delete_token_for(_service: &str, _host_id: &str) {}
