//! One-shot sessions for headless consumers. Each run owns a fresh ACP session;
//! it never takes over a desktop session or mutates application history.
use dunlin_agent::{
    AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate, PermissionChoice, PermissionKind,
};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Clone, Default)]
pub struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PermissionDecision {
    Deny,
    AllowOnce,
}

#[derive(Debug, Serialize)]
pub struct Outcome {
    pub session_id: String,
    pub text: String,
    pub stop_reason: String,
    pub permissions: usize,
}

/// The callback makes a decision for each individual request. There is no
/// "approve all" fallback, and permanent grants are never selected here.
pub fn execute(
    config: HarnessConfig,
    prompt: &str,
    timeout: Duration,
    cancel: &Cancellation,
    mut permission: impl FnMut(&str, &[PermissionChoice]) -> PermissionDecision,
) -> Result<Outcome, String> {
    if prompt.trim().is_empty() || prompt.len() > 1024 * 1024 {
        return Err("Prompt must contain between 1 byte and 1 MiB of text".into());
    }
    if timeout.is_zero() || timeout > Duration::from_secs(3600) {
        return Err("Timeout must be between 1 and 3600 seconds".into());
    }
    if config.resume_session_id.is_some() {
        return Err("One-shot execution cannot take over an existing session".into());
    }
    if !config.cwd.is_absolute() {
        return Err("The session working directory must be absolute".into());
    }
    if cancel.is_cancelled() {
        return Err("Run cancelled".into());
    }
    let remote = config.is_remote();
    let cwd = config.cwd.clone();
    let model = config.default_model.clone();
    let harness = AcpHarness::spawn(config);
    let mut outcome = Outcome {
        session_id: String::new(),
        text: String::new(),
        stop_reason: String::new(),
        permissions: 0,
    };
    let started = Instant::now();
    let result = loop {
        if cancel.is_cancelled() {
            harness.cancel();
            break Err("Run cancelled".into());
        }
        if started.elapsed() >= timeout {
            harness.cancel();
            break Err("Run deadline exceeded".into());
        }
        let event = match harness.events().try_recv() {
            Ok(event) => event,
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) => break Err("ACP event stream closed before completion".into()),
        };
        match event {
            HarnessEvent::Ready { .. } if remote => {
                if !harness.new_session_with_model(&cwd, model.clone()) {
                    break Err("Could not create remote ACP session".into());
                }
            }
            HarnessEvent::SessionCreated { session_id } => {
                if !outcome.session_id.is_empty() {
                    break Err("Duplicate ACP session initialization".into());
                }
                outcome.session_id = session_id;
                if !harness.prompt(prompt) {
                    break Err("Could not submit prompt".into());
                }
            }
            HarnessEvent::Update {
                session_id,
                update:
                    HarnessUpdate::AgentMessage(text)
                    | HarnessUpdate::MessageChunk {
                        text,
                        thought: false,
                        ..
                    },
            } if session_id == outcome.session_id => {
                if outcome.text.len().saturating_add(text.len()) > 4 * 1024 * 1024 {
                    break Err("ACP response exceeds 4 MiB".into());
                }
                outcome.text.push_str(&text);
            }
            HarnessEvent::PermissionRequested {
                request_id,
                session_id,
                title,
                options,
            } => {
                let decision = if session_id == outcome.session_id && !outcome.session_id.is_empty()
                {
                    outcome.permissions += 1;
                    permission(&title, &options)
                } else {
                    PermissionDecision::Deny
                };
                let decision = if cancel.is_cancelled() || started.elapsed() >= timeout {
                    PermissionDecision::Deny
                } else {
                    decision
                };
                let kind = if decision == PermissionDecision::AllowOnce {
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
            HarnessEvent::PromptFinished {
                session_id,
                stop_reason,
                ..
            } if session_id == outcome.session_id && !outcome.session_id.is_empty() => {
                outcome.stop_reason = stop_reason;
                break Ok(outcome);
            }
            HarnessEvent::SessionLoadFailed { .. } => {
                break Err("Unexpected ACP session load".into());
            }
            HarnessEvent::Error { .. } => {
                break Err(
                    "ACP harness reported an error; check its configuration and credentials".into(),
                );
            }
            HarnessEvent::Exited { .. } => break Err("ACP harness exited before completion".into()),
            _ => {}
        }
    };
    harness.shutdown_and_wait();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_runs_before_spawning_any_process() {
        let config = || HarnessConfig::new("this-must-never-run", "/project");
        let deny = |_: &str, _: &[PermissionChoice]| PermissionDecision::Deny;
        assert!(
            execute(
                config(),
                " ",
                Duration::from_secs(1),
                &Cancellation::default(),
                deny
            )
            .is_err()
        );
        assert!(
            execute(
                config(),
                "hello",
                Duration::ZERO,
                &Cancellation::default(),
                deny
            )
            .is_err()
        );
        let cancel = Cancellation::default();
        cancel.cancel();
        assert_eq!(
            execute(config(), "hello", Duration::from_secs(1), &cancel, deny).unwrap_err(),
            "Run cancelled"
        );
        let mut resumed = config();
        resumed.resume_session_id = Some("desktop-session".into());
        assert!(
            execute(
                resumed,
                "hello",
                Duration::from_secs(1),
                &Cancellation::default(),
                deny
            )
            .is_err()
        );
    }
}
