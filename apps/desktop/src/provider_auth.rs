//! Native provider sign-in. Secrets travel only in authenticated HTTP bodies to
//! the registered local OpenCode service, never through argv, files or logs.
use std::path::PathBuf;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dunlin_ui::component::button::{Button, ButtonVariants};
use dunlin_ui::component::input::{Input, InputState};
use dunlin_ui::component::{Disableable, Sizable};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, InteractiveElement, IntoElement,
    ParentElement, StatefulInteractiveElement, Styled, Window, div, palette, px,
};
use serde::Deserialize;
use serde_json::{Value, json};
use url::Url;

use crate::app::DunlinApp;
use crate::providers::{api, command_output, encode};

/// This is the public V2 service-registration contract, also used by
/// @opencode/client/service. It must never implement Debug or Serialize.
#[derive(Deserialize)]
struct Registration {
    url: String,
    pid: u64,
    version: Option<String>,
    password: Option<String>,
}

#[derive(Clone)]
struct LocalService {
    url: String,
    authorization: Option<String>,
    agent: ureq::Agent,
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(Duration::from_secs(20))
        .build()
}

fn local_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "Invalid local OpenCode service address")?;
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(host)) => host == "localhost",
        _ => false,
    };
    if !loopback
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Provider sign-in requires a registered local OpenCode service".into());
    }
    Ok(url)
}

impl LocalService {
    fn discover(directory: &str) -> Result<Self, String> {
        // The CLI ensures/discovers the same service as ACP. Verify the
        // registration against that service's identity before using its password.
        let identity = api(directory, "info")?;
        let output = command_output(
            crate::harness_discovery::local_command("opencode")
                .args(["debug", "paths", "state"])
                .current_dir(directory),
            Duration::from_secs(15),
        )
        .map_err(|_| "Could not discover OpenCode's local service".to_string())?;
        if !output.status.success() {
            return Err("Could not discover OpenCode's local service".into());
        }
        let directory =
            String::from_utf8(output.stdout).map_err(|_| "Invalid OpenCode state directory")?;
        let path = PathBuf::from(directory.trim());
        if !path.is_absolute() {
            return Err("Invalid OpenCode state directory".into());
        }
        let bytes = std::fs::read(path.join("service.json"))
            .map_err(|_| "Could not read OpenCode's service registration")?;
        let registration: Registration = serde_json::from_slice(&bytes)
            .map_err(|_| "Unsupported OpenCode service registration")?;
        if identity.get("pid").and_then(Value::as_u64) != Some(registration.pid)
            || registration.version.as_deref().is_some_and(|version| {
                identity.get("version").and_then(Value::as_str) != Some(version)
            })
        {
            return Err("OpenCode's service changed. Close this dialog and try again.".into());
        }
        let url = local_url(&registration.url)?.to_string();
        let service = Self {
            url,
            authorization: registration.password.map(|password| {
                format!("Basic {}", STANDARD.encode(format!("opencode:{password}")))
            }),
            // Never forward a provider key through an environment proxy or a redirect.
            agent: http_agent(),
        };
        let health = service.request("GET", "/api/info", None)?;
        if health.get("pid").and_then(Value::as_u64) != Some(registration.pid) {
            return Err("OpenCode service identity could not be verified".into());
        }
        Ok(service)
    }

    fn request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let mut request = self.agent.request(
            method,
            &format!("{}{}", self.url.trim_end_matches('/'), path),
        );
        if let Some(authorization) = &self.authorization {
            request = request.set("Authorization", authorization);
        }
        let response = match body {
            Some(body) => request.send_json(body),
            None => request.call(),
        };
        // The response can echo invalid input, including secrets. Do not log,
        // debug-format or render the original error/body under any circumstances.
        let response = response.map_err(|error| match error {
            ureq::Error::Status(401, _) => "OpenCode rejected the connection. Reopen sign-in and check your account.".into(),
            ureq::Error::Status(400, _) => "Sign-in was not accepted. Check the key or authorization code and try again.".into(),
            ureq::Error::Status(status, _) => format!("OpenCode could not complete sign-in (HTTP {status}). Check the connection and try again."),
            _ => "Could not reach OpenCode. Check its connection and try again.".into(),
        })?;
        if response.status() == 204 {
            return Ok(Value::Null);
        }
        if (300..400).contains(&response.status()) {
            return Err("OpenCode redirected sign-in; the request was blocked for safety".into());
        }
        response
            .into_json()
            .map_err(|_| "OpenCode returned an unsupported sign-in response".into())
    }
}

#[derive(Clone)]
enum MethodKind {
    Key,
    OAuth(String),
}

#[derive(Clone)]
struct AuthMethod {
    label: String,
    kind: MethodKind,
    answer: Value,
}

fn methods(integration: &Value) -> Vec<AuthMethod> {
    integration
        .get("methods")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|method| {
            let kind = match method.get("type")?.as_str()? {
                "key" => MethodKind::Key,
                "oauth" => MethodKind::OAuth(method.get("id")?.as_str()?.into()),
                _ => return None,
            };
            let mut answer = serde_json::Map::new();
            for field in method
                .get("form")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if let Some(value) = field.get("default") {
                    answer.insert(field.get("key")?.as_str()?.into(), value.clone());
                } else if field.get("required").and_then(Value::as_bool) == Some(true) {
                    // Do not submit incomplete provider-specific forms or invent values.
                    return None;
                }
            }
            let fallback = match kind {
                MethodKind::Key => "API key",
                MethodKind::OAuth(_) => "Sign in with browser",
            };
            Some(AuthMethod {
                label: method
                    .get("label")
                    .and_then(Value::as_str)
                    .unwrap_or(fallback)
                    .into(),
                kind,
                answer: Value::Object(answer),
            })
        })
        .collect()
}

/// Resolve from the public provider record, including when the composer's
/// startup catalog is absent. Never guess that providerID == integrationID.
fn resolved_integration(provider_id: &str, response: &Value) -> Result<String, String> {
    let provider_id = crate::providers::account_provider(provider_id);
    let provider = response.get("data").unwrap_or(response);
    if provider.get("id").and_then(Value::as_str) != Some(provider_id)
        || provider.get("activation").and_then(Value::as_str) == Some("disabled")
    {
        return Err(
            "OpenCode has not advertised this provider. Refresh providers and try again.".into(),
        );
    }
    Ok(provider
        .get("integrationID")
        .and_then(Value::as_str)
        .unwrap_or(provider_id)
        .into())
}

fn preferred_method(methods: &[AuthMethod]) -> usize {
    methods
        .iter()
        .position(|method| matches!(method.kind, MethodKind::OAuth(_)))
        .unwrap_or(0)
}

#[derive(Clone, Deserialize)]
struct OAuthAttempt {
    #[serde(rename = "attemptID")]
    id: String,
    url: String,
    instructions: String,
    mode: String,
}

fn browser_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|url| {
        url.username().is_empty()
            && url.password().is_none()
            && (url.scheme() == "https" || local_url(value).is_ok())
    })
}

enum Phase {
    Loading,
    Ready,
    Waiting(OAuthAttempt),
    Connected,
}

pub(crate) struct AuthDialog {
    id: uuid::Uuid,
    name: String,
    provider_id: String,
    integration_id: String,
    directory: String,
    service: Option<LocalService>,
    methods: Vec<AuthMethod>,
    selected: usize,
    phase: Phase,
    busy: bool,
    error: Option<String>,
    // Session-only entity, never included in StoredSettings or transcripts.
    input: Entity<InputState>,
}

fn secret_input(
    placeholder: &'static str,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> Entity<InputState> {
    cx.new(|cx| {
        InputState::new(window, cx)
            .placeholder(placeholder)
            .masked(true)
    })
}

impl DunlinApp {
    pub(crate) fn connect_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.local_opencode()
            || self.projects.is_empty()
            || self.chat_controls_busy()
            || self
                .provider_auth
                .as_ref()
                .is_some_and(|dialog| dialog.busy)
        {
            return;
        }
        let Some(provider_id) = self.selected_provider().map(str::to_owned) else {
            return;
        };
        let provider = self
            .provider_catalog
            .providers
            .iter()
            .find(|provider| provider.id == provider_id);
        let name = provider
            .map(|provider| provider.name.clone())
            .unwrap_or_else(|| crate::providers::provider_name(&provider_id).into());
        let directory = self.project().path.clone();
        let id = uuid::Uuid::now_v7();
        self.chat_menu_open = false;
        self.provider_auth = Some(AuthDialog {
            id,
            name,
            provider_id: provider_id.clone(),
            integration_id: String::new(),
            directory: directory.clone(),
            service: None,
            methods: Vec::new(),
            selected: 0,
            phase: Phase::Loading,
            busy: false,
            error: None,
            input: secret_input("Paste your API key", window, cx),
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx.background_executor().spawn(async move {
                let service = LocalService::discover(&directory)?;
                let account_provider = crate::providers::account_provider(&provider_id);
                let provider = service.request("GET", &format!("/api/provider/{}?location%5Bdirectory%5D={}", encode(account_provider), encode(&directory)), None)?;
                let integration_id = resolved_integration(&provider_id, &provider)?;
                let response = service.request("GET", &auth_path(&directory, &integration_id, ""), None)?;
                let choices = methods(response.get("data").unwrap_or(&response));
                Ok::<_, String>((service, integration_id, choices))
            }).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(dialog) = this.provider_auth.as_mut().filter(|dialog| dialog.id == id) else { return; };
                dialog.phase = Phase::Ready;
                match result {
                    Ok((service, integration_id, choices)) => {
                        dialog.service = Some(service);
                        dialog.integration_id = integration_id;
                        dialog.methods = choices;
                        // Prefer account OAuth when the integration actually offers it.
                        dialog.selected = preferred_method(&dialog.methods);
                        if dialog.methods.is_empty() { dialog.error = Some("This integration needs additional provider-specific setup; no supported sign-in method was advertised.".into()); }
                        else if matches!(dialog.methods[dialog.selected].kind, MethodKind::Key) {
                            dialog.input.update(cx, |input, cx| input.focus(window, cx));
                        }
                    }
                    Err(error) => dialog.error = Some(error),
                }
                cx.notify();
            });
        }).detach();
    }

    pub(crate) fn close_provider_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .provider_auth
            .as_ref()
            .is_some_and(|dialog| dialog.busy)
        {
            return;
        }
        if let Some(dialog) = self.provider_auth.take()
            && let (Some(service), Phase::Waiting(attempt)) = (dialog.service, dialog.phase)
        {
            let path = auth_path(
                &dialog.directory,
                &dialog.integration_id,
                &format!("/connect/oauth/{}", encode(&attempt.id)),
            );
            cx.background_executor()
                .spawn(async move {
                    let _ = service.request("DELETE", &path, None);
                })
                .detach();
        }
        self.composer
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }

    fn choose_auth_method(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let input = secret_input("Paste your API key", window, cx);
        if let Some(dialog) = self.provider_auth.as_mut()
            && !dialog.busy
            && matches!(dialog.phase, Phase::Ready)
            && index < dialog.methods.len()
        {
            dialog.selected = index;
            dialog.input = input;
            dialog.error = None;
            dialog.input.update(cx, |input, cx| input.focus(window, cx));
        }
        cx.notify();
    }

    pub(crate) fn submit_provider_auth(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.provider_auth.as_ref() else {
            return;
        };
        if dialog.busy || matches!(dialog.phase, Phase::Loading | Phase::Connected) {
            return;
        }
        let Some(service) = dialog.service.clone() else {
            return;
        };
        let id = dialog.id;
        let is_key;
        let (suffix, payload) = match &dialog.phase {
            Phase::Ready => {
                let Some(method) = dialog.methods.get(dialog.selected) else {
                    return;
                };
                match &method.kind {
                    MethodKind::Key => {
                        let key = dialog.input.read(cx).text().to_string().trim().to_string();
                        if key.is_empty() {
                            self.provider_auth.as_mut().unwrap().error =
                                Some("Enter an API key to connect your account.".into());
                            cx.notify();
                            return;
                        }
                        is_key = true;
                        (
                            "/connect/key".into(),
                            json!({"key":key,"answer":method.answer}),
                        )
                    }
                    MethodKind::OAuth(method_id) => {
                        is_key = false;
                        (
                            "/connect/oauth".into(),
                            json!({"methodID":method_id,"answer":method.answer}),
                        )
                    }
                }
            }
            Phase::Waiting(attempt) if attempt.mode == "code" => {
                let code = dialog.input.read(cx).text().to_string().trim().to_string();
                if code.is_empty() {
                    return;
                }
                is_key = true; // Both operations finish with an empty success response.
                (
                    format!("/connect/oauth/{}/complete", encode(&attempt.id)),
                    json!({"code":code}),
                )
            }
            _ => return,
        };
        let path = auth_path(&dialog.directory, &dialog.integration_id, &suffix);
        let placeholder = if matches!(dialog.phase, Phase::Waiting(_)) {
            "Paste the authorization code"
        } else {
            "Paste your API key"
        };
        let input = secret_input(placeholder, window, cx);
        let dialog = self.provider_auth.as_mut().unwrap();
        dialog.busy = true;
        dialog.error = None;
        // Drop the old entity (including its undo history) instead of clearing
        // the field and retaining a recoverable copy of the API key.
        dialog.input = input;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let response = service.request("POST", &path, Some(payload))?;
                    if is_key {
                        return Ok(None);
                    }
                    let attempt: OAuthAttempt =
                        serde_json::from_value(response.get("data").cloned().unwrap_or(response))
                            .map_err(|_| {
                            "OpenCode returned an unsupported browser sign-in response".to_string()
                        })?;
                    if !browser_url(&attempt.url)
                        || !matches!(attempt.mode.as_str(), "auto" | "code")
                    {
                        return Err(
                            "OpenCode returned an unsafe or unsupported sign-in link".into()
                        );
                    }
                    Ok::<_, String>(Some(attempt))
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(dialog) = this.provider_auth.as_mut().filter(|dialog| dialog.id == id)
                else {
                    return;
                };
                dialog.busy = false;
                match result {
                    Ok(Some(attempt)) => {
                        cx.open_url(&attempt.url);
                        if attempt.mode == "code" {
                            dialog.input = secret_input("Paste the authorization code", window, cx);
                            dialog.input.update(cx, |input, cx| input.focus(window, cx));
                        }
                        dialog.phase = Phase::Waiting(attempt);
                        this.poll_provider_oauth(id, window, cx);
                    }
                    Ok(None) => this.provider_auth_connected(id, cx),
                    Err(error) => {
                        dialog.error = Some(error);
                        dialog.input.update(cx, |input, cx| input.focus(window, cx));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn provider_auth_connected(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        let Some(dialog) = self.provider_auth.as_mut().filter(|dialog| dialog.id == id) else {
            return;
        };
        dialog.phase = Phase::Connected;
        dialog.busy = false;
        dialog.error = None;
        self.provider_catalog.confirm_connection(
            &dialog.directory,
            crate::providers::account_provider(&dialog.provider_id),
            if dialog.provider_id == crate::providers::FREE {
                "OpenCode Console"
            } else {
                &dialog.name
            },
            &dialog.integration_id,
        );
        self.invalidate_usage();
        if self.local_opencode()
            && self.provider_auth.as_ref().is_some_and(|dialog| {
                dialog.directory == self.project().path
                    && Some(dialog.provider_id.as_str()) == self.selected_provider()
            })
        {
            self.reload_provider_models(cx);
            self.refresh_usage(cx);
        }
        cx.notify();
    }

    fn poll_provider_oauth(&mut self, id: uuid::Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.provider_auth.as_ref().filter(|dialog| dialog.id == id) else {
            return;
        };
        let (Some(service), Phase::Waiting(attempt)) = (&dialog.service, &dialog.phase) else {
            return;
        };
        let service = service.clone();
        let path = auth_path(
            &dialog.directory,
            &dialog.integration_id,
            &format!("/connect/oauth/{}", encode(&attempt.id)),
        );
        cx.spawn_in(window, async move |this, cx| {
            for _ in 0..300 {
                cx.background_executor().timer(Duration::from_secs(2)).await;
                let active = this.update_in(cx, |this, _, _| this.provider_auth.as_ref().is_some_and(|dialog| dialog.id == id && matches!(dialog.phase, Phase::Waiting(_))));
                if !matches!(active, Ok(true)) { return; }
                let service = service.clone(); let path = path.clone();
                let result = cx.background_executor().spawn(async move { service.request("GET", &path, None) }).await;
                let keep_polling = this.update_in(cx, |this, _, cx| {
                    let Some(dialog) = this.provider_auth.as_mut().filter(|dialog| dialog.id == id && matches!(dialog.phase, Phase::Waiting(_))) else { return false; };
                    let response = match result {
                        Ok(response) => response,
                        Err(error) => { dialog.error = Some(error); cx.notify(); return false; }
                    };
                    let status = response.get("data").unwrap_or(&response).get("status").and_then(Value::as_str);
                    match status {
                        Some("pending") => true,
                        Some("complete") => { this.provider_auth_connected(id, cx); false }
                        _ => {
                            dialog.busy = false;
                            dialog.error = Some("Browser sign-in expired or was declined. Close this dialog and try again.".into());
                            cx.notify(); false
                        }
                    }
                });
                if !matches!(keep_polling, Ok(true)) { return; }
            }
            let _ = this.update_in(cx, |this, _, cx| {
                if let Some(dialog) = this.provider_auth.as_mut().filter(|dialog| dialog.id == id) {
                    dialog.error = Some("Sign-in timed out. Close this dialog and try again.".into()); cx.notify();
                }
            });
        }).detach();
    }
}

fn auth_path(directory: &str, integration: &str, suffix: &str) -> String {
    format!(
        "/api/integration/{}{}?location%5Bdirectory%5D={}",
        encode(integration),
        suffix,
        encode(directory)
    )
}

/// Verified loopback transport for session settings; metadata never enters argv.
pub(crate) fn local_request(
    directory: &str,
    method: &str,
    resource: &str,
    body: Option<Value>,
) -> Result<Value, String> {
    LocalService::discover(directory)?.request(
        method,
        &format!(
            "/api/{resource}?location%5Bdirectory%5D={}",
            encode(directory)
        ),
        body,
    )
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let dialog = this.provider_auth.as_ref().expect("open auth dialog");
    let waiting = match &dialog.phase {
        Phase::Waiting(attempt) => Some(attempt.clone()),
        _ => None,
    };
    let ready = matches!(dialog.phase, Phase::Ready);
    let connected = matches!(dialog.phase, Phase::Connected);
    let key_method = dialog
        .methods
        .get(dialog.selected)
        .is_some_and(|method| matches!(method.kind, MethodKind::Key));
    let code = waiting
        .as_ref()
        .is_some_and(|attempt| attempt.mode == "code");
    let mut content = div().flex().flex_col().gap(px(14.));
    if matches!(dialog.phase, Phase::Loading) {
        content = content.child(
            div()
                .text_color(p.muted_fg)
                .child("Loading secure sign-in methods…"),
        );
    }
    if ready {
        if dialog.methods.len() > 1 {
            for (index, method) in dialog.methods.iter().enumerate() {
                content = content.child(
                    Button::new(("auth-method", index))
                        .label(method.label.clone())
                        .small()
                        .disabled(dialog.busy)
                        .when(index == dialog.selected, |button| button.primary())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_auth_method(index, window, cx)
                        })),
                );
            }
        }
        if key_method {
            content = content
                .child(div().text_color(p.muted_fg).text_size(px(12.)).child("Paste your provider API key. It is sent directly to your local OpenCode service and is not saved in Dunlin or chat history."))
                .child(div().text_size(px(12.)).child("API key"))
                .child(Input::new(&dialog.input).disabled(dialog.busy))
                .when(dialog.provider_id == crate::providers::GO, |view| view.child(
                    Button::new("auth-console").ghost().small().label("Get a Go API key ↗")
                        .on_click(|_, _, cx| cx.open_url("https://opencode.ai/console/go"))));
        } else if !dialog.methods.is_empty() {
            content = content.child(div().text_color(p.muted_fg).text_size(px(12.)).child("Continue in your browser to authorize your account. Dunlin will detect completion and refresh your models automatically."));
        }
    }
    if let Some(attempt) = waiting {
        let url = attempt.url.clone();
        let instructions = attempt.instructions.clone();
        content = content
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(p.muted_fg)
                    .child("Waiting for authorization in your browser…"),
            )
            .child(div().text_size(px(13.)).child(attempt.instructions))
            .child(
                Button::new("auth-open-browser")
                    .label("Open sign-in page ↗")
                    .small()
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            )
            .child(
                Button::new("auth-copy-code")
                    .ghost()
                    .small()
                    .label("Copy authorization instructions")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(instructions.clone()))
                    }),
            );
        if code {
            content = content
                .child(div().text_size(px(12.)).child("Authorization code"))
                .child(Input::new(&dialog.input).disabled(dialog.busy));
        }
    }
    if connected {
        content = content
            .child(div().text_color(p.green).child("Account connected"))
            .child(div().text_size(px(12.)).text_color(p.muted_fg).child("Your models are refreshing. Choose a model in the composer to start. Availability and usage limits depend on your provider plan."));
    }
    if let Some(error) = &dialog.error {
        content = content.child(
            div()
                .rounded(px(7.))
                .p(px(10.))
                .bg(p.red_soft)
                .text_size(px(12.))
                .text_color(p.red)
                .child(error.clone()),
        );
    }
    let primary_label = if dialog.busy {
        "Connecting…"
    } else if key_method {
        "Connect account"
    } else if code {
        "Finish sign-in"
    } else {
        "Continue in browser"
    };
    div()
        .id("provider-auth-overlay")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(|this, _, window, cx| this.close_provider_auth(window, cx)))
        .child(
            div()
                .id("provider-auth-dialog")
                .child(crate::effects::rounded_scroll_probe(
                    &this.overlay_backdrop,
                    14.,
                ))
                .w(px(460.))
                .max_h(px(650.))
                .overflow_y_scroll()
                .rounded(px(14.))
                .border_1()
                .border_color(p.border_strong)
                .bg(p.card)
                .p(px(24.))
                .flex()
                .flex_col()
                .gap(px(20.))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(18.))
                                .child(format!("Connect {}", dialog.name)),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(p.muted_fg)
                                .child("Provider account · OpenCode harness"),
                        ),
                )
                .child(content)
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("auth-cancel")
                                .ghost()
                                .label(if connected { "Done" } else { "Cancel" })
                                .disabled(dialog.busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.close_provider_auth(window, cx)
                                })),
                        )
                        .when((ready && !dialog.methods.is_empty()) || code, |view| {
                            view.child(
                                Button::new("auth-submit")
                                    .primary()
                                    .label(primary_label)
                                    .disabled(dialog.busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_provider_auth(window, cx)
                                    })),
                            )
                        })
                        .when(ready && dialog.service.is_none(), |view| {
                            view.child(Button::new("auth-retry").primary().label("Retry").on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.connect_provider(window, cx)
                                }),
                            ))
                        }),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    #[test]
    fn credentials_only_go_to_local_non_redirecting_service() {
        for url in [
            "http://127.0.0.1:1234",
            "http://[::1]:1234",
            "https://localhost:1234",
        ] {
            assert!(local_url(url).is_ok());
        }
        for url in [
            "https://example.com",
            "http://127.0.0.1.evil.test",
            "http://user:secret@localhost",
            "file:///etc/passwd",
            "http://localhost/path",
            "http://localhost?redirect=evil",
        ] {
            assert!(local_url(url).is_err());
        }
        assert!(browser_url(
            "https://accounts.example.com/authorize?code=example"
        ));
        assert!(!browser_url("javascript:alert(1)"));
        assert!(!browser_url("http://accounts.example.com/authorize"));
    }

    #[test]
    fn only_advertised_supported_auth_methods_are_offered() {
        let choices = methods(&json!({"methods":[
            {"type":"key"}, {"type":"env","names":["API_KEY"]},
            {"type":"oauth","id":"device","label":"Account","form":[{"key":"server","hidden":true,"default":"https://example.com"}]},
            {"type":"oauth","id":"custom","label":"Custom","form":[{"key":"tenant","required":true}]}
        ]}));
        assert_eq!(choices.len(), 2);
        assert!(matches!(choices[0].kind, MethodKind::Key));
        assert_eq!(choices[1].answer["server"], "https://example.com");
        assert!(matches!(&choices[1].kind, MethodKind::OAuth(id) if id == "device"));
        assert_eq!(preferred_method(&choices), 1);
        assert_eq!(
            preferred_method(&methods(&json!({"methods":[{"type":"key"}]}))),
            0
        );
    }

    #[test]
    fn absent_composer_catalog_does_not_route_go_to_the_key_only_integration() {
        // Resolution uses a fresh provider response, not a missing/stale UI entry.
        let go =
            json!({"data":{"id":"opencode-go","integrationID":"opencode","activation":"auto"}});
        assert_eq!(
            resolved_integration(crate::providers::GO, &go).unwrap(),
            "opencode"
        );
        let console =
            json!({"data":{"id":"opencode","integrationID":"opencode","activation":"auto"}});
        assert_eq!(
            resolved_integration(crate::providers::FREE, &console).unwrap(),
            "opencode"
        );
        assert!(resolved_integration(crate::providers::GO, &json!({"data":null})).is_err());
        assert!(resolved_integration(crate::providers::GO, &console).is_err());
        assert!(resolved_integration("off", &json!({"id":"off","activation":"disabled"})).is_err());
        // A genuinely key-only provider still uses its own advertised integration.
        assert_eq!(
            resolved_integration("direct", &json!({"id":"direct","integrationID":"key-only"}))
                .unwrap(),
            "key-only"
        );
    }

    #[test]
    #[ignore = "requires the installed local OpenCode service; public discovery only"]
    fn local_console_discovery_exposes_browser_auth_without_signing_in() {
        let directory = std::env::current_dir().unwrap();
        let directory = directory.to_str().unwrap();
        let service = LocalService::discover(directory).unwrap();
        for provider_id in [crate::providers::GO, crate::providers::FREE] {
            let provider = api(
                directory,
                &format!(
                    "provider/{}",
                    crate::providers::account_provider(provider_id)
                ),
            )
            .unwrap();
            let integration = resolved_integration(provider_id, &provider).unwrap();
            let response = service
                .request("GET", &auth_path(directory, &integration, ""), None)
                .unwrap();
            let choices = methods(response.get("data").unwrap());
            assert!(matches!(
                choices[preferred_method(&choices)].kind,
                MethodKind::OAuth(_)
            ));
        }
    }

    #[test]
    fn auth_paths_encode_integration_attempt_and_directory() {
        let path = auth_path("/a b/?c", "provider/alias", "/connect/key");
        assert_eq!(
            path,
            "/api/integration/provider%2Falias/connect/key?location%5Bdirectory%5D=%2Fa%20b%2F%3Fc"
        );
    }

    #[test]
    fn key_stays_in_http_body_and_provider_errors_never_echo_it() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]);
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .map(|length| length.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            let body = "fixture-only-secret";
            write!(
                socket,
                "HTTP/1.1 400 Bad Request\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            String::from_utf8(bytes).unwrap()
        });
        let service = LocalService {
            url,
            authorization: Some("Basic fixture-service-auth".into()),
            agent: http_agent(),
        };
        let error = service
            .request(
                "POST",
                "/api/integration/opencode-go/connect/key",
                Some(json!({"key":"fixture-only-secret"})),
            )
            .unwrap_err();
        assert!(!error.contains("fixture-only-secret"));
        let request = server.join().unwrap();
        let (headers, body) = request.split_once("\r\n\r\n").unwrap();
        assert!(!headers.contains("fixture-only-secret"));
        assert!(
            headers
                .to_lowercase()
                .contains("authorization: basic fixture-service-auth")
        );
        assert_eq!(
            serde_json::from_str::<Value>(body).unwrap()["key"],
            "fixture-only-secret"
        );
    }

    #[test]
    fn redirects_cannot_forward_a_key_to_another_service() {
        let target = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        target.set_nonblocking(true).unwrap();
        let target_url = format!("http://{}/collect", target.local_addr().unwrap());
        let observer = std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_millis(300);
            while std::time::Instant::now() < deadline {
                match target.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        let mut buffer = [0; 4096];
                        let _ = socket.read(&mut buffer);
                        let _ = socket.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                        );
                        return true;
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    Err(error) => panic!("redirect observer failed: {error}"),
                }
            }
            false
        });
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            while !request.ends_with(b"}") {
                let count = socket.read(&mut buffer).unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
            }
            write!(socket, "HTTP/1.1 307 Temporary Redirect\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        });
        let service = LocalService {
            url,
            authorization: None,
            agent: http_agent(),
        };
        assert!(
            service
                .request(
                    "POST",
                    "/connect/key",
                    Some(json!({"key":"fixture-only-secret"}))
                )
                .is_err()
        );
        server.join().unwrap();
        assert!(
            !observer.join().unwrap(),
            "auth request followed a redirect"
        );
    }

    #[test]
    #[ignore = "read-only integration check against an installed local OpenCode service"]
    fn discovers_local_service_without_modifying_accounts() {
        let directory = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let service = LocalService::discover(&directory).unwrap_or_else(|error| panic!("{error}"));
        let provider = api(&directory, "provider/opencode-go").unwrap();
        let integration = resolved_integration(crate::providers::GO, &provider).unwrap();
        let response = service
            .request("GET", &auth_path(&directory, &integration, ""), None)
            .unwrap();
        assert!(!methods(&response["data"]).is_empty());
    }
}
