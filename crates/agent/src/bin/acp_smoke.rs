use std::path::PathBuf;
use std::str::FromStr;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    ContentBlock, InitializeRequest, NewSessionRequest, PermissionOptionKind, PromptRequest,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, TextContent,
};
use agent_client_protocol::{AcpAgent, Agent, Client, ConnectionTo};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let command = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "opencode acp".to_string());
    let prompt = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "Reply with exactly: ACP OK".to_string());
    let cwd = std::env::current_dir()?;
    async_io::block_on(run(command, prompt, cwd))
}

async fn run(
    command: String,
    prompt: String,
    cwd: PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    eprintln!("[spawn] {command}");
    let agent = AcpAgent::from_str(&command)?;

    Client
        .builder()
        .name("kestrel-smoke")
        .on_receive_notification(
            async move |notification: SessionNotification, _cx| {
                match &notification.update {
                    SessionUpdate::AgentMessageChunk(chunk) => {
                        if let ContentBlock::Text(text) = &chunk.content {
                            print!("{}", text.text);
                        }
                    }
                    SessionUpdate::AgentThoughtChunk(chunk) => {
                        if let ContentBlock::Text(text) = &chunk.content {
                            eprint!("{}", text.text);
                        }
                    }
                    SessionUpdate::ToolCall(tool) => {
                        eprintln!("\n[tool] {} ({:?})", tool.title, tool.kind);
                    }
                    SessionUpdate::ToolCallUpdate(update) => {
                        eprintln!("\n[tool update] {:?}", update.fields);
                    }
                    other => eprintln!("\n[update] {other:?}"),
                }
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            async move |request: RequestPermissionRequest, responder, _cx| {
                eprintln!("\n[permission] {:?}", request);
                let option = request
                    .options
                    .iter()
                    .find(|option| option.kind == PermissionOptionKind::AllowOnce)
                    .or_else(|| request.options.first());
                let response = if let Some(option) = option {
                    RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                        SelectedPermissionOutcome::new(option.option_id.clone()),
                    ))
                } else {
                    RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
                };
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(agent, move |connection: ConnectionTo<Agent>| async move {
            let init = connection
                .send_request(InitializeRequest::new(ProtocolVersion::V1))
                .block_task()
                .await?;
            eprintln!("\n[init] {:?}", init.agent_info);

            let session = connection
                .send_request(NewSessionRequest::new(cwd))
                .block_task()
                .await?;
            eprintln!("[session] {}", session.session_id);

            let response = connection
                .send_request(PromptRequest::new(
                    session.session_id,
                    vec![ContentBlock::Text(TextContent::new(prompt))],
                ))
                .block_task()
                .await?;
            eprintln!("\n[done] {:?}", response.stop_reason);
            Ok(())
        })
        .await?;

    Ok(())
}
