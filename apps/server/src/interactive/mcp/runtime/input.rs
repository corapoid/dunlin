//! The Pending handle never leaves this one authenticated client or operation.
use super::*;
use dunlin_mcp_client::negotiated::{ElicitationAction, ElicitationResponse, Outcome};
use dunlin_session_runtime::{McpInputChallenge, McpInputScope, PermissionDecision};

pub(super) struct Call<'a> {
    pub broker: &'a Arc<Broker>,
    pub permit: &'a McpDispatchPermit,
    pub invocation: &'a McpInvocation,
    pub entry: &'a Entry,
    pub deadline: Instant,
    pub bearer: &'a str,
    pub devices: &'a Mutex<Vec<PermissionDecision>>,
}
impl Broker {
    pub(super) fn input_devices(&self, devices: &Mutex<Vec<PermissionDecision>>) -> Result<()> {
        let devices = devices.lock().map_err(|_| "mcp_input_unavailable")?;
        if devices.is_empty() {
            return Ok(());
        }
        let cut = self.authority.sql(&self.state)?;
        let store = self.state.store.lock().map_err(|_| "store_unavailable")?;
        self.context(&store, false, cut.as_ref())?;
        for device in devices.iter() {
            cut.permission_device(&store, &self.pins().owner, device)?;
        }
        Ok(())
    }
}
impl Call<'_> {
    fn checked(&self) -> std::result::Result<(), dunlin_mcp_client::Error> {
        self.broker
            .check(self.deadline)
            .and_then(|()| self.broker.permit(self.permit, self.invocation))
            .and_then(|()| self.broker.input_devices(self.devices))
            .map_err(|_| dunlin_mcp_client::Error::Unauthorized)
    }
    pub fn run(
        &self,
        client: &mut Client<'_>,
        arguments: Value,
    ) -> std::result::Result<Value, dunlin_mcp_client::Error> {
        let mut outcome = client.begin_call(&self.entry.tool, arguments)?;
        loop {
            self.checked()?;
            match outcome {
                Outcome::Complete(value) => return Ok(value),
                Outcome::InputRequired(pending) => {
                    let mut inputs = Vec::new();
                    for (key, url) in pending.input_requests() {
                        let request = super::super::urls::request(
                            self.broker.connections[self.entry.connection]
                                .oauth()
                                .map_err(|_| dunlin_mcp_client::Error::Unauthorized)?,
                            &self.entry.tool.name,
                            url,
                        )?;
                        let encoded = serde_json::to_string(&request)
                            .map_err(|_| dunlin_mcp_client::Error::Protocol)?;
                        if encoded.contains(self.bearer) || key.contains(self.bearer) {
                            return Err(dunlin_mcp_client::Error::Protocol);
                        }
                        let input = self
                            .broker
                            .journal(|j, a| {
                                j.register_mcp_input(
                                    a,
                                    &self.broker.pins().session,
                                    &self.broker.pins().lease,
                                    McpInputScope {
                                        permit: self.permit,
                                        invocation: self.invocation,
                                    },
                                    McpInputChallenge {
                                        request,
                                        schema: Value::Null,
                                    },
                                )
                            })
                            .map_err(|_| dunlin_mcp_client::Error::Unauthorized)?;
                        inputs.push((key.clone(), input, true));
                    }
                    for (key, form) in pending.form_requests() {
                        let request = super::super::forms::request(
                            self.broker.connections[self.entry.connection]
                                .oauth()
                                .map_err(|_| dunlin_mcp_client::Error::Unauthorized)?,
                            &self.entry.tool.name,
                            form,
                        )?;
                        let encoded = serde_json::to_string(&request)
                            .map_err(|_| dunlin_mcp_client::Error::Protocol)?;
                        if encoded.contains(self.bearer)
                            || key.contains(self.bearer)
                            || form.schema.schema().to_string().contains(self.bearer)
                        {
                            return Err(dunlin_mcp_client::Error::Protocol);
                        }
                        let input = self
                            .broker
                            .journal(|j, a| {
                                j.register_mcp_input(
                                    a,
                                    &self.broker.pins().session,
                                    &self.broker.pins().lease,
                                    McpInputScope {
                                        permit: self.permit,
                                        invocation: self.invocation,
                                    },
                                    McpInputChallenge {
                                        request,
                                        schema: form.schema.schema().clone(),
                                    },
                                )
                            })
                            .map_err(|_| dunlin_mcp_client::Error::Unauthorized)?;
                        inputs.push((key.clone(), input, false));
                    }
                    let mut responses = BTreeMap::new();
                    while !inputs.is_empty() {
                        self.checked()?;
                        let mut remaining = Vec::new();
                        for (key, input, url) in inputs {
                            let decision = self
                                .broker
                                .journal(|j, a| {
                                    j.take_mcp_input(
                                        a,
                                        &self.broker.pins().session,
                                        &self.broker.pins().lease,
                                        McpInputScope {
                                            permit: self.permit,
                                            invocation: self.invocation,
                                        },
                                        &input,
                                    )
                                })
                                .map_err(|_| dunlin_mcp_client::Error::Unauthorized)?;
                            if let Some(decision) = decision {
                                if decision
                                    .content
                                    .as_ref()
                                    .is_some_and(|c| c.to_string().contains(self.bearer))
                                {
                                    return Err(dunlin_mcp_client::Error::Protocol);
                                }
                                self.devices
                                    .lock()
                                    .map_err(|_| dunlin_mcp_client::Error::Protocol)?
                                    .push(decision.device);
                                self.checked()?;
                                let action = match decision.response {
                                    ahp_types::state::ChatInputResponseKind::Accept => {
                                        ElicitationAction::Accept
                                    }
                                    ahp_types::state::ChatInputResponseKind::Decline => {
                                        ElicitationAction::Decline
                                    }
                                    ahp_types::state::ChatInputResponseKind::Cancel => {
                                        ElicitationAction::Cancel
                                    }
                                };
                                responses.insert(
                                    key,
                                    if url {
                                        if decision.content.is_some() {
                                            return Err(dunlin_mcp_client::Error::Protocol);
                                        }
                                        ElicitationResponse::Url(action)
                                    } else {
                                        ElicitationResponse::Form {
                                            action,
                                            content: decision.content,
                                        }
                                    },
                                );
                            } else {
                                remaining.push((key, input, url));
                            }
                        }
                        inputs = remaining;
                        if !inputs.is_empty() {
                            std::thread::sleep(Duration::from_millis(50));
                        }
                    }
                    self.checked()?;
                    outcome = client.continue_with_responses(pending, responses)?;
                }
            }
        }
    }
}
