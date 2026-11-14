//! ACK is a factual completion of the sole immutable writer. Age, activation
//! and process restart are not evidence; unacknowledged writes are never deleted.
use super::*;
use dunlin_vault::scoped::CleanupClaim;
pub(crate) const MIGRATION: &str = "
ALTER TABLE scoped_credentials ADD COLUMN cleanup_cursor_time INTEGER NOT NULL DEFAULT 0;
ALTER TABLE scoped_credentials ADD COLUMN cleanup_cursor_reference TEXT NOT NULL DEFAULT '';
ALTER TABLE scoped_secret_references ADD COLUMN cleanup_state TEXT NOT NULL DEFAULT 'retained' CHECK(cleanup_state IN ('retained','deleting','deleted') AND (cleanup_state='retained' OR write_ack=1));
ALTER TABLE scoped_secret_references ADD COLUMN cleanup_nonce TEXT;
ALTER TABLE scoped_secret_references ADD COLUMN cleanup_receipt TEXT CHECK(cleanup_receipt IS NULL OR length(cleanup_receipt)<=4096);
CREATE INDEX scoped_cleanup ON scoped_secret_references(workspace,cleanup_state,created_at,reference);
";
fn count(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?).map_err(|_| rusqlite::Error::InvalidQuery)
}
#[derive(Serialize)]
pub(crate) struct Retention {
    pub retained: u64,
    pub pending_writes: u64,
    pub eligible: u64,
    pub deleting: u64,
    pub deleted_history: u64,
}
impl Store {
    pub(crate) fn scoped_secret_retention(
        &self,
        proof: &Proof,
        credential: &str,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Retention> {
        if !token(credential) {
            return Err(Error::Invalid);
        }
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let b = &proof.binding;
        let result=tx.query_row("SELECT count(*) FILTER(WHERE r.cleanup_state!='deleted'),count(*) FILTER(WHERE r.write_ack=0),count(*) FILTER(WHERE r.write_ack=1 AND r.cleanup_state!='deleted' AND (c.state='disabled' OR (r.reference IS NOT c.active_reference AND r.reference IS NOT c.staged_reference))),count(*) FILTER(WHERE r.cleanup_state='deleting'),count(*) FILTER(WHERE r.cleanup_state='deleted') FROM scoped_secret_references r JOIN scoped_credentials c USING(workspace,project,host,credential) WHERE r.workspace=?1 AND r.project=?2 AND r.host=?3 AND r.credential=?4",params![b.workspace,b.project,b.host,credential],|r|Ok(Retention {retained:count(r,0)?,pending_writes:count(r,1)?,eligible:count(r,2)?,deleting:count(r,3)?,deleted_history:count(r,4)?})).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    pub(crate) fn scoped_secret_cleanup_claims(
        &self,
        proof: &Proof,
        credential: &str,
        expected: u64,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Vec<CleanupClaim>> {
        if !token(credential) || expected == 0 || expected > 9_007_199_254_740_991 {
            return Err(Error::Invalid);
        }
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let b = &proof.binding;
        let current:Option<(u64,i64,String)>=tx.query_row("SELECT revision,cleanup_cursor_time,cleanup_cursor_reference FROM scoped_credentials WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4",params![b.workspace,b.project,b.host,credential],|r|Ok((super::super::positive_revision(r,0)?,r.get(1)?,r.get(2)?))).optional().map_err(storage)?;
        let Some((revision, cursor_time, cursor_reference)) = current else {
            return Err(Error::Conflict);
        };
        if revision != expected {
            return Err(Error::Conflict);
        }
        // Disabled pointers have no authority and cannot be republished by an old
        // claim. Detach them atomically before selecting completed writers.
        tx.execute("UPDATE scoped_credentials SET active_reference=NULL WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4 AND state='disabled'",params![b.workspace,b.project,b.host,credential]).map_err(storage)?;
        let rows = {
            let mut stmt=tx.prepare("SELECT r.reference,r.revision,r.sha256,r.cleanup_nonce,r.created_at FROM scoped_secret_references r JOIN scoped_credentials c USING(workspace,project,host,credential) WHERE r.workspace=?1 AND r.project=?2 AND r.host=?3 AND r.credential=?4 AND r.write_ack=1 AND r.cleanup_state!='deleted' AND r.reference IS NOT c.active_reference AND r.reference IS NOT c.staged_reference ORDER BY (r.created_at>?5 OR (r.created_at=?5 AND r.reference>?6)) DESC,r.created_at,r.reference LIMIT 16").map_err(storage)?;
            stmt.query_map(
                params![
                    b.workspace,
                    b.project,
                    b.host,
                    credential,
                    cursor_time,
                    cursor_reference
                ],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        super::super::positive_revision(r, 1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, Option<String>>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )
            .map_err(storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
        };
        let mut claims = Vec::new();
        for (reference, revision, sha256, nonce, created_at) in rows {
            let nonce = nonce.unwrap_or_else(dunlin_vault::protocol::new_token);
            tx.execute("UPDATE scoped_secret_references SET cleanup_state='deleting',cleanup_nonce=?2 WHERE reference=?1",params![reference,nonce]).map_err(storage)?;
            tx.execute("UPDATE scoped_credentials SET cleanup_cursor_time=?5,cleanup_cursor_reference=?6 WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4",params![b.workspace,b.project,b.host,credential,created_at,reference]).map_err(storage)?;
            claims.push(CleanupClaim::reserved(
                Version {
                    binding: b.clone(),
                    credential: credential.into(),
                    reference,
                    revision,
                    sha256,
                },
                nonce,
            )?);
        }
        tx.commit().map_err(storage)?;
        Ok(claims)
    }
    fn cleanup_matches(&self, claim: &CleanupClaim) -> Result<bool> {
        let v = claim.version();
        let b = &v.binding;
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM scoped_secret_references r JOIN scoped_credentials c USING(workspace,project,host,credential) WHERE r.reference=?1 AND r.workspace=?2 AND r.project=?3 AND r.host=?4 AND r.credential=?5 AND r.revision=?6 AND r.sha256=?7 AND r.write_ack=1 AND r.cleanup_state='deleting' AND r.cleanup_nonce=?8 AND r.reference IS NOT c.active_reference AND r.reference IS NOT c.staged_reference)",params![v.reference,b.workspace,b.project,b.host,v.credential,v.revision as i64,v.sha256,claim.nonce()],|r|r.get(0)).map_err(storage)
    }
    pub(crate) fn scoped_secret_cleanup_check(
        &self,
        proof: &Proof,
        claim: &CleanupClaim,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<()> {
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        if claim.version().binding != proof.binding || !self.cleanup_matches(claim)? {
            return Err(Error::Denied);
        }
        tx.commit().map_err(storage)
    }
    pub(crate) fn scoped_secret_cleanup_ack(
        &self,
        proof: &Proof,
        claim: &CleanupClaim,
    ) -> Result<()> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        if claim.version().binding != proof.binding || !self.cleanup_matches(claim)? {
            return Err(Error::Conflict);
        }
        let receipt=serde_json::json!({"actor":proof.browser.user,"device":proof.browser.device,"nonce":claim.nonce(),"at":super::super::now_secs()}).to_string();
        tx.execute("UPDATE scoped_secret_references SET cleanup_state='deleted',cleanup_receipt=?2 WHERE reference=?1",params![claim.version().reference,receipt]).map_err(storage)?;
        // Metadata history is bounded independently of the live-reference quota.
        // REST never accepts references; SDK writes always use a fresh random slot.
        tx.execute("DELETE FROM scoped_secret_references WHERE reference IN (SELECT reference FROM scoped_secret_references WHERE workspace=?1 AND cleanup_state='deleted' AND write_ack=1 AND cleanup_receipt IS NOT NULL ORDER BY created_at DESC,reference DESC LIMIT -1 OFFSET 512)",[&proof.binding.workspace]).map_err(storage)?;
        tx.commit().map_err(storage)
    }
}
#[cfg(test)]
mod tests;
