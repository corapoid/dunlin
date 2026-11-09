//! Manual-only deployed relay probe. Deliberately absent from NATIVE_SMOKES,
//! release QA, cargo tests and release builds. The private manifest is stdin.
use crate::remote::{RemoteHost, RemoteWorkspace};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::OnceLock,
    time::{Duration, Instant},
};

pub(crate) mod ui;
pub(crate) static TLS_CA: OnceLock<Vec<u8>> = OnceLock::new();

#[derive(Clone, Deserialize)]
pub(crate) struct Manifest {
    pub action: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub project_id: String,
    #[serde(default)]
    pub cwd: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub contents: String,
    #[serde(default)]
    pub tls_ca_pem: Option<String>,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub fixture: PathBuf,
}

pub(crate) fn workspace(spec: &Manifest) -> RemoteWorkspace {
    RemoteWorkspace {
        host: RemoteHost {
            id: "manual-remote-e2e".into(),
            name: "Manual fixture".into(),
            url: spec.url.clone(),
            user_name: "admin".into(),
            user_role: "admin".into(),
            device_id: String::new(),
        },
        project: dunlin_domain::RemoteProject {
            host_id: "manual-remote-e2e".into(),
            project_id: spec.project_id.clone(),
            path: spec.cwd.clone(),
        },
        token: spec.token.clone(),
    }
}

fn execute(spec: Manifest, input: &mut impl BufRead) -> Result<Value, String> {
    match spec.action.as_str() {
        "pair" => {
            let pair = crate::remote::pair(&spec.url, &spec.code, "Dunlin manual remote E2E")?;
            Ok(
                json!({"token": pair.token, "device_id":pair.device_id,"user":pair.user.name,"role":pair.user.role}),
            )
        }
        "projects" => Ok(json!(
            crate::remote::projects(&spec.url, &spec.token)?
                .into_iter()
                .map(|p| json!({"id":p.id,"name":p.name,"path":p.path}))
                .collect::<Vec<_>>()
        )),
        "request" => {
            crate::remote::validate_relay_url(&spec.url)?;
            if !spec.path.starts_with('/')
                || spec.path.starts_with("//")
                || spec.path.contains(['\r', '\n'])
            {
                return Err("Invalid relative relay endpoint".into());
            }
            let mut builder = dunlin_http::AgentBuilder::new()
                .try_proxy_from_env(false)
                .redirects(0);
            if let Some(pem) = &spec.tls_ca_pem {
                builder = builder.root_certificate_pem(pem.as_bytes())?;
            }
            let method = spec.payload["method"].as_str().unwrap_or("GET");
            if !matches!(method, "GET" | "POST" | "PUT" | "DELETE") {
                return Err("Invalid HTTP method".into());
            }
            let agent = builder.build();
            let mut request = agent
                .request(
                    method,
                    &format!("{}{}", spec.url.trim_end_matches('/'), spec.path),
                )
                .timeout(Duration::from_secs(15));
            if !spec.token.is_empty() {
                request = request.set("Authorization", &format!("Bearer {}", spec.token));
            }
            let response = if let Some(body) = spec.payload.get("body") {
                request.send_json(body)
            } else {
                request.call()
            };
            match response {
                Ok(response) => response_value(response),
                Err(dunlin_http::Error::Status(_, response)) => response_value(*response),
                Err(error) => Err(error.to_string()),
            }
        }
        "read" => Ok(json!(workspace(&spec).read_file(&spec.path)?)),
        "write" => Ok(json!(
            workspace(&spec).write_file(
                &spec.path,
                &spec.contents,
                spec.payload
                    .get("expected_mtime_ms")
                    .and_then(Value::as_u64)
            )?
        )),
        "git" => Ok(json!(workspace(&spec).git_status()?)),
        "git-stage" => {
            workspace(&spec).git_stage_file(&spec.path)?;
            Ok(json!({"staged":true}))
        }
        "git-commit" => {
            workspace(&spec).git_commit(&spec.contents)?;
            Ok(json!({"committed":true}))
        }
        "git-diff" => Ok(json!({"patch":workspace(&spec).git_file_diff(&spec.path, false)?})),
        "acp" => {
            let acceptance = dunlin_agent::acceptance::Acceptance {
                url: spec.url,
                token: spec.token,
                project_id: spec.project_id,
                cwd: spec.cwd,
                model: spec.model,
                prompt: spec.payload["prompt"]
                    .as_str()
                    .ok_or("Missing prompt")?
                    .into(),
                expected: spec.payload["expected"]
                    .as_str()
                    .ok_or("Missing expected text")?
                    .into(),
                timeout_secs: spec.payload["timeout_secs"].as_u64().unwrap_or(180),
                resume_session_id: spec.payload["resume_session_id"]
                    .as_str()
                    .map(str::to_owned),
                cancel_after_ms: spec.payload["cancel_after_ms"].as_u64(),
                allow_permissions: spec.payload["allow_permissions"].as_bool().unwrap_or(false),
                expect_denied: spec.payload["expect_denied"].as_bool().unwrap_or(false),
                tls_ca_pem: spec.tls_ca_pem,
            };
            serde_json::to_value(dunlin_agent::acceptance::run(acceptance)?)
                .map_err(|e| e.to_string())
        }
        "tunnel" | "bootstrap" | "ssh-check" => {
            let host: dunlin_domain::SshHost =
                serde_json::from_value(spec.payload["host"].clone()).map_err(|e| e.to_string())?;
            let options = dunlin_ssh::service::ConnectionOptions {
                config_file: Some(PathBuf::from(
                    spec.payload["ssh_config"]
                        .as_str()
                        .ok_or("Missing SSH config")?,
                )),
                control_socket: Some(PathBuf::from(
                    spec.payload["control_socket"]
                        .as_str()
                        .ok_or("Missing SSH socket")?,
                )),
            };
            if spec.action == "ssh-check" {
                dunlin_ssh::service::ensure_master_with_options(&host, None, &options)?;
                dunlin_ssh::service::exit_master_with_options(&host, &options);
                return Ok(json!({"connected":true}));
            }
            let (mut tunnel, code) = if spec.action == "bootstrap" {
                let binary = PathBuf::from(
                    spec.payload["relay_binary"]
                        .as_str()
                        .ok_or("Missing relay binary")?,
                );
                let result =
                    dunlin_ssh::service::bootstrap_with_options(&host, &binary, None, &options)?;
                (result.tunnel, Some(result.pairing_code))
            } else {
                (
                    dunlin_ssh::service::start_tunnel_with_options(&host, None, &options)?,
                    None,
                )
            };
            println!(
                "{}",
                json!({"url":format!("http://127.0.0.1:{}",tunnel.local_port),"code":code})
            );
            std::io::stdout().flush().map_err(|e| e.to_string())?;
            let mut line = String::new();
            let _ = input.read_line(&mut line);
            tunnel.stop();
            dunlin_ssh::service::exit_master_with_options(&host, &options);
            Ok(json!({"closed":true}))
        }
        "terminal" => {
            let session = crate::terminal::relay::connect(&workspace(&spec), 24, 80)?;
            session.resize(33, 111)?;
            // Don't accept the shell's echo of our command as evidence of execution.
            session.write(b"printf '\\nDUNLIN_%s\\n' PTY_OK; stty size\n")?;
            let started = Instant::now();
            let output = session.output();
            let exit = session.exit();
            let mut bytes = Vec::new();
            loop {
                while let Ok(chunk) = output.try_recv() {
                    bytes.extend(chunk);
                }
                let text = String::from_utf8_lossy(&bytes);
                if text.contains("DUNLIN_PTY_OK") && text.contains("33 111") {
                    break;
                }
                if let Ok(status) = exit.try_recv() {
                    return Err(format!("PTY exited before assertions: {status:?}"));
                }
                if started.elapsed() > Duration::from_secs(20) || bytes.len() > 1024 * 1024 {
                    return Err("PTY deadline/size limit exceeded".into());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            session.kill();
            Ok(json!({"executed":true,"resized":true}))
        }
        "keychain" => {
            if spec.payload["consent"] != true {
                return Err("Keychain acceptance requires explicit consent".into());
            }
            let id = spec.payload["host_id"]
                .as_str()
                .ok_or("Missing private keychain fixture id")?;
            if !id.starts_with("dunlin-remote-e2e-") {
                return Err("Invalid fixture keychain id".into());
            }
            crate::remote::save_token(id, &spec.token)?;
            let result = crate::remote::load_token(id).and_then(|value| {
                if value.as_deref() == Some(&spec.token) {
                    Ok(())
                } else {
                    Err("Token roundtrip mismatch".into())
                }
            });
            crate::remote::delete_token_checked(id)?;
            result?;
            if crate::remote::load_token(id)?.is_some() {
                return Err("Fixture token not deleted".into());
            }
            Ok(json!({"roundtrip":true,"deleted":true}))
        }
        "ui" => ui::run(spec),
        _ => Err("Unknown manual acceptance action".into()),
    }
}

fn response_value(response: dunlin_http::Response) -> Result<Value, String> {
    use std::io::Read;
    let status = response.status();
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err("Relay response exceeds acceptance limit".into());
    }
    let body = serde_json::from_slice::<Value>(&bytes)
        .unwrap_or_else(|_| json!(String::from_utf8_lossy(&bytes)));
    Ok(json!({"status":status,"body":body}))
}

pub(crate) fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut line = String::new();
    let result = (|| {
        input.read_line(&mut line).map_err(|e| e.to_string())?;
        if line.len() > 2 * 1024 * 1024 {
            return Err("Manifest exceeds size limit".into());
        }
        let spec: Manifest =
            serde_json::from_str(&line).map_err(|_| "Invalid private manifest".to_string())?;
        if let Some(pem) = &spec.tls_ca_pem {
            dunlin_http::AgentBuilder::new().root_certificate_pem(pem.as_bytes())?;
            TLS_CA
                .set(pem.as_bytes().to_vec())
                .map_err(|_| "CA already configured")?;
        }
        execute(spec, &mut input)
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            // Do not print request manifests, tokens, pairing codes or protocol payloads.
            eprintln!("Remote acceptance failed: {error}");
            1
        }
    }
}
