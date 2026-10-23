//! A deterministic ACP agent used to exercise permission handling.
//!
//! It speaks just enough ACP v1 to be useful: initialize, open a session, and
//! answer a prompt by asking the client for permission before replying. The
//! client's choice decides whether the mock reports success or refusal.
//!
//! Run it over stdio; Kestrel spawns it like any other ACP harness:
//! `mock_acp_agent`.

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, PromptRequest,
    PromptResponse, RequestPermissionOutcome, RequestPermissionRequest, SessionNotification,
    SessionUpdate, StopReason, TextContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields,
};
use agent_client_protocol::{Agent, Result, Stdio};

fn main() -> Result<()> {
    async_io::block_on(
        Agent
            .builder()
            .name("kestrel-mock-agent")
            .on_receive_request(
                async |request: InitializeRequest,
                       responder,
                       _connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_capabilities(AgentCapabilities::new())
                            .agent_info(
                                agent_client_protocol::schema::v1::Implementation::new(
                                    "Kestrel Mock",
                                    "0.1.0",
                                )
                                .title("Kestrel Mock"),
                            ),
                    )
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |_request: NewSessionRequest,
                       responder,
                       _connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| { responder.respond(NewSessionResponse::new("mock-session-1")) },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: PromptRequest,
                       responder,
                       connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    let session_id = request.session_id;
                    connection.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("The mock agent needs permission to write a file."),
                        ))),
                    ))?;

                    let tool_call = ToolCallUpdate::new(
                        "mock-write-1",
                        ToolCallUpdateFields::new()
                            .title("Write hello.txt")
                            .kind(agent_client_protocol::schema::v1::ToolKind::Edit)
                            .status(ToolCallStatus::Pending),
                    );
                    let options = vec![
                        PermissionOption::new(
                            "allow-once",
                            "Allow once",
                            PermissionOptionKind::AllowOnce,
                        ),
                        PermissionOption::new(
                            "allow-always",
                            "Always allow",
                            PermissionOptionKind::AllowAlways,
                        ),
                        PermissionOption::new(
                            "reject-once",
                            "Reject",
                            PermissionOptionKind::RejectOnce,
                        ),
                    ];

                    connection
                        .send_request(RequestPermissionRequest::new(
                            session_id.clone(),
                            tool_call,
                            options,
                        ))
                        .on_receiving_result(async move |result| {
                            let outcome = result?.outcome;
                            let (text, stop_reason) = match outcome {
                                RequestPermissionOutcome::Selected(selected) => {
                                    if selected.option_id.0.as_ref() == "reject-once" {
                                        (
                                            "Permission denied. The mock agent did not write anything.",
                                            StopReason::Refusal,
                                        )
                                    } else {
                                        (
                                            "Permission granted. The mock agent wrote hello.txt.",
                                            StopReason::EndTurn,
                                        )
                                    }
                                }
                                _ => (
                                    "Permission request was cancelled.",
                                    StopReason::Cancelled,
                                ),
                            };

                            connection.send_notification(SessionNotification::new(
                                session_id,
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(
                                    ContentBlock::Text(TextContent::new(text)),
                                )),
                            ))?;
                            responder.respond(PromptResponse::new(stop_reason))
                        })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .connect_to(Stdio::new()),
    )
}
