//! Finish only the local journal; unknown AS effects stay unknown. Never replay.
use super::*;
impl Store {
    pub(crate) fn mcp_oauth_reconcile_revocation(
        &self,
        context: &AuthContext,
        lease: &RevocationLease,
        check: &Authority<'_>,
    ) -> Result<RevocationSummary, PersistenceError> {
        if !lease.belongs_to(self) {
            return Err(PersistenceError::Conflict);
        }
        let tx = transaction(self, context)?;
        check(self, context)?;
        let value: String = tx.query_row(
            "SELECT revocation FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND state='disabled' AND revocation IS NOT NULL",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64], |r|r.get(0)
        ).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
        let mut ledger = decode(&value)?;
        if ledger.state != JobState::Running
            || !ledger.lease_protected
            || ledger.disabled_revision != context.connection_revision
            || ledger.source_revision.checked_add(1) != Some(ledger.disabled_revision)
        {
            return Err(PersistenceError::Conflict);
        }
        let summary = ledger.summary();
        ledger.state = if summary.confirmed_known_tokens {
            JobState::Confirmed
        } else {
            JobState::Unconfirmed
        };
        save(&tx, context, &ledger)?;
        match self.team_audit_capacity() {
            Ok(()) => {
                tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,'mcp_oauth_revocation_reconciled',?3,?4,?5)",
                    params![context.workspace_id,context.user_id,context.connection_revision as i64,super::super::super::now_secs(),context.connection_id]).map_err(storage)?;
            }
            Err(super::super::super::StoreError::Limit(_)) => {}
            Err(_) => return Err(PersistenceError::Storage),
        }
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(summary)
    }
}
