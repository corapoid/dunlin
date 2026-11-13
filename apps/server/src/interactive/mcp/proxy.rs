//! Bounded stdio/Unix facade. This process reads no database or credential store.
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, BufRead, Read, Write},
    os::unix::net::UnixStream,
    time::Duration,
};
const MAX_FRAME: usize = 256 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub token: String,
    pub request_id: String,
    pub method: String,
    pub params: Value,
}
pub(crate) fn read_frame(reader: &mut impl Read) -> io::Result<Value> {
    let mut header = [0; 4];
    reader.read_exact(&mut header)?;
    let length = u32::from_be_bytes(header) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|_| io::ErrorKind::InvalidData.into())
}
pub(crate) fn write_frame(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| io::ErrorKind::InvalidData)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    writer.write_all(&(bytes.len() as u32).to_be_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}
fn reply(writer: &mut impl Write, id: Value, result: Option<Value>) -> io::Result<()> {
    let value = if let Some(result) = result {
        json!({"jsonrpc":"2.0","id":id,"result":result})
    } else {
        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32001,"message":"MCP request rejected or outcome unavailable. It was not retried."}})
    };
    let bytes = serde_json::to_vec(&value).map_err(|_| io::ErrorKind::InvalidData)?;
    if bytes.len() > MAX_FRAME {
        return Err(io::ErrorKind::InvalidData.into());
    }
    writer.write_all(&bytes)?;
    writer.write_all(b"\n")?;
    writer.flush()
}
pub(crate) fn run() -> io::Result<()> {
    let socket = std::env::var_os("DUNLIN_MCP_SOCKET").ok_or(io::ErrorKind::PermissionDenied)?;
    let token = zeroize::Zeroizing::new(
        std::env::var("DUNLIN_MCP_TOKEN").map_err(|_| io::ErrorKind::PermissionDenied)?,
    );
    if token.len() > 128 || token.is_empty() {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    serve(&mut reader, &mut writer, &socket, &token)
}

fn serve(
    reader: &mut impl BufRead,
    writer: &mut impl Write,
    socket: &std::ffi::OsStr,
    token: &str,
) -> io::Result<()> {
    loop {
        let mut bytes = Vec::new();
        let size = (&mut *reader)
            .take(MAX_FRAME as u64 + 1)
            .read_until(b'\n', &mut bytes)?;
        if size == 0 {
            return Ok(());
        }
        if size > MAX_FRAME {
            return Err(io::ErrorKind::InvalidData.into());
        }
        let Ok(request) = serde_json::from_slice::<Value>(&bytes) else {
            reply(writer, Value::Null, None)?;
            continue;
        };
        if request["jsonrpc"] != "2.0" {
            reply(writer, Value::Null, None)?;
            continue;
        }
        let Some(id) = request.get("id").cloned() else {
            continue;
        };
        if !(id.as_i64().is_some()
            || id.as_str().is_some_and(|s| {
                !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
            }))
        {
            reply(writer, Value::Null, None)?;
            continue;
        }
        let method = request["method"].as_str().unwrap_or("");
        let mut params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        if let Some(params) = params.as_object_mut() {
            params.remove("_meta");
        }
        match method {
            "initialize" => {
                let version = params["protocolVersion"]
                    .as_str()
                    .filter(|v| ["2025-03-26", "2025-06-18", "2025-11-25"].contains(v))
                    .unwrap_or("2025-06-18");
                reply(
                    writer,
                    id,
                    Some(
                        json!({"protocolVersion":version,"serverInfo":{"name":"Dunlin session tools","version":env!("CARGO_PKG_VERSION")},"capabilities":{"tools":{}},"instructions":"Each call requires human approval in Dunlin. Never retry an uncertain operation automatically."}),
                    ),
                )?;
            }
            "ping" => reply(writer, id, Some(json!({})))?,
            "tools/list" | "tools/call" => {
                // JSON-RPC correlation IDs may be reused after a response.
                // Each new request is a distinct operation; never retry it here.
                let request_id = uuid::Uuid::now_v7().to_string();
                let result = (|| -> io::Result<Value> {
                    let mut stream = UnixStream::connect(socket)?;
                    stream.set_read_timeout(Some(Duration::from_secs(150)))?;
                    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
                    write_frame(
                        &mut stream,
                        &json!({"token":token,"request_id":request_id,"method":method,"params":params}),
                    )?;
                    let mut response = read_frame(&mut stream)?;
                    response
                        .as_object_mut()
                        .and_then(|r| r.remove("result"))
                        .ok_or(io::ErrorKind::PermissionDenied.into())
                })();
                reply(writer, id, result.ok())?;
            }
            _ => reply(writer, id, None)?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::net::UnixListener;

    #[test]
    fn stdio_reused_rpc_ids_create_distinct_operations_and_failed_calls_are_not_retried() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let worker = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for index in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let request: Request =
                    serde_json::from_value(read_frame(&mut stream).unwrap()).unwrap();
                assert_eq!(request.token, "session-proof");
                assert!(request.params.get("_meta").is_none());
                if index < 2 {
                    write_frame(&mut stream, &json!({"result":{"content":[],"index":index}}))
                        .unwrap();
                }
                requests.push(request);
                // The third response is lost after receipt; the facade must not retry.
            }
            listener.set_nonblocking(true).unwrap();
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            requests
        });
        let line = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"lookup","arguments":{},"_meta":{"untrusted":"metadata"}}}).to_string()+"\n";
        let mut input = Cursor::new(line.repeat(3));
        let mut output = Vec::new();
        serve(&mut input, &mut output, path.as_os_str(), "session-proof").unwrap();
        let requests = worker.join().unwrap();
        assert_eq!(requests.len(), 3);
        let ids = requests
            .iter()
            .map(|r| r.request_id.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(ids.len(), 3);
        assert!(requests.iter().all(|r| r.method == "tools/call"));
        let replies = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|s| serde_json::from_str::<Value>(s).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(replies.len(), 3);
        assert!(replies.iter().all(|r| r["id"] == 1));
        assert_eq!(replies[0]["result"]["index"], 0);
        assert_eq!(replies[1]["result"]["index"], 1);
        assert!(replies[2].get("error").is_some());
    }

    #[test]
    fn frame_bounds_and_truncation_are_rejected() {
        for size in [0u32, MAX_FRAME as u32 + 1] {
            assert!(read_frame(&mut Cursor::new(size.to_be_bytes())).is_err());
        }
        assert!(read_frame(&mut Cursor::new([0, 0, 0, 5, b'{'])).is_err());
        assert!(write_frame(&mut Vec::new(), &json!("x".repeat(MAX_FRAME))).is_err());
        let mut output = Vec::new();
        let mut input = Cursor::new(vec![b'x'; MAX_FRAME + 1]);
        assert!(
            serve(
                &mut input,
                &mut output,
                std::ffi::OsStr::new("unused"),
                "proof"
            )
            .is_err()
        );
        assert!(output.is_empty());
    }
}
