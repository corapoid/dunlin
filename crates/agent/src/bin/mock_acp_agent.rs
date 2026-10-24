//! A deterministic ACP agent used to exercise permission handling.
//!
//! It speaks just enough ACP v1 to be useful: initialize, open a session, and
//! answer a prompt by asking the client for permission before replying. The
//! client's choice decides whether the mock reports success or refusal.
//!
//! Run it over stdio; Kestrel spawns it like any other ACP harness:
//! `mock_acp_agent`.

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, Diff, InitializeRequest, InitializeResponse,
    LoadSessionRequest, LoadSessionResponse, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority, PlanEntryStatus,
    PromptRequest, PromptResponse, RequestPermissionOutcome, RequestPermissionRequest,
    SessionNotification, SessionUpdate, StopReason, TextContent, ToolCall, ToolCallContent,
    ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, UsageUpdate,
};
use agent_client_protocol::{Agent, Result, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

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
                >| {
                    let id = format!(
                        "mock-{}-{}",
                        std::process::id(),
                        SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
                    );
                    responder.respond(NewSessionResponse::new(id))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: LoadSessionRequest,
                       responder,
                       connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    connection.send_notification(SessionNotification::new(
                        request.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("Restored session history."),
                        ))),
                    ))?;
                    responder.respond(LoadSessionResponse::new())
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: PromptRequest,
                       responder,
                       connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    let session_id = request.session_id;
                    if let Ok(value) = std::env::var("KESTREL_MOCK_DELAY_MS")
                        && let Ok(ms) = value.parse::<u64>()
                    {
                        async_io::Timer::after(std::time::Duration::from_millis(ms)).await;
                    }
                    connection.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::AgentThoughtChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("The mock agent needs permission to write a file."),
                        ))),
                    ))?;
                    async_io::Timer::after(std::time::Duration::from_millis(60)).await;
                    connection.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::UsageUpdate(UsageUpdate::new(1_200, 200_000)),
                    ))?;
                    connection.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::Plan(Plan::new(vec![
                            PlanEntry::new(
                                "Inspect the workspace",
                                PlanEntryPriority::High,
                                PlanEntryStatus::Completed,
                            ),
                            PlanEntry::new(
                                "Write hello.txt",
                                PlanEntryPriority::Medium,
                                PlanEntryStatus::InProgress,
                            ),
                            PlanEntry::new(
                                "Verify the change",
                                PlanEntryPriority::Low,
                                PlanEntryStatus::Pending,
                            ),
                        ])),
                    ))?;
                    async_io::Timer::after(std::time::Duration::from_millis(40)).await;
                    connection.send_notification(SessionNotification::new(
                        session_id.clone(),
                        SessionUpdate::ToolCall(
                            ToolCall::new("mock-write-1", "Write hello.txt")
                                .kind(agent_client_protocol::schema::v1::ToolKind::Edit)
                                .status(ToolCallStatus::InProgress)
                                .content(vec![ToolCallContent::Diff(
                                    Diff::new("/tmp/kestrel-smoke/hello.txt", "Hello from Kestrel\n")
                                        .old_text(String::new()),
                                )]),
                        ),
                    ))?;
                    async_io::Timer::after(std::time::Duration::from_millis(80)).await;

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

                            let denied = matches!(stop_reason, StopReason::Refusal);
                            async_io::Timer::after(std::time::Duration::from_millis(40)).await;
                            connection.send_notification(SessionNotification::new(
                                session_id.clone(),
                                SessionUpdate::Plan(Plan::new(vec![
                                    PlanEntry::new(
                                        "Inspect the workspace",
                                        PlanEntryPriority::High,
                                        PlanEntryStatus::Completed,
                                    ),
                                    PlanEntry::new(
                                        "Write hello.txt",
                                        PlanEntryPriority::Medium,
                                        if denied {
                                            PlanEntryStatus::Pending
                                        } else {
                                            PlanEntryStatus::Completed
                                        },
                                    ),
                                    PlanEntry::new(
                                        "Verify the change",
                                        PlanEntryPriority::Low,
                                        if denied {
                                            PlanEntryStatus::Pending
                                        } else {
                                            PlanEntryStatus::Completed
                                        },
                                    ),
                                ])),
                            ))?;
                            connection.send_notification(SessionNotification::new(
                                session_id.clone(),
                                SessionUpdate::ToolCallUpdate(ToolCallUpdate::new(
                                    "mock-write-1",
                                    ToolCallUpdateFields::new()
                                        .status(if denied {
                                            ToolCallStatus::Failed
                                        } else {
                                            ToolCallStatus::Completed
                                        })
                                        .content(vec![ToolCallContent::from(ContentBlock::Text(
                                            TextContent::new(text),
                                        ))]),
                                )),
                            ))?;
                            async_io::Timer::after(std::time::Duration::from_millis(30)).await;
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
