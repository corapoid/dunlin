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
                        json!({"content":[{"type":"text","text":"Scoped HTTP call completed."}]}),
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
        let input = scoped::tool::input(&version.credential, invocation.arguments.clone())
            .map_err(|_| "mcp_arguments_invalid")?;
        let input_sha256 = hex::encode(Sha256::digest(
            serde_json::to_vec(&input).map_err(|_| "mcp_arguments_invalid")?,
        ));
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
        });
        let result = self.state.scoped_secrets.http(
            scope.clone(),
            version.clone(),
            input,
            deadline.into_std(),
            call.clone(),
        );
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
            Ok(output) => Ok(
                json!({"content":[{"type":"text","text":serde_json::to_string(&output).map_err(|_| "mcp_reply_limit")?}]}),
            ),
            Err(scoped::Error::OutcomeUnknown) => Err("mcp_outcome_unknown"),
            Err(_) => Ok(
                json!({"isError":true,"content":[{"type":"text","text":"Scoped HTTP call rejected or failed. It was not retried."}]}),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
