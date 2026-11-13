//! Scoped subscriptions/listen over POST SSE. Reading notifications changes only
//! the owning client's observations; it never dispatches work or grants access.
use super::{Error, MAX_BYTES, Modern, complete, http_error, metadata};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::{BufRead, BufReader, Read},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
};

const MAX_STREAMS: usize = 4;
const MAX_FRAME: usize = 64 * 1024;
const MAX_MESSAGES: usize = 1024;
const SUB_ID: &str = "io.modelcontextprotocol/subscriptionId";

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscriptionFilter {
    #[serde(default)]
    pub tools_list_changed: bool,
    #[serde(default)]
    pub prompts_list_changed: bool,
    #[serde(default)]
    pub resources_list_changed: bool,
    #[serde(default)]
    pub resource_subscriptions: Vec<String>,
}
impl SubscriptionFilter {
    fn validate(&self) -> Result<(), Error> {
        if self.resource_subscriptions.len() > 64 {
            return Err(Error::Limit);
        }
        let mut seen = HashSet::new();
        for uri in &self.resource_subscriptions {
            if uri.len() > 4096 || url::Url::parse(uri).is_err() || !seen.insert(uri) {
                return Err(Error::Protocol);
            }
        }
        Ok(())
    }
    fn subset_of(&self, other: &Self) -> bool {
        (!self.tools_list_changed || other.tools_list_changed)
            && (!self.prompts_list_changed || other.prompts_list_changed)
            && (!self.resources_list_changed || other.resources_list_changed)
            && self
                .resource_subscriptions
                .iter()
                .all(|uri| other.resource_subscriptions.contains(uri))
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Tools,
    Prompts,
    Resources,
    Resource { uri: String },
}

#[derive(Default)]
pub(super) struct Signals {
    tools: AtomicU64,
    prompts: AtomicU64,
    resources: AtomicU64,
    cancelled: AtomicBool,
    streams: AtomicUsize,
}
impl Signals {
    pub(super) fn cache_epoch(&self, method: &str) -> [u64; 3] {
        match method {
            "tools/list" => [self.tools.load(Ordering::Acquire), 0, 0],
            "prompts/list" => [0, self.prompts.load(Ordering::Acquire), 0],
            "resources/list" | "resources/templates/list" | "resources/read" => {
                [0, 0, self.resources.load(Ordering::Acquire)]
            }
            "server/discover" => [
                self.tools.load(Ordering::Acquire),
                self.prompts.load(Ordering::Acquire),
                self.resources.load(Ordering::Acquire),
            ],
            _ => [0; 3],
        }
    }
    pub(super) fn tool_epoch(&self) -> u64 {
        self.tools.load(Ordering::Acquire)
    }
    pub(super) fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub(super) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    fn bump(&self, counter: &AtomicU64) {
        let mut current = counter.load(Ordering::Acquire);
        loop {
            let Some(next) = current.checked_add(1) else {
                self.cancel();
                return;
            };
            match counter.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return,
                Err(observed) => current = observed,
            }
        }
    }
    fn invalidate(&self, filter: &SubscriptionFilter) {
        if filter.tools_list_changed {
            self.bump(&self.tools);
        }
        if filter.prompts_list_changed {
            self.bump(&self.prompts);
        }
        if filter.resources_list_changed || !filter.resource_subscriptions.is_empty() {
            self.bump(&self.resources);
        }
    }
}

struct Permit(Arc<Signals>);
impl Permit {
    fn acquire(signals: &Arc<Signals>) -> Result<Self, Error> {
        let mut current = signals.streams.load(Ordering::Acquire);
        loop {
            if current >= MAX_STREAMS {
                return Err(Error::Limit);
            }
            match signals.streams.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self(signals.clone())),
                Err(observed) => current = observed,
            }
        }
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.streams.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Owns the HTTP body. Drop closes it; no GET, Last-Event-ID, cancellation POST
/// or automatic reconnect is used. Reads obey the originating HTTP deadline.
pub struct Subscription {
    guard: Option<crate::DispatchGuard>,
    reader: Option<Sse>,
    permit: Option<Permit>,
    signals: Arc<Signals>,
    filter: SubscriptionFilter,
    id: u64,
    closed: bool,
}
impl Subscription {
    pub fn acknowledged_filter(&self) -> &SubscriptionFilter {
        &self.filter
    }

    pub fn next_event(&mut self) -> Result<Option<Change>, Error> {
        if let Err(error) = crate::authorize(self.guard.as_ref()) {
            self.end();
            return Err(error);
        }
        if self.closed {
            return Ok(None);
        }
        if self.signals.cancelled() {
            self.end();
            return Err(Error::Protocol);
        }
        let result = self.read_event();
        if let Err(error) = crate::authorize(self.guard.as_ref()) {
            self.end();
            return Err(error);
        }
        if self.signals.cancelled() {
            self.end();
            return Err(Error::Protocol);
        }
        if !matches!(result, Ok(Some(_))) {
            self.end();
        }
        result
    }

    fn read_event(&mut self) -> Result<Option<Change>, Error> {
        let message = self
            .reader
            .as_mut()
            .ok_or(Error::Protocol)?
            .message()?
            .ok_or(Error::Transport)?;
        if message["jsonrpc"] != "2.0" {
            return Err(Error::Protocol);
        }
        if message.get("id").is_some() {
            if message.get("method").is_some() {
                return Err(Error::Protocol);
            }
            let result = crate::rpc_result(message, self.id)?;
            complete(&result)?;
            if result["_meta"][SUB_ID].as_u64() != Some(self.id) {
                return Err(Error::Protocol);
            }
            return Ok(None);
        }
        if message.get("result").is_some()
            || message.get("error").is_some()
            || message["params"]["_meta"][SUB_ID].as_u64() != Some(self.id)
        {
            return Err(Error::Protocol);
        }
        let change = match message["method"].as_str() {
            Some("notifications/tools/list_changed") if self.filter.tools_list_changed => {
                self.signals.bump(&self.signals.tools);
                Change::Tools
            }
            Some("notifications/prompts/list_changed") if self.filter.prompts_list_changed => {
                self.signals.bump(&self.signals.prompts);
                Change::Prompts
            }
            Some("notifications/resources/list_changed") if self.filter.resources_list_changed => {
                self.signals.bump(&self.signals.resources);
                Change::Resources
            }
            Some("notifications/resources/updated") => {
                let uri = message["params"]["uri"].as_str().ok_or(Error::Protocol)?;
                if !self
                    .filter
                    .resource_subscriptions
                    .iter()
                    .any(|wanted| wanted == uri)
                {
                    return Err(Error::Protocol);
                }
                self.signals.bump(&self.signals.resources);
                Change::Resource { uri: uri.into() }
            }
            _ => return Err(Error::Protocol),
        };
        Ok(Some(change))
    }

    fn end(&mut self) {
        if !self.closed {
            self.closed = true;
            self.signals.invalidate(&self.filter);
            self.reader.take();
            self.permit.take();
        }
    }
}
impl Drop for Subscription {
    fn drop(&mut self) {
        self.end();
    }
}

impl Modern<'_> {
    pub(super) fn listen(&mut self, filter: SubscriptionFilter) -> Result<Subscription, Error> {
        filter.validate()?;
        if !filter.tools_list_changed
            && !filter.prompts_list_changed
            && !filter.resources_list_changed
            && filter.resource_subscriptions.is_empty()
        {
            return Err(Error::Protocol);
        }
        let permit = Permit::acquire(&self.signals)?;
        // Covers the gap before acknowledgement and failed opens as well.
        self.signals.invalidate(&filter);
        let id = self.next_id;
        self.next_id = id.checked_add(1).ok_or(Error::Limit)?;
        let prepared = metadata::request_with_capabilities(
            id,
            "subscriptions/listen",
            json!({"notifications":filter}),
            self.input_support.metadata(),
        )?;
        let response = self.post(&prepared, None)?;
        match response.status() {
            200 => {}
            401 | 403 => return Err(self.reject_auth()),
            status => return Err(http_error(response, id, status)),
        }
        let content_type = response
            .header("Content-Type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if !content_type.eq_ignore_ascii_case("text/event-stream") {
            return match crate::decode_response(response, id) {
                Err(error) => Err(error),
                Ok(_) => Err(Error::Protocol),
            };
        }
        let mut subscription = Subscription {
            guard: self.guard.clone(),
            reader: Some(Sse::new(Box::new(response.into_reader()))),
            permit: Some(permit),
            signals: self.signals.clone(),
            filter,
            id,
            closed: false,
        };
        let message = subscription
            .reader
            .as_mut()
            .ok_or(Error::Protocol)?
            .message()?
            .ok_or(Error::Transport)?;
        if message.get("id").is_some() {
            return match crate::rpc_result(message, id) {
                Err(error) => Err(error),
                Ok(_) => Err(Error::Protocol),
            };
        }
        if message["jsonrpc"] != "2.0"
            || message.get("id").is_some()
            || message.get("result").is_some()
            || message.get("error").is_some()
            || message["method"] != "notifications/subscriptions/acknowledged"
            || message["params"]["_meta"][SUB_ID].as_u64() != Some(id)
        {
            return Err(Error::Protocol);
        }
        let accepted: SubscriptionFilter =
            serde_json::from_value(message["params"]["notifications"].clone())
                .map_err(|_| Error::Protocol)?;
        crate::authorize(subscription.guard.as_ref())?;
        accepted.validate()?;
        if !accepted.subset_of(&subscription.filter) {
            return Err(Error::Protocol);
        }
        subscription.filter = accepted;
        Ok(subscription)
    }
}

struct Sse {
    reader: BufReader<Box<dyn Read + Send>>,
    bytes: usize,
    messages: usize,
}
impl Sse {
    fn new(reader: Box<dyn Read + Send>) -> Self {
        Self {
            reader: BufReader::new(reader),
            bytes: 0,
            messages: 0,
        }
    }
    fn message(&mut self) -> Result<Option<Value>, Error> {
        let mut data = String::new();
        let mut frame = 0usize;
        let mut has_data = false;
        loop {
            let mut line = String::new();
            let count = self
                .reader
                .by_ref()
                .take((MAX_FRAME - frame + 1) as u64)
                .read_line(&mut line)
                .map_err(|error| {
                    if error.kind() == std::io::ErrorKind::InvalidData {
                        Error::Protocol
                    } else {
                        Error::Transport
                    }
                })?;
            frame += count;
            self.bytes += count;
            if frame > MAX_FRAME || self.bytes > MAX_BYTES {
                return Err(Error::Limit);
            }
            if count == 0 {
                return if has_data {
                    Err(Error::Protocol)
                } else {
                    Ok(None)
                };
            }
            let line = line.trim_end_matches(['\r', '\n']);
            if line.is_empty() {
                frame = 0;
                if !has_data {
                    continue;
                }
                self.messages += 1;
                if self.messages > MAX_MESSAGES {
                    return Err(Error::Limit);
                }
                return serde_json::from_str(&data)
                    .map(Some)
                    .map_err(|_| Error::Protocol);
            }
            if let Some(value) = line.strip_prefix("data:") {
                if has_data {
                    data.push('\n');
                }
                data.push_str(value.strip_prefix(' ').unwrap_or(value));
                has_data = true;
            }
        }
    }
}

#[cfg(test)]
mod tests;
