//! Reusable persistent ACP engine. It owns pipes, never spawning or authority.
//! The caller owns the sandbox guardian and must obtain its actual StopProof.
//! ACP has no turn IDs: attribution requires an ordered producer that emits all
//! turn notifications before its matching prompt response. Observed idle updates
//! are rejected; a malicious old update after a new prompt is indistinguishable.
use dunlin_domain::agent::AgentDefinition;
use dunlin_relay_protocol::worker::{MAX_EXECUTION_INTEGER, session::TurnTerminalStatus};
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
    sync::{mpsc, oneshot, watch},
    time::Instant,
};
const FRAME_BYTES: usize = 64 * 1024;
const INPUT_BYTES: usize = 16 * 1024;
const CHUNK_BYTES: usize = 4 * 1024;
pub(crate) type Result<T> = std::result::Result<T, &'static str>;
pub(crate) type HookFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;
/// Callbacks must return promptly and yield; blocking SQL/FS belongs to bounded
/// caller-owned workers. Guard is a cheap synchronous snapshot, not a SQL query.
#[derive(Clone)]
pub(crate) struct Hooks {
    pub guard: Arc<dyn Fn() -> Result<Instant> + Send + Sync>,
    pub before_prompt: Arc<dyn Fn(PromptIntent) -> HookFuture + Send + Sync>,
    pub stream: Arc<dyn Fn(StreamChunk) -> HookFuture + Send + Sync>,
}
pub(crate) struct EngineConfig {
    pub cwd: PathBuf,
    pub definition: AgentDefinition,
    pub tools: Option<super::runtime::mcp::HarnessTools>,
    pub max_session_seconds: u64,
    pub max_output_bytes: usize,
    pub max_turns: u64,
    pub idle_timeout_ms: u64,
}
#[derive(Clone, Debug)]
pub(crate) struct TurnSpec {
    pub turn_id: String,
    pub text: String,
    pub max_seconds: u64,
    pub max_output_bytes: usize,
}
#[derive(Clone)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "Prompt audit metadata is verified by engine/containment acceptance; the live durable inbox addresses turn identities"
    )
)]
pub(crate) struct PromptIntent {
    pub turn: TurnSpec,
    pub rpc_id: u64,
    pub first_turn: bool,
    /// Exact rendered ACP prompt, including immutable instructions on first turn.
    pub prompt: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamKind {
    Text,
    Reasoning,
}
#[derive(Clone, Debug)]
pub(crate) struct StreamChunk {
    pub turn_id: String,
    pub kind: StreamKind,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadyInfo {
    pub remote_session_id: String,
    pub model: String,
    pub mode: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TurnResult {
    pub turn_id: String,
    pub status: TurnTerminalStatus,
}
struct Queued {
    turn: TurnSpec,
    reply: oneshot::Sender<Result<TurnResult>>,
}
fn mark_closed(signal: &watch::Sender<Option<&'static str>>, reason: &'static str) {
    signal.send_if_modified(|state| {
        if state.is_some() {
            false
        } else {
            *state = Some(reason);
            true
        }
    });
}
/// One owning handle. Dropping a turn waiter does not cancel accepted work;
/// dropping this handle closes pipes and reports closed, not process stopped.
pub(crate) struct Engine {
    ready: ReadyInfo,
    sender: mpsc::Sender<Queued>,
    task: tokio::task::AbortHandle,
    closed: watch::Receiver<Option<&'static str>>,
    close_signal: watch::Sender<Option<&'static str>>,
}
impl Engine {
    pub(crate) async fn start(
        input: ChildStdin,
        output: ChildStdout,
        config: EngineConfig,
        hooks: Hooks,
    ) -> Result<Self> {
        config.validate()?;
        let lifetime = Instant::now() + Duration::from_secs(config.max_session_seconds);
        let (sender, receiver) = mpsc::channel(1);
        let (close_signal, closed) = watch::channel(None);
        let original_guard = hooks.guard.clone();
        let failure = closed.clone();
        let hooks = Hooks {
            guard: Arc::new(move || {
                if let Some(error) = *failure.borrow() {
                    return Err(error);
                }
                original_guard()
            }),
            ..hooks
        };
        let phase = Arc::new(Mutex::new(ReadPhase::Idle));
        let bytes = Arc::new(AtomicUsize::new(0));
        let (frames, incoming) = mpsc::channel(8);
        let reader_hooks = hooks.clone();
        let reader_phase = phase.clone();
        let reader_bytes = bytes.clone();
        let reader_signal = close_signal.clone();
        let max_bytes = config.max_output_bytes;
        let reader = tokio::spawn(async move {
            let mut reader = Reader {
                output: BufReader::new(output),
                partial: Vec::new(),
                observed: None,
                bytes: 0,
                frames: 0,
                max_bytes,
                phase: reader_phase,
                total: reader_bytes,
            };
            let result: Result<()> = async {
                loop {
                    let frame = guarded(&reader_hooks, lifetime, None, reader.frame()).await?;
                    guarded(&reader_hooks, lifetime, None, async {
                        frames.send(frame).await.map_err(|_| "engine_reader_closed")
                    })
                    .await?;
                }
            }
            .await;
            mark_closed(
                &reader_signal,
                result.err().unwrap_or("engine_reader_closed"),
            );
        });
        let (started, ready) = oneshot::channel();
        let signal = close_signal.clone();
        let task = tokio::spawn(async move {
            let mut actor = Actor {
                io: Pipes {
                    input,
                    incoming,
                    phase,
                    bytes,
                    _reader: ReaderCleanup(reader.abort_handle()),
                },
                config,
                hooks,
                lifetime,
                next_id: 1,
                session: String::new(),
                turns: 0,
                seen: BTreeSet::new(),
            };
            let result = actor.initialize().await;
            let initialized = result.is_ok();
            let _ = started.send(result);
            let reason = if initialized {
                actor.run(receiver).await.err().unwrap_or("engine_closed")
            } else {
                "engine_initialization_failed"
            };
            mark_closed(&signal, reason);
        });
        let mut engine = Self {
            ready: ReadyInfo {
                remote_session_id: String::new(),
                model: String::new(),
                mode: String::new(),
            },
            sender,
            task: task.abort_handle(),
            closed,
            close_signal,
        };
        engine.ready = ready.await.map_err(|_| "engine_initialization_failed")??;
        Ok(engine)
    }
    pub(crate) fn ready(&self) -> &ReadyInfo {
        &self.ready
    }
    pub(crate) async fn turn(&self, turn: TurnSpec) -> Result<TurnResult> {
        if self.closed.borrow().is_some() {
            return Err("engine_closed");
        }
        turn.validate()?;
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Queued { turn, reply })
            .map_err(|_| "engine_queue_unavailable")?;
        result.await.map_err(|_| "engine_closed")?
    }
    pub(crate) async fn closed(&self) -> &'static str {
        let mut closed = self.closed.clone();
        loop {
            if let Some(reason) = *closed.borrow() {
                return reason;
            }
            if closed.changed().await.is_err() {
                return "engine_closed";
            }
        }
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        mark_closed(&self.close_signal, "engine_handle_dropped");
        self.task.abort();
    }
}
impl EngineConfig {
    fn validate(&self) -> Result<()> {
        self.definition.validate().map_err(|_| "invalid_profile")?;
        match (&self.tools, self.definition.mcp.is_empty()) {
            (None, true) => {}
            (Some(tools), false) => tools.validate(&self.definition)?,
            _ => return Err("agent_mcp_runtime_unavailable"),
        }
        if !self.cwd.is_absolute()
            || self.cwd.to_str().is_none()
            || !(1..=self.definition.max_seconds.min(3600)).contains(&self.max_session_seconds)
            || !(1024..=self.definition.max_output_bytes).contains(&self.max_output_bytes)
            || !(1..=128).contains(&self.max_turns)
            || !(1..=30000).contains(&self.idle_timeout_ms)
            || self.idle_timeout_ms > self.max_session_seconds * 1000
        {
            return Err("invalid_session_budget");
        }
        Ok(())
    }
}
impl TurnSpec {
    fn validate(&self) -> Result<()> {
        if self.turn_id.is_empty()
            || self.turn_id.len() > 128
            || !self
                .turn_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            || self.text.trim().is_empty()
            || self.text.len() > INPUT_BYTES
            || self
                .text
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
            || !(1..=1800).contains(&self.max_seconds)
            || !(1024..=4 * 1024 * 1024).contains(&self.max_output_bytes)
        {
            return Err("invalid_turn");
        }
        Ok(())
    }
}
async fn guarded<T>(
    hooks: &Hooks,
    hard: Instant,
    limit: Option<Duration>,
    future: impl Future<Output = Result<T>>,
) -> Result<T> {
    let hard = limit.map_or(hard, |duration| hard.min(Instant::now() + duration));
    tokio::pin!(future);
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    loop {
        let deadline = (hooks.guard)()?.min(hard);
        if Instant::now() >= deadline {
            return Err("engine_deadline_expired");
        }
        tokio::select! { biased;
            _=tokio::time::sleep_until(deadline)=>return Err("engine_deadline_expired"),
            _=tick.tick()=>{},
            result=&mut future=>return result,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
enum ReadPhase {
    Idle,
    Request { id: u64, turn: Option<String> },
}
struct TaggedFrame {
    value: Value,
    phase: ReadPhase,
    bytes: usize,
}
struct ReaderCleanup(tokio::task::AbortHandle);
impl Drop for ReaderCleanup {
    fn drop(&mut self) {
        self.0.abort();
    }
}
struct Reader {
    output: BufReader<ChildStdout>,
    partial: Vec<u8>,
    observed: Option<ReadPhase>,
    bytes: usize,
    frames: usize,
    max_bytes: usize,
    phase: Arc<Mutex<ReadPhase>>,
    total: Arc<AtomicUsize>,
}
impl Reader {
    async fn frame(&mut self) -> Result<TaggedFrame> {
        loop {
            let buffer = self
                .output
                .fill_buf()
                .await
                .map_err(|_| "agent_read_failed")?;
            if buffer.is_empty() {
                return Err("agent_closed");
            }
            if self.observed.is_none() {
                self.observed = Some(
                    self.phase
                        .lock()
                        .map_err(|_| "engine_state_failed")?
                        .clone(),
                );
            }
            let newline = buffer.iter().position(|b| *b == b'\n');
            let n = newline.map_or(buffer.len(), |i| i + 1);
            if self.partial.len() + n > FRAME_BYTES {
                return Err("agent_frame_limit");
            }
            self.bytes = self.bytes.checked_add(n).ok_or("agent_output_limit")?;
            if self.bytes > self.max_bytes {
                return Err("agent_output_limit");
            }
            self.total.store(self.bytes, Ordering::Release);
            self.partial.extend_from_slice(&buffer[..n]);
            self.output.consume(n);
            if newline.is_some() {
                self.frames += 1;
                if self.frames > 16384 {
                    return Err("agent_frame_count_limit");
                }
                let value: Value = serde_json::from_slice(&std::mem::take(&mut self.partial))
                    .map_err(|_| "agent_invalid_json")?;
                if value["jsonrpc"] != "2.0" || !value.is_object() {
                    return Err("agent_invalid_rpc");
                }
                let observed = self.observed.take().ok_or("engine_state_failed")?;
                if value.get("method").is_none() {
                    let ReadPhase::Request { id, .. } = &observed else {
                        return Err("unexpected_idle_response");
                    };
                    if value.get("id") != Some(&json!(id)) {
                        return Err("agent_response_mismatch");
                    }
                    let mut phase = self.phase.lock().map_err(|_| "engine_state_failed")?;
                    if *phase != observed {
                        return Err("agent_response_scope_mismatch");
                    }
                    *phase = ReadPhase::Idle;
                } else if value.get("id").is_none()
                    && !matches!(observed, ReadPhase::Request { turn: Some(_), .. })
                {
                    return Err("unexpected_idle_update");
                }
                // Bytes already fetched with this frame retain the phase at
                // this protocol boundary even if the actor starts a new request
                // while this bounded enqueue waits for capacity.
                if !self.output.buffer().is_empty() {
                    self.observed = Some(
                        self.phase
                            .lock()
                            .map_err(|_| "engine_state_failed")?
                            .clone(),
                    );
                }
                return Ok(TaggedFrame {
                    value,
                    phase: observed,
                    bytes: self.bytes,
                });
            }
        }
    }
}
struct Pipes {
    input: ChildStdin,
    incoming: mpsc::Receiver<TaggedFrame>,
    phase: Arc<Mutex<ReadPhase>>,
    bytes: Arc<AtomicUsize>,
    _reader: ReaderCleanup,
}
impl Pipes {
    async fn frame(&mut self) -> Result<TaggedFrame> {
        self.incoming.recv().await.ok_or("engine_reader_closed")
    }
    fn begin(&self, id: u64, turn: Option<&TurnSpec>) -> Result<ReadPhase> {
        let mut phase = self.phase.lock().map_err(|_| "engine_state_failed")?;
        if *phase != ReadPhase::Idle {
            return Err("engine_request_overlap");
        }
        let value = ReadPhase::Request {
            id,
            turn: turn.map(|turn| turn.turn_id.clone()),
        };
        *phase = value.clone();
        Ok(value)
    }
    async fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.input
            .write_all(bytes)
            .await
            .map_err(|_| "agent_write_failed")?;
        self.input.flush().await.map_err(|_| "agent_write_failed")
    }
}
fn encode(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| "request_encoding_failed")?;
    if bytes.len() + 1 > FRAME_BYTES {
        return Err("request_frame_limit");
    }
    bytes.push(b'\n');
    Ok(bytes)
}
struct Actor {
    io: Pipes,
    config: EngineConfig,
    hooks: Hooks,
    lifetime: Instant,
    next_id: u64,
    session: String,
    turns: u64,
    seen: BTreeSet<String>,
}
impl Actor {
    fn id(&mut self) -> Result<u64> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .filter(|v| *v <= MAX_EXECUTION_INTEGER)
            .ok_or("rpc_id_limit")?;
        Ok(id)
    }
    async fn send(&mut self, value: Value, hard: Instant) -> Result<()> {
        let bytes = encode(&value)?;
        guarded(
            &self.hooks,
            hard,
            Some(Duration::from_secs(2)),
            self.io.write(&bytes),
        )
        .await
    }
    async fn deny_request(&mut self, value: &Value, hard: Instant) -> Result<()> {
        let id = value
            .get("id")
            .filter(|id| {
                id.as_u64().is_some_and(|n| n <= MAX_EXECUTION_INTEGER)
                    || id.as_str().is_some_and(|s| {
                        !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control)
                    })
            })
            .ok_or("unsupported_agent_rpc_id")?;
        let response = if value["method"] == "session/request_permission" {
            json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"cancelled"}}})
        } else {
            json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Worker broker capability is not supported"}})
        };
        self.send(response, hard).await?;
        Err("agent_broker_not_supported")
    }
    async fn request(
        &mut self,
        id: u64,
        method: &str,
        params: Value,
        hard: Instant,
        turn: Option<&TurnSpec>,
    ) -> Result<Value> {
        let expected = self.io.begin(id, turn)?;
        let start_bytes = self.io.bytes.load(Ordering::Acquire);
        self.send(
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            hard,
        )
        .await?;
        loop {
            let frame = guarded(&self.hooks, hard, None, self.io.frame()).await?;
            if frame.phase != expected {
                return Err("agent_frame_scope_mismatch");
            }
            let value = frame.value;
            if let Some(turn) = turn
                && frame.bytes.saturating_sub(start_bytes) > turn.max_output_bytes
            {
                return Err("turn_output_limit");
            }
            if value.get("method").is_some() {
                if value.get("id").is_some() {
                    return self.deny_request(&value, hard).await.map(|_| Value::Null);
                }
                let turn = turn.ok_or("unexpected_idle_update")?;
                self.update(&value, turn, hard).await?;
                continue;
            }
            if value.get("id") != Some(&json!(id))
                || value.get("error").is_some()
                || value.get("result").is_none()
            {
                return Err("agent_response_mismatch");
            }
            return Ok(value["result"].clone());
        }
    }
    async fn update(&mut self, value: &Value, turn: &TurnSpec, hard: Instant) -> Result<()> {
        if value["method"] != "session/update" || value["params"]["sessionId"] != self.session {
            return Err("agent_update_scope_mismatch");
        }
        let update = &value["params"]["update"];
        let kind = match update["sessionUpdate"].as_str() {
            Some("agent_message_chunk") => StreamKind::Text,
            Some("agent_thought_chunk") => StreamKind::Reasoning,
            _ => return Err("agent_update_not_supported"),
        };
        if update["content"]["type"] != "text" {
            return Err("agent_content_not_supported");
        }
        let mut text = update["content"]["text"]
            .as_str()
            .ok_or("agent_text_missing")?;
        if text
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err("agent_text_invalid");
        }
        while !text.is_empty() {
            let mut end = text.len().min(CHUNK_BYTES);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let chunk = StreamChunk {
                turn_id: turn.turn_id.clone(),
                kind,
                text: text[..end].into(),
            };
            guarded(
                &self.hooks,
                hard,
                Some(Duration::from_secs(2)),
                (self.hooks.stream)(chunk),
            )
            .await?;
            text = &text[end..];
        }
        Ok(())
    }
    async fn initialize(&mut self) -> Result<ReadyInfo> {
        let hard = self.lifetime.min(Instant::now() + Duration::from_secs(15));
        let id = self.id()?;
        let initialized=self.request(id,"initialize",json!({"protocolVersion":1,"clientInfo":{"name":"Dunlin persistent worker","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}),hard,None).await?;
        if initialized["protocolVersion"] != 1 {
            return Err("unsupported_acp_version");
        }
        let id = self.id()?;
        let result = self
            .request(
                id,
                "session/new",
                json!({"cwd":self.config.cwd.to_str().ok_or("invalid_cwd")?,"mcpServers":self.config.tools.as_ref().map_or(&[][..],|tools|tools.configs())}),
                hard,
                None,
            )
            .await?;
        self.session = result["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .ok_or("agent_session_missing")?
            .into();
        let mut options = result["configOptions"].clone();
        for (category, wanted) in [
            ("model", self.config.definition.model.clone()),
            ("mode", self.config.definition.mode.clone()),
        ] {
            let config = options
                .as_array()
                .and_then(|items| items.iter().find(|o| o["category"] == category))
                .and_then(|o| o["id"].as_str())
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or("agent_profile_unverifiable")?
                .to_string();
            let id = self.id()?;
            options = self
                .request(
                    id,
                    "session/set_config_option",
                    json!({"sessionId":self.session,"configId":config,"value":wanted}),
                    hard,
                    None,
                )
                .await?["configOptions"]
                .clone();
        }
        for (category, wanted) in [
            ("model", &self.config.definition.model),
            ("mode", &self.config.definition.mode),
        ] {
            if options
                .as_array()
                .and_then(|items| items.iter().find(|o| o["category"] == category))
                .is_none_or(|o| o["currentValue"] != *wanted)
            {
                return Err("agent_profile_changed");
            }
        }
        Ok(ReadyInfo {
            remote_session_id: self.session.clone(),
            model: self.config.definition.model.clone(),
            mode: self.config.definition.mode.clone(),
        })
    }
    async fn run(&mut self, mut receiver: mpsc::Receiver<Queued>) -> Result<()> {
        loop {
            let hard = self
                .lifetime
                .min(Instant::now() + Duration::from_millis(self.config.idle_timeout_ms));
            enum Incoming {
                Frame(Result<TaggedFrame>),
                Turn(Option<Queued>),
            }
            let incoming=guarded(&self.hooks,hard,None,async{
                tokio::select!{biased;frame=self.io.frame()=>Ok(Incoming::Frame(frame)),turn=receiver.recv()=>Ok(Incoming::Turn(turn))}
            }).await?;
            match incoming {
                Incoming::Frame(frame) => {
                    let value = frame?.value;
                    if value.get("method").is_some() && value.get("id").is_some() {
                        self.deny_request(&value, hard).await?;
                    }
                    return Err("unexpected_idle_update");
                }
                Incoming::Turn(None) => return Ok(()),
                Incoming::Turn(Some(queued)) => {
                    let result = self.turn(queued.turn).await;
                    let failed = result.is_err();
                    let _ = queued.reply.send(result);
                    if failed {
                        return Err("turn_failed_engine_closed");
                    }
                }
            }
        }
    }
    async fn turn(&mut self, turn: TurnSpec) -> Result<TurnResult> {
        turn.validate()?;
        // Durable dedupe belongs to the caller ledger. The engine never writes
        // a duplicate turn identity again, even if that caller misroutes it.
        if !self.seen.insert(turn.turn_id.clone()) {
            return Err("duplicate_turn_identity");
        }
        if self.turns >= self.config.max_turns
            || turn.max_seconds > self.config.max_session_seconds
            || turn.max_seconds > self.config.definition.max_seconds
            || turn.max_output_bytes > self.config.definition.max_output_bytes
            || turn.max_output_bytes > self.config.max_output_bytes
        {
            return Err("turn_budget_exceeded");
        }
        // Capture before awaited durable intent; SQL latency never resets budget.
        let hard = self
            .lifetime
            .min(Instant::now() + Duration::from_secs(turn.max_seconds));
        let first = self.turns == 0;
        self.turns += 1;
        let instructions = self.config.definition.render_instructions();
        let prompt = if first && !instructions.is_empty() {
            format!("{instructions}\n\n## User request\n{}", turn.text)
        } else {
            turn.text.clone()
        };
        let id = self.id()?;
        // Encode/check the exact bytes before persisting an intent that cannot fit.
        encode(
            &json!({"jsonrpc":"2.0","id":id,"method":"session/prompt","params":{"sessionId":self.session,"prompt":[{"type":"text","text":prompt}]}}),
        )?;
        guarded(
            &self.hooks,
            hard,
            Some(Duration::from_secs(5)),
            (self.hooks.before_prompt)(PromptIntent {
                turn: turn.clone(),
                rpc_id: id,
                first_turn: first,
                prompt: prompt.clone(),
            }),
        )
        .await?;
        let result = self
            .request(
                id,
                "session/prompt",
                json!({"sessionId":self.session,"prompt":[{"type":"text","text":prompt}]}),
                hard,
                Some(&turn),
            )
            .await?;
        let status = match result["stopReason"].as_str() {
            Some("end_turn") => TurnTerminalStatus::Completed,
            Some("refusal" | "max_tokens") => TurnTerminalStatus::Failed,
            Some("cancelled") => TurnTerminalStatus::Cancelled,
            _ => return Err("unknown_stop_reason"),
        };
        Ok(TurnResult {
            turn_id: turn.turn_id,
            status,
        })
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests;
