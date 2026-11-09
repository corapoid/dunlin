//! Manual smoke test for the remote ACP transport.
//!
//! Usage:
//!   remote_smoke <relay-url> <device-token> <project-id> <remote-cwd> [prompt]

use std::path::PathBuf;
use std::time::{Duration, Instant};

use dunlin_agent::{AcpHarness, HarnessConfig, HarnessEvent};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() < 4 {
        eprintln!("usage: remote_smoke <relay-url> <token> <project-id> <cwd> [prompt]");
        std::process::exit(2);
    }
    let cwd = PathBuf::from(&args[3]);
    let config = HarnessConfig::remote(
        args[0].clone(),
        args[1].clone(),
        args[2].clone(),
        cwd.clone(),
    );
    let prompt = args
        .get(4)
        .cloned()
        .unwrap_or_else(|| "Say hello from the relay".into());

    let harness = AcpHarness::spawn(config);
    let events = harness.events();
    let mut started = false;
    let mut prompted = false;
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut completed = false;
    while Instant::now() < deadline {
        let event = match events.try_recv() {
            Ok(event) => event,
            Err(async_channel::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }
            Err(_) => break,
        };
        println!("{event:?}");
        match event {
            HarnessEvent::Ready { .. } if !started => {
                started = true;
                harness.new_session(cwd.clone());
            }
            HarnessEvent::SessionCreated { .. } if !prompted => {
                prompted = true;
                harness.prompt(prompt.clone());
            }
            HarnessEvent::PermissionRequested {
                request_id,
                options,
                ..
            } => {
                let option = options.first().map(|option| option.id.clone());
                harness.respond_permission(request_id, option);
            }
            HarnessEvent::PromptFinished { .. } => {
                completed = prompted;
                break;
            }
            HarnessEvent::Exited { .. } | HarnessEvent::Error { .. } => break,
            _ => {}
        }
    }
    harness.shutdown();
    if !completed {
        eprintln!(
            "FAIL: remote ACP exited, failed, or exceeded its deadline before prompt completion"
        );
        std::process::exit(1);
    }
    println!("PASS: remote ACP prompt completed");
}
