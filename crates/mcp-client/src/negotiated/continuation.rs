//! Explicit MRTR continuation. Pending handles are neither cloneable nor
//! serializable and cannot be moved between authenticated client instances.
use super::{Error, Modern, Tool, complete};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

const MAX_ROUNDS: u8 = 8;
const MAX_PENDING: usize = 8;
const MAX_INPUTS: usize = 16;
const MAX_STATE: usize = 16 * 1024;

/// The integration must provide review/decline/cancel UI before enabling URL
/// input. Form support requires review/edit/decline/cancel UI; roots and sampling
/// are not advertised. No input support is enabled by default.
#[derive(Clone, Copy, Default)]
pub struct InputSupport {
    pub elicitation_url: bool,
    pub elicitation_form: bool,
}
impl InputSupport {
    pub(super) fn metadata(self) -> Value {
        let mut modes = serde_json::Map::new();
        if self.elicitation_url {
            modes.insert("url".into(), json!({}));
        }
        if self.elicitation_form {
            modes.insert("form".into(), json!({}));
        }
        if modes.is_empty() {
            json!({})
        } else {
            json!({"elicitation":modes})
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ElicitationAction {
    Accept,
    Decline,
    Cancel,
}

/// Untrusted display data. The transport never navigates to this URL. A user
/// must see the requesting server and target host and decide before navigation.
pub struct UrlElicitation {
    pub message: String,
    pub url: String,
}
pub struct FormElicitation {
    pub message: String,
    pub schema: crate::schema::FormSchema,
}
/// Explicit user responses, never a source of user identity or authorization.
/// No Debug/Serialize: form content must not accidentally enter application logs.
pub enum ElicitationResponse {
    Url(ElicitationAction),
    Form {
        action: ElicitationAction,
        content: Option<Value>,
    },
}

pub enum Outcome {
    Complete(Value),
    InputRequired(Pending),
}

#[derive(Default)]
pub(super) struct Identity {
    pending: AtomicUsize,
}
struct Ticket(Arc<Identity>);
impl Ticket {
    fn acquire(identity: &Arc<Identity>) -> Result<Self, Error> {
        let mut current = identity.pending.load(Ordering::Acquire);
        loop {
            if current >= MAX_PENDING {
                return Err(Error::Limit);
            }
            match identity.pending.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(Self(identity.clone())),
                Err(observed) => current = observed,
            }
        }
    }
}
impl Drop for Ticket {
    fn drop(&mut self) {
        self.0.pending.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Single-use continuation bound to one endpoint/authenticated client and one
/// original operation. Opaque state is never parsed, displayed or logged.
pub struct Pending {
    ticket: Ticket,
    operation: Operation,
    state: Option<String>,
    requests: BTreeMap<String, UrlElicitation>,
    forms: BTreeMap<String, FormElicitation>,
    had_requests: bool,
    round: u8,
    started: Instant,
}
impl Pending {
    pub fn input_requests(&self) -> &BTreeMap<String, UrlElicitation> {
        &self.requests
    }
    pub fn form_requests(&self) -> &BTreeMap<String, FormElicitation> {
        &self.forms
    }
}

pub(super) enum Operation {
    Tool { tool: Tool, arguments: Value },
    Prompt { name: String, arguments: Value },
    Resource { uri: String },
}
impl Operation {
    pub(super) fn request(&self) -> (&'static str, Value) {
        match self {
            Self::Tool { tool, arguments } => (
                "tools/call",
                json!({"name":tool.name,"arguments":arguments}),
            ),
            Self::Prompt { name, arguments } => {
                ("prompts/get", json!({"name":name,"arguments":arguments}))
            }
            Self::Resource { uri } => ("resources/read", json!({"uri":uri})),
        }
    }
}

impl Modern<'_> {
    pub(super) fn begin(&mut self, operation: Operation) -> Result<Outcome, Error> {
        let ticket = Ticket::acquire(&self.identity)?;
        self.round(operation, None, 0, Instant::now(), ticket)
    }

    pub(super) fn continue_request(
        &mut self,
        pending: Pending,
        responses: BTreeMap<String, ElicitationAction>,
    ) -> Result<Outcome, Error> {
        self.continue_with_responses(
            pending,
            responses
                .into_iter()
                .map(|(key, action)| (key, ElicitationResponse::Url(action)))
                .collect(),
        )
    }

    pub(super) fn continue_with_responses(
        &mut self,
        pending: Pending,
        responses: BTreeMap<String, ElicitationResponse>,
    ) -> Result<Outcome, Error> {
        if !Arc::ptr_eq(&self.identity, &pending.ticket.0) {
            return Err(Error::Protocol);
        }
        if pending.round + 1 >= MAX_ROUNDS || pending.started.elapsed() >= Duration::from_secs(60) {
            return Err(Error::Limit);
        }
        if responses.len() != pending.requests.len() + pending.forms.len() {
            return Err(Error::Protocol);
        }
        let mut encoded = serde_json::Map::new();
        for (key, response) in responses {
            let value = match response {
                ElicitationResponse::Url(action) => {
                    if !pending.requests.contains_key(&key) {
                        return Err(Error::Protocol);
                    }
                    json!({"action":action})
                }
                ElicitationResponse::Form { action, content } => {
                    let form = pending.forms.get(&key).ok_or(Error::Protocol)?;
                    if action == ElicitationAction::Accept {
                        let content = content.ok_or(Error::Arguments)?;
                        form.schema.validate(&content)?;
                        json!({"action":action,"content":content})
                    } else {
                        if content.is_some() {
                            return Err(Error::Arguments);
                        }
                        json!({"action":action})
                    }
                }
            };
            encoded.insert(key, value);
        }
        let Pending {
            ticket,
            operation,
            state,
            had_requests,
            round,
            started,
            ..
        } = pending;
        let mut extra = serde_json::Map::new();
        if let Some(state) = state {
            extra.insert("requestState".into(), Value::String(state));
        }
        if had_requests {
            extra.insert("inputResponses".into(), Value::Object(encoded));
        }
        self.round(operation, Some(extra), round + 1, started, ticket)
    }

    fn round(
        &mut self,
        operation: Operation,
        extra: Option<serde_json::Map<String, Value>>,
        round: u8,
        started: Instant,
        ticket: Ticket,
    ) -> Result<Outcome, Error> {
        let headers = match &operation {
            Operation::Tool { tool, arguments } => self.tool_headers(tool, arguments)?,
            Operation::Prompt { .. } => {
                if !self.discovery["capabilities"]["prompts"].is_object() {
                    return Err(Error::Unsupported);
                }
                Vec::new()
            }
            Operation::Resource { .. } => {
                if !self.discovery["capabilities"]["resources"].is_object() {
                    return Err(Error::Unsupported);
                }
                Vec::new()
            }
        };
        let (method, mut params) = operation.request();
        if let Some(extra) = extra {
            params.as_object_mut().ok_or(Error::Protocol)?.extend(extra);
        }
        let result = self.request_until(
            method,
            params,
            headers,
            Some(started + Duration::from_secs(60)),
        )?;
        if started.elapsed() >= Duration::from_secs(60) {
            return Err(Error::Limit);
        }
        if result["resultType"] != "input_required" {
            complete(&result)?;
            if let Operation::Tool { tool, .. } = &operation {
                if result.get("isError").is_some_and(|flag| !flag.is_boolean()) {
                    return Err(Error::Protocol);
                }
                if result["isError"] == true {
                    return Err(Error::ToolFailed);
                }
                self.catalog
                    .get(&tool.name)
                    .ok_or(Error::Unsupported)?
                    .validation
                    .output(&result)?;
            }
            return Ok(Outcome::Complete(result));
        }
        let state = match result.get("requestState") {
            None => None,
            Some(value) => {
                let value = value.as_str().ok_or(Error::Protocol)?;
                if value.len() > MAX_STATE {
                    return Err(Error::Limit);
                }
                Some(value.to_owned())
            }
        };
        let had_requests = result.get("inputRequests").is_some();
        if state.is_none() && !had_requests {
            return Err(Error::Protocol);
        }
        let mut requests = BTreeMap::new();
        let mut forms = BTreeMap::new();
        if let Some(inputs) = result.get("inputRequests") {
            let inputs = inputs.as_object().ok_or(Error::Protocol)?;
            if inputs.len() > MAX_INPUTS {
                return Err(Error::Limit);
            }
            for (key, request) in inputs {
                if key.is_empty() || key.len() > 128 {
                    return Err(Error::Protocol);
                }
                if request["method"] != "elicitation/create" {
                    return Err(Error::Unsupported);
                }
                let params = &request["params"];
                let form = params.get("mode").is_none() || params["mode"] == "form";
                if (form && !self.input_support.elicitation_form)
                    || (!form && (params["mode"] != "url" || !self.input_support.elicitation_url))
                {
                    return Err(Error::Unsupported);
                }
                let message = params["message"].as_str().ok_or(Error::Protocol)?;
                if message.len() > 8192 {
                    return Err(Error::Limit);
                }
                if form {
                    let schema = crate::schema::FormSchema::compile(&params["requestedSchema"])?;
                    forms.insert(
                        key.clone(),
                        FormElicitation {
                            message: message.into(),
                            schema,
                        },
                    );
                    continue;
                }
                let url = params["url"].as_str().ok_or(Error::Protocol)?;
                if message.len() > 8192 || url.len() > 4096 {
                    return Err(Error::Limit);
                }
                let parsed = url::Url::parse(url).map_err(|_| Error::Protocol)?;
                // Local integration policy is intentionally narrower than a URL
                // parser: no javascript/file, embedded credentials or plain HTTP.
                if url
                    .bytes()
                    .any(|b| b.is_ascii_whitespace() || b.is_ascii_control() || b == b'\\')
                    || parsed.scheme() != "https"
                    || parsed.host().is_none()
                    || !parsed.username().is_empty()
                    || parsed.password().is_some()
                {
                    return Err(Error::Protocol);
                }
                requests.insert(
                    key.clone(),
                    UrlElicitation {
                        message: message.into(),
                        url: url.into(),
                    },
                );
            }
        }
        Ok(Outcome::InputRequired(Pending {
            ticket,
            operation,
            state,
            requests,
            forms,
            had_requests,
            round,
            started,
        }))
    }
}

#[cfg(test)]
mod tests;
