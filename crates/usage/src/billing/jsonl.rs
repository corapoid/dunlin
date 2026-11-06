//! Project billing metadata while serde skips prompt, response and tool bodies.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{self, BufRead, Read};

#[derive(Default, Deserialize, Serialize)]
pub(super) struct Metadata {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    kind: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    time: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cwd: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    info: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<Box<Metadata>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payload: Option<Box<Metadata>>,
    #[serde(rename = "sessionId", skip_serializing_if = "Option::is_none")]
    session: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<Value>,
    #[serde(rename = "parentSessionId", skip_serializing_if = "Option::is_none")]
    parent_session: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_session_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_thread_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    forked_from_id: Option<Value>,
    #[serde(rename = "agentId", skip_serializing_if = "Option::is_none")]
    agent_id: Option<Value>,
    #[serde(rename = "agent_id", skip_serializing_if = "Option::is_none")]
    legacy_agent_id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    agent: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    subagent: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    originator: Option<Value>,
    #[serde(rename = "isSidechain", skip_serializing_if = "Option::is_none")]
    sidechain: Option<Value>,
    #[serde(rename = "isCompactSummary", skip_serializing_if = "Option::is_none")]
    compact_summary: Option<Value>,
    #[serde(rename = "isApiErrorMessage", skip_serializing_if = "Option::is_none")]
    api_error: Option<Value>,
}

pub(super) struct Line<'a, R> {
    reader: &'a mut R,
    pub bytes: u64,
    pub complete: bool,
}

impl<'a, R: BufRead> Line<'a, R> {
    pub fn new(reader: &'a mut R) -> Self {
        Self {
            reader,
            bytes: 0,
            complete: false,
        }
    }
}

impl<R: BufRead> Read for Line<'_, R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if self.complete || out.is_empty() {
            return Ok(0);
        }
        let data = self.reader.fill_buf()?;
        let available = data.len().min(out.len());
        let count = data[..available]
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or(available, |i| i + 1);
        out[..count].copy_from_slice(&data[..count]);
        self.complete = count > 0 && data[count - 1] == b'\n';
        self.bytes += count as u64;
        self.reader.consume(count);
        Ok(count)
    }
}
