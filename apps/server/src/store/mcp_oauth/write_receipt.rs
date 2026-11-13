//! Only the live host may report the result of its one claimed private write.
//! The acknowledgement contains no token and grants no current authority.
use super::*;
pub(in crate::store) const MIGRATION: &str = "
ALTER TABLE mcp_oauth_secret_references ADD COLUMN write_operation_id TEXT CHECK(write_operation_id IS NULL OR length(write_operation_id) BETWEEN 1 AND 128);
ALTER TABLE mcp_oauth_secret_references ADD COLUMN write_claimed INTEGER NOT NULL DEFAULT 0 CHECK(write_claimed IN (0,1));
ALTER TABLE mcp_oauth_secret_references ADD COLUMN write_acknowledged INTEGER NOT NULL DEFAULT 0 CHECK(write_acknowledged IN (0,1) AND (write_acknowledged=0 OR write_claimed=1));
";
fn valid(c: &AuthContext, b: &str, i: &CredentialIntent, h: &str) -> Result<(), PersistenceError> {
    c.validate().map_err(|_| PersistenceError::Unauthorized)?;
    if !hex_digest(b) || !hex_digest(h) || i.generation == 0 || i.generation > 9_007_199_254_740_991
    {
        return Err(PersistenceError::Conflict);
    }
    Ok(())
}
impl CheckedJournal<'_> {
    pub(super) fn claim_credential_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        valid(c, b, i, h)?;
        let tx = self.transaction(c)?;
        let changed=tx.execute("UPDATE mcp_oauth_secret_references SET write_claimed=1
            WHERE reference=?1 AND connection_id=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6 AND write_operation_id=?7
            AND write_claimed=0 AND write_acknowledged=0
            AND (writer_protected=0 OR ?10=1)
            AND EXISTS(SELECT 1 FROM mcp_oauth_connections c WHERE c.connection_id=?2 AND c.revision=?3
                AND c.workspace_id=?8 AND c.user_id=?9 AND c.binding=?4 AND c.generation=?11
                AND c.operation_id=?7 AND c.staged_reference=?1 AND c.state IN ('installing','refreshing'))",
            params![i.staged_reference,c.connection_id,c.connection_revision as i64,b,i.generation as i64,h,i.operation_id,c.workspace_id,c.user_id,
                self.oauth_writer_owner.load(std::sync::atomic::Ordering::Acquire),(i.generation-1) as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        tx.commit().map_err(storage)
    }
}
impl Store {
    pub(crate) fn mcp_oauth_acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        valid(c, b, i, h)?;
        // Deliberately no positive authorization: this bit can only close the
        // exact host-created claim, after I/O succeeded. Cookie/device/member,
        // connection revision and active generation may already have changed.
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        let changed=tx.execute("UPDATE mcp_oauth_secret_references SET write_acknowledged=1
            WHERE reference=?1 AND connection_id=?2 AND credential_revision=?3 AND binding=?4 AND generation=?5 AND sha256=?6 AND write_operation_id=?7
            AND write_claimed=1 AND write_acknowledged=0
            AND (writer_protected=0 OR ?10=1)
            AND EXISTS(SELECT 1 FROM mcp_oauth_connections c WHERE c.connection_id=?2 AND c.workspace_id=?8 AND c.user_id=?9)",
            params![i.staged_reference,c.connection_id,c.connection_revision as i64,b,i.generation as i64,h,i.operation_id,c.workspace_id,c.user_id,
                self.oauth_writer_owner.load(std::sync::atomic::Ordering::Acquire)]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        tx.commit().map_err(storage)
    }
}

#[cfg(test)]
impl Store {
    /// Fixture instrumentation only; never exposed as a browser/runner route.
    pub(crate) fn mcp_oauth_test_write_counts(&self, connection: &str) -> (i64, i64) {
        self.connection.query_row("SELECT coalesce(sum(write_claimed),0),coalesce(sum(write_acknowledged),0) FROM mcp_oauth_secret_references WHERE connection_id=?1",[connection],|r|Ok((r.get(0)?,r.get(1)?))).unwrap()
    }
}
