//! A deterministic ACP agent used to exercise permission handling.
//!
//! It speaks just enough ACP v1 to be useful: initialize, open a session, and
//! answer a prompt by asking the client for permission before replying. The
//! client's choice decides whether the mock reports success or refusal.
//!
//! Run it over stdio; Dunlin spawns it like any other ACP harness:
//! `mock_acp_agent`.

use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, CurrentModeUpdate, Diff, InitializeRequest,
    InitializeResponse, LoadSessionRequest, LoadSessionResponse, NewSessionRequest,
    NewSessionResponse, PermissionOption, PermissionOptionKind, Plan, PlanEntry, PlanEntryPriority,
    PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOption, SessionMode, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, SetSessionModeRequest,
    SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, UsageUpdate,
};
use agent_client_protocol::{Agent, Result, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

fn config_options(current: &str, mode: &str) -> Vec<SessionConfigOption> {
    let mut options = vec![
        SessionConfigOption::select(
            "model",
            "Model",
            current.to_string(),
            vec![
                SessionConfigSelectOption::new("mock-a", "Mock A"),
                SessionConfigSelectOption::new("mock-b", "Mock B"),
            ],
        )
        .category(SessionConfigOptionCategory::Model),
    ];
    if std::env::var_os("DUNLIN_MOCK_CONFIG_MODES").is_some() {
        options.push(
            SessionConfigOption::select(
                "agent-mode",
                "Mode",
                mode.to_string(),
                vec![
                    SessionConfigSelectOption::new("build", "Build")
                        .description("Edit files and run tools"),
                    SessionConfigSelectOption::new("plan", "Plan")
                        .description("Plan before making changes"),
                ],
            )
            .category(SessionConfigOptionCategory::Mode),
        );
    }
    options
}

fn mode_state() -> SessionModeState {
    SessionModeState::new(
        "build",
        vec![
            SessionMode::new("build", "Build").description("Edit files and run tools"),
            SessionMode::new("plan", "Plan").description("Plan before making changes"),
        ],
    )
}

fn main() -> Result<()> {
    async_io::block_on(
        Agent
            .builder()
            .name("dunlin-mock-agent")
            .on_receive_request(
                async |request: InitializeRequest,
                       responder,
                       _connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_capabilities(AgentCapabilities::new().load_session(std::env::var_os("DUNLIN_MOCK_NO_LOAD").is_none()))
                            .agent_info(
                                agent_client_protocol::schema::v1::Implementation::new(
                                    "Dunlin Mock",
                                    "0.1.0",
                                )
                                .title("Dunlin Mock"),
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
                    responder.respond(NewSessionResponse::new(id).config_options(config_options("mock-a", "build")).modes(mode_state()))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: LoadSessionRequest,
                       responder,
                       connection: agent_client_protocol::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    if request.session_id.to_string() == "missing-session" {
                        return responder.respond_with_error(agent_client_protocol::schema::v1::Error::new(-32602, "Session not found"));
                    }
                    connection.send_notification(SessionNotification::new(
                        request.session_id.clone(),
                        SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                            TextContent::new("Restored session history."),
                        ))),
                    ))?;
                    responder.respond(LoadSessionResponse::new().config_options(config_options("mock-a", "build")).modes(mode_state()))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: SetSessionConfigOptionRequest, responder, _connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    let selected = request.value.as_value_id().map(ToString::to_string).unwrap_or_default();
                    if request.config_id.to_string() == "agent-mode" {
                        if !matches!(selected.as_str(), "build" | "plan") {
                            return responder.respond_with_error(agent_client_protocol::schema::v1::Error::new(-32602, "Unknown mock mode"));
                        }
                        return responder.respond(SetSessionConfigOptionResponse::new(config_options("mock-a", &selected)));
                    }
                    if request.config_id.to_string() != "model" || !matches!(selected.as_str(), "mock-a" | "mock-b") {
                        return responder.respond_with_error(agent_client_protocol::schema::v1::Error::new(-32602, "Unknown mock model"));
                    }
                    responder.respond(SetSessionConfigOptionResponse::new(config_options(&selected, "build")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: SetSessionModeRequest, responder, connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Client>| {
                    if !matches!(request.mode_id.to_string().as_str(), "build" | "plan") {
                        return responder.respond_with_error(agent_client_protocol::schema::v1::Error::new(-32602, "Unknown mock mode"));
                    }
                    connection.send_notification(SessionNotification::new(request.session_id,
                        SessionUpdate::CurrentModeUpdate(CurrentModeUpdate::new(request.mode_id))))?;
                    responder.respond(SetSessionModeResponse::new())
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
                    if std::env::var_os("DUNLIN_MOCK_RICH_CHAT").is_some() {
                        use serde_json::json;
                        let mut updates=vec![json!({"sessionUpdate":"agent_message_chunk","messageId":"commentary","content":{"type":"text","text":"Intermediate status: checking two cities."}})];
                        for (slug,city) in [("krakow","Kraków"),("warsaw","Warszawa")] {
                            let child=format!("child-{slug}");
                            updates.push(json!({"sessionUpdate":"tool_call","toolCallId":format!("delegate-{city}"),"title":"subagent","kind":"think","status":"completed","content":[{"type":"content","content":{"type":"text","text":format!("<subagent sessionID=\"{child}\" state=\"completed\">Child result</subagent>")}}]}));
                            updates.push(json!({"sessionUpdate":"agent_message_chunk","messageId":format!("msg-{city}"),"content":{"type":"text","text":format!("Child-only details for {city}")},"_meta":{"opencode/child-session":{"id":child,"parentID":session_id.to_string(),"depth":1,"title":format!("Weather {city}")}}}));
                        }
                        updates.push(json!({"sessionUpdate":"agent_message_chunk","messageId":"final","content":{"type":"text","text":"# Weather summary\n\n**Kraków:** clear.\n\n"}}));
                        updates.push(json!({"sessionUpdate":"agent_thought_chunk","messageId":"reasoning","content":{"type":"text","text":"Hidden reasoning between final text chunks."}}));
                        updates.push(json!({"sessionUpdate":"agent_message_chunk","messageId":"final","content":{"type":"text","text":"**Warszawa:** cloudy.\n\n| City | Temperature |\n| --- | --- |\n| Kraków | 12 C |\n| Warszawa | 10 C |\n\n- Two delegated sessions\n- No network calls\n\n[Example source](https://example.com)\n\n```rust\nlet cities = 2;\n```\n\nMARKDOWN END"}}));
                        for update in updates {
                            connection.send_notification(SessionNotification::new(session_id.clone(),serde_json::from_value::<SessionUpdate>(update).unwrap()))?;
                            async_io::Timer::after(std::time::Duration::from_millis(80)).await;
                        }
                        let mut response = PromptResponse::new(StopReason::EndTurn);
                        if std::env::var_os("DUNLIN_MOCK_TOKEN_USAGE").is_some() {
                            response = response.usage(agent_client_protocol::schema::v1::Usage::new(1200, 1000, 200));
                        }
                        return responder.respond(response);
                    }
                    if let Ok(value) = std::env::var("DUNLIN_MOCK_DELAY_MS")
                        && let Ok(ms) = value.parse::<u64>()
                    {
                        async_io::Timer::after(std::time::Duration::from_millis(ms)).await;
                    }
                    // No permissions, tools, network or file writes: a long,
                    // growing reply for native tail-following regression checks.
                    if let Ok(value) = std::env::var("DUNLIN_MOCK_STREAM_CHUNKS")
                        && let Ok(chunks) = value.parse::<usize>()
                    {
                        for index in 0..chunks.min(500) {
                            connection.send_notification(SessionNotification::new(
                                session_id.clone(),
                                SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                                    TextContent::new(format!("Stream line {} — deterministic scroll fixture.\n\n", index + 1)),
                                ))),
                            ))?;
                            async_io::Timer::after(std::time::Duration::from_millis(60)).await;
                        }
                        connection.send_notification(SessionNotification::new(
                            session_id,
                            SessionUpdate::AgentMessageChunk(ContentChunk::new(ContentBlock::Text(
                                TextContent::new("SCROLL END — latest reply is visible."),
                            ))),
                        ))?;
                        let mut response = PromptResponse::new(StopReason::EndTurn);
                        if std::env::var_os("DUNLIN_MOCK_TOKEN_USAGE").is_some() {
                            response = response.usage(agent_client_protocol::schema::v1::Usage::new(1200, 1000, 200));
                        }
                        return responder.respond(response);
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
                                    Diff::new("/tmp/dunlin-smoke/hello.txt", "Hello from Dunlin\n")
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
