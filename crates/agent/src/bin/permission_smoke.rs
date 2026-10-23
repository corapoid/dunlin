//! Round-trip test for ACP permission requests.
//!
//! Spawns the deterministic `mock_acp_agent`, sends a prompt, waits for
//! `session/request_permission`, answers it, and checks the final turn.
//!
//! Usage: `permission_smoke [allow|reject]`

use std::path::PathBuf;
use std::time::Duration;

use kestrel_agent::{AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate, PermissionKind};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "allow".into());
    let expected_denied = mode == "reject";

    let agent = mock_agent_path()?;
    let command = format!("\"{}\"", agent.display());
    println!("[agent] {command}");

    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(20));
        eprintln!("[timeout] permission round-trip did not finish in 20s");
        std::process::exit(2);
    });

    let harness = AcpHarness::spawn(HarnessConfig::new(command, std::env::current_dir()?));
    let events = harness.events().clone();

    let mut sent = false;
    let mut permission_seen = false;
    let mut reply = String::new();

    loop {
        let event = async_io::block_on(events.recv())?;
        match event {
            HarnessEvent::Ready { agent, version } => {
                println!("[ready] {agent} {version}");
            }
            HarnessEvent::SessionCreated { session_id } => {
                println!("[session] {session_id}");
                if !sent {
                    sent = true;
                    assert!(harness.prompt("Please write hello.txt"));
                }
            }
            HarnessEvent::PermissionRequested {
                request_id,
                title,
                options,
                ..
            } => {
                permission_seen = true;
                println!("[permission] {title} · {} options", options.len());
                let wanted = if expected_denied {
                    PermissionKind::RejectOnce
                } else {
                    PermissionKind::AllowOnce
                };
                let choice = options
                    .iter()
                    .find(|option| option.kind == wanted)
                    .unwrap_or_else(|| panic!("missing {wanted:?} option"));
                println!("[permission] choosing {} ({:?})", choice.id, choice.kind);
                assert!(harness.respond_permission(request_id, Some(choice.id.clone())));
            }
            HarnessEvent::Update {
                update: HarnessUpdate::AgentMessage(text),
                ..
            } => {
                reply.push_str(&text);
            }
            HarnessEvent::Update {
                update: HarnessUpdate::AgentThought(text),
                ..
            } => {
                println!("[thought] {text}");
            }
            HarnessEvent::PromptFinished { stop_reason, .. } => {
                println!("[done] {stop_reason}");
                println!("[reply] {reply}");
                assert!(permission_seen, "the mock agent never asked for permission");
                if expected_denied {
                    assert_eq!(stop_reason, "refusal");
                    assert!(reply.contains("Permission denied"));
                } else {
                    assert_eq!(stop_reason, "end_turn");
                    assert!(reply.contains("Permission granted"));
                }
                println!("[ok] permission round-trip verified ({mode})");
                return Ok(());
            }
            HarnessEvent::Error { message } => {
                panic!("harness error: {message}");
            }
            HarnessEvent::Exited { reason } => {
                panic!("harness exited early: {reason}");
            }
            _ => {}
        }
    }
}

fn mock_agent_path() -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(path) = std::env::var("KESTREL_MOCK_AGENT") {
        return Ok(PathBuf::from(path));
    }
    let exe = std::env::current_exe()?;
    let candidate = exe
        .parent()
        .ok_or("no parent directory for the test binary")?
        .join("mock_acp_agent");
    if candidate.is_file() {
        Ok(candidate)
    } else {
        Err(format!(
            "mock agent not found at {}; build it with `cargo build -p kestrel-agent --bins`",
            candidate.display()
        )
        .into())
    }
}
