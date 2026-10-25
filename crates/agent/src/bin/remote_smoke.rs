//! Manual smoke test for the remote ACP transport.
//!
//! Usage:
//!   remote_smoke <relay-url> <device-token> <project-id> <remote-cwd> [prompt]

use std::path::PathBuf;

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
    while let Ok(event) = events.recv_blocking() {
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
            HarnessEvent::PromptFinished { .. } => break,
            HarnessEvent::Exited { .. } | HarnessEvent::Error { .. } => break,
            _ => {}
        }
    }
    harness.shutdown();
}
