//! End-to-end smoke test for the reusable ACP harness.
//!
//! Usage: `cargo run -p dunlin-agent --bin harness_smoke -- "opencode acp" "Reply with: HARNESS OK"`

use dunlin_agent::{AcpHarness, HarnessConfig, HarnessEvent, HarnessUpdate};
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "opencode acp".into());
    let prompt = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Reply with exactly: HARNESS OK".into());
    let cwd = std::env::current_dir()?;

    let harness = AcpHarness::spawn(HarnessConfig::new(command, cwd));
    let events = harness.events().clone();

    let mut sent = false;
    let mut output = String::new();
    let mut saw_session = false;

    while let Ok(event) = async_io::block_on(events.recv()) {
        if handle(
            event,
            &mut sent,
            &mut saw_session,
            &mut output,
            &harness,
            &prompt,
        ) {
            break;
        }
    }

    println!("\n--- final ---\n{output}");
    Ok(())
}

fn handle(
    event: HarnessEvent,
    sent: &mut bool,
    saw_session: &mut bool,
    output: &mut String,
    harness: &AcpHarness,
    prompt: &str,
) -> bool {
    match event {
        HarnessEvent::Ready { agent, version } => {
            println!("[ready] {agent} {version}");
        }
        HarnessEvent::SessionCreated { session_id } => {
            println!("[session] {session_id}");
            *saw_session = true;
            if !*sent {
                *sent = true;
                println!("[prompt] {prompt}");
                harness.prompt(prompt);
            }
        }
        HarnessEvent::SessionLoaded { session_id } => {
            println!("[loaded] {session_id}");
        }
        HarnessEvent::Update { update, .. } => match update {
            HarnessUpdate::UserMessage(text) => println!("[user] {text}"),
            HarnessUpdate::Configuration { models, modes } => {
                println!("[configuration] models={models:?} modes={modes:?}")
            }
            HarnessUpdate::AgentMessage(text) => {
                print!("{text}");
                output.push_str(&text);
            }
            HarnessUpdate::AgentThought(text) => {
                eprint!("{text}");
            }
            HarnessUpdate::MessageChunk { text, thought, .. } => {
                if thought {
                    eprint!("{text}");
                } else {
                    print!("{text}");
                    output.push_str(&text);
                }
            }
            HarnessUpdate::Child { id, .. } => println!("\n[subagent update] {id}"),
            HarnessUpdate::ToolCall(tool) => {
                println!(
                    "\n[tool] {} ({:?} / {:?})",
                    tool.title, tool.kind, tool.status
                );
            }
            HarnessUpdate::ToolCallUpdate(delta) => {
                println!("\n[tool update] {} {:?}", delta.id, delta.status);
            }
            HarnessUpdate::Plan(items) => {
                println!("\n[plan] {} items", items.len());
            }
            HarnessUpdate::Usage {
                used,
                size,
                cost_usd,
            } => {
                println!("\n[usage] {used}/{size} ${:.4}", cost_usd.unwrap_or(0.0));
            }
            HarnessUpdate::AvailableCommands(commands) => {
                println!("\n[commands] {commands:?}");
            }
        },
        HarnessEvent::PermissionRequested {
            request_id,
            title,
            options,
            ..
        } => {
            println!("\n[permission] {title} {options:?}");
            let allow = options
                .iter()
                .find(|option| {
                    matches!(
                        option.kind,
                        dunlin_agent::PermissionKind::AllowOnce
                            | dunlin_agent::PermissionKind::AllowAlways
                    )
                })
                .or_else(|| options.first());
            harness.respond_permission(request_id, allow.map(|option| option.id.clone()));
        }
        HarnessEvent::PromptFinished { stop_reason, .. } => {
            println!("\n[done] {stop_reason}");
            return true;
        }
        HarnessEvent::Error { message } => {
            eprintln!("\n[error] {message}");
        }
        HarnessEvent::SessionLoadFailed { message, .. }
        | HarnessEvent::ModelChangeFailed { message, .. }
        | HarnessEvent::ModeChangeFailed { message, .. } => eprintln!("[error] {message}"),
        HarnessEvent::Models { state, .. } => println!("[models] {state:?}"),
        HarnessEvent::Modes { state, .. } => println!("[modes] {state:?}"),
        HarnessEvent::Exited { reason } => {
            eprintln!("\n[exited] {reason}");
            if !*saw_session {
                return true;
            }
        }
    }
    let _ = std::io::stdout().flush();
    false
}
