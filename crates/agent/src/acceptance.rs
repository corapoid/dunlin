//! Explicitly invoked deployed-host ACP acceptance. No #[test], model fallback,
//! account discovery or background execution. Used by the manual desktop probe.
use crate::{AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate, PermissionKind};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Deserialize)]
pub struct Acceptance {
    pub url: String,
    pub token: String,
    pub project_id: String,
    pub cwd: String,
    pub model: String,
    pub prompt: String,
    pub expected: String,
    #[serde(default = "timeout")]
    pub timeout_secs: u64,
    #[serde(default)]
    pub resume_session_id: Option<String>,
    #[serde(default)]
    pub cancel_after_ms: Option<u64>,
    #[serde(default)]
    pub allow_permissions: bool,
    #[serde(default)]
    pub expect_denied: bool,
    #[serde(default)]
    pub tls_ca_pem: Option<String>,
}

fn timeout() -> u64 {
    180
}

#[derive(Debug, Serialize)]
pub struct Evidence {
    pub agent: String,
    pub version: String,
    pub session_id: String,
    pub model: String,
    pub text: String,
    pub chunks: usize,
    pub permissions: usize,
    pub stop_reason: String,
}

pub fn run(spec: Acceptance) -> Result<Evidence, String> {
    if !spec.model.starts_with("opencode/") || !spec.model.ends_with("-free") {
        return Err(
            "Manual acceptance requires an explicit anonymous opencode/*-free model".into(),
        );
    }
    if spec.expected.is_empty() || !(1..=600).contains(&spec.timeout_secs) {
        return Err("Expected response and a bounded timeout are mandatory".into());
    }
    let mut config = HarnessConfig::remote(&spec.url, &spec.token, &spec.project_id, &spec.cwd);
    config.default_model = Some(spec.model.clone());
    config.tls_root_pem = spec.tls_ca_pem.as_ref().map(|pem| pem.as_bytes().to_vec());
    let harness = AcpHarness::spawn(config);
    let events = harness.events();
    let started = Instant::now();
    let mut prompted_at = None;
    let mut cancelled = false;
    let mut evidence = Evidence {
        agent: String::new(),
        version: String::new(),
        session_id: String::new(),
        model: spec.model.clone(),
        text: String::new(),
        chunks: 0,
        permissions: 0,
        stop_reason: String::new(),
    };
    let result = loop {
        if started.elapsed() > Duration::from_secs(spec.timeout_secs) {
            break Err("ACP acceptance deadline exceeded".into());
        }
        if let (Some(at), Some(ms)) = (prompted_at, spec.cancel_after_ms)
            && Instant::now().duration_since(at) >= Duration::from_millis(ms)
            && !cancelled
        {
            harness.cancel();
            cancelled = true;
        }
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) => break Err("ACP event stream closed before completion".into()),
        };
        match event {
            HarnessEvent::Ready { agent, version } => {
                if !agent.to_lowercase().contains("opencode") {
                    break Err(format!("Expected the real OpenCode harness, got {agent}"));
                }
                evidence.agent = agent;
                evidence.version = version;
                if let Some(id) = &spec.resume_session_id {
                    harness.load_session(id, &spec.cwd);
                } else {
                    harness.new_session_with_model(&spec.cwd, Some(spec.model.clone()));
                }
            }
            HarnessEvent::SessionCreated { session_id }
            | HarnessEvent::SessionLoaded { session_id } => {
                evidence.session_id = session_id;
                evidence.text.clear(); // session/load replays earlier messages
                evidence.chunks = 0;
                if prompted_at.is_some() {
                    break Err("Duplicate session initialization".into());
                }
                harness.prompt(&spec.prompt);
                eprintln!("ACP_ACCEPTANCE: prompt_started");
                prompted_at = Some(Instant::now());
            }
            HarnessEvent::Update {
                update:
                    HarnessUpdate::AgentMessage(text)
                    | HarnessUpdate::MessageChunk {
                        text,
                        thought: false,
                        ..
                    },
                ..
            } if prompted_at.is_some() => {
                evidence.chunks += 1;
                evidence.text.push_str(&text);
                if evidence.text.len() > 1024 * 1024 {
                    break Err("Unbounded ACP response".into());
                }
            }
            HarnessEvent::PermissionRequested {
                request_id,
                options,
                ..
            } => {
                evidence.permissions += 1;
                let kind = if spec.allow_permissions {
                    PermissionKind::AllowOnce
                } else {
                    PermissionKind::RejectOnce
                };
                let option = options
                    .iter()
                    .find(|option| option.kind == kind)
                    .map(|option| option.id.clone());
                harness.respond_permission(request_id, option);
            }
            HarnessEvent::PromptFinished { stop_reason, .. } => {
                if prompted_at.is_none() {
                    break Err("Prompt completed before it was submitted".into());
                }
                if spec.expect_denied {
                    if spec.allow_permissions
                        || evidence.permissions == 0
                        || !matches!(stop_reason.as_str(), "cancelled" | "end_turn")
                    {
                        break Err(
                            "Expected an observed rejected permission and a terminal stop reason"
                                .into(),
                        );
                    }
                    // OpenCode rejects pending tool work by cancelling the turn;
                    // an assistant refusal message is not required in that case.
                    if stop_reason == "end_turn" && !evidence.text.contains(&spec.expected) {
                        break Err("Denied turn did not acknowledge refusal".into());
                    }
                } else if spec.cancel_after_ms.is_some() {
                    if !cancelled || stop_reason != "cancelled" {
                        break Err(format!("Expected cancellation, got {stop_reason}"));
                    }
                } else if stop_reason != "end_turn"
                    || evidence.chunks == 0
                    || !evidence.text.contains(&spec.expected)
                {
                    break Err(format!(
                        "Missing expected response or unsuccessful stop reason ({stop_reason}; {} chunks)",
                        evidence.chunks
                    ));
                }
                evidence.stop_reason = stop_reason;
                break Ok(evidence);
            }
            HarnessEvent::Error { message }
            | HarnessEvent::SessionLoadFailed { message, .. }
            | HarnessEvent::ModelChangeFailed { message, .. } => break Err(message),
            HarnessEvent::Exited { reason } => break Err(format!("Harness exited: {reason}")),
            _ => {}
        }
    };
    // shutdown sends a command; the caller also supervises the process group.
    harness.shutdown();
    result
}
