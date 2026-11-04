//! Client used by the `Dunlin --mcp-vault` proxy. It has no keychain access.

#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::path::Path;
#[cfg(unix)]
use std::time::Duration;

use dunlin_domain::vault::SecretSummary;

use crate::http::{HttpInput, HttpOutput};
use crate::protocol::{PROTOCOL_VERSION, WireRequest, WireResponse, read_frame, write_frame};
use crate::recipe::{RecipeInput, RecipeOutput};

pub struct VaultClient {
    #[cfg(unix)]
    stream: UnixStream,
}

impl VaultClient {
    pub fn connect(socket: &Path, token: &str) -> Result<Self, String> {
        #[cfg(unix)]
        {
            let stream = UnixStream::connect(socket).map_err(|error| {
                format!("Could not reach the Dunlin credential service: {error}")
            })?;
            stream
                .set_read_timeout(Some(Duration::from_secs(180)))
                .map_err(|error| {
                    format!("Could not configure the credential connection: {error}")
                })?;
            stream
                .set_write_timeout(Some(Duration::from_secs(30)))
                .map_err(|error| {
                    format!("Could not configure the credential connection: {error}")
                })?;
            let mut client = Self { stream };
            client.request(&WireRequest::Hello {
                protocol: PROTOCOL_VERSION,
                token: token.to_string(),
                pid: std::process::id(),
            })?;
            Ok(client)
        }
        #[cfg(not(unix))]
        {
            let _ = (socket, token);
            Err("The credential broker requires Unix sockets".into())
        }
    }

    pub fn list(&mut self) -> Result<Vec<SecretSummary>, String> {
        match self.request(&WireRequest::List)? {
            WireResponse::List { secrets } => Ok(secrets),
            other => Err(unexpected(other)),
        }
    }

    pub fn http(&mut self, input: HttpInput) -> Result<HttpOutput, String> {
        match self.request(&WireRequest::Http(input))? {
            WireResponse::Http { output } => Ok(output),
            other => Err(unexpected(other)),
        }
    }

    pub fn run(&mut self, input: RecipeInput) -> Result<RecipeOutput, String> {
        match self.request(&WireRequest::Run(input))? {
            WireResponse::Run { output } => Ok(output),
            other => Err(unexpected(other)),
        }
    }

    fn request(&mut self, request: &WireRequest) -> Result<WireResponse, String> {
        #[cfg(unix)]
        {
            write_frame(&mut self.stream, request)?;
            let response = read_frame::<WireResponse>(&mut self.stream)?
                .ok_or_else(|| "The Dunlin credential service closed the connection".to_string())?;
            match response {
                WireResponse::Error { message } => Err(message),
                WireResponse::Hello { ok: false, message } => {
                    Err(message.unwrap_or_else(|| "The credential session was rejected".into()))
                }
                response => Ok(response),
            }
        }
        #[cfg(not(unix))]
        {
            let _ = request;
            Err("The credential broker requires Unix sockets".into())
        }
    }
}

fn unexpected(response: WireResponse) -> String {
    format!("Unexpected credential broker response: {response:?}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{PROTOCOL_VERSION, read_frame, write_frame};

    #[cfg(unix)]
    #[test]
    fn client_handshakes_and_forwards_a_list_request() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("vault.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let hello: WireRequest = read_frame(&mut stream).unwrap().unwrap();
            assert!(
                matches!(hello, WireRequest::Hello { protocol, .. } if protocol == PROTOCOL_VERSION)
            );
            write_frame(
                &mut stream,
                &WireResponse::Hello {
                    ok: true,
                    message: None,
                },
            )
            .unwrap();
            let request: WireRequest = read_frame(&mut stream).unwrap().unwrap();
            assert!(matches!(request, WireRequest::List));
            write_frame(&mut stream, &WireResponse::List { secrets: vec![] }).unwrap();
        });
        let mut client = VaultClient::connect(&socket, "token").unwrap();
        assert!(client.list().unwrap().is_empty());
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn client_surfaces_broker_errors_without_values() {
        let temp = tempfile::tempdir().unwrap();
        let socket = temp.path().join("vault.sock");
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let _: WireRequest = read_frame(&mut stream).unwrap().unwrap();
            write_frame(
                &mut stream,
                &WireResponse::Hello {
                    ok: true,
                    message: None,
                },
            )
            .unwrap();
            let _: WireRequest = read_frame(&mut stream).unwrap().unwrap();
            write_frame(
                &mut stream,
                &WireResponse::Error {
                    message: "That credential is not available in this project".into(),
                },
            )
            .unwrap();
        });
        let mut client = VaultClient::connect(&socket, "token").unwrap();
        let error = client.list().unwrap_err();
        assert!(error.contains("not available"));
        assert!(!error.contains("secret-value"));
        server.join().unwrap();
    }
}
