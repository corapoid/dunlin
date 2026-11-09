//! Relay terminal transport.
//!
//! The UI owns the `Session`; a named worker thread owns a small tokio runtime
//! and the WebSocket. Binary frames are raw PTY bytes. Text frames are the
//! relay control protocol (`ready`, `exit`, `error`, `resize`, `close`).

use std::time::Duration;

use dunlin_terminal::{Control, Exit, Session};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use crate::remote::{RemoteWorkspace, encode, validate_relay_url};

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);
const HEARTBEAT: Duration = Duration::from_secs(30);

/// Start a relay PTY. Connection failures are reported through the normal
/// `Exit` channel so the tab UI has one lifecycle path.
pub(crate) fn connect(
    workspace: &RemoteWorkspace,
    rows: u16,
    cols: u16,
) -> Result<Session, String> {
    let request = request(workspace, rows, cols)?;
    connect_request(request)
}

pub(super) fn connect_setup(
    host: &crate::remote::RemoteHost,
    token: &str,
    agent: &str,
) -> Result<Session, String> {
    let request = build_request(
        host,
        token,
        &format!("setup={}", encode(agent)),
        dunlin_terminal::DEFAULT_ROWS,
        dunlin_terminal::DEFAULT_COLS,
    )?;
    connect_request(request)
}

fn connect_request(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
) -> Result<Session, String> {
    let (output_tx, output_rx) = async_channel::bounded(128);
    let (exit_tx, exit_rx) = async_channel::bounded(1);
    let (input_tx, input_rx) = async_channel::bounded(256);
    let (control_tx, control_rx) = async_channel::bounded(16);
    std::thread::Builder::new()
        .name("dunlin-relay-terminal".into())
        .spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| format!("Could not start the relay terminal runtime: {error}"))
                .and_then(|runtime| {
                    runtime.block_on(run(
                        request,
                        output_tx,
                        exit_tx.clone(),
                        input_rx,
                        control_rx,
                    ))
                });
            if let Err(error) = result {
                let _ = exit_tx.try_send(Exit {
                    success: false,
                    code: None,
                    signal: Some(error),
                });
            }
        })
        .map_err(|error| format!("Could not start the relay terminal transport: {error}"))?;
    Ok(Session::remote(output_rx, exit_rx, input_tx, control_tx))
}

fn request(
    workspace: &RemoteWorkspace,
    rows: u16,
    cols: u16,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    build_request(
        &workspace.host,
        &workspace.token,
        &format!("project={}", encode(&workspace.project.project_id)),
        rows,
        cols,
    )
}

fn build_request(
    host: &crate::remote::RemoteHost,
    token: &str,
    query: &str,
    rows: u16,
    cols: u16,
) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, String> {
    let parsed = validate_relay_url(&host.url)?;
    let base = host.url.trim_end_matches('/');
    let ws = match parsed.scheme() {
        "https" => format!("wss://{}", base.strip_prefix("https://").unwrap_or(base)),
        "http" => format!("ws://{}", base.strip_prefix("http://").unwrap_or(base)),
        _ => return Err("Relay URL must use HTTPS or loopback HTTP".into()),
    };
    let endpoint = format!(
        "{ws}/v1/terminal?{}&rows={}&cols={}",
        query,
        rows.clamp(4, 400),
        cols.clamp(20, 500),
    );
    let mut request = endpoint
        .into_client_request()
        .map_err(|error| format!("Invalid relay terminal endpoint: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {token}"))
            .map_err(|_| "The relay token contains invalid header characters".to_string())?,
    );
    Ok(request)
}

async fn run(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    output: async_channel::Sender<Vec<u8>>,
    exit: async_channel::Sender<Exit>,
    input: async_channel::Receiver<Vec<u8>>,
    control: async_channel::Receiver<Control>,
) -> Result<(), String> {
    #[allow(unused_mut)]
    let mut connector = None;
    #[cfg(debug_assertions)]
    if let Some(pem) = crate::remote_e2e::TLS_CA.get() {
        connector = Some(tokio_tungstenite::Connector::Rustls(
            dunlin_agent::relay_tls_config(pem)?,
        ));
    }
    let (socket, _) = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, connector),
    )
    .await
    .map_err(|_| "The relay terminal handshake timed out".to_string())?
    .map_err(|error| format!("Could not open the relay terminal: {error}"))?;
    let (mut sink, mut stream) = socket.split();
    let mut heartbeat = tokio::time::interval(HEARTBEAT);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut reported = false;
    loop {
        tokio::select! {
            chunk = input.recv() => {
                match chunk {
                    Ok(bytes) => {
                        if sink.send(Message::Binary(bytes.into())).await.is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            command = control.recv() => {
                match command {
                    Ok(Control::Resize { rows, cols }) => {
                        let payload = json!({
                            "type": "resize",
                            "rows": rows.clamp(4, 400),
                            "cols": cols.clamp(20, 500),
                        });
                        if sink
                            .send(Message::Text(payload.to_string().into()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    Ok(Control::Close) | Err(_) => {
                        let _ = sink.send(Message::Close(None)).await;
                        break;
                    }
                }
            }
            frame = stream.next() => {
                match frame {
                    Some(Ok(Message::Binary(bytes))) => {
                        if output.send(bytes.to_vec()).await.is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
                        match value["type"].as_str() {
                            Some("exit") => {
                                if let Ok(status) = serde_json::from_value::<Exit>(value["exit"].clone()) {
                                    let _ = exit.try_send(status);
                                    reported = true;
                                }
                                break;
                            }
                            Some("error") => {
                                let message = value["message"]
                                    .as_str()
                                    .unwrap_or("The relay rejected the terminal");
                                let _ = exit.try_send(Exit {
                                    success: false,
                                    code: None,
                                    signal: Some(message.to_string()),
                                });
                                reported = true;
                                break;
                            }
                            _ => {}
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        return Err(format!("Relay terminal connection failed: {error}"));
                    }
                }
            }
            _ = heartbeat.tick() => {
                if sink.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
        }
    }
    if !reported {
        let _ = exit.try_send(Exit {
            success: false,
            code: None,
            signal: Some("The relay terminal connection closed".into()),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::RemoteHost;
    use dunlin_domain::RemoteProject;
    use std::sync::{Arc, Mutex};

    fn workspace(url: &str) -> RemoteWorkspace {
        RemoteWorkspace {
            host: RemoteHost {
                id: "host".into(),
                name: "Relay".into(),
                url: url.into(),
                user_name: "user".into(),
                user_role: "member".into(),
                device_id: "device".into(),
            },
            project: RemoteProject {
                host_id: "host".into(),
                project_id: "project with space".into(),
                path: "/tmp/project".into(),
            },
            token: "token".into(),
        }
    }

    #[test]
    fn builds_authenticated_loopback_websocket_requests() {
        let request = request(&workspace("http://127.0.0.1:8787"), 30, 90).unwrap();
        assert_eq!(request.uri().scheme_str(), Some("ws"));
        assert_eq!(request.uri().path(), "/v1/terminal");
        assert!(request.uri().query().unwrap().contains("rows=30"));
        assert!(request.uri().query().unwrap().contains("cols=90"));
        assert!(
            request
                .uri()
                .query()
                .unwrap()
                .contains("project%20with%20space")
        );
        assert_eq!(request.headers()["authorization"], "Bearer token");
    }

    #[test]
    fn refuses_plain_http_off_loopback_and_credentials() {
        assert!(request(&workspace("http://relay.example.test"), 30, 90).is_err());
        assert!(request(&workspace("https://user:pass@relay.example.test"), 30, 90).is_err());
    }

    #[test]
    #[allow(clippy::result_large_err)]
    fn relay_client_streams_binary_frames_and_returns_the_exit_status() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let auth = Arc::new(Mutex::new(None));
        let captured = auth.clone();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            runtime.block_on(async move {
                let listener = tokio::net::TcpListener::from_std(listener).unwrap();
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_hdr_async(
                    stream,
                    |request: &tokio_tungstenite::tungstenite::http::Request<()>,
                     response: tokio_tungstenite::tungstenite::handshake::server::Response| {
                        *captured.lock().unwrap() = request
                            .headers()
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .map(str::to_string);
                        Ok(response)
                    },
                )
                .await
                .unwrap();
                socket
                    .send(Message::Text(
                        json!({"type":"ready","pid":42}).to_string().into(),
                    ))
                    .await
                    .unwrap();
                let input = socket.next().await.unwrap().unwrap();
                assert_eq!(input, Message::Binary(b"hello".as_slice().into()));
                socket
                    .send(Message::Binary(b"echo:hello".as_slice().into()))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        json!({
                            "type": "exit",
                            "exit": {"success": true, "code": 0, "signal": null}
                        })
                        .to_string()
                        .into(),
                    ))
                    .await
                    .unwrap();
                let _ = socket.close(None).await;
            });
        });
        let session = connect(&workspace(&format!("http://{addr}")), 24, 80).unwrap();
        session.write(b"hello").unwrap();
        let output = session.output();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut chunk = None;
        while std::time::Instant::now() < deadline {
            match output.try_recv() {
                Ok(bytes) => {
                    chunk = Some(bytes);
                    break;
                }
                Err(async_channel::TryRecvError::Empty) => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(async_channel::TryRecvError::Closed) => break,
            }
        }
        assert_eq!(chunk.as_deref(), Some(b"echo:hello".as_slice()));
        let exit = session.exit().recv_blocking().unwrap();
        assert!(exit.success);
        assert_eq!(exit.code, Some(0));
        assert_eq!(auth.lock().unwrap().as_deref(), Some("Bearer token"));
        server.join().unwrap();
    }
}
