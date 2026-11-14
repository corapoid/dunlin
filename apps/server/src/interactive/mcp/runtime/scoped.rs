//! Built-in HTTP stays on the controller. Only the static schema, arguments and
//! bounded redacted result cross the harness/runner boundary.
use super::*;
use crate::store::scoped_secrets::Grant;
use dunlin_session_runtime::PermissionDecision;
use dunlin_vault::scoped::{self, Completion, UseScope, Version};

struct Call {
    broker: Arc<Broker>,
    scope: UseScope,
    version: Version,
    grant: Grant,
    invocation: Arc<McpInvocation>,
    input_sha256: String,
    deadline: Instant,
    consumed: AtomicBool,
    permit: Mutex<Option<McpDispatchPermit>>,
    decision: PermissionDecision,
    recipe_target: Option<scoped::RecipeTarget>,
}
impl Call {
    fn live(&self) -> Result<()> {
        self.broker.check(self.deadline)?;
        let turn = self.broker.authority.turn()?;
        if turn.id != self.scope.turn || Instant::now() >= turn.deadline {
            return Err("mcp_secret_turn_changed");
        }
        let cut = self.broker.authority.sql(&self.broker.state)?;
        let store = self
            .broker
            .state
            .store
            .lock()
            .map_err(|_| "store_unavailable")?;
        let context = self.broker.context(&store, false, cut.as_ref())?;
        let (_, version) = store
            .resolve_secret_grant(
                &context,
                &self.scope.binding,
                &self.broker.pins.profile,
                &self.grant.id,
                self.grant.revision,
            )
            .map_err(|_| "mcp_secret_grant_changed")?;
        if version != self.version {
            return Err("mcp_secret_version_changed");
        }
        cut.permission_device(&store, &self.broker.pins.owner, &self.decision)?;
        if let Some(permit) = self
            .permit
            .lock()
            .map_err(|_| "mcp_scope_unavailable")?
            .as_ref()
        {
            store
                .interactive_journal()
                .validate_mcp_dispatch(
                    &context,
                    &self.broker.pins.session,
                    &self.broker.pins.lease,
                    permit,
                    &self.invocation,
                )
                .map_err(|_| "mcp_permission_unavailable")?;
        }
        Ok(())
    }
}
impl scoped::Authority for Call {
    fn check(&self, scope: &UseScope, version: &Version) -> scoped::Result<()> {
        if *scope != self.scope || *version != self.version {
            return Err(scoped::Error::Denied);
        }
        self.live().map_err(|_| scoped::Error::Denied)
    }
    fn consume(&self, scope: &UseScope, version: &Version, digest: &str) -> scoped::Result<()> {
        self.check(scope, version)?;
        if digest != self.input_sha256
            || self
                .consumed
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return Err(scoped::Error::Denied);
        }
        // The SDK permit was already durably consumed for this exact invocation.
        // This second fence transfers it once into the non-cloneable vault permit.
        Ok(())
    }
    fn complete(
        &self,
        scope: &UseScope,
        version: &Version,
        outcome: Completion,
    ) -> scoped::Result<()> {
        if *scope != self.scope || *version != self.version {
            return Err(scoped::Error::Denied);
        }
        let permit = self
            .permit
            .lock()
            .map_err(|_| scoped::Error::Storage)?
            .take()
            .ok_or(scoped::Error::Denied)?;
        let result = self.broker.journal(|j, a| {
            j.finish_mcp_dispatch(
                a,
                &self.broker.pins.session,
                &self.broker.pins.lease,
                permit,
                &self.invocation,
                match outcome {
                    // Do not journal response bodies. The actual redacted result is
                    // returned only to this call; the durable row records completion.
                    Completion::Complete => McpCompletion::Complete(
                        json!({"content":[{"type":"text","text":"Scoped credential call completed."}]}),
                    ),
                    Completion::Failed => McpCompletion::Failed,
                    Completion::Unknown => McpCompletion::Unknown,
                },
            )
        });
        if result.is_err() || outcome == Completion::Unknown {
            self.broker.authority.interrupt();
        }
        result.map_err(|_| scoped::Error::OutcomeUnknown)
    }
}
impl scoped::RecipeAuthority for Call {
    fn check_target(
        &self,
        scope: &UseScope,
        version: &Version,
        target: &scoped::RecipeTarget,
    ) -> scoped::Result<()> {
        scoped::Authority::check(self, scope, version)?;
        if self.recipe_target.as_ref() != Some(target)
            || self
                .broker
                .authority
                .recipe_target(&self.broker.state, self.deadline)
                .map_err(|_| scoped::Error::Denied)?
                != *target
        {
            return Err(scoped::Error::Denied);
        }
        Ok(())
    }
}
enum Input {
    Http(dunlin_vault::http::HttpInput),
    Recipe(
        dunlin_vault::recipe::RecipeInput,
        scoped::RecipeTarget,
        Arc<dyn dunlin_vault::recipe::RecipeExecutor>,
    ),
}
impl Broker {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn scoped_call(
        self: &Arc<Self>,
        request: &str,
        turn: &str,
        deadline: Instant,
        invocation: Arc<McpInvocation>,
        permit: McpDispatchPermit,
        grant: Grant,
        version: Version,
    ) -> Result<Value> {
        let prepared = (|| -> Result<(Input, String)> {
            match invocation.tool_name.as_str() {
                "http" => {
                    let input =
                        scoped::tool::input(&version.credential, invocation.arguments.clone())
                            .map_err(|_| "mcp_arguments_invalid")?;
                    let digest = hex::encode(Sha256::digest(
                        serde_json::to_vec(&input).map_err(|_| "mcp_arguments_invalid")?,
                    ));
                    Ok((Input::Http(input), digest))
                }
                "recipe" => {
                    let input = scoped::tool::recipe_input(
                        &version.credential,
                        invocation.arguments.clone(),
                    )
                    .map_err(|_| "mcp_arguments_invalid")?;
                    let target = self.authority.recipe_target(&self.state, deadline)?;
                    let executor =
                        self.authority
                            .recipe_executor(&self.state, &target, deadline)?;
                    let digest = target
                        .input_digest(&input)
                        .map_err(|_| "mcp_arguments_invalid")?;
                    Ok((Input::Recipe(input, target, executor), digest))
                }
                _ => Err("mcp_tool_unavailable"),
            }
        })();
        let (input, input_sha256) = match prepared {
            Ok(value) => value,
            Err(error) => {
                let _ = self.journal(|j, a| {
                    j.finish_mcp_dispatch(
                        a,
                        &self.pins.session,
                        &self.pins.lease,
                        permit,
                        &invocation,
                        McpCompletion::Unknown,
                    )
                });
                self.authority.interrupt();
                return Err(error);
            }
        };
        let decision = self.journal(|j, a| {
            j.validate_mcp_dispatch(
                a,
                &self.pins.session,
                &self.pins.lease,
                &permit,
                &invocation,
            )
        })?;
        let scope = UseScope {
            principal_kind: dunlin_domain::collaboration::PrincipalKind::Human,
            actor: self.pins.owner.clone(),
            binding: self.pins.secret_target.clone(),
            session: self.pins.session.clone(),
            turn: turn.into(),
            request: request.into(),
            profile_digest: self.pins.profile_digest.clone(),
            lease_instance: self.pins.lease.instance.clone(),
            lease_epoch: self.pins.lease.epoch,
            grant_revision: grant.revision,
            read_only: grant.read_only,
        };
        let call = Arc::new(Call {
            broker: self.clone(),
            scope: scope.clone(),
            version: version.clone(),
            grant,
            invocation,
            input_sha256,
            deadline,
            consumed: AtomicBool::new(false),
            permit: Mutex::new(Some(permit)),
            decision,
            recipe_target: match &input {
                Input::Recipe(_, target, _) => Some(target.clone()),
                _ => None,
            },
        });
        let result = match input {
            Input::Http(input) => self
                .state
                .scoped_secrets
                .http(
                    scope.clone(),
                    version.clone(),
                    input,
                    deadline.into_std(),
                    call.clone(),
                )
                .and_then(|output| {
                    serde_json::to_value(output)
                        .map(|v| (v, false))
                        .map_err(|_| scoped::Error::OutcomeUnknown)
                }),
            Input::Recipe(input, target, executor) => self
                .state
                .scoped_secrets
                .recipe(
                    scope.clone(),
                    version.clone(),
                    input,
                    target,
                    deadline.into_std(),
                    call.clone(),
                    executor.as_ref(),
                )
                .and_then(|output| {
                    let failed =
                        output.exit_code != Some(0) || output.timed_out || output.signal.is_some();
                    serde_json::to_value(output)
                        .map(|v| (v, failed))
                        .map_err(|_| scoped::Error::OutcomeUnknown)
                }),
        };
        if call
            .permit
            .lock()
            .map_err(|_| "mcp_scope_unavailable")?
            .is_some()
        {
            // Capacity/validation failure before vault dispatch still cannot replay
            // the already consumed SDK request after restart.
            let _ =
                scoped::Authority::complete(call.as_ref(), &scope, &version, Completion::Unknown);
            self.authority.interrupt();
        }
        match result {
            Ok((output, failed)) => {
                let mut result = json!({"content":[{"type":"text","text":serde_json::to_string(&output).map_err(|_| "mcp_reply_limit")?}]});
                if failed {
                    result["isError"] = json!(true);
                }
                Ok(result)
            }
            Err(scoped::Error::OutcomeUnknown) => Err("mcp_outcome_unknown"),
            Err(_) => Ok(
                json!({"isError":true,"content":[{"type":"text","text":"Scoped credential call rejected or failed. It was not retried."}]}),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
