//! SSH passphrase storage. Only passphrases are stored; private keys never are.

#[cfg(target_os = "macos")]
const SERVICE: &str = "dev.dunlin.ssh";

#[cfg(target_os = "macos")]
pub fn set_passphrase(host_id: &str, passphrase: &str) -> Result<(), String> {
    security_framework::passwords::set_generic_password(SERVICE, host_id, passphrase.as_bytes())
        .map_err(|error| format!("Could not save the SSH passphrase in Keychain: {error}"))
}

#[cfg(target_os = "macos")]
pub fn get_passphrase(host_id: &str) -> Result<String, String> {
    let bytes = security_framework::passwords::get_generic_password(SERVICE, host_id)
        .map_err(|error| format!("Could not read the SSH passphrase from Keychain: {error}"))?;
    String::from_utf8(bytes).map_err(|_| "The SSH passphrase in Keychain is not valid UTF-8".into())
}

#[cfg(target_os = "macos")]
pub fn delete_passphrase(host_id: &str) -> Result<(), String> {
    security_framework::passwords::delete_generic_password(SERVICE, host_id)
        .map_err(|error| format!("Could not remove the SSH passphrase from Keychain: {error}"))
}

#[cfg(not(target_os = "macos"))]
pub fn set_passphrase(_host_id: &str, _passphrase: &str) -> Result<(), String> {
    Err("SSH Keychain storage requires macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn get_passphrase(_host_id: &str) -> Result<String, String> {
    Err("SSH Keychain storage requires macOS".into())
}

#[cfg(not(target_os = "macos"))]
pub fn delete_passphrase(_host_id: &str) -> Result<(), String> {
    Err("SSH Keychain storage requires macOS".into())
}
