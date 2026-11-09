//! Validate uploaded executable format against the actual SSH target, not the Mac.
use std::path::Path;

pub(crate) fn validate(binary: &Path, remote: &str) -> Result<(), String> {
    let bytes = std::fs::read(binary).map_err(|error| error.to_string())?;
    let lines = remote.lines().collect::<Vec<_>>();
    let (os, arch) = match lines.as_slice() {
        ["Linux", "aarch64" | "arm64"] => ("Linux", 183u16),
        ["Linux", "x86_64"] => ("Linux", 62u16),
        ["Darwin", "arm64"] => ("Darwin", 0u16),
        _ => return Err("Unsupported remote OS/architecture".into()),
    };
    let valid = if os == "Linux" {
        bytes.len() >= 20
            && bytes[..4] == *b"\x7fELF"
            && bytes[4] == 2
            && bytes[5] == 1
            && u16::from_le_bytes([bytes[18], bytes[19]]) == arch
    } else {
        bytes.len() >= 8
            && bytes[..4] == [0xcf, 0xfa, 0xed, 0xfe]
            && u32::from_le_bytes(bytes[4..8].try_into().unwrap()) == 0x0100000c
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "Relay binary does not match {remote:?}; supply the target platform artifact"
        ))
    }
}
