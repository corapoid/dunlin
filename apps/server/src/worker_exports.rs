//! Operator-local worker checkout authority. No controller paths, OS-user
//! impersonation or network operations are implemented here. Execution ledgers
//! remain separate from the live runtime that supplies process authority.
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};
mod checkout;
mod cli;
pub(crate) mod execution;
pub(crate) mod policy;
pub(crate) mod session;
pub(crate) use crate::checkout_identity::{CapturedCheckout, LinkedRoots, RepoKind};
const MAX_REVISION: u64 = 9_007_199_254_740_991;
const MAX_AUDIT: i64 = 4096;
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("{0}")]
    Conflict(&'static str),
    #[error("Worker export authority denied")]
    Denied,
    #[error("Worker export persistence unavailable")]
    Sql(#[from] rusqlite::Error),
    #[error("Worker export filesystem unavailable")]
    Io(#[from] std::io::Error),
    #[error("Invalid worker export record")]
    Json(#[from] serde_json::Error),
}
impl From<crate::checkout_identity::Error> for Error {
    fn from(error: crate::checkout_identity::Error) -> Self {
        match error {
            crate::checkout_identity::Error::Invalid(message) => Self::Invalid(message),
            crate::checkout_identity::Error::Conflict(message) => Self::Conflict(message),
            crate::checkout_identity::Error::Io(error) => Self::Io(error),
            crate::checkout_identity::Error::Json(error) => Self::Json(error),
        }
    }
}
pub(crate) type Result<T> = std::result::Result<T, Error>;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportScope {
    pub controller_node_id: String,
    pub worker_node_id: String,
    pub workspace_id: String,
    pub host_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
}
impl ExportScope {
    fn validate(&self) -> Result<()> {
        if ![
            &self.controller_node_id,
            &self.worker_node_id,
            &self.workspace_id,
            &self.host_id,
        ]
        .into_iter()
        .all(|v| identifier(v))
            || self.controller_node_id == self.worker_node_id
            || self.enrollment_epoch == 0
            || self.enrollment_epoch > i64::MAX as u64
            || self.credential_revision == 0
            || self.credential_revision > i64::MAX as u64
        {
            return Err(Error::Invalid("Invalid worker export scope"));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PrincipalKind {
    Human,
    Service,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ExportPrincipal {
    pub kind: PrincipalKind,
    pub id: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExportPermission {
    Read,
    Execute,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ExportSummary {
    pub id: String,
    pub name: String,
    pub repo_kind: RepoKind,
    pub checkout_identity_digest: String,
    pub settings_digest: String,
    pub revision: u64,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GrantSummary {
    pub principal: ExportPrincipal,
    pub permissions: Vec<ExportPermission>,
    pub revision: u64,
    pub export_revision: u64,
    pub active: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ExportStamp {
    pub scope: ExportScope,
    pub export: ExportSummary,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct GrantStamp {
    pub export_id: String,
    pub grant: GrantSummary,
}
/// Opaque immutable candidate; validate filesystem outside a WorkerStore lock.
pub(crate) struct ExportCandidate {
    scope: ExportScope,
    export: ExportSummary,
    grant: GrantSummary,
    permission: ExportPermission,
    checkout: CapturedCheckout,
}
pub(crate) struct ValidatedExport(ExportCandidate);
impl ValidatedExport {
    /// Negative-only runtime boundary: explicit export consent cannot mount a
    /// parent of private control state, another principal's home or credentials.
    pub(crate) fn reject_private_overlap(&self, protected: &[std::path::PathBuf]) -> Result<()> {
        let mut mounted = self.0.checkout.captured_metadata_paths();
        mounted.push(self.0.checkout.canonical_root().to_owned());
        for mount in &mounted {
            for private in protected {
                let path = match private.canonicalize() {
                    Ok(path) => path,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => private.clone(),
                    Err(error) => return Err(error.into()),
                };
                if mount.starts_with(&path) || path.starts_with(mount) {
                    return Err(Error::Denied);
                }
            }
        }
        Ok(())
    }
}

/// Not Serialize/Deserialize: a remote stamp can never construct local authority.
pub(crate) struct ExportAuthority {
    candidate: ExportCandidate,
}
impl ExportCandidate {
    pub fn validate_filesystem(self) -> Result<ValidatedExport> {
        self.checkout.revalidate()?;
        Ok(ValidatedExport(self))
    }
}
impl ExportAuthority {
    pub fn export_stamp(&self) -> ExportStamp {
        ExportStamp {
            scope: self.candidate.scope.clone(),
            export: self.candidate.export.clone(),
        }
    }
    pub fn grant_stamp(&self) -> GrantStamp {
        GrantStamp {
            export_id: self.candidate.export.id.clone(),
            grant: self.candidate.grant.clone(),
        }
    }
    pub fn validate_filesystem(&self) -> Result<()> {
        self.candidate.checkout.revalidate().map_err(Error::from)
    }
}
pub(crate) struct WorkerStore {
    connection: Connection,
    scope: ExportScope,
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
}
fn next(current: u64, positive: bool) -> Result<u64> {
    if !positive && current == MAX_REVISION {
        return Ok(current);
    }
    let next = current
        .checked_add(1)
        .filter(|v| *v <= MAX_REVISION && (!positive || *v < MAX_REVISION))
        .ok_or(Error::Conflict(
            "Worker export revision exhausted; positive authority cannot be reused",
        ))?;
    Ok(next)
}
fn revision(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(index)?)
        .ok()
        .filter(|value| (1..=MAX_REVISION).contains(value))
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn sql_revision(value: u64) -> Result<i64> {
    if !(1..=MAX_REVISION).contains(&value) {
        return Err(Error::Invalid("Invalid worker export revision"));
    }
    i64::try_from(value).map_err(|_| Error::Invalid("Invalid worker export revision"))
}
fn kind(value: PrincipalKind) -> &'static str {
    match value {
        PrincipalKind::Human => "human",
        PrincipalKind::Service => "service",
    }
}
fn decode<T: serde::de::DeserializeOwned>(raw: String) -> Result<T> {
    Ok(serde_json::from_str(&raw)?)
}
impl WorkerStore {
    pub fn open(directory: &Path, scope: ExportScope) -> Result<Self> {
        Self::open_mode(directory, scope, true)
    }
    pub fn open_existing(directory: &Path, scope: ExportScope) -> Result<Self> {
        Self::open_mode(directory, scope, false)
    }
    fn open_mode(directory: &Path, scope: ExportScope, create: bool) -> Result<Self> {
        scope.validate()?;
        let connection = checkout::open_state(directory, create)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        let tx = rusqlite::Transaction::new_unchecked(
            &connection,
            if create {
                rusqlite::TransactionBehavior::Immediate
            } else {
                rusqlite::TransactionBehavior::Deferred
            },
        )?;
        let version: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > 4 {
            return Err(Error::Conflict(
                "Worker export state requires a newer version",
            ));
        }
        if version == 0 {
            if !create {
                return Err(Error::Conflict("Worker export catalog is not initialized"));
            }
            tx.execute_batch(SCHEMA)?;
            tx.execute(
                "INSERT INTO export_scope VALUES(1,?1)",
                [serde_json::to_string(&scope)?],
            )?;
        }
        if create && version < 2 {
            tx.execute_batch(policy::SCHEMA)?;
        }
        if create && version < 3 {
            tx.execute_batch(execution::SCHEMA)?;
        }
        if create && version < 4 {
            tx.execute_batch(session::SCHEMA)?;
        }
        let stored: ExportScope = decode(tx.query_row(
            "SELECT scope FROM export_scope WHERE singleton=1",
            [],
            |r| r.get(0),
        )?)?;
        if stored != scope {
            return Err(Error::Conflict(
                "Worker export scope changed; explicit operator reapproval in a new state directory is required",
            ));
        }
        tx.commit()?;
        Ok(Self { connection, scope })
    }
    fn check_scope(&self, scope: &ExportScope) -> Result<()> {
        scope.validate()?;
        let stored: ExportScope = decode(self.connection.query_row(
            "SELECT scope FROM export_scope WHERE singleton=1",
            [],
            |r| r.get(0),
        )?)?;
        if scope != &self.scope || stored != *scope {
            return Err(Error::Denied);
        }
        Ok(())
    }
    fn row(&self, id: &str) -> Result<Option<(ExportSummary, CapturedCheckout)>> {
        self.connection.query_row("SELECT id,name,repo_kind,checkout_digest,settings_digest,revision,active,checkout FROM worker_exports WHERE id=?1",[id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,revision(r,5)?,r.get::<_,bool>(6)?,r.get::<_,String>(7)?))).optional()?.map(|(id,name,repo,checkout_identity_digest,settings_digest,revision,active,checkout)|Ok((ExportSummary{id,name,repo_kind:decode(repo)?,checkout_identity_digest,settings_digest,revision,active},CapturedCheckout::decoded(checkout)?))).transpose()
    }
    fn grant(&self, id: &str, principal: &ExportPrincipal) -> Result<Option<GrantSummary>> {
        self.connection.query_row("SELECT permissions,revision,export_revision,active FROM export_grants WHERE export_id=?1 AND principal_kind=?2 AND principal_id=?3",params![id,kind(principal.kind),principal.id],|r|Ok((r.get::<_,String>(0)?,revision(r,1)?,revision(r,2)?,r.get::<_,bool>(3)?))).optional()?.map(|(permissions,revision,export_revision,active)|Ok(GrantSummary{principal:principal.clone(),permissions:decode(permissions)?,revision,export_revision,active})).transpose()
    }
    fn audit(
        &self,
        id: &str,
        principal: Option<&ExportPrincipal>,
        action: &str,
        revision: u64,
        terminal: bool,
    ) -> Result<()> {
        let (pk, pi) = principal
            .map(|p| (kind(p.kind), p.id.as_str()))
            .unwrap_or(("export", ""));
        let revision = sql_revision(revision)?;
        if terminal {
            self.connection.execute("INSERT INTO export_revocations VALUES(?1,?2,?3,?4,unixepoch()) ON CONFLICT(export_id,principal_kind,principal_id) DO UPDATE SET revision=excluded.revision,at=excluded.at",params![id,pk,pi,revision])?;
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM export_audit", [], |r| r.get(0))?;
        if count >= MAX_AUDIT {
            if terminal {
                return Ok(());
            }
            return Err(Error::Conflict("Worker export audit capacity reached"));
        }
        self.connection.execute("INSERT INTO export_audit(export_id,principal_kind,principal_id,action,revision,at) VALUES(?1,?2,?3,?4,?5,unixepoch())",params![id,pk,pi,action,revision])?;
        Ok(())
    }
    pub fn list(&self, scope: &ExportScope) -> Result<Vec<ExportSummary>> {
        self.check_scope(scope)?;
        let mut q = self
            .connection
            .prepare("SELECT id FROM worker_exports ORDER BY id LIMIT 64")?;
        let ids = q
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| self.row(&id)?.map(|r| r.0).ok_or(Error::Denied))
            .collect()
    }
    #[cfg(test)]
    #[cfg(test)]
    pub fn register_captured(
        &self,
        scope: &ExportScope,
        name: &str,
        checkout: &CapturedCheckout,
    ) -> Result<ExportSummary> {
        self.register_captured_with_policy(scope, name, checkout, None)
    }
    pub fn register_captured_with_policy(
        &self,
        scope: &ExportScope,
        name: &str,
        checkout: &CapturedCheckout,
        policy: Option<&policy::PolicySnapshot>,
    ) -> Result<ExportSummary> {
        if let Some(policy) = policy {
            policy.validate_checkout(checkout)?;
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(Error::Invalid("Invalid export name"));
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM worker_exports", [], |r| r.get(0))?;
        if count >= 64 {
            return Err(Error::Conflict("Worker export capacity reached"));
        }
        let id = format!("export-{}", uuid::Uuid::now_v7());
        self.connection.execute(
            "INSERT INTO worker_exports VALUES(?1,?2,?3,?4,?5,1,1,?6)",
            params![
                id,
                name,
                serde_json::to_string(&checkout.repo_kind())?,
                checkout.identity_digest()?,
                checkout.settings_digest(),
                checkout.encoded()?
            ],
        )?;
        self.write_policy(&id, checkout.settings_digest(), policy)?;
        self.audit(&id, None, "registered", 1, false)?;
        let result = self.row(&id)?.ok_or(Error::Denied)?.0;
        tx.commit()?;
        Ok(result)
    }
    #[cfg(test)]
    #[cfg(test)]
    pub fn rebind_captured(
        &self,
        scope: &ExportScope,
        id: &str,
        expected: u64,
        name: &str,
        checkout: &CapturedCheckout,
    ) -> Result<ExportSummary> {
        self.rebind_captured_with_policy(scope, id, expected, name, checkout, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn rebind_captured_with_policy(
        &self,
        scope: &ExportScope,
        id: &str,
        expected: u64,
        name: &str,
        checkout: &CapturedCheckout,
        policy: Option<&policy::PolicySnapshot>,
    ) -> Result<ExportSummary> {
        if let Some(policy) = policy {
            policy.validate_checkout(checkout)?;
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let current = self.row(id)?.ok_or(Error::Denied)?.0;
        if current.revision != expected {
            return Err(Error::Conflict("Export changed; reload before rebinding"));
        }
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err(Error::Invalid("Invalid export name"));
        }
        let revision = next(expected, true)?;
        self.invalidate_grants(id)?;
        self.connection.execute("UPDATE worker_exports SET name=?2,repo_kind=?3,checkout_digest=?4,settings_digest=?5,revision=?6,active=1,checkout=?7 WHERE id=?1",params![id,name,serde_json::to_string(&checkout.repo_kind())?,checkout.identity_digest()?,checkout.settings_digest(),sql_revision(revision)?,checkout.encoded()?])?;
        self.write_policy(id, checkout.settings_digest(), policy)?;
        self.audit(id, None, "rebound", revision, false)?;
        let result = self.row(id)?.ok_or(Error::Denied)?.0;
        tx.commit()?;
        Ok(result)
    }
    pub fn grants(&self, scope: &ExportScope, id: &str) -> Result<Vec<GrantSummary>> {
        self.check_scope(scope)?;
        self.row(id)?.ok_or(Error::Denied)?;
        let mut q=self.connection.prepare("SELECT principal_kind,principal_id FROM export_grants WHERE export_id=?1 ORDER BY principal_kind,principal_id LIMIT 64")?;
        let rows = q
            .query_map([id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(k, p)| {
                self.grant(
                    id,
                    &ExportPrincipal {
                        kind: decode(format!("\"{k}\""))?,
                        id: p,
                    },
                )?
                .ok_or(Error::Denied)
            })
            .collect()
    }
    fn invalidate_grants(&self, id: &str) -> Result<()> {
        let grants = self.grants(&self.scope, id)?;
        for grant in grants.into_iter().filter(|g| g.active) {
            let revision = next(grant.revision, false)?;
            self.connection.execute("UPDATE export_grants SET active=0,revision=?4 WHERE export_id=?1 AND principal_kind=?2 AND principal_id=?3",params![id,kind(grant.principal.kind),grant.principal.id,sql_revision(revision)?])?;
            self.audit(id, Some(&grant.principal), "revoked", revision, true)?;
        }
        Ok(())
    }
    pub fn revoke_export(
        &self,
        scope: &ExportScope,
        id: &str,
        expected: u64,
    ) -> Result<ExportSummary> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let current = self.row(id)?.ok_or(Error::Denied)?.0;
        if current.revision != expected {
            return Err(Error::Conflict("Export changed; reload before revoking"));
        }
        if !current.active {
            return Ok(current);
        }
        let revision = next(expected, false)?;
        self.invalidate_grants(id)?;
        self.connection.execute(
            "UPDATE worker_exports SET active=0,revision=?2 WHERE id=?1",
            params![id, sql_revision(revision)?],
        )?;
        self.audit(id, None, "revoked", revision, true)?;
        let result = self.row(id)?.ok_or(Error::Denied)?.0;
        tx.commit()?;
        Ok(result)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn set_grant(
        &self,
        scope: &ExportScope,
        id: &str,
        principal: &ExportPrincipal,
        permissions: &[ExportPermission],
        active: bool,
        expected: Option<u64>,
    ) -> Result<GrantSummary> {
        if !identifier(&principal.id) {
            return Err(Error::Invalid("Invalid remote principal ID"));
        }
        let unique = permissions.iter().copied().collect::<BTreeSet<_>>();
        if active
            && (unique.is_empty()
                || unique.len() != permissions.len()
                || unique.contains(&ExportPermission::Execute)
                    && !unique.contains(&ExportPermission::Read))
        {
            return Err(Error::Invalid(
                "Explicit read permission is required alongside execute",
            ));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.check_scope(scope)?;
        let export = self.row(id)?.ok_or(Error::Denied)?.0;
        let previous = self.grant(id, principal)?;
        if previous.as_ref().map(|g| g.revision) != expected {
            return Err(Error::Conflict(
                "Export grant changed; reload before editing",
            ));
        }
        if let Some(old) = &previous {
            if !active && !old.active {
                return Ok(old.clone());
            }
            if active
                && old.active
                && old.export_revision == export.revision
                && old.permissions.iter().copied().collect::<BTreeSet<_>>() == unique
            {
                return Ok(old.clone());
            }
        }
        if active && !export.active {
            return Err(Error::Denied);
        }
        if previous.is_none() {
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM export_grants WHERE export_id=?1",
                [id],
                |r| r.get(0),
            )?;
            if count >= 64 {
                return Err(Error::Conflict("Export principal capacity reached"));
            }
            if !active {
                return Err(Error::Denied);
            }
        }
        let revision = next(expected.unwrap_or(0), active)?;
        let permissions = if active {
            unique.into_iter().collect::<Vec<_>>()
        } else {
            previous
                .as_ref()
                .map(|g| g.permissions.clone())
                .unwrap_or_default()
        };
        self.connection.execute("INSERT INTO export_grants VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(export_id,principal_kind,principal_id) DO UPDATE SET permissions=excluded.permissions,revision=excluded.revision,export_revision=excluded.export_revision,active=excluded.active",params![id,kind(principal.kind),principal.id,serde_json::to_string(&permissions)?,sql_revision(revision)?,sql_revision(export.revision)?,active])?;
        self.audit(
            id,
            Some(principal),
            if active { "granted" } else { "revoked" },
            revision,
            !active,
        )?;
        let result = self.grant(id, principal)?.ok_or(Error::Denied)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn candidate(
        &self,
        scope: &ExportScope,
        id: &str,
        principal: &ExportPrincipal,
        permission: ExportPermission,
    ) -> Result<ExportCandidate> {
        let tx = self.connection.unchecked_transaction()?;
        let candidate = self.candidate_snapshot(scope, id, principal, permission)?;
        tx.commit()?;
        Ok(candidate)
    }
    fn candidate_snapshot(
        &self,
        scope: &ExportScope,
        id: &str,
        principal: &ExportPrincipal,
        permission: ExportPermission,
    ) -> Result<ExportCandidate> {
        self.check_scope(scope)?;
        let (export, checkout) = self.row(id)?.ok_or(Error::Denied)?;
        let grant = self.grant(id, principal)?.ok_or(Error::Denied)?;
        if !export.active
            || !grant.active
            || grant.export_revision != export.revision
            || !grant.permissions.contains(&permission)
        {
            return Err(Error::Denied);
        }
        self.policy_snapshot(id, &export.settings_digest)?;
        Ok(ExportCandidate {
            scope: scope.clone(),
            export,
            grant,
            permission,
            checkout,
        })
    }
    pub fn authorize_validated(
        &self,
        scope: &ExportScope,
        validated: ValidatedExport,
    ) -> Result<ExportAuthority> {
        let tx = self.connection.unchecked_transaction()?;
        let candidate = validated.0;
        let current = self.candidate_snapshot(
            scope,
            &candidate.export.id,
            &candidate.grant.principal,
            candidate.permission,
        )?;
        if current.scope != candidate.scope
            || current.export != candidate.export
            || current.grant != candidate.grant
        {
            return Err(Error::Denied);
        }
        tx.commit()?;
        Ok(ExportAuthority { candidate })
    }
    pub fn revalidate_authority(
        &self,
        scope: &ExportScope,
        authority: &ExportAuthority,
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        let c = &authority.candidate;
        let current =
            self.candidate_snapshot(scope, &c.export.id, &c.grant.principal, c.permission)?;
        if current.scope != c.scope || current.export != c.export || current.grant != c.grant {
            return Err(Error::Denied);
        }
        tx.commit()?;
        Ok(())
    }
}
const SCHEMA:&str="
CREATE TABLE export_scope(singleton INTEGER PRIMARY KEY CHECK(singleton=1),scope TEXT NOT NULL);
CREATE TABLE worker_exports(id TEXT PRIMARY KEY,name TEXT NOT NULL,repo_kind TEXT NOT NULL,checkout_digest TEXT NOT NULL,settings_digest TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),active INTEGER NOT NULL CHECK(active IN(0,1)),checkout TEXT NOT NULL);
CREATE TABLE export_grants(export_id TEXT NOT NULL REFERENCES worker_exports(id),principal_kind TEXT NOT NULL CHECK(principal_kind IN('human','service')),principal_id TEXT NOT NULL,permissions TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),export_revision INTEGER NOT NULL,active INTEGER NOT NULL CHECK(active IN(0,1)),PRIMARY KEY(export_id,principal_kind,principal_id));
CREATE TABLE export_audit(seq INTEGER PRIMARY KEY,export_id TEXT NOT NULL,principal_kind TEXT NOT NULL,principal_id TEXT NOT NULL,action TEXT NOT NULL,revision INTEGER NOT NULL,at INTEGER NOT NULL);
CREATE TABLE export_revocations(export_id TEXT NOT NULL REFERENCES worker_exports(id),principal_kind TEXT NOT NULL,principal_id TEXT NOT NULL,revision INTEGER NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(export_id,principal_kind,principal_id));
PRAGMA user_version=1;
";
pub(crate) use cli::command;
#[cfg(test)]
mod tests;

#[cfg(test)]
mod checkout_error_compatibility {
    use super::Error;
    use crate::checkout_identity::Error as IdentityError;

    #[test]
    fn shared_checkout_errors_preserve_worker_variants_and_public_messages() {
        let invalid = Error::from(IdentityError::Invalid("original invalid checkout"));
        assert!(matches!(
            invalid,
            Error::Invalid("original invalid checkout")
        ));
        assert_eq!(invalid.to_string(), "original invalid checkout");
        let conflict = Error::from(IdentityError::Conflict("original checkout conflict"));
        assert!(matches!(
            conflict,
            Error::Conflict("original checkout conflict")
        ));
        assert_eq!(conflict.to_string(), "original checkout conflict");
        let io = Error::from(IdentityError::Io(std::io::Error::from(
            std::io::ErrorKind::NotFound,
        )));
        assert!(matches!(&io, Error::Io(source) if source.kind() == std::io::ErrorKind::NotFound));
        assert_eq!(io.to_string(), "Worker export filesystem unavailable");
        let malformed = serde_json::from_str::<serde_json::Value>("not-json").unwrap_err();
        let json = Error::from(IdentityError::Json(malformed));
        assert!(matches!(&json, Error::Json(_)));
        assert_eq!(json.to_string(), "Invalid worker export record");
    }
}
