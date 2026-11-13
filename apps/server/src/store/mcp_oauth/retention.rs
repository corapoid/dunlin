//! Active quota is separate from bounded, repeatably sweepable history.
use super::*;
pub(super) const SAFE_HISTORY: usize = 512;
pub(super) const UNCERTAIN_HISTORY: usize = 256;
// A live/unfinished sweep may temporarily retain another256 verified rows.
pub(super) const MAX_REFERENCES: usize = 256 + SAFE_HISTORY + 256 + UNCERTAIN_HISTORY;
pub(in crate::store) const MIGRATION:&str="
ALTER TABLE mcp_oauth_secret_references ADD COLUMN as_confirmed INTEGER NOT NULL DEFAULT 0 CHECK(as_confirmed IN (0,1));
ALTER TABLE mcp_oauth_secret_references ADD COLUMN retired INTEGER NOT NULL DEFAULT 0 CHECK(retired IN (0,1) AND (retired=0 OR as_confirmed=1));
ALTER TABLE mcp_oauth_secret_references ADD COLUMN delete_verified INTEGER NOT NULL DEFAULT 0 CHECK(delete_verified IN (0,1) AND (delete_verified=0 OR (retired=1 AND as_confirmed=1 AND writer_protected=1 AND write_claimed=1 AND write_acknowledged=1 AND write_operation_id IS NOT NULL)));
ALTER TABLE mcp_oauth_secret_references ADD COLUMN cleanup_attempted_revision INTEGER NOT NULL DEFAULT 0 CHECK(cleanup_attempted_revision BETWEEN 0 AND 9007199254740991);
CREATE INDEX mcp_oauth_retention ON mcp_oauth_secret_references(connection_id,retired,as_confirmed,cleanup_attempted_revision);
";
#[derive(serde::Serialize)]
pub(crate) struct Storage {
    pub pending_references: usize,
    pub retained_history: usize,
    pub uncertain_history: usize,
    pub available_slots: usize,
    pub cleanup_available: bool,
}
pub(super) fn storage(db: &Connection, id: &str) -> Result<Storage, PersistenceError> {
    let (pending,history,uncertain):(u32,u32,u32)=db.query_row("SELECT coalesce(sum(retired=0),0),coalesce(sum(retired=1),0),coalesce(sum(retired=1 AND delete_verified=0),0) FROM mcp_oauth_secret_references WHERE connection_id=?1",[id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).map_err(storage_error)?;
    let (pending, history, uncertain) = (pending as usize, history as usize, uncertain as usize);
    if pending > 256
        || uncertain > UNCERTAIN_HISTORY
        || history > SAFE_HISTORY + 256 + UNCERTAIN_HISTORY
    {
        return Err(PersistenceError::Limit);
    }
    let available=db.query_row("SELECT EXISTS(SELECT 1 FROM mcp_oauth_secret_references r JOIN mcp_oauth_connections c ON c.connection_id=r.connection_id WHERE r.connection_id=?1 AND r.as_confirmed=1 AND (c.state='disabled' OR (r.reference IS NOT c.secret_reference AND r.reference IS NOT c.staged_reference)))",[id],|r|r.get(0)).map_err(storage_error)?;
    Ok(Storage {
        pending_references: pending,
        retained_history: history,
        uncertain_history: uncertain,
        available_slots: 256 - pending,
        cleanup_available: available,
    })
}
fn storage_error(e: rusqlite::Error) -> PersistenceError {
    super::storage(e)
}
pub(super) fn prune(
    db: &Connection,
    c: &AuthContext,
    writer: bool,
) -> Result<usize, PersistenceError> {
    if !writer {
        return Ok(0);
    }
    let count:u32=db.query_row("SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE connection_id=?1 AND retired=1 AND delete_verified=1",[&c.connection_id],|r|r.get(0)).map_err(storage_error)?;
    let count = count as usize;
    if count > SAFE_HISTORY + 256 {
        return Err(PersistenceError::Limit);
    }
    let excess = count.saturating_sub(SAFE_HISTORY);
    if excess == 0 {
        return Ok(0);
    }
    db.execute("DELETE FROM mcp_oauth_secret_references WHERE reference IN (
        SELECT r.reference FROM mcp_oauth_secret_references r JOIN mcp_oauth_connections c ON c.connection_id=r.connection_id
        WHERE r.connection_id=?1 AND c.user_id=?2 AND c.workspace_id=?3 AND c.revision=?4
        AND r.retired=1 AND r.delete_verified=1 AND r.as_confirmed=1 AND r.writer_protected=1 AND r.write_claimed=1 AND r.write_acknowledged=1
        AND (c.state='disabled' OR (r.reference IS NOT c.secret_reference AND r.reference IS NOT c.staged_reference))
        ORDER BY r.created_at,r.reference LIMIT ?5)",params![c.connection_id,c.user_id,c.workspace_id,c.connection_revision as i64,excess as i64]).map_err(storage_error)
}
#[cfg(test)]
impl Store {
    pub(crate) fn mcp_oauth_test_legacy_writer(&self, id: &str) {
        self.connection.execute("UPDATE mcp_oauth_secret_references SET writer_protected=0 WHERE connection_id=?1 AND delete_verified=0",[id]).unwrap();
    }
}
