//! Durable callback fence, installed before redirecting the browser.
use super::*;

pub(in crate::store) const MIGRATION: &str = "
CREATE TABLE mcp_oauth_connections_next (
    connection_id TEXT PRIMARY KEY CHECK(length(connection_id) BETWEEN 1 AND 128),
    workspace_id TEXT NOT NULL, user_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
    binding TEXT NOT NULL CHECK(length(binding)=64),
    generation INTEGER NOT NULL CHECK(generation BETWEEN 0 AND 9007199254740991),
    state TEXT NOT NULL CHECK(state IN ('authorizing','installing','active','refreshing','disabled')),
    secret_reference TEXT, sha256 TEXT CHECK(sha256 IS NULL OR length(sha256)=64),
    operation_id TEXT, staged_reference TEXT,
    provider_id TEXT CHECK(provider_id IS NULL OR length(provider_id) BETWEEN 1 AND 64),
    FOREIGN KEY(workspace_id,user_id) REFERENCES team_memberships(workspace_id,user_id),
    CHECK((state='active' AND generation>0 AND secret_reference IS NOT NULL AND sha256 IS NOT NULL AND operation_id IS NULL AND staged_reference IS NULL)
        OR (state IN ('installing','refreshing') AND operation_id IS NOT NULL AND staged_reference IS NOT NULL AND (state='installing' OR (generation>0 AND secret_reference IS NOT NULL AND sha256 IS NOT NULL)))
        OR (state IN ('authorizing','disabled') AND operation_id IS NULL AND staged_reference IS NULL))
);
CREATE TABLE mcp_oauth_secret_references_next (
    reference TEXT PRIMARY KEY, connection_id TEXT NOT NULL REFERENCES mcp_oauth_connections_next(connection_id), created_at INTEGER NOT NULL
);
INSERT INTO mcp_oauth_connections_next(connection_id,workspace_id,user_id,revision,binding,generation,state,secret_reference,sha256,operation_id,staged_reference)
    SELECT connection_id,workspace_id,user_id,revision,binding,generation,state,secret_reference,sha256,operation_id,staged_reference FROM mcp_oauth_connections;
INSERT INTO mcp_oauth_secret_references_next SELECT reference,connection_id,created_at FROM mcp_oauth_secret_references;
DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references;
DROP TABLE mcp_oauth_connections;
ALTER TABLE mcp_oauth_connections_next RENAME TO mcp_oauth_connections;
ALTER TABLE mcp_oauth_secret_references_next RENAME TO mcp_oauth_secret_references;
CREATE INDEX mcp_oauth_owner ON mcp_oauth_connections(workspace_id,user_id);
CREATE INDEX mcp_oauth_references_connection ON mcp_oauth_secret_references(connection_id);
";
impl Store {
    pub(crate) fn mcp_oauth_can_authorize(
        &self,
        context: &AuthContext,
    ) -> Result<(), PersistenceError> {
        authorize(&self.connection, context)?;
        super::revocation::positive_allowed(&self.connection, &context.connection_id)?;
        let row:Option<(String,String,u64,String)>=self.connection.query_row(
            "SELECT workspace_id,user_id,revision,state FROM mcp_oauth_connections WHERE connection_id=?1",[&context.connection_id],
            |r|Ok((r.get(0)?,r.get(1)?,super::super::positive_revision(r,2)?,r.get(3)?))).optional().map_err(storage)?;
        match row {
            None if context.connection_revision == 1 => Ok(()),
            Some((workspace, user, revision, state))
                if workspace == context.workspace_id
                    && user == context.user_id
                    && revision == context.connection_revision
                    && matches!(state.as_str(), "disabled" | "authorizing") =>
            {
                Ok(())
            }
            _ => Err(PersistenceError::Conflict),
        }
    }
    pub(crate) fn mcp_oauth_prepare(
        &self,
        context: &AuthContext,
        binding: &str,
        provider: &str,
        display_name: Option<&str>,
        check: &super::Authority<'_>,
    ) -> Result<u64, PersistenceError> {
        if !hex_digest(binding)
            || provider.is_empty()
            || provider.len() > 64
            || !provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
        {
            return Err(PersistenceError::Unauthorized);
        }
        if display_name.is_some_and(|name| {
            name.trim() != name
                || name.is_empty()
                || name.chars().count() > 120
                || name.chars().any(char::is_control)
        }) {
            return Err(PersistenceError::Unauthorized);
        }
        let tx = transaction(self, context)?;
        check(self, context)?;
        self.team_audit_capacity().map_err(|error| match error {
            super::super::StoreError::Limit(_) => PersistenceError::Limit,
            _ => PersistenceError::Storage,
        })?;
        self.mcp_oauth_can_authorize(context)?;
        let previous_provider: Option<Option<String>> = tx
            .query_row(
                "SELECT provider_id FROM mcp_oauth_connections WHERE connection_id=?1",
                [&context.connection_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        if previous_provider
            .flatten()
            .is_some_and(|old| old != provider)
        {
            return Err(PersistenceError::Conflict);
        }
        let previous: Option<String> = tx
            .query_row(
                "SELECT state FROM mcp_oauth_connections WHERE connection_id=?1",
                [&context.connection_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(storage)?;
        // A completed negative receipt belongs to the previous credential epoch.
        // A new authorization must fence stale revoke CAS requests and keep its
        // historical receipt distinguishable from the newly issued credentials.
        let has_revocation: bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM mcp_oauth_connections WHERE connection_id=?1 AND revocation IS NOT NULL)", [&context.connection_id], |r|r.get(0)).map_err(storage)?;
        let revision = if previous.as_deref() == Some("authorizing") || has_revocation {
            context
                .connection_revision
                .checked_add(1)
                .filter(|r| *r <= 9_007_199_254_740_991)
                .ok_or(PersistenceError::Limit)?
        } else {
            context.connection_revision
        };
        if let Some(name) = display_name {
            let stored: Option<Option<String>> = tx
                .query_row(
                    "SELECT display_name FROM mcp_oauth_connections WHERE connection_id=?1",
                    [&context.connection_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(storage)?;
            if stored.flatten().is_some_and(|old| old != name) {
                return Err(PersistenceError::Conflict);
            }
        }
        if previous.is_none() {
            let count:i64=tx.query_row("SELECT COUNT(*) FROM mcp_oauth_connections WHERE workspace_id=?1 AND user_id=?2",params![context.workspace_id,context.user_id],|r|r.get(0)).map_err(storage)?;
            if count >= 64 {
                return Err(PersistenceError::Limit);
            }
            tx.execute("INSERT INTO mcp_oauth_connections(connection_id,workspace_id,user_id,revision,binding,generation,state,provider_id,display_name) VALUES(?1,?2,?3,?4,?5,0,'authorizing',?6,?7)",params![context.connection_id,context.workspace_id,context.user_id,revision as i64,binding,provider,display_name]).map_err(storage)?;
        } else {
            tx.execute("UPDATE mcp_oauth_connections SET state='authorizing',revision=?2,binding=?3,provider_id=?4,display_name=COALESCE(display_name,?5) WHERE connection_id=?1",params![context.connection_id,revision as i64,binding,provider,display_name]).map_err(storage)?;
        }
        tx.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,'mcp_oauth_authorization_started',?3,?4,?5)",params![context.workspace_id,context.user_id,revision as i64,super::super::now_secs(),context.connection_id]).map_err(storage)?;
        tx.commit().map_err(storage)?;
        self.catalog_changed();
        Ok(revision)
    }
    pub(crate) fn mcp_oauth_authorizing(
        &self,
        context: &AuthContext,
        binding: &str,
    ) -> Result<(), PersistenceError> {
        authorize(&self.connection, context)?;
        let valid:bool=self.connection.query_row("SELECT EXISTS(SELECT 1 FROM mcp_oauth_connections WHERE connection_id=?1 AND workspace_id=?2 AND user_id=?3 AND revision=?4 AND binding=?5 AND state='authorizing')",params![context.connection_id,context.workspace_id,context.user_id,context.connection_revision as i64,binding],|r|r.get(0)).map_err(storage)?;
        if valid {
            Ok(())
        } else {
            Err(PersistenceError::Conflict)
        }
    }
}

impl Store {
    pub(crate) fn mcp_oauth_authorizing_checked(
        &self,
        context: &AuthContext,
        binding: &str,
        check: &super::Authority<'_>,
    ) -> Result<(), PersistenceError> {
        let tx = transaction(self, context)?;
        check(self, context)?;
        self.mcp_oauth_authorizing(context, binding)?;
        tx.commit().map_err(storage)
    }
}
