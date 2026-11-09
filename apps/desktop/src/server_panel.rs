//! Project-independent relay management. Local and server configuration never mix.
use crate::{
    app::{DunlinApp, SettingsTab},
    remote::RemoteHost,
};
use dunlin_http as ureq;
use dunlin_ui::{
    component::{Disableable, button::ButtonVariants as _, input::InputState},
    *,
};
use dunlin_workspace::{
    project_copy::{self, Snapshot},
    server::{Job, Server},
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub const INSTALL: &str = "bash <(curl -fsSL https://raw.githubusercontent.com/corapoid/dunlin/main/scripts/install-relay.sh)";

pub fn selected_host(this: &DunlinApp) -> Option<String> {
    if this.settings_open
        && this.settings_tab == SettingsTab::Remote
        && let Some(id) = &this.server_panel.host
    {
        return Some(id.clone());
    }
    this.project().remote.as_ref().map(|p| p.host_id.clone())
}

#[derive(Default)]
pub struct State {
    pub host: Option<String>,
    pub info: Option<Server>,
    pub error: Option<String>,
    pub connection_error: Option<String>,
    pub loading: bool,
    pub job: Option<Job>,
    pub snapshot: Option<Arc<Snapshot>>,
    pub snapshot_temp: Option<Arc<tempfile::TempDir>>,
    pub history: bool,
    pub include_ignored: bool,
    pub exclusions: Option<Entity<InputState>>,
    pub transfer: Option<Arc<AtomicBool>>,
    pub progress: Arc<Mutex<String>>,
    pub repo: Option<Entity<InputState>>,
    pub name: Option<Entity<InputState>>,
    pub tailscale: Option<String>,
    pub watching: Option<String>,
}

fn request<T: DeserializeOwned>(
    host: &RemoteHost,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<T, String> {
    let response = raw(host, token, method, path, body)?;
    decode(response)
}

fn decode<T: DeserializeOwned>(response: ureq::Response) -> Result<T, String> {
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read server response")?;
    if bytes.len() > 1024 * 1024 {
        return Err("Server response exceeds the management limit".into());
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| "Invalid server response. Update the relay and retry.".into())
}
fn raw(
    host: &RemoteHost,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<ureq::Response, String> {
    let agent = crate::remote::http_agent();
    raw_with_agent(&agent, host, token, method, path, body)
}

fn raw_with_agent(
    agent: &ureq::Agent,
    host: &RemoteHost,
    token: &str,
    method: &str,
    path: &str,
    body: Option<&impl Serialize>,
) -> Result<ureq::Response, String> {
    crate::remote::validate_relay_url(&host.url)?;
    let req = agent
        .request(
            method,
            &format!("{}{}", host.url.trim_end_matches('/'), path),
        )
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(if path == "/v1/server/clone" {
            660
        } else if method == "GET" {
            15
        } else {
            180
        }));
    let result = if let Some(body) = body {
        req.send_json(serde_json::to_value(body).map_err(|e| e.to_string())?)
    } else {
        req.call()
    };
    result.map_err(|error| match error {
        ureq::Error::Status(404 | 405, _) => "This action requires a newer Dunlin relay.".into(),
        ureq::Error::Status(code, response) => decode::<Value>(*response)
            .ok()
            .and_then(|v| v["error"].as_str().map(str::to_owned))
            .unwrap_or_else(|| format!("Relay error {code}")),
        _ => "Could not reach the server. A submitted action may still have completed remotely; inspect server state before retrying. Nothing ran locally.".into(),
    })
}

impl DunlinApp {
    fn watch_server(&mut self, cx: &mut Context<Self>) {
        if self.server_panel.watching.is_some() {
            return;
        }
        let Some(host) = self.server_panel.host.clone() else {
            return;
        };
        let ticket = uuid::Uuid::now_v7().to_string();
        self.server_panel.watching = Some(ticket.clone());
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(5)).await;
                let keep = this
                    .update(cx, |this, cx| {
                        if this.server_panel.watching.as_deref() != Some(&ticket) {
                            return false;
                        }
                        if !this.settings_open
                            || this.settings_tab != SettingsTab::Remote
                            || this.server_panel.host.as_deref() != Some(&host)
                        {
                            this.server_panel.watching = None;
                            return false;
                        }
                        this.refresh_server(cx);
                        true
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
    }
    pub(crate) fn manage_server(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.server_panel.host.as_deref() != Some(id)
            && (self.server_panel.transfer.is_some()
                || self
                    .server_panel
                    .job
                    .as_ref()
                    .is_some_and(|job| !job.finished))
        {
            self.set_notice("Finish or cancel the current server operation first", cx);
            return;
        }
        self.settings_open = true;
        self.settings_tab = SettingsTab::Remote;
        if self.server_panel.host.as_deref() != Some(id) {
            self.server_panel = State {
                host: Some(id.into()),
                ..State::default()
            };
        }
        if self.server_panel.repo.is_none() {
            self.server_panel.repo = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("owner/repository or group/project")
            }));
            self.server_panel.name =
                Some(cx.new(|cx| InputState::new(window, cx).placeholder("Server name")));
            self.server_panel.exclusions = Some(cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Exclude paths, comma separated: .env, node_modules, target")
            }));
        }
        self.refresh_server(cx);
    }
    fn server_credentials(&mut self) -> Result<(RemoteHost, String), String> {
        let id = self
            .server_panel
            .host
            .clone()
            .ok_or("Select a server first")?;
        let host = self.remote_host(&id).ok_or("Server is no longer paired")?;
        let token = self.remote_token(&id)?;
        Ok((host, token))
    }
    pub(crate) fn refresh_server(&mut self, cx: &mut Context<Self>) {
        if self.server_panel.loading {
            return;
        }
        let (host, token) = match self.server_credentials() {
            Ok(v) => v,
            Err(e) => {
                self.server_panel.error = Some(e);
                return;
            }
        };
        let id = host.id.clone();
        self.server_panel.loading = true;
        self.server_panel.connection_error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    request::<Server>(&host, &token, "GET", "/v1/server", None::<&Value>)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&id) {
                    return;
                }
                this.server_panel.loading = false;
                match result {
                    Ok(info) => this.server_panel.info = Some(info),
                    Err(error) => {
                        this.server_panel.info = None;
                        this.server_panel.connection_error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn server_update(&mut self, value: Value, cx: &mut Context<Self>) {
        let (host, token) = match self.server_credentials() {
            Ok(v) => v,
            Err(e) => {
                self.set_notice(e, cx);
                return;
            }
        };
        let id = host.id.clone();
        cx.spawn(async move |this, cx| {
            let result =
                cx.background_executor()
                    .spawn(async move {
                        raw(&host, &token, "PUT", "/v1/server", Some(&value)).map(|_| ())
                    })
                    .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&id) {
                    return;
                }
                match result {
                    Ok(()) => this.refresh_server(cx),
                    Err(e) => {
                        this.server_panel.error = Some(e);
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }
    fn server_action(&mut self, action: &'static str, target: String, cx: &mut Context<Self>) {
        if self.server_panel.job.as_ref().is_some_and(|j| !j.finished) {
            return;
        }
        let (host, token) = match self.server_credentials() {
            Ok(v) => v,
            Err(e) => {
                self.set_notice(e, cx);
                return;
            }
        };
        let id = host.id.clone();
        self.server_panel.job = Some(Job {
            id: String::new(),
            kind: format!("{action}:{target}"),
            output: "Starting on the server…".into(),
            finished: false,
            success: false,
        });
        self.server_panel.error = None;
        cx.spawn(async move |this, cx| {
            let task_host = host.clone();
            let task_token = token.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    request::<Job>(
                        &task_host,
                        &task_token,
                        "POST",
                        "/v1/server/actions",
                        Some(&json!({"action":action,"target":target})),
                    )
                })
                .await;
            let job = match result {
                Ok(job) => job,
                Err(error) => {
                    let _ = this.update(cx, |this, cx| {
                        if this.server_panel.host.as_deref() == Some(&id) {
                            this.server_panel.job = None;
                            this.server_panel.error = Some(error);
                            cx.notify();
                        }
                    });
                    return;
                }
            };
            let job_id = job.id.clone();
            let mut current = job;
            let mut first = true;
            loop {
                let finished = current.finished;
                let keep = this
                    .update(cx, |this, cx| {
                    if this.server_panel.host.as_deref() != Some(&id) {
                        return false;
                    }
                    if !first && this.server_panel.job.as_ref().map(|job| job.id.as_str()) != Some(job_id.as_str()) { return false; }
                        this.server_panel.job = Some(current);
                        if finished {
                            this.refresh_server(cx);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep || finished {
                    break;
                }
                first = false;
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let host = host.clone();
                let token = token.clone();
                let request_job_id = job_id.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        request::<Job>(
                            &host,
                            &token,
                            "GET",
                            &format!("/v1/server/jobs/{request_job_id}"),
                            None::<&Value>,
                        )
                    })
                    .await;
                match result {
                    Ok(job) => current = job,
                    Err(error) => {
                        let _ = this.update(cx, |this, cx| {
                            if this.server_panel.host.as_deref() == Some(&id) {
                                if let Some(job) = this.server_panel.job.as_mut().filter(|job| job.id == job_id) {
                                    job.finished = true;
                                    job.output.push_str("\nConnection lost. The server operation may still be running. Cancel it or inspect the server before retrying.");
                                }
                                this.server_panel.error = Some(error);
                                cx.notify();
                            }
                        });
                        break;
                    }
                }
            }
        })
        .detach();
        cx.notify();
    }
    fn cancel_server_job(&mut self, cx: &mut Context<Self>) {
        let Some(job) = self.server_panel.job.clone() else {
            return;
        };
        let Ok((host, token)) = self.server_credentials() else {
            return;
        };
        let host_id = host.id.clone();
        let job_id = job.id.clone();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    raw(
                        &host,
                        &token,
                        "DELETE",
                        &format!("/v1/server/jobs/{}", job.id),
                        None::<&Value>,
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&host_id) { return; }
                if let Some(job) = this.server_panel.job.as_mut().filter(|job| job.id == job_id) {
                    job.finished = true;
                    job.success = false;
                    job.output.push_str(if result.is_ok() { "\nCancellation requested. Inspect server state before retrying." } else { "\nCancellation was not confirmed. The operation may still be running on the server." });
                }
                if let Err(error) = result { this.server_panel.error = Some(error); }
                cx.notify();
            });
        })
        .detach();
    }
    fn clone_on_server(&mut self, provider: &'static str, cx: &mut Context<Self>) {
        let Some(input) = &self.server_panel.repo else {
            return;
        };
        let repository = input.read(cx).value().trim().to_string();
        let Ok((host, token)) = self.server_credentials() else {
            return;
        };
        let id = host.id.clone();
        self.server_panel.loading = true;
        self.server_panel.error = None;
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    request::<crate::remote::RemoteProjectInfo>(
                        &host,
                        &token,
                        "POST",
                        "/v1/server/clone",
                        Some(&json!({"provider":provider,"repository":repository})),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&id) {
                    return;
                }
                this.server_panel.loading = false;
                match result {
                    Ok(project) => this.set_notice(
                        format!(
                            "Cloned {} on the server. Choose Open server project to continue.",
                            project.name
                        ),
                        cx,
                    ),
                    Err(error) => this.server_panel.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn choose_server_copy(&mut self, cx: &mut Context<Self>) {
        if self.server_panel.transfer.is_some() {
            return;
        }
        let history = self.server_panel.history;
        let include_ignored = self.server_panel.include_ignored;
        let exclusions: Vec<String> = self
            .server_panel
            .exclusions
            .as_ref()
            .map(|s| {
                s.read(cx)
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose a local project to copy to the server".into()),
        });
        let id = self.server_panel.host.clone();
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(root) = paths.first().cloned() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.server_panel.loading = true;
                this.server_panel.snapshot = None;
                this.server_panel.snapshot_temp = None;
                cx.notify();
            });
            let result = cx
                .background_executor()
                .spawn(async move {
                    let name = root
                        .file_name()
                        .and_then(|s| s.to_str())
                        .ok_or("Invalid project folder name")?
                        .to_owned();
                    let scratch = Arc::new(tempfile::tempdir().map_err(|e| e.to_string())?);
                    let snapshot = project_copy::snapshot(
                        &root,
                        name,
                        history,
                        include_ignored,
                        &exclusions,
                        scratch.path(),
                    )?;
                    Ok::<_, String>((Arc::new(snapshot), scratch))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host != id {
                    return;
                }
                this.server_panel.loading = false;
                match result {
                    Ok((snapshot, scratch)) => {
                        this.server_panel.snapshot = Some(snapshot);
                        this.server_panel.snapshot_temp = Some(scratch);
                    }
                    Err(e) => this.server_panel.error = Some(e),
                }
                cx.notify();
            });
        })
        .detach();
    }
    fn send_server_copy(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.server_panel.snapshot.clone() else {
            return;
        };
        let exclusions: Vec<String> = self
            .server_panel
            .exclusions
            .as_ref()
            .map(|s| {
                s.read(cx)
                    .value()
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if exclusions != snapshot.exclusions {
            self.server_panel.error = Some("Exclusions changed. Choose the project folder again to review an updated preview before sending.".into());
            cx.notify();
            return;
        }
        let Ok((host, token)) = self.server_credentials() else {
            return;
        };
        let id = host.id.clone();
        let cancel = Arc::new(AtomicBool::new(false));
        self.server_panel.transfer = Some(cancel.clone());
        let progress = self.server_panel.progress.clone();
        let scratch = self.server_panel.snapshot_temp.clone();
        let ticker = progress.clone();
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(250))
                    .await;
                if !this
                    .update(cx, |this, cx| {
                        cx.notify();
                        this.server_panel.transfer.is_some()
                    })
                    .unwrap_or(false)
                {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let _private_snapshot = scratch;
                send_copy(&host, &token, &snapshot, &cancel, &progress)
            }).await;
            let _ = this.update(cx, |this, cx| {
                if this.server_panel.host.as_deref() != Some(&id) { return; }
                this.server_panel.transfer = None;
                match result {
                    Ok(project) => { this.server_panel.snapshot = None; this.server_panel.snapshot_temp = None; this.set_notice(format!("Copied {} to the server. Choose Open server project to start working there.", project.name), cx); },
                    Err(error) => this.server_panel.error = Some(error),
                }
                if let Ok(mut value) = ticker.lock() { value.clear(); }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn check_tailscale(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let status = cx
                .background_executor()
                .spawn(async {
                    let mut command = crate::harness_discovery::local_command("tailscale");
                    if Path::new("/Applications/Tailscale.app/Contents/MacOS/Tailscale").is_file() {
                        command = std::process::Command::new(
                            "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
                        );
                    }
                    command.args(["status", "--json"]);
                    // Only report state; peer names, addresses and identities stay out of UI/logs.
                    match command.output() {
                        Ok(out) if out.status.success() => {
                            serde_json::from_slice::<Value>(&out.stdout)
                                .ok()
                                .and_then(|v| v["BackendState"].as_str().map(str::to_owned))
                                .unwrap_or_else(|| "Unknown state".into())
                        }
                        _ => "Not installed, not running or awaiting sign-in".into(),
                    }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.server_panel.tailscale = Some(status);
                cx.notify();
            });
        })
        .detach();
    }
}

fn send_copy(
    host: &RemoteHost,
    token: &str,
    snapshot: &Snapshot,
    cancel: &AtomicBool,
    progress: &Mutex<String>,
) -> Result<crate::remote::RemoteProjectInfo, String> {
    use base64::Engine;
    use std::io::Read;
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .build();
    let begin: Value = request(
        host,
        token,
        "POST",
        "/v1/server/copies",
        Some(&snapshot.manifest),
    )?;
    let id = begin["id"]
        .as_str()
        .filter(|id| id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-'))
        .ok_or("Invalid transfer identity")?;
    let result = (|| {
        for (index, entry) in snapshot.manifest.files.iter().enumerate() {
            if cancel.load(Ordering::Relaxed) {
                return Err("Transfer cancelled; local project unchanged".into());
            }
            if let Ok(mut progress) = progress.lock() {
                *progress = format!(
                    "Copying file {} / {} · {}",
                    index + 1,
                    snapshot.manifest.files.len(),
                    entry.path
                );
            }
            let mut file = if entry.path == project_copy::BUNDLE {
                std::fs::File::open(snapshot.bundle.as_deref().ok_or("Missing Git bundle")?)
                    .map_err(|e| e.to_string())?
            } else {
                project_copy::open_source(&snapshot.root, &entry.path)?
            };
            if !file.metadata().map_err(|e| e.to_string())?.is_file() {
                return Err("Source changed into a special file".into());
            }
            let mut offset = 0u64;
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Transfer cancelled; local project unchanged".into());
                }
                let mut bytes = vec![0u8; project_copy::CHUNK];
                let count = file.read(&mut bytes).map_err(|e| e.to_string())?;
                bytes.truncate(count);
                if count == 0 && offset > 0 {
                    break;
                }
                if offset + count as u64 > entry.size {
                    return Err("Source changed during transfer. Preview again.".into());
                }
                raw_with_agent(
                    &agent,
                    host,
                    token,
                    "POST",
                    &format!("/v1/server/copies/{id}/chunk"),
                    Some(
                        &json!({"path":entry.path,"offset":offset,"contents":base64::engine::general_purpose::STANDARD.encode(bytes)}),
                    ),
                )?;
                offset += count as u64;
                if count == 0 {
                    break;
                }
            }
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Transfer cancelled; local project unchanged".into());
        }
        request(
            host,
            token,
            "POST",
            &format!("/v1/server/copies/{id}/finish"),
            None::<&Value>,
        )
    })();
    if result.is_err() {
        let _ = raw(
            host,
            token,
            "DELETE",
            &format!("/v1/server/copies/{id}"),
            None::<&Value>,
        );
    }
    result
}

pub fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = div().flex().flex_col().gap(px(12.));
    body = body
        .child(
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .child("Server workspace"),
        )
        .child(
            div().text_size(px(11.)).text_color(p.muted_fg).child(
                "Server tools, accounts and projects are remote. App appearance remains local.",
            ),
        )
        .child(
            button("copy-relay-installer")
                .label("Copy server installation command")
                .on_click(|_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(INSTALL.into()))
                }),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    button("tailscale-install")
                        .label("Install Tailscale on this Mac…")
                        .on_click(|_, _, cx| cx.open_url("https://tailscale.com/download/mac")),
                )
                .child(
                    button("tailscale-login")
                        .label("Open Tailscale / sign in…")
                        .on_click(|_, _, cx| cx.open_url("tailscale://")),
                )
                .child(
                    button("tailscale-status")
                        .label("Check Tailscale")
                        .on_click(cx.listener(|this, _, _, cx| this.check_tailscale(cx))),
                ),
        );
    if let Some(status) = &this.server_panel.tailscale {
        body = body.child(format!(
            "Client Tailscale: {status}. Join the server's tailnet before pairing."
        ));
    }
    let Some(id) = this.server_panel.host.clone() else {
        return body
            .child("Choose Manage on a paired server above. Installation and authorization actions run only when explicitly requested.")
            .into_any_element();
    };
    this.watch_server(cx);
    let admin = this
        .server_panel
        .info
        .as_ref()
        .is_some_and(|info| info.role == "admin");
    let busy = this.server_panel.loading
        || this.server_panel.transfer.is_some()
        || this.server_panel.job.as_ref().is_some_and(|j| !j.finished);
    body = body.child(
        div()
            .flex()
            .gap(px(8.))
            .child(
                button("server-refresh")
                    .label(if this.server_panel.loading {
                        "Connecting…"
                    } else {
                        "Refresh connection"
                    })
                    .disabled(this.server_panel.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh_server(cx))),
            )
            .child(
                button("server-open-project")
                    .label("Open server project…")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.open_remote_picker_for(&id, cx)),
                    ),
            ),
    );
    if let Some(info) = this.server_panel.info.clone() {
        if !info.isolated_agents {
            body = body.child(div().text_size(px(11.)).text_color(p.amber).child("Legacy host agent configuration is retained. Running relay setup opts into dedicated homes and may require provider sign-in and new conversations."));
        }
        body = body
            .child(div().font_weight(FontWeight::SEMIBOLD).child(format!(
                "Connected to {} · {} / {} · {}",
                info.name, info.os, info.arch, info.version
            )))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(info.account_scope),
            );
        if !info.sandbox_available {
            body = body.child(div().text_color(p.red).child("Harness sandbox is unavailable on this server. Install platform support before starting agents."));
        }
        if admin && let Some(input) = &this.server_panel.name {
            body = body.child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(
                        dunlin_ui::input(input)
                            .aria_label("Server name")
                            .disabled(busy),
                    )
                    .child(
                        button("server-rename")
                            .label("Rename server")
                            .disabled(busy)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let name = this
                                    .server_panel
                                    .name
                                    .as_ref()
                                    .unwrap()
                                    .read(cx)
                                    .value()
                                    .to_string();
                                this.server_update(json!({"name":name}), cx);
                            })),
                    ),
            );
        }
        body = body.child("Harnesses installed and configured on this server");
        for harness in info.harnesses {
            let selected = info.default_harness == harness.command;
            let id = harness.id.clone();
            let install = id.clone();
            let setup = id.clone();
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(format!(
                        "{} · {}{}",
                        harness.name,
                        if harness.installed {
                            "Installed · provider login may be required"
                        } else {
                            "Not installed"
                        },
                        if selected { " · Default" } else { "" }
                    )))
                    .child(
                        button(format!("server-default:{id}"))
                            .label("Use by default")
                            .disabled(!admin || busy || !harness.installed)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.server_update(json!({"harness":id}), cx)
                            })),
                    )
                    .child(
                        button(format!("server-install:{install}"))
                            .label(if harness.installed {
                                "Update"
                            } else {
                                "Install"
                            })
                            .disabled(!admin || busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.server_action("install", install.clone(), cx)
                            })),
                    )
                    .child(
                        button(format!("server-agent-login:{setup}"))
                            .label("Authorize…")
                            .disabled(!admin || busy || !harness.installed)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                match this.server_credentials() {
                                    Ok((host, token)) => this
                                        .open_server_agent_setup(host, token, &setup, window, cx),
                                    Err(e) => this.set_notice(e, cx),
                                }
                            })),
                    ),
            );
        }
        body = body.child("Git accounts · authorization runs on the server; approve the displayed code in your local browser.");
        for (provider, label) in [("github", "GitHub"), ("gitlab", "GitLab")] {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().child(label))
                    .child(
                        button(format!("server-login:{provider}"))
                            .label("Authorize on server…")
                            .disabled(!admin || busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.server_action("login", provider.into(), cx)
                            })),
                    )
                    .child(
                        button(format!("server-status:{provider}"))
                            .label("Check account")
                            .disabled(!admin || busy)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.server_action("status", provider.into(), cx)
                            })),
                    ),
            );
        }
        if let Some(input) = &this.server_panel.repo {
            body = body
                .child(
                    dunlin_ui::input(input)
                        .aria_label("Repository URL")
                        .disabled(!admin || busy),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .child(
                            button("server-clone-github")
                                .label("Clone from GitHub")
                                .disabled(!admin || busy)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.clone_on_server("github", cx)
                                    }),
                                ),
                        )
                        .child(
                            button("server-clone-gitlab")
                                .label("Clone from GitLab")
                                .disabled(!admin || busy)
                                .on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.clone_on_server("gitlab", cx)
                                    }),
                                ),
                        ),
                );
        }
        body = body.child(
            div()
                .flex()
                .flex_wrap()
                .gap(px(8.))
                .child(
                    button("server-copy-preview")
                        .label("Copy local project…")
                        .disabled(!admin || busy)
                        .on_click(cx.listener(|this, _, _, cx| this.choose_server_copy(cx))),
                )
                .child(
                    button("server-copy-history")
                        .label(if this.server_panel.history {
                            "Git history: included"
                        } else {
                            "Git history: files only"
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.server_panel.history = !this.server_panel.history;
                            this.server_panel.snapshot = None;
                            this.server_panel.snapshot_temp = None;
                            cx.notify();
                        })),
                )
                .child(
                    button("server-copy-ignored")
                        .label(if this.server_panel.include_ignored {
                            "Ignored files: included"
                        } else {
                            "Ignored files: excluded"
                        })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.server_panel.include_ignored = !this.server_panel.include_ignored;
                            this.server_panel.snapshot = None;
                            this.server_panel.snapshot_temp = None;
                            cx.notify();
                        })),
                ),
        );
        if let Some(input) = &this.server_panel.exclusions {
            body = body.child(
                dunlin_ui::input(input)
                    .aria_label("Project copy exclusions")
                    .disabled(busy),
            );
        }
    }
    if let Some(snapshot) = this.server_panel.snapshot.clone() {
        let size: u64 = snapshot.manifest.files.iter().map(|f| f.size).sum();
        body = body.child(format!("Preview: {} · {} files · {:.1} MiB", snapshot.manifest.name, snapshot.manifest.files.len(), size as f64 / 1048576.))
            .child(div().text_size(px(11.)).text_color(p.amber).child("Review before sending: files/history can contain secrets. Exclusions apply to working files, NOT Git history. Staging becomes unstaged; remotes/hooks/credentials are not copied. Local files stay unchanged. Change exclusions and choose the folder again to refresh."));
        let file_list = snapshot
            .manifest
            .files
            .iter()
            .map(|e| e.path.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        body = body.child(
            button("server-copy-file-list")
                .label("Copy full file list for review")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(file_list.clone()))
                }),
        );
        for entry in snapshot.manifest.files.iter().take(60) {
            body = body.child(div().text_size(px(10.5)).child(entry.path.clone()));
        }
        if snapshot.manifest.files.len() > 60 {
            body = body.child(format!(
                "… {} more files",
                snapshot.manifest.files.len() - 60
            ));
        }
        body = body.child(
            button("server-copy-confirm")
                .primary()
                .label("Copy reviewed project to server")
                .disabled(busy)
                .on_click(cx.listener(|this, _, _, cx| this.send_server_copy(cx))),
        );
    }
    if let Some(cancel) = this.server_panel.transfer.clone() {
        let progress = this
            .server_panel
            .progress
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default();
        body = body.child(progress).child(
            button("server-copy-cancel")
                .label("Cancel transfer")
                .on_click(move |_, _, _| cancel.store(true, Ordering::Relaxed)),
        );
    }
    if let Some(job) = this.server_panel.job.clone() {
        body = body.child(format!(
            "{} · {}",
            job.kind,
            if !job.finished {
                "Running on server"
            } else if job.success {
                "Finished"
            } else {
                "Failed — see details"
            }
        ));
        for line in job.output.lines().take(120) {
            body = body.child(div().text_size(px(11.)).child(line.to_owned()));
        }
        let output = job.output.clone();
        body = body.child(
            button("server-job-copy-output")
                .label("Copy verification code / output")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(output.clone()))
                }),
        );
        if job.kind.starts_with("login:") && !job.finished {
            let url = if job.kind.ends_with("github") {
                "https://github.com/login/device"
            } else {
                "https://gitlab.com/oauth/device"
            };
            body = body.child(
                button("server-open-auth")
                    .label("Open verification page")
                    .on_click(move |_, _, cx| cx.open_url(url)),
            );
        }
        if !job.id.is_empty() && (!job.finished || !job.success) {
            body = body.child(
                button("server-job-cancel")
                    .label("Cancel server action")
                    .on_click(cx.listener(|this, _, _, cx| this.cancel_server_job(cx))),
            );
        }
    }
    if let Some(error) = &this.server_panel.error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    if let Some(error) = &this.server_panel.connection_error {
        body = body.child(div().text_color(p.red).child(error.clone()));
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn management_responses_are_bounded_and_plain_http_never_receives_a_token() {
        let response = ureq::Response::new(200, "OK", r#"{"name":"fixture"}"#).unwrap();
        assert_eq!(decode::<Value>(response).unwrap()["name"], "fixture");
        let response = ureq::Response::new(200, "OK", &"x".repeat(1024 * 1024 + 1)).unwrap();
        assert!(decode::<Value>(response).unwrap_err().contains("limit"));
        let host = RemoteHost {
            id: "fixture".into(),
            name: "fixture".into(),
            url: "http://example.test".into(),
            user_name: "fixture".into(),
            user_role: "admin".into(),
            device_id: "fixture".into(),
        };
        assert!(
            raw(
                &host,
                "dummy-never-sent",
                "POST",
                "/v1/server/actions",
                Some(&json!({}))
            )
            .is_err()
        );
    }
}
