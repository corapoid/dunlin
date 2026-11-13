//! Operator-attested shutdown of pre-lease workers, never an inferred stop proof.
use super::*;
use crate::mcp_oauth::maintenance::StoppedLegacyRelays;
use sha2::{Digest, Sha256};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Recovery {
    actor: String,
    operator_uid: u32,
    change_id: String,
    reviewed_sha256: String,
    previous_revision: u64,
    original_source_revision: u64,
    // This records an operator declaration, not a kernel or AS acknowledgement.
    operator_attested_shutdown: bool,
}

#[derive(Serialize)]
pub(crate) struct LegacyReview {
    pub workspace_id: String,
    pub connection_id: String,
    pub owner_user_id: String,
    pub actor_user_id: String,
    pub revision: u64,
    pub review_sha256: String,
    pub confirmed: usize,
    pub unconfirmed: usize,
    pub requires_operator_attested_shutdown: bool,
}

fn database_scope(db: &Connection) -> Result<serde_json::Value, PersistenceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let path = db
            .path()
            .filter(|p| !p.is_empty())
            .ok_or(PersistenceError::Storage)?;
        let path = std::path::Path::new(path)
            .canonicalize()
            .map_err(|_| PersistenceError::Storage)?;
        let meta = std::fs::metadata(&path).map_err(|_| PersistenceError::Storage)?;
        if !meta.is_file()
            || meta.nlink() != 1
            || meta.uid() != unsafe { libc::geteuid() }
            || meta.mode() & 0o022 != 0
        {
            return Err(PersistenceError::Storage);
        }
        Ok(serde_json::json!([
            path.to_str().ok_or(PersistenceError::Storage)?,
            meta.dev(),
            meta.ino(),
            meta.uid()
        ]))
    }
    #[cfg(not(unix))]
    {
        let _ = db;
        Err(PersistenceError::Storage)
    }
}

fn review(
    db: &Connection,
    actor: &str,
    workspace: &str,
    connection: &str,
) -> Result<(LegacyReview, Ledger), PersistenceError> {
    let actor_revision:u64=db.query_row("SELECT revision FROM team_memberships WHERE workspace_id=?1 AND user_id=?2 AND active=1 AND role IN ('owner','admin')",params![workspace,actor],|r|super::super::super::positive_revision(r,0)).optional().map_err(storage)?.ok_or(PersistenceError::Unauthorized)?;
    let (owner,revision,raw):(String,u64,String)=db.query_row("SELECT user_id,revision,revocation FROM mcp_oauth_connections WHERE workspace_id=?1 AND connection_id=?2 AND state='disabled' AND revocation IS NOT NULL",params![workspace,connection],|r|Ok((r.get(0)?,super::super::super::positive_revision(r,1)?,r.get(2)?))).optional().map_err(storage)?.ok_or(PersistenceError::Conflict)?;
    AuthContext {
        user_id: owner.clone(),
        workspace_id: workspace.into(),
        connection_id: connection.into(),
        connection_revision: revision,
    }
    .validate()
    .map_err(|_| PersistenceError::Storage)?;
    let ledger = decode(&raw)?;
    if ledger.state != JobState::Running
        || ledger.lease_protected
        || ledger.operator_recovery.is_some()
        || ledger.source_revision == 0
        || ledger.source_revision.checked_add(1) != Some(revision)
        || ledger.disabled_revision != revision
        || revision >= 9_007_199_254_740_991
    {
        return Err(PersistenceError::Conflict);
    }
    let scope = database_scope(db)?;
    let mut hash = Sha256::new();
    hash.update(b"dunlin.mcp.legacy-review.v1\0");
    hash.update(
        serde_json::to_vec(&serde_json::json!([
            scope,
            actor,
            actor_revision,
            workspace,
            connection,
            owner,
            revision,
            raw
        ]))
        .map_err(|_| PersistenceError::Storage)?,
    );
    let summary = ledger.summary();
    Ok((
        LegacyReview {
            workspace_id: workspace.into(),
            connection_id: connection.into(),
            owner_user_id: owner,
            actor_user_id: actor.into(),
            revision,
            review_sha256: hex::encode(hash.finalize()),
            confirmed: summary.confirmed,
            unconfirmed: summary.unconfirmed,
            requires_operator_attested_shutdown: true,
        },
        ledger,
    ))
}

impl Store {
    /// Existing current-schema database only. Never creates a file or migrates it.
    pub(crate) fn open_mcp_legacy(
        path: &std::path::Path,
        write: bool,
    ) -> Result<Self, PersistenceError> {
        let flags = if write {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
        } else {
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        };
        let connection =
            Connection::open_with_flags(path, flags | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX)
                .map_err(storage)?;
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(storage)?;
        if version != crate::store::SCHEMA_VERSION {
            return Err(PersistenceError::Conflict);
        }
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(storage)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage)?;
        connection
            .busy_timeout(std::time::Duration::from_secs(1))
            .map_err(storage)?;
        database_scope(&connection)?;
        Ok(Self {
            connection,
            oauth_revocation_owner: Default::default(),
            oauth_writer_owner: Default::default(),
            catalog_changes: tokio::sync::broadcast::channel(64).0,
            interactive_changes: tokio::sync::broadcast::channel(64).0,
        })
    }
    pub(crate) fn mcp_oauth_review_legacy(
        &self,
        actor: &str,
        workspace: &str,
        connection: &str,
    ) -> Result<LegacyReview, PersistenceError> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Deferred)
            .map_err(storage)?;
        let result = review(&tx, actor, workspace, connection)?.0;
        tx.commit().map_err(storage)?;
        Ok(result)
    }
    /// No HTTP, secret-store I/O, write receipts, or inferred confirmations.
    pub(crate) fn mcp_oauth_recover_legacy(
        &self,
        workspace: &str,
        connection: &str,
        expected: u64,
        sha: &str,
        stopped: &StoppedLegacyRelays,
    ) -> Result<RevocationSummary, PersistenceError> {
        if !hex_digest(sha) {
            return Err(PersistenceError::Conflict);
        }
        // These exclude modern workers only. The separate operator declaration
        // is indispensable for old binaries which never participated in locks.
        let _revocation = self.mcp_oauth_revocation_lease()?;
        let _writer = self.mcp_oauth_writer_lease()?;
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        let (snapshot, mut ledger) = review(&tx, stopped.actor(), workspace, connection)?;
        if snapshot.revision != expected || snapshot.review_sha256 != sha {
            return Err(PersistenceError::Conflict);
        }
        ledger.operator_recovery = Some(Recovery {
            actor: stopped.actor().into(),
            operator_uid: stopped.uid(),
            change_id: stopped.change_id().into(),
            reviewed_sha256: sha.into(),
            previous_revision: expected,
            original_source_revision: ledger.source_revision,
            operator_attested_shutdown: true,
        });
        ledger.id = uuid::Uuid::now_v7().to_string();
        ledger.source_revision = expected;
        ledger.disabled_revision = expected + 1;
        let summary = ledger.summary();
        ledger.state = if summary.confirmed_known_tokens {
            JobState::Confirmed
        } else {
            JobState::Unconfirmed
        };
        let changed=tx.execute("UPDATE mcp_oauth_connections SET revision=?4 WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?5 AND state='disabled'",params![connection,workspace,snapshot.owner_user_id,ledger.disabled_revision as i64,expected as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        save(
            &tx,
            &AuthContext {
                user_id: snapshot.owner_user_id.clone(),
                workspace_id: workspace.into(),
                connection_id: connection.into(),
                connection_revision: expected + 1,
            },
            &ledger,
        )?;
        match self.team_audit_capacity() {
            Ok(()) => {
                tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,'mcp_oauth_legacy_operator_recovery',?4,?5,?6)",params![workspace,stopped.actor(),snapshot.owner_user_id,expected as i64+1,crate::store::now_secs(),connection]).map_err(storage)?;
            }
            Err(crate::store::StoreError::Limit(_)) => {}
            Err(_) => return Err(PersistenceError::Storage),
        }
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(summary)
    }
}
