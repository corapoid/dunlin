//! One ACP turn. No filesystem, terminal, MCP or human-approval broker exists.
use super::*;
use serde_json::{Value, json};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{ChildStdin, ChildStdout},
};
const REPORT_LIMIT: usize = 16 * 1024;
struct Acp {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    partial: Vec<u8>,
    bytes: usize,
    frames: usize,
    session: String,
    text: String,
    prompt_sent: bool,
}
pub(super) struct ResultText {
    pub status: wire::ExecutionTerminalStatus,
    pub text: String,
}
async fn guarded<T>(
    runtime: &Runtime,
    active: &Active,
    future: impl std::future::Future<Output = Result<T, &'static str>>,
) -> Result<T, &'static str> {
    tokio::pin!(future);
    loop {
        if runtime.current(active).is_err() {
            return Err("execution_guard_stopped");
        }
        tokio::select! {biased;_=tokio::time::sleep(Duration::from_millis(50))=>{},result=&mut future=>return result}
    }
}
impl Acp {
    async fn send(
        &mut self,
        runtime: &Runtime,
        active: &Active,
        message: Value,
    ) -> Result<(), &'static str> {
        #[cfg(test)]
        let paid = message["method"] == "session/prompt";
        let mut bytes = serde_json::to_vec(&message).map_err(|_| "request_encoding_failed")?;
        if bytes.len() > 64 * 1024 {
            return Err("request_frame_limit");
        }
        bytes.push(b'\n');
        guarded(runtime, active, async {
            let mut sent = 0;
            while sent < bytes.len() {
                let count = std::future::poll_fn(|context| {
                    let result = tokio::io::AsyncWrite::poll_write(
                        std::pin::Pin::new(&mut self.input),
                        context,
                        &bytes[sent..],
                    );
                    #[cfg(test)]
                    if paid && sent > 0 && result.is_pending() {
                        super::tests::blocked_write_observed();
                    }
                    result
                })
                .await
                .map_err(|_| "agent_write_failed")?;
                if count == 0 {
                    return Err("agent_write_failed");
                }
                sent += count;
            }
            self.input.flush().await.map_err(|_| "agent_write_failed")
        })
        .await
    }
    async fn frame(&mut self, runtime: &Runtime, active: &Active) -> Result<Value, &'static str> {
        guarded(runtime, active, async {
            loop {
                let bytes = self
                    .output
                    .fill_buf()
                    .await
                    .map_err(|_| "agent_read_failed")?;
                if bytes.is_empty() {
                    return Err("agent_closed");
                }
                let newline = bytes.iter().position(|b| *b == b'\n');
                let n = newline.map(|i| i + 1).unwrap_or(bytes.len());
                self.bytes = self.bytes.checked_add(n).ok_or("output_limit")?;
                if self.bytes > active.command.budgets.max_output_bytes
                    || self.partial.len() + n > 64 * 1024
                {
                    return Err("output_limit");
                }
                self.partial.extend_from_slice(&bytes[..n]);
                self.output.consume(n);
                if newline.is_some() {
                    self.frames += 1;
                    if self.frames > 4096 {
                        return Err("agent_frame_limit");
                    }
                    return serde_json::from_slice(&std::mem::take(&mut self.partial))
                        .map_err(|_| "agent_invalid_json");
                }
            }
        })
        .await
    }
    async fn request(
        &mut self,
        runtime: &Runtime,
        active: &Active,
        id: u64,
        method: &str,
        params: Value,
    ) -> Result<Value, &'static str> {
        if method == "session/prompt" {
            self.prompt_sent = true;
        }
        self.send(
            runtime,
            active,
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )
        .await?;
        loop {
            let value = self.frame(runtime, active).await?;
            if value["jsonrpc"] != "2.0" {
                return Err("agent_invalid_rpc");
            }
            if let Some(method) = value["method"].as_str() {
                if let Some(request_id) = value.get("id") {
                    if method == "session/request_permission" {
                        self.send(runtime,active,json!({"jsonrpc":"2.0","id":request_id,"result":{"outcome":{"outcome":"cancelled"}}})).await?;
                        return Err("human_approval_not_supported");
                    }
                    self.send(runtime,active,json!({"jsonrpc":"2.0","id":request_id,"error":{"code":-32601,"message":"Worker broker capability is not supported"}})).await?;
                    return Err("agent_broker_not_supported");
                }
                if method != "session/update"
                    || !self.prompt_sent
                    || self.session.is_empty()
                    || value["params"]["sessionId"] != self.session
                {
                    return Err("unexpected_agent_notification");
                }
                let update = &value["params"]["update"];
                match update["sessionUpdate"].as_str() {
                    Some("agent_message_chunk") => {
                        if update["content"]["type"] != "text" {
                            return Err("agent_content_not_supported");
                        }
                        let text = update["content"]["text"]
                            .as_str()
                            .ok_or("agent_text_missing")?;
                        if self.text.len() + text.len() > REPORT_LIMIT
                            || text
                                .chars()
                                .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
                        {
                            return Err("report_limit");
                        }
                        self.text.push_str(text);
                        #[cfg(test)]
                        super::tests::fault_checkpoint("running", &active.group);
                    }
                    Some("agent_thought_chunk") => {}
                    _ => return Err("agent_update_not_supported"),
                }
                continue;
            }
            if value["id"] != id {
                return Err("agent_response_identity_mismatch");
            }
            if value.get("error").is_some() {
                return Err("agent_response_error");
            }
            return value.get("result").cloned().ok_or("agent_result_missing");
        }
    }
    async fn run(
        &mut self,
        runtime: &Runtime,
        active: &Active,
    ) -> Result<wire::ExecutionTerminalStatus, &'static str> {
        let initialized=self.request(runtime,active,1,"initialize",json!({"protocolVersion":1,"clientInfo":{"name":"Dunlin bounded worker","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}})).await?;
        if initialized["protocolVersion"] != 1 {
            return Err("unsupported_acp_version");
        }
        let root = active
            .lease
            .lock()
            .map_err(|_| "lease_unavailable")?
            .root()
            .to_str()
            .ok_or("invalid_checkout_root")?
            .to_string();
        let session = self
            .request(
                runtime,
                active,
                2,
                "session/new",
                json!({"cwd":root,"mcpServers":[]}),
            )
            .await?;
        self.session = session["sessionId"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control))
            .ok_or("agent_session_missing")?
            .into();
        let mut options = session["configOptions"].clone();
        for (offset, (category, wanted)) in [
            ("model", &active.command.definition.model),
            ("mode", &active.command.definition.mode),
        ]
        .into_iter()
        .enumerate()
        {
            let option = options
                .as_array()
                .and_then(|v| v.iter().find(|o| o["category"] == category))
                .ok_or("agent_profile_unverifiable")?;
            let config = option["id"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or("agent_config_id_missing")?
                .to_string();
            options = self
                .request(
                    runtime,
                    active,
                    3 + offset as u64,
                    "session/set_config_option",
                    json!({"sessionId":self.session,"configId":config,"value":wanted}),
                )
                .await?["configOptions"]
                .clone();
        }
        for (category, wanted) in [
            ("model", &active.command.definition.model),
            ("mode", &active.command.definition.mode),
        ] {
            if options
                .as_array()
                .and_then(|v| v.iter().find(|o| o["category"] == category))
                .is_none_or(|o| o["currentValue"] != *wanted)
            {
                return Err("agent_profile_changed");
            }
        }
        let instructions = active.command.definition.render_instructions();
        let prompt = if instructions.is_empty() {
            active.command.input.clone()
        } else {
            format!(
                "{instructions}\n\n## User request\n{}",
                active.command.input
            )
        };
        let response = self
            .request(
                runtime,
                active,
                10,
                "session/prompt",
                json!({"sessionId":self.session,"prompt":[{"type":"text","text":prompt}]}),
            )
            .await?;
        match response["stopReason"].as_str() {
            Some("end_turn") => Ok(wire::ExecutionTerminalStatus::Completed),
            Some("refusal" | "max_tokens") => Ok(wire::ExecutionTerminalStatus::Failed),
            Some("cancelled") => Ok(wire::ExecutionTerminalStatus::Cancelled),
            _ => Err("unknown_agent_stop_reason"),
        }
    }
}
pub(super) async fn conversation(
    runtime: &Runtime,
    active: &Active,
    child: &mut tokio::process::Child,
) -> ResultText {
    let (Some(input), Some(output)) = (child.stdin.take(), child.stdout.take()) else {
        return ResultText {
            status: wire::ExecutionTerminalStatus::Failed,
            text: "Agent pipes are unavailable".into(),
        };
    };
    #[cfg(test)]
    super::tests::configure_stdin_backpressure(&input);
    let mut acp = Acp {
        input,
        output: BufReader::new(output),
        partial: vec![],
        bytes: 0,
        frames: 0,
        session: String::new(),
        text: String::new(),
        prompt_sent: false,
    };
    match acp.run(runtime, active).await {
        Ok(status) => ResultText {
            status,
            text: if acp.text.is_empty() {
                "Agent returned no textual result".into()
            } else {
                acp.text
            },
        },
        Err(reason) => ResultText {
            status: if acp.prompt_sent {
                wire::ExecutionTerminalStatus::Unknown
            } else {
                wire::ExecutionTerminalStatus::Failed
            },
            text: format!(
                "Execution stopped: {reason}. A complete result is unavailable; effects may require review."
            ),
        },
    }
}
