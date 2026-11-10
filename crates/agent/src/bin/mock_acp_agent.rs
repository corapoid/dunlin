//! A deterministic ACP agent used to exercise permission handling.
//!
//! It speaks just enough ACP v1 to be useful: initialize, open a session, and
//! answer a prompt by asking the client for permission before replying. The
//! client's choice decides whether the mock reports success or refusal.
//!
//! Run it over stdio; Dunlin spawns it like any other ACP harness:
//! `mock_acp_agent`.

use agent_client_protocol::v1::schema::{
    AgentCapabilities, CancelNotification, ContentBlock, ContentChunk, CurrentModeUpdate, Diff,
    InitializeRequest, InitializeResponse, LoadSessionRequest, LoadSessionResponse, McpServer,
    NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind, Plan, PlanEntry,
    PlanEntryPriority, PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOption, SessionMode, SessionModeState, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionConfigOptionResponse, SetSessionModeRequest,
    SetSessionModeResponse, StopReason, TextContent, ToolCall, ToolCallContent, ToolCallStatus,
    ToolCallUpdate, ToolCallUpdateFields, UsageUpdate,
};
use agent_client_protocol::{Agent, Result, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SESSION_COUNTER: AtomicU64 = AtomicU64::new(1);

static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// Opt-in GUI instrumentation; never write into an ordinary project directory.
fn gui_root() -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(std::env::var_os("DUNLIN_MOCK_GUI_ROOT")?);
    assert!(
        path.is_absolute()
            && path.file_name().is_some_and(|name| name == "project")
            && path
                .parent()
                .and_then(|p| p.file_name())
                .is_some_and(|name| name.to_string_lossy().starts_with("dunlin-gui-smoke-"))
    );
    Some(path)
}
fn gui_event(value: serde_json::Value) {
    use std::io::Write;
    if let Some(root) = gui_root() {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("requests.jsonl"))
            .unwrap();
        writeln!(file, "{value}").unwrap();
    }
}

fn assert_vault_mcp(servers: &[McpServer]) {
    if std::env::var_os("DUNLIN_MOCK_REQUIRE_MCP").is_none() {
        return;
    }
    let found = servers.iter().any(|server| match server {
        McpServer::Stdio(server) => {
            server.name == "dunlin-secrets"
                && server.args.iter().any(|arg| arg == "--mcp-vault")
                && server
                    .env
                    .iter()
                    .any(|env| env.name == "DUNLIN_VAULT_TOKEN")
        }
        _ => false,
    });
    assert!(
        found,
        "vault MCP server was not passed to session/new or session/load"
    );
}

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
    if std::env::var_os("DUNLIN_MOCK_CONFIG_VARIANTS").is_some() {
        options.push(SessionConfigOption::select(
            "reasoning-effort",
            "Reasoning effort",
            "medium",
            vec![
                SessionConfigSelectOption::new("low", "Low"),
                SessionConfigSelectOption::new("medium", "Medium"),
                SessionConfigSelectOption::new("high", "High"),
            ],
        ));
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
            .v1()
            .name("dunlin-mock-agent")
            .on_receive_request(
                async |request: InitializeRequest,
                       responder,
                       _connection: agent_client_protocol::v1::ConnectionTo<
                    agent_client_protocol::Client,
                 >| {
                    if std::env::var_os("DUNLIN_MOCK_MISSING_AUTH").is_some() {
                        eprintln!("mock harness: API key is missing or not configured");
                        return Err(agent_client_protocol::Error::new(-32000, "API key is missing or not configured"));
                    }
                    if std::env::var_os("DUNLIN_MOCK_HANG_INITIALIZE").is_some() {
                        std::future::pending::<()>().await;
                    }
                    responder.respond(
                        InitializeResponse::new(request.protocol_version)
                            .agent_capabilities(AgentCapabilities::new().load_session(std::env::var_os("DUNLIN_MOCK_NO_LOAD").is_none()))
                            .agent_info(
                                agent_client_protocol::v1::schema::Implementation::new(
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
                async |request: NewSessionRequest,
                       responder,
                       _connection: agent_client_protocol::v1::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    assert_vault_mcp(&request.mcp_servers);
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
                       connection: agent_client_protocol::v1::ConnectionTo<
                    agent_client_protocol::Client,
                >| {
                    assert_vault_mcp(&request.mcp_servers);
                    if request.session_id.to_string() == "missing-session" {
                        return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Session not found"));
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
                async |request: SetSessionConfigOptionRequest, responder, _connection: agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Client>| {
                    let selected = request.value.as_value_id().map(ToString::to_string).unwrap_or_default();
                    if request.config_id.to_string() == "reasoning-effort" {
                        if !matches!(selected.as_str(), "low" | "medium" | "high") {
                            return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Unknown mock effort"));
                        }
                        let mut options = serde_json::to_value(config_options("mock-a", "build")).unwrap();
                        for option in options.as_array_mut().unwrap() {
                            if option["id"] == "reasoning-effort" { option["currentValue"] = serde_json::Value::String(selected.clone()); }
                        }
                        let options: Vec<SessionConfigOption> = serde_json::from_value(options).unwrap();
                        return responder.respond(SetSessionConfigOptionResponse::new(options));
                    }
                    if request.config_id.to_string() == "agent-mode" {
                        if !matches!(selected.as_str(), "build" | "plan") {
                            return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Unknown mock mode"));
                        }
                        return responder.respond(SetSessionConfigOptionResponse::new(config_options("mock-a", &selected)));
                    }
                     if request.config_id.to_string() != "model" || !matches!(selected.as_str(), "mock-a" | "mock-b") {
                         return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Unknown mock model"));
                     }
                     if std::env::var_os("DUNLIN_MOCK_REJECT_MODEL").is_some() {
                         return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Mock model selection denied"));
                     }
                    responder.respond(SetSessionConfigOptionResponse::new(config_options(&selected, "build")))
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_request(
                async |request: SetSessionModeRequest, responder, connection: agent_client_protocol::v1::ConnectionTo<agent_client_protocol::Client>| {
                    if !matches!(request.mode_id.to_string().as_str(), "build" | "plan") {
                        return responder.respond_with_error(agent_client_protocol::Error::new(-32602, "Unknown mock mode"));
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
                       connection: agent_client_protocol::v1::ConnectionTo<
                    agent_client_protocol::Client,
                 >| {
                    // Long turns must not hold the ordered message dispatcher:
                    // it must still receive session/cancel while the gate waits.
                    let task_connection = connection.clone();
                    connection.spawn(async move {
                    let connection = task_connection;
                    CANCELLED.store(false, Ordering::SeqCst);
                    gui_event(serde_json::json!({"event":"prompt","request":request}));
                    if let Some(root) = gui_root() {
                        if let Ok(message) = std::fs::read_to_string(root.join("fail-next-prompt")) {
                            std::fs::remove_file(root.join("fail-next-prompt")).unwrap();
                            return responder.respond_with_error(agent_client_protocol::Error::new(-32603, message));
                        }
                        let started = std::time::Instant::now();
                        while !root.join("release").exists() && !CANCELLED.load(Ordering::SeqCst) {
                            assert!(started.elapsed() < std::time::Duration::from_secs(20), "GUI fixture did not release the mock");
                            async_io::Timer::after(std::time::Duration::from_millis(10)).await;
                        }
                        if CANCELLED.load(Ordering::SeqCst) {
                            gui_event(serde_json::json!({"event":"finished","reason":"cancelled"}));
                            return responder.respond(PromptResponse::new(StopReason::Cancelled));
                        }
                    }
                    if let Ok(expected)=std::env::var("DUNLIN_MOCK_EXPECT_CONTINUATION"){
                        assert_eq!(request.meta.as_ref().and_then(|m|m.get("dunlin/continuation")).and_then(serde_json::Value::as_str),Some(expected.as_str()));
                    }
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
                            response = response.usage(agent_client_protocol::v1::schema::Usage::new(1200, 1000, 200));
                        }
                        gui_event(serde_json::json!({"event":"finished","reason":"end_turn"}));
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
                            response = response.usage(agent_client_protocol::v1::schema::Usage::new(1200, 1000, 200));
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
                                .kind(agent_client_protocol::v1::schema::ToolKind::Edit)
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
                            .kind(agent_client_protocol::v1::schema::ToolKind::Edit)
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
                            gui_event(serde_json::json!({"event":"permission","outcome":outcome}));
                            if let Some(root) = gui_root()
                                && matches!(&outcome, RequestPermissionOutcome::Selected(selected) if matches!(selected.option_id.0.as_ref(), "allow-once" | "allow-always"))
                            {
                                std::fs::write(root.join("hello.txt"), "GUI permission granted\n").unwrap();
                            }
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
                    })
                },
                agent_client_protocol::on_receive_request!(),
            )
            .on_receive_notification(async |_: CancelNotification, _| {
                CANCELLED.store(true, Ordering::SeqCst);
                gui_event(serde_json::json!({"event":"cancel"}));
                Ok(())
            }, agent_client_protocol::on_receive_notification!())
            .connect_to(Stdio::new()),
    )
}
