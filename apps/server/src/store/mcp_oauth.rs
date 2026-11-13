//! Personal OAuth journal. SQL contains no provider credentials. Every allocated
//! secret reference is retained, even after a late vault writer loses its CAS.
use super::Store;
mod authorization;
mod retention;
mod write_receipt;
pub(super) use retention::MIGRATION as RETENTION_MIGRATION;
pub(super) use write_receipt::MIGRATION as WRITE_RECEIPT_MIGRATION;
mod revocation;
pub(super) use authorization::MIGRATION as AUTHORIZATION_MIGRATION;
use dunlin_mcp_client::oauth::{
    AuthContext,
    persistence::{CredentialIntent, CredentialJournal, CredentialRecord, PersistenceError},
};
pub(crate) use revocation::LegacyReview;
pub(super) use revocation::MIGRATION as REVOCATION_MIGRATION;
pub(crate) use revocation::WriterLease;
pub(super) use revocation::cleanup::MIGRATION as CLEANUP_MIGRATION;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

pub(super) const SCHEMA: &str = "
CREATE TABLE mcp_oauth_connections (
    connection_id TEXT PRIMARY KEY CHECK(length(connection_id) BETWEEN 1 AND 128),
    workspace_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    binding TEXT NOT NULL CHECK(length(binding)=64),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 0 AND 9007199254740991),
    state TEXT NOT NULL CHECK(state IN ('installing','active','refreshing','disabled')),
    secret_reference TEXT,
    sha256 TEXT CHECK(sha256 IS NULL OR length(sha256)=64),
    operation_id TEXT,
    staged_reference TEXT,
    FOREIGN KEY(workspace_id,user_id) REFERENCES team_memberships(workspace_id,user_id),
    CHECK((state='active' AND generation>0 AND secret_reference IS NOT NULL AND sha256 IS NOT NULL
               AND operation_id IS NULL AND staged_reference IS NULL)
        OR (state IN ('installing','refreshing') AND operation_id IS NOT NULL AND staged_reference IS NOT NULL
               AND (state='installing' OR (generation>0 AND secret_reference IS NOT NULL AND sha256 IS NOT NULL)))
        OR (state='disabled' AND operation_id IS NULL AND staged_reference IS NULL))
);
CREATE INDEX mcp_oauth_owner ON mcp_oauth_connections(workspace_id,user_id);
CREATE TABLE mcp_oauth_secret_references (
    reference TEXT PRIMARY KEY,
    connection_id TEXT NOT NULL REFERENCES mcp_oauth_connections(connection_id),
    created_at INTEGER NOT NULL
);
CREATE INDEX mcp_oauth_references_connection ON mcp_oauth_secret_references(connection_id);
";
// Unknown historical retired references deliberately remain unbound. Only the
// current committed envelope has enough trusted metadata to backfill a digest.
pub(super) const WRITE_METADATA_MIGRATION: &str = "
ALTER TABLE mcp_oauth_secret_references ADD COLUMN credential_revision INTEGER CHECK(credential_revision BETWEEN 1 AND 9007199254740991);
ALTER TABLE mcp_oauth_secret_references ADD COLUMN binding TEXT CHECK(length(binding)=64);
ALTER TABLE mcp_oauth_secret_references ADD COLUMN generation INTEGER CHECK(generation BETWEEN 1 AND 9007199254740991);
ALTER TABLE mcp_oauth_secret_references ADD COLUMN sha256 TEXT CHECK(length(sha256)=64);
UPDATE mcp_oauth_secret_references AS r SET
 credential_revision=c.revision,binding=c.binding,generation=c.generation,sha256=c.sha256
 FROM mcp_oauth_connections AS c WHERE r.connection_id=c.connection_id
 AND r.reference=c.secret_reference AND c.generation>0 AND c.sha256 IS NOT NULL
 AND c.state IN ('active','refreshing');
UPDATE mcp_oauth_secret_references AS r SET
 credential_revision=c.revision,binding=c.binding,generation=c.generation+1
 FROM mcp_oauth_connections AS c WHERE r.connection_id=c.connection_id
 AND r.reference=c.staged_reference AND c.state IN ('installing','refreshing')
 AND c.generation<9007199254740991;
";
fn storage(_: rusqlite::Error) -> PersistenceError {
    PersistenceError::Storage
}
fn hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn authorize(db: &Connection, context: &AuthContext) -> Result<(), PersistenceError> {
    context
        .validate()
        .map_err(|_| PersistenceError::Unauthorized)?;
    let active: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM team_memberships WHERE workspace_id=?1 AND user_id=?2 AND active=1)",
        params![context.workspace_id, context.user_id], |row| row.get(0)).map_err(storage)?;
    if !active {
        return Err(PersistenceError::Unauthorized);
    }
    Ok(())
}
fn transaction<'a>(
    store: &'a Store,
    context: &AuthContext,
) -> Result<Transaction<'a>, PersistenceError> {
    // The transaction prevents membership/revision changes between authorization
    // and mutation, including callers using another SQLite connection/process.
    let tx = Transaction::new_unchecked(&store.connection, TransactionBehavior::Immediate)
        .map_err(storage)?;
    authorize(&tx, context)?;
    Ok(tx)
}
fn active(
    db: &Connection,
    context: &AuthContext,
    binding: &str,
) -> Result<CredentialRecord, PersistenceError> {
    if !hex_digest(binding) {
        return Err(PersistenceError::Unauthorized);
    }
    db.query_row("SELECT generation,secret_reference,sha256 FROM mcp_oauth_connections
        WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND binding=?5 AND state='active'",
        params![context.connection_id, context.workspace_id, context.user_id, context.connection_revision as i64, binding],
        |r| Ok(CredentialRecord { generation: super::positive_revision(r, 0)?, secret_reference: r.get(1)?, sha256: r.get(2)? }))
        .optional().map_err(storage)?.ok_or(PersistenceError::Conflict)
}
fn reserve(
    tx: &Transaction<'_>,
    context: &AuthContext,
    generation: u64,
    binding: &str,
    writer_protected: bool,
) -> Result<CredentialIntent, PersistenceError> {
    if generation > 9_007_199_254_740_991 {
        return Err(PersistenceError::Limit);
    }
    let count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE connection_id=?1 AND retired=0",
            [&context.connection_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    // Retained tombstones have a separate bound; only cleanup can retire rows.
    if count >= 256 {
        return Err(PersistenceError::Limit);
    }
    let total: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM mcp_oauth_secret_references WHERE connection_id=?1",
            [&context.connection_id],
            |r| r.get(0),
        )
        .map_err(storage)?;
    if total >= retention::MAX_REFERENCES as i64 {
        return Err(PersistenceError::Limit);
    }
    let queued_revision = revocation::cleanup::revision(tx, &context.connection_id)?;
    let intent = CredentialIntent {
        generation,
        operation_id: uuid::Uuid::now_v7().to_string(),
        staged_reference: format!("mcp-oauth:{}", uuid::Uuid::now_v7()),
    };
    tx.execute("INSERT INTO mcp_oauth_secret_references(reference,connection_id,created_at,credential_revision,binding,generation,writer_protected,write_operation_id,cleanup_attempted_revision) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
        params![intent.staged_reference, context.connection_id, super::now_secs(), context.connection_revision as i64, binding, generation as i64, writer_protected, intent.operation_id,queued_revision as i64]).map_err(storage)?;
    Ok(intent)
}
pub(crate) type Authority<'a> = dyn Fn(&Store, &AuthContext) -> Result<(), PersistenceError> + 'a;
struct CheckedJournal<'a> {
    store: &'a Store,
    check: Option<&'a Authority<'a>>,
}
impl std::ops::Deref for CheckedJournal<'_> {
    type Target = Store;
    fn deref(&self) -> &Store {
        self.store
    }
}
impl CheckedJournal<'_> {
    fn transaction(&self, context: &AuthContext) -> Result<Transaction<'_>, PersistenceError> {
        let tx = transaction(self.store, context)?;
        if let Some(check) = self.check {
            check(self.store, context)?;
        }
        Ok(tx)
    }
}
impl Store {
    pub(crate) fn mcp_oauth_journal<'a>(
        &'a self,
        check: &'a Authority<'a>,
    ) -> impl CredentialJournal
    + dunlin_mcp_client::oauth::persistence::RevocationJournal
    + dunlin_mcp_client::oauth::persistence::CleanupJournal
    + 'a {
        CheckedJournal {
            store: self,
            check: Some(check),
        }
    }
}
impl CredentialJournal for Store {
    fn active(&self, c: &AuthContext, b: &str) -> Result<CredentialRecord, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .active(c, b)
    }
    fn begin_install(
        &self,
        c: &AuthContext,
        b: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .begin_install(c, b)
    }
    fn begin_refresh(
        &self,
        c: &AuthContext,
        b: &str,
        r: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .begin_refresh(c, b, r)
    }
    fn prepare_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .prepare_write(c, b, i, h)
    }
    fn claim_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .claim_write(c, b, i, h)
    }
    fn acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        // Completion is not a positive permission and must survive revocation.
        self.mcp_oauth_acknowledge_write(c, b, i, h)
    }
    fn commit(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .commit(c, b, i, h)
    }
    fn disable(&self, c: &AuthContext) -> Result<u64, PersistenceError> {
        CheckedJournal {
            store: self,
            check: None,
        }
        .disable(c)
    }
}
impl CredentialJournal for CheckedJournal<'_> {
    fn claim_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.claim_credential_write(c, b, i, h)
    }
    fn acknowledge_write(
        &self,
        c: &AuthContext,
        b: &str,
        i: &CredentialIntent,
        h: &str,
    ) -> Result<(), PersistenceError> {
        self.store.mcp_oauth_acknowledge_write(c, b, i, h)
    }
    fn active(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialRecord, PersistenceError> {
        // Keep external session checks and the read in one SQLite cut.
        let _authority = self.check.map(|_| self.transaction(context)).transpose()?;
        // A single joined read avoids a membership-removal race between reads.
        context
            .validate()
            .map_err(|_| PersistenceError::Unauthorized)?;
        if !hex_digest(binding) {
            return Err(PersistenceError::Unauthorized);
        }
        self.connection
            .query_row(
                "SELECT c.generation,c.secret_reference,c.sha256 FROM mcp_oauth_connections c
            JOIN team_memberships m ON m.workspace_id=c.workspace_id AND m.user_id=c.user_id
            WHERE c.connection_id=?1 AND c.workspace_id=?2 AND c.user_id=?3 AND c.revision=?4
            AND c.binding=?5 AND c.state='active' AND m.active=1",
                params![
                    context.connection_id,
                    context.workspace_id,
                    context.user_id,
                    context.connection_revision as i64,
                    binding
                ],
                |r| {
                    Ok(CredentialRecord {
                        generation: super::positive_revision(r, 0)?,
                        secret_reference: r.get(1)?,
                        sha256: r.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(storage)?
            .ok_or(PersistenceError::Unauthorized)
    }
    fn begin_install(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<CredentialIntent, PersistenceError> {
        if !hex_digest(binding) {
            return Err(PersistenceError::Unauthorized);
        }
        let tx = self.transaction(context)?;
        revocation::positive_allowed(&tx, &context.connection_id)?;
        let row: Option<(String, String, u64, i64, String)> = tx.query_row(
            "SELECT workspace_id,user_id,revision,generation,state FROM mcp_oauth_connections WHERE connection_id=?1",
            [&context.connection_id], |r| Ok((r.get(0)?,r.get(1)?,super::positive_revision(r,2)?,r.get(3)?,r.get(4)?))).optional().map_err(storage)?;
        let generation = if let Some((workspace, user, revision, generation, state)) = row {
            if workspace != context.workspace_id
                || user != context.user_id
                || revision != context.connection_revision
                || !matches!(state.as_str(), "disabled" | "authorizing")
            {
                return Err(PersistenceError::Conflict);
            }
            generation
        } else {
            if context.connection_revision != 1 {
                return Err(PersistenceError::Conflict);
            }
            let count: i64 = tx.query_row("SELECT COUNT(*) FROM mcp_oauth_connections WHERE workspace_id=?1 AND user_id=?2",
                params![context.workspace_id, context.user_id], |r| r.get(0)).map_err(storage)?;
            if count >= 64 {
                return Err(PersistenceError::Limit);
            }
            // Disabled placeholder remains inside this transaction only.
            tx.execute("INSERT INTO mcp_oauth_connections(connection_id,workspace_id,user_id,revision,binding,generation,state)
                VALUES(?1,?2,?3,1,?4,0,'disabled')", params![context.connection_id,context.workspace_id,context.user_id,binding]).map_err(storage)?;
            0
        };
        let authorizing_binding: Option<String> = tx.query_row("SELECT binding FROM mcp_oauth_connections WHERE connection_id=?1 AND state='authorizing'", [&context.connection_id], |r|r.get(0)).optional().map_err(storage)?;
        if authorizing_binding
            .as_deref()
            .is_some_and(|stored| stored != binding)
        {
            return Err(PersistenceError::Conflict);
        }
        let intent = reserve(
            &tx,
            context,
            u64::try_from(generation).map_err(|_| PersistenceError::Storage)? + 1,
            binding,
            self.oauth_writer_owner
                .load(std::sync::atomic::Ordering::Acquire),
        )?;
        tx.execute("UPDATE mcp_oauth_connections SET state='installing',binding=?2,operation_id=?3,staged_reference=?4
            WHERE connection_id=?1", params![context.connection_id,binding,intent.operation_id,intent.staged_reference]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(intent)
    }
    fn begin_refresh(
        &self,
        context: &AuthContext,
        binding: &str,
        expected: &CredentialRecord,
    ) -> Result<CredentialIntent, PersistenceError> {
        let tx = self.transaction(context)?;
        if active(&tx, context, binding)? != *expected {
            return Err(PersistenceError::Conflict);
        }
        let intent = reserve(
            &tx,
            context,
            expected.generation + 1,
            binding,
            self.oauth_writer_owner
                .load(std::sync::atomic::Ordering::Acquire),
        )?;
        tx.execute("UPDATE mcp_oauth_connections SET state='refreshing',operation_id=?2,staged_reference=?3 WHERE connection_id=?1",
            params![context.connection_id,intent.operation_id,intent.staged_reference]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        Ok(intent)
    }
    fn prepare_write(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError> {
        if !hex_digest(binding)
            || !hex_digest(sha256)
            || intent.generation == 0
            || intent.generation > 9_007_199_254_740_991
        {
            return Err(PersistenceError::Conflict);
        }
        let tx = self.transaction(context)?;
        let changed = tx.execute("UPDATE mcp_oauth_secret_references SET sha256=?7
            WHERE reference=?10 AND connection_id=?1 AND credential_revision=?4 AND binding=?5 AND generation=?6
            AND write_claimed=0 AND write_operation_id=?9 AND (sha256 IS NULL OR sha256=?7)
            AND EXISTS(SELECT 1 FROM mcp_oauth_connections c WHERE c.connection_id=?1
              AND c.workspace_id=?2 AND c.user_id=?3 AND c.revision=?4 AND c.binding=?5
              AND c.generation=?8 AND c.operation_id=?9 AND c.staged_reference=?10
              AND c.state IN ('installing','refreshing'))",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64,binding,
                intent.generation as i64,sha256,(intent.generation-1) as i64,intent.operation_id,intent.staged_reference]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        tx.commit().map_err(storage)
    }
    fn commit(
        &self,
        context: &AuthContext,
        binding: &str,
        intent: &CredentialIntent,
        sha256: &str,
    ) -> Result<(), PersistenceError> {
        if !hex_digest(binding)
            || !hex_digest(sha256)
            || intent.generation == 0
            || intent.generation > 9_007_199_254_740_991
        {
            return Err(PersistenceError::Conflict);
        }
        let tx = self.transaction(context)?;
        let changed = tx.execute("UPDATE mcp_oauth_connections SET state='active',generation=?6,
            secret_reference=staged_reference,sha256=?7,operation_id=NULL,staged_reference=NULL
            WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND binding=?5
            AND generation=?8 AND operation_id=?9 AND staged_reference=?10 AND state IN ('installing','refreshing')
            AND EXISTS(SELECT 1 FROM mcp_oauth_secret_references r WHERE r.reference=?10 AND r.connection_id=?1
                AND r.credential_revision=?4 AND r.binding=?5 AND r.generation=?6 AND r.sha256=?7
                AND (r.writer_protected=0 OR (r.write_claimed=1 AND r.write_acknowledged=1 AND ?11=1)))",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64,binding,
                intent.generation as i64,sha256,(intent.generation-1) as i64,intent.operation_id,intent.staged_reference,
                self.oauth_writer_owner.load(std::sync::atomic::Ordering::Acquire)]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        tx.commit().map_err(storage)
    }
    fn disable(&self, context: &AuthContext) -> Result<u64, PersistenceError> {
        let tx = self.transaction(context)?;
        if context.connection_revision >= 9_007_199_254_740_991 {
            return Err(PersistenceError::Limit);
        }
        let changed = tx.execute("UPDATE mcp_oauth_connections SET state='disabled',revision=revision+1,operation_id=NULL,staged_reference=NULL
            WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4",
            params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(PersistenceError::Conflict);
        }
        tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id)
            VALUES(?1,?2,?2,'mcp_oauth_disconnected',?3,?4,?5)",
            params![context.workspace_id,context.user_id,(context.connection_revision+1) as i64,super::now_secs(),context.connection_id]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(context.connection_revision + 1)
    }
}

#[cfg(test)]
mod tests;

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ConnectionStatus {
    Connected,
    AuthorizationIncomplete,
    ReauthorizationRequired,
    Disabled,
}
#[derive(serde::Serialize)]
pub(crate) struct OAuthConnection {
    pub id: String,
    pub revision: u64,
    pub status: ConnectionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revocation: Option<revocation::RevocationReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cleanup: Option<revocation::cleanup::CleanupReceipt>,
    pub storage: retention::Storage,
}
impl Store {
    /// Personal connection metadata only. A workspace administrator cannot read
    /// another member's connection or its secret references through this API.
    pub(crate) fn mcp_oauth_connection(
        &self,
        user: &str,
        workspace: &str,
        connection: &str,
    ) -> Result<Option<OAuthConnection>, PersistenceError> {
        AuthContext {
            user_id: user.into(),
            workspace_id: workspace.into(),
            connection_id: connection.into(),
            connection_revision: 1,
        }
        .validate()
        .map_err(|_| PersistenceError::Unauthorized)?;
        self.connection
            .query_row(
                "SELECT c.connection_id,c.revision,c.state,c.provider_id,c.display_name,c.revocation,c.cleanup FROM mcp_oauth_connections c
            JOIN team_memberships m ON m.workspace_id=c.workspace_id AND m.user_id=c.user_id
            WHERE c.connection_id=?1 AND c.workspace_id=?2 AND c.user_id=?3 AND m.active=1",
                params![connection, workspace, user],
                |row| {
                    let status = match row.get::<_, String>(2)?.as_str() {
                        "active" => ConnectionStatus::Connected,
                        "authorizing" | "installing" => ConnectionStatus::AuthorizationIncomplete,
                        "refreshing" => ConnectionStatus::ReauthorizationRequired,
                        "disabled" => ConnectionStatus::Disabled,
                        _ => return Err(rusqlite::Error::InvalidQuery),
                    };
                    Ok(OAuthConnection {
                        id: row.get(0)?,
                        revision: super::positive_revision(row, 1)?,
                        status,
                        provider_id: row.get(3)?,
                        display_name: row.get(4)?,
                        revocation: row.get::<_, Option<String>>(5)?.map(|v| revocation::receipt(&v).map_err(|_| rusqlite::Error::InvalidQuery)).transpose()?,
                        cleanup: row.get::<_,Option<String>>(6)?.map(|value| revocation::cleanup::receipt(&value).map_err(|_|rusqlite::Error::InvalidQuery)).transpose()?,
                        storage:retention::storage(&self.connection,connection).map_err(|_|rusqlite::Error::InvalidQuery)?,
                    })
                },
            )
            .optional()
            .map_err(storage)
    }
}

impl Store {
    pub(crate) fn mcp_oauth_connections(
        &self,
        user: &str,
        workspace: &str,
    ) -> Result<Vec<OAuthConnection>, PersistenceError> {
        authorize(
            &self.connection,
            &AuthContext {
                user_id: user.into(),
                workspace_id: workspace.into(),
                connection_id: "catalog".into(),
                connection_revision: 1,
            },
        )?;
        let mut query=self.connection.prepare("SELECT connection_id FROM mcp_oauth_connections WHERE user_id=?1 AND workspace_id=?2 ORDER BY connection_id DESC LIMIT 64").map_err(storage)?;
        let ids = query
            .query_map(params![user, workspace], |r| r.get::<_, String>(0))
            .map_err(storage)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(storage)?;
        ids.into_iter()
            .map(|id| self.mcp_oauth_connection(user, workspace, &id))
            .filter_map(|row| match row {
                Ok(None) => None,
                Ok(Some(row)) => Some(Ok(row)),
                Err(error) => Some(Err(error)),
            })
            .collect()
    }
}
