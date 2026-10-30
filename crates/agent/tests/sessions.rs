use std::time::Duration;

use dunlin_agent::{AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate};
use futures_util::future::{Either, select};

fn config() -> HarnessConfig {
    HarnessConfig::new(
        format!("\"{}\"", env!("CARGO_BIN_EXE_mock_acp_agent")),
        std::env::current_dir().unwrap(),
    )
}

fn next(harness: &AcpHarness) -> HarnessEvent {
    async_io::block_on(async {
        match select(
            Box::pin(harness.events().recv()),
            Box::pin(async_io::Timer::after(Duration::from_secs(10))),
        )
        .await
        {
            Either::Left((event, _)) => event.expect("worker closed unexpectedly"),
            Either::Right(_) => panic!("timed out waiting for ACP event"),
        }
    })
}

fn next_model_change(harness: &AcpHarness) -> HarnessEvent {
    loop {
        match next(harness) {
            HarnessEvent::Modes { .. } => continue,
            event => return event,
        }
    }
}

#[test]
fn synthetic_stream_grows_one_reply_without_tools_permissions_or_network() {
    let harness = AcpHarness::spawn(HarnessConfig::new(
        format!(
            "/usr/bin/env DUNLIN_MOCK_STREAM_CHUNKS=24 \"{}\"",
            env!("CARGO_BIN_EXE_mock_acp_agent")
        ),
        std::env::current_dir().unwrap(),
    ));
    loop {
        match next(&harness) {
            HarnessEvent::SessionCreated { .. } => break,
            HarnessEvent::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
    assert!(harness.prompt("Scroll fixture"));
    let mut text = String::new();
    loop {
        match next(&harness) {
            HarnessEvent::Update {
                update: HarnessUpdate::AgentMessage(chunk),
                ..
            } => text.push_str(&chunk),
            HarnessEvent::Update {
                update: HarnessUpdate::ToolCall(_) | HarnessUpdate::ToolCallUpdate(_),
                ..
            }
            | HarnessEvent::PermissionRequested { .. } => {
                panic!("Streaming fixture must not perform tools or request permission")
            }
            HarnessEvent::Error { message } => panic!("{message}"),
            HarnessEvent::PromptFinished { .. } => break,
            _ => {}
        }
    }
    assert_eq!(text.matches("Stream line ").count(), 24);
    assert!(text.ends_with("SCROLL END — latest reply is visible."));
}

#[test]
fn optional_end_turn_usage_survives_the_acp_transport_without_provider_calls() {
    let harness = AcpHarness::spawn(HarnessConfig::new(
        format!(
            "/usr/bin/env DUNLIN_MOCK_STREAM_CHUNKS=2 DUNLIN_MOCK_TOKEN_USAGE=1 \"{}\"",
            env!("CARGO_BIN_EXE_mock_acp_agent")
        ),
        std::env::current_dir().unwrap(),
    ));
    loop {
        match next(&harness) {
            HarnessEvent::SessionCreated { .. } => break,
            HarnessEvent::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
    assert!(harness.prompt("Usage fixture"));
    let mut chunks = 0;
    loop {
        match next(&harness) {
            HarnessEvent::Update {
                update: HarnessUpdate::AgentMessage(_),
                ..
            } => chunks += 1,
            HarnessEvent::PromptFinished { usage, .. } => {
                let usage = usage.expect("mock sent optional end-turn usage");
                assert_eq!((usage.total, usage.input, usage.output), (1200, 1000, 200));
                assert!(chunks > 1, "response was streamed before completion");
                break;
            }
            HarnessEvent::Error { message } => panic!("{message}"),
            HarnessEvent::PermissionRequested { .. } => {
                panic!("no permissions in synthetic stream")
            }
            _ => {}
        }
    }
}

#[test]
fn restores_the_existing_session_instead_of_creating_a_new_one() {
    let first = AcpHarness::spawn(config());
    let id = loop {
        match next(&first) {
            HarnessEvent::SessionCreated { session_id } => break session_id,
            HarnessEvent::Error { message } => panic!("{message}"),
            _ => {}
        }
    };
    drop(first);
    let mut resumed = config();
    resumed.resume_session_id = Some(id.clone());
    let resumed = AcpHarness::spawn(resumed);
    let mut saw_replay = false;
    loop {
        match next(&resumed) {
            HarnessEvent::SessionCreated { .. } => {
                panic!("resuming must not create another session")
            }
            HarnessEvent::Update {
                session_id,
                update: HarnessUpdate::AgentMessage(text),
            } => {
                assert_eq!(session_id, id);
                saw_replay |= text.contains("Restored session history");
            }
            HarnessEvent::SessionLoaded { session_id } => {
                assert_eq!(session_id, id);
                assert!(saw_replay);
                break;
            }
            HarnessEvent::Error { message } | HarnessEvent::SessionLoadFailed { message, .. } => {
                panic!("{message}")
            }
            _ => {}
        }
    }
}

#[test]
fn discovers_models_changes_model_and_reports_a_rejected_change() {
    let harness = AcpHarness::spawn(config());
    let (id, state) = loop {
        match next(&harness) {
            HarnessEvent::Models {
                session_id,
                state: Some(state),
            } => break (session_id, state),
            HarnessEvent::Error { message } => panic!("{message}"),
            _ => {}
        }
    };
    assert_eq!(state.current, "mock-a");
    assert_eq!(state.choices.len(), 2);
    assert!(harness.set_model(id.clone(), state.config_id.clone(), "mock-b".into()));
    match next_model_change(&harness) {
        HarnessEvent::Models {
            session_id,
            state: Some(state),
        } => {
            assert_eq!(session_id, id);
            assert_eq!(state.current, "mock-b");
        }
        event => panic!("unexpected model acknowledgement: {event:?}"),
    }
    assert!(harness.set_model(id.clone(), state.config_id, "unknown".into()));
    match next_model_change(&harness) {
        HarnessEvent::ModelChangeFailed {
            session_id,
            message,
        } => {
            assert_eq!(session_id, id);
            assert!(message.contains("Unknown mock model"));
        }
        event => panic!("unexpected rejection: {event:?}"),
    }
}

#[test]
fn failed_resume_does_not_silently_start_a_blank_session() {
    let mut config = config();
    config.resume_session_id = Some("missing-session".into());
    let harness = AcpHarness::spawn(config);
    loop {
        match next(&harness) {
            HarnessEvent::SessionCreated { .. } => {
                panic!("must not silently replace a missing session")
            }
            HarnessEvent::SessionLoadFailed {
                session_id,
                message,
            } => {
                assert_eq!(session_id, "missing-session");
                assert!(message.contains("Session not found"));
                break;
            }
            _ => {}
        }
    }
    // Recovery is explicit: the user can still choose New Session.
    assert!(harness.new_session(std::env::current_dir().unwrap()));
    loop {
        match next(&harness) {
            HarnessEvent::SessionCreated { .. } => break,
            HarnessEvent::Error { message } => panic!("{message}"),
            _ => {}
        }
    }
}

#[test]
fn unsupported_resume_is_reported_without_creating_a_session() {
    let mut config = config();
    if let dunlin_agent::HarnessTransport::Local { command } = &mut config.transport {
        *command = format!("env DUNLIN_MOCK_NO_LOAD=1 {command}");
    }
    config.resume_session_id = Some("existing-session".into());
    let harness = AcpHarness::spawn(config);
    loop {
        match next(&harness) {
            HarnessEvent::SessionCreated { .. } => panic!("must not silently replace a session"),
            HarnessEvent::SessionLoadFailed { message, .. } => {
                assert!(message.contains("cannot resume"));
                break;
            }
            _ => {}
        }
    }
}

#[test]
fn discovers_and_switches_modes_using_both_acp_setters() {
    for config_mode in [false, true] {
        let mut config = config();
        if config_mode
            && let dunlin_agent::HarnessTransport::Local { command } = &mut config.transport
        {
            *command = format!("env DUNLIN_MOCK_CONFIG_MODES=1 {command}");
        }
        let harness = AcpHarness::spawn(config);
        let (id, modes) = loop {
            match next(&harness) {
                HarnessEvent::Modes {
                    session_id,
                    state: Some(state),
                } => break (session_id, state),
                HarnessEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        };
        assert_eq!(modes.current, "build");
        assert_eq!(modes.config_id.is_some(), config_mode);
        assert_eq!(modes.choices.len(), 2);
        assert!(harness.set_mode(id.clone(), modes.config_id.clone(), "plan".into()));
        loop {
            match next(&harness) {
                HarnessEvent::Modes {
                    session_id,
                    state: Some(state),
                } => {
                    assert_eq!(session_id, id);
                    assert_eq!(state.current, "plan");
                    break;
                }
                HarnessEvent::ModeChangeFailed { message, .. }
                | HarnessEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        }
        assert!(harness.set_mode(id.clone(), modes.config_id, "unsupported".into()));
        loop {
            match next(&harness) {
                HarnessEvent::ModeChangeFailed {
                    session_id,
                    message,
                } => {
                    assert_eq!(session_id, id);
                    assert!(message.contains("Unknown mock mode"));
                    break;
                }
                HarnessEvent::Error { message } => panic!("{message}"),
                _ => {}
            }
        }
    }
}
