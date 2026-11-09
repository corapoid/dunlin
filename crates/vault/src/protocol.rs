//! Local IPC between the harness-spawned MCP proxy and the Dunlin process.

use std::io::{Read, Write};

use dunlin_domain::vault::SecretSummary;
use rand::Rng;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::http::{HttpInput, HttpOutput};
use crate::recipe::{RecipeInput, RecipeOutput};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireRequest {
    Hello {
        protocol: u32,
        token: String,
        pid: u32,
    },
    List,
    Http(HttpInput),
    Run(RecipeInput),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WireResponse {
    Hello { ok: bool, message: Option<String> },
    List { secrets: Vec<SecretSummary> },
    Http { output: HttpOutput },
    Run { output: RecipeOutput },
    Error { message: String },
}

pub fn new_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, value: &T) -> Result<(), String> {
    let payload = serde_json::to_vec(value).map_err(|error| error.to_string())?;
    if payload.len() > MAX_FRAME {
        return Err("The credential broker message is too large".into());
    }
    writer
        .write_all(&(payload.len() as u32).to_be_bytes())
        .map_err(|error| format!("Could not write to the credential broker: {error}"))?;
    writer
        .write_all(&payload)
        .map_err(|error| format!("Could not write to the credential broker: {error}"))?;
    writer
        .flush()
        .map_err(|error| format!("Could not flush the credential broker: {error}"))
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<Option<T>, String> {
    let mut length = [0u8; 4];
    let mut read = 0;
    while read < length.len() {
        match reader.read(&mut length[read..]) {
            Ok(0) if read == 0 => return Ok(None),
            Ok(0) => return Err("The credential broker closed the connection".into()),
            Ok(count) => read += count,
            Err(error) => {
                return Err(format!(
                    "Could not read from the credential broker: {error}"
                ));
            }
        }
    }
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err("The credential broker sent an invalid frame".into());
    }
    let mut payload = vec![0u8; length];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("Could not read from the credential broker: {error}"))?;
    let value = serde_json::from_slice(&payload)
        .map_err(|error| format!("The credential broker sent invalid data: {error}"))?;
    Ok(Some(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn frames_round_trip_and_reject_oversize_messages() {
        let request = WireRequest::Http(HttpInput {
            secret: "github".into(),
            method: "GET".into(),
            url: "https://api.github.com/user".into(),
            headers: Default::default(),
            body: None,
            operation: None,
            params: Default::default(),
        });
        let mut buffer = Vec::new();
        write_frame(&mut buffer, &request).unwrap();
        let decoded: WireRequest = read_frame(&mut Cursor::new(buffer)).unwrap().unwrap();
        matches!(decoded, WireRequest::Http(_));

        let mut buffer = 0u32.to_be_bytes().to_vec();
        assert!(read_frame::<WireRequest>(&mut Cursor::new(buffer.clone())).is_err());
        buffer = ((MAX_FRAME as u32) + 1).to_be_bytes().to_vec();
        assert!(read_frame::<WireRequest>(&mut Cursor::new(buffer)).is_err());
        assert!(
            read_frame::<WireRequest>(&mut Cursor::new(Vec::new()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn tokens_are_long_and_not_derived_from_time() {
        let a = new_token();
        let b = new_token();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
    }
}
