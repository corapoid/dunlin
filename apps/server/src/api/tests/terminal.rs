use std::time::Duration;

use axum::http::StatusCode;
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;

use super::{Fixture, router};

fn endpoint(
    f: &Fixture,
    addr: std::net::SocketAddr,
    token: Option<&str>,
) -> tokio_tungstenite::tungstenite::http::Request<()> {
    let url = format!(
        "ws://{addr}/v1/terminal?project={}&rows=10&cols=40",
        f.project
    );
    let mut request = url.into_client_request().unwrap();
    if let Some(token) = token {
        request.headers_mut().insert(
            "Authorization",
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
    }
    request
}

fn status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP handshake rejection, got {other}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn relay_terminal_requires_auth_and_a_project_grant_then_streams_pty_bytes() {
    let f = Fixture::new();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(f.state.clone());
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, None))
        .await
        .unwrap_err();
    assert_eq!(status(denied), StatusCode::UNAUTHORIZED);
    let denied = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.ungranted_token)))
        .await
        .unwrap_err();
    assert_eq!(status(denied), StatusCode::FORBIDDEN);

    let (mut socket, _) = tokio_tungstenite::connect_async(endpoint(&f, addr, Some(&f.token)))
        .await
        .unwrap();
    let ready = tokio::time::timeout(Duration::from_secs(10), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let ready = ready.into_text().unwrap();
    let ready: serde_json::Value = serde_json::from_str(&ready).unwrap();
    assert_eq!(ready["type"], "ready");
    assert_eq!(ready["project"], f.project);

    // The relay shell starts with shell integration, so wait for command output
    // rather than requiring the very first frame to contain it.
    socket
        .send(Message::Binary(
            b"printf 'dunlin-relay-terminal\\n'; exit\n"
                .as_slice()
                .into(),
        ))
        .await
        .unwrap();
    let mut output = Vec::new();
    let mut exited = false;
    for _ in 0..80 {
        let Some(frame) = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .ok()
            .flatten()
        else {
            break;
        };
        match frame.unwrap() {
            Message::Binary(bytes) => output.extend_from_slice(&bytes),
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "exit" {
                    exited = true;
                    break;
                }
                if value["type"] == "error" {
                    panic!("relay terminal error: {value}");
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
        if String::from_utf8_lossy(&output).contains("dunlin-relay-terminal") {
            break;
        }
    }
    assert!(
        String::from_utf8_lossy(&output).contains("dunlin-relay-terminal"),
        "missing PTY output: {output:?}"
    );
    // Exit may legitimately race the last output frame; drain until it arrives.
    for _ in 0..200 {
        if exited {
            break;
        }
        let Some(frame) = tokio::time::timeout(Duration::from_secs(10), socket.next())
            .await
            .ok()
            .flatten()
        else {
            break;
        };
        match frame.unwrap() {
            Message::Text(text) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                if value["type"] == "exit" {
                    exited = true;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    assert!(exited, "relay did not report the shell exit");
}
