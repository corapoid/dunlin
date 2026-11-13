//! Read-only resolution of one explicit target. This observation cannot spawn a
//! process, reserve capacity, renew execution authority or select another target.
use super::*;
use dunlin_domain::binding::BindingRef;
pub(crate) mod execution;
pub(crate) mod sessions;

pub(crate) struct RemoteTargetCandidate {
    binding: RemoteBinding,
    controller_node_id: String,
    authority: AuthorityStamp,
}
impl RemoteTargetCandidate {
    pub fn host_id(&self) -> &str {
        &self.binding.host_id
    }
    pub fn export_id(&self) -> &str {
        &self.binding.export_id
    }
    pub fn export_revision(&self) -> u64 {
        self.binding.export_revision
    }
}
/// Opaque and deliberately not serializable. Read permission is not an execute
/// permit; admission must independently recheck current execution authority.
pub(crate) struct ResolvedRemoteTarget {
    summary: ResolvedRemoteTargetSummary,
    _proof: ObservedExport,
    _candidate: RemoteTargetCandidate,
}
impl ResolvedRemoteTarget {
    pub fn summary(&self) -> &ResolvedRemoteTargetSummary {
        &self.summary
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ResolvedRemoteTargetSummary {
    pub binding: BindingRef,
    pub workspace_id: String,
    pub project_id: String,
    pub host_id: String,
    pub host_enrollment_epoch: u64,
    pub credential_revision: u64,
    pub export_id: String,
    pub export_revision: u64,
    pub repo_kind: RepoKind,
    pub checkout_identity_digest: String,
    pub approved_settings_digest: String,
    pub principal: ExportPrincipal,
    pub grant_revision: u64,
    pub grant_permissions: Vec<ExportPermission>,
    pub observed_at_ms: i64,
    pub required_worker_permission: &'static str,
    pub observed_state_only: bool,
    pub execution_ready: bool,
}
impl Store {
    pub(crate) fn capture_remote_target_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        reference: &BindingRef,
        current_controller_node_id: &str,
    ) -> Result<RemoteTargetCandidate> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "A remote target authority transaction is required".into(),
            ));
        }
        reference
            .validate()
            .map_err(|_| StoreError::Limit("Invalid remote binding reference".into()))?;
        let context = self.team_project_context(workspace, project, actor)?;
        context.authorize_project(workspace, project, Permission::ViewProject)?;
        let authority = context.authorize_project(workspace, project, Permission::CreateSession)?;
        let binding = self
            .remote_binding_row(workspace, project, &reference.id)?
            .filter(|binding| binding.active && binding.revision == reference.revision)
            .ok_or_else(conflict)?;
        let controller_node_id: String = self.connection.query_row(
            "SELECT controller_node_id FROM team_remote_bindings WHERE id=?1",
            [&binding.id],
            |row| row.get(0),
        )?;
        if controller_node_id != current_controller_node_id {
            return Err(conflict());
        }
        let host_current:bool=self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM team_hosts WHERE id=?1 AND workspace_id=?2 AND status='approved' AND enrollment_epoch=?3 AND credential_revision=?4)",
            params![binding.host_id,workspace,binding.host_enrollment_epoch as i64,binding.credential_revision as i64],|row|row.get(0),
        )?;
        if !host_current {
            return Err(conflict());
        }
        // Drain is intentionally allowed for read-only metadata; this is not
        // execution admission or a claim that the target is ready to run.
        Ok(RemoteTargetCandidate {
            binding,
            controller_node_id,
            authority,
        })
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn finish_remote_target_in_transaction(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        candidate: RemoteTargetCandidate,
        proof: ObservedExport,
        current_controller_boot_id: &str,
        current_controller_node_id: &str,
        now_ms: i64,
    ) -> Result<ResolvedRemoteTarget> {
        let reference = BindingRef {
            id: candidate.binding.id.clone(),
            revision: candidate.binding.revision,
        };
        let current = self.capture_remote_target_in_transaction(
            actor,
            workspace,
            project,
            &reference,
            current_controller_node_id,
        )?;
        if current.binding != candidate.binding
            || current.controller_node_id != candidate.controller_node_id
            || current.authority != candidate.authority
        {
            return Err(conflict());
        }
        if !proof.is_fresh()
            || now_ms < 0
            || proof.observed_at_ms() < 0
            || proof.observed_at_ms() > now_ms
        {
            return Err(StoreError::Conflict(
                "Remote target observation expired; resolve the same reference again".into(),
            ));
        }
        let descriptor = proof.descriptor();
        descriptor.validate().map_err(|_| denied())?;
        let binding = &candidate.binding;
        if proof.identity().host_id() != binding.host_id
            || proof.identity().workspace_id() != workspace
            || proof.identity().host().enrollment_epoch != binding.host_enrollment_epoch
            || proof.identity().host().credential_revision != binding.credential_revision
            || descriptor.scope.controller_node_id != current_controller_node_id
            || descriptor.scope.worker_node_id != proof.identity().host().definition.node_id
            || descriptor.scope.workspace_id != workspace
            || descriptor.scope.host_id != binding.host_id
            || descriptor.scope.enrollment_epoch != binding.host_enrollment_epoch
            || descriptor.scope.credential_revision != binding.credential_revision
            || descriptor.export.id != binding.export_id
            || descriptor.export.revision != binding.export_revision
            || descriptor.export.repo_kind != binding.repo_kind
            || descriptor.export.checkout_identity_digest != binding.checkout_identity_digest
            || descriptor.export.settings_digest != binding.approved_settings_digest
            || descriptor.grant.principal.kind != PrincipalKind::Human
            || descriptor.grant.principal.id != actor
            || !descriptor
                .grant
                .permissions
                .contains(&ExportPermission::Read)
        {
            return Err(denied());
        }
        self.validate_worker_channel_in_transaction(
            proof.identity(),
            proof.lease(),
            current_controller_boot_id,
            now_ms,
        )?;
        let summary = ResolvedRemoteTargetSummary {
            binding: reference,
            workspace_id: workspace.into(),
            project_id: project.into(),
            host_id: binding.host_id.clone(),
            host_enrollment_epoch: binding.host_enrollment_epoch,
            credential_revision: binding.credential_revision,
            export_id: binding.export_id.clone(),
            export_revision: binding.export_revision,
            repo_kind: binding.repo_kind,
            checkout_identity_digest: binding.checkout_identity_digest.clone(),
            approved_settings_digest: binding.approved_settings_digest.clone(),
            principal: descriptor.grant.principal.clone(),
            grant_revision: descriptor.grant.revision,
            grant_permissions: descriptor.grant.permissions.clone(),
            observed_at_ms: proof.observed_at_ms(),
            required_worker_permission: "read",
            observed_state_only: true,
            execution_ready: false,
        };
        Ok(ResolvedRemoteTarget {
            summary,
            _proof: proof,
            _candidate: candidate,
        })
    }
}
