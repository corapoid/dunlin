//! Provisional worker registry. Enrollment proves possession of a ticket only;
//! no network membership, execution admission or user authority is implied.
use super::{Result, Store, StoreError, auth, now_secs};
use dunlin_domain::{
    collaboration::{Permission, PolicyError},
    hosts::{HostCapabilities, HostDefinition, HostStatus},
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
pub(crate) const HOST_SCHEMA:&str="
CREATE TABLE team_hosts (
 id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
 node_id TEXT NOT NULL, serve_origin TEXT NOT NULL, definition TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('pending','claimed','approved','revoked')),
 revision INTEGER NOT NULL CHECK(revision>0), enrollment_epoch INTEGER NOT NULL CHECK(enrollment_epoch>0),
 credential_revision INTEGER NOT NULL CHECK(credential_revision>0), capability_revision INTEGER NOT NULL DEFAULT 0 CHECK(capability_revision>=0), drain INTEGER NOT NULL CHECK(drain IN (0,1)),
 capabilities TEXT, last_seen INTEGER, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
 ticket_hash TEXT UNIQUE, ticket_expires INTEGER, credential_hash TEXT UNIQUE,
 UNIQUE(workspace_id,node_id), UNIQUE(workspace_id,serve_origin)
);
CREATE TABLE team_host_revocations (
 host_id TEXT PRIMARY KEY REFERENCES team_hosts(id),
 workspace_id TEXT NOT NULL REFERENCES team_workspaces(id),
 actor_id TEXT NOT NULL REFERENCES users(id),
 revision INTEGER NOT NULL CHECK(revision>0),
 enrollment_epoch INTEGER NOT NULL CHECK(enrollment_epoch>0),
 credential_revision INTEGER NOT NULL CHECK(credential_revision>0),
 created_at INTEGER NOT NULL
);
";
/// Latest terminal receipt, bounded to one row per host across enrollment cycles.
/// Earlier revocations remain in ordinary audit until its bounded capacity fills.
#[derive(Clone, Debug, Serialize)]
pub struct HostRevocation {
    pub host_id: String,
    pub workspace_id: String,
    pub actor_id: String,
    pub revision: u64,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub created_at: i64,
}
#[derive(Clone, Debug, Serialize)]
pub struct Host {
    pub id: String,
    pub workspace_id: String,
    pub definition: HostDefinition,
    pub status: HostStatus,
    pub revision: u64,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub capability_revision: u64,
    pub drain: bool,
    pub capabilities: Option<HostCapabilities>,
    pub last_seen: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub network_verification: &'static str,
    pub execution_ready: bool,
}
#[derive(Serialize)]
pub struct HostEnrollment {
    pub host: Host,
    pub enrollment: EnrollmentTicket,
}
#[derive(Serialize)]
pub struct EnrollmentTicket {
    pub ticket: String,
    pub expires_at: i64,
}
#[derive(Serialize)]
pub struct WorkerCredential {
    pub host_id: String,
    pub workspace_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
    pub credential: String,
}
struct RawHost {
    id: String,
    workspace: String,
    definition: String,
    status: String,
    revision: u64,
    epoch: u64,
    credential_revision: u64,
    capability_revision: u64,
    drain: bool,
    capabilities: Option<String>,
    last_seen: Option<i64>,
    created: i64,
    updated: i64,
}
const COLUMNS: &str = "id,workspace_id,definition,status,revision,enrollment_epoch,credential_revision,capability_revision,drain,capabilities,last_seen,created_at,updated_at";
fn raw(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawHost> {
    Ok(RawHost {
        id: row.get(0)?,
        workspace: row.get(1)?,
        definition: row.get(2)?,
        status: row.get(3)?,
        revision: super::positive_revision(row, 4)?,
        epoch: super::positive_revision(row, 5)?,
        credential_revision: super::positive_revision(row, 6)?,
        capability_revision: u64::try_from(row.get::<_, i64>(7)?)
            .map_err(|_| rusqlite::Error::InvalidQuery)?,
        drain: row.get(8)?,
        capabilities: row.get(9)?,
        last_seen: row.get(10)?,
        created: row.get(11)?,
        updated: row.get(12)?,
    })
}
fn decoded(row: RawHost) -> Result<Host> {
    Ok(Host {
        id: row.id,
        workspace_id: row.workspace,
        definition: serde_json::from_str(&row.definition)?,
        status: serde_json::from_value(serde_json::Value::String(row.status))?,
        revision: row.revision,
        enrollment_epoch: row.epoch,
        credential_revision: row.credential_revision,
        capability_revision: row.capability_revision,
        drain: row.drain,
        capabilities: row
            .capabilities
            .map(|raw| serde_json::from_str(&raw))
            .transpose()?,
        last_seen: row.last_seen,
        created_at: row.created,
        updated_at: row.updated,
        network_verification: "unverified",
        execution_ready: false,
    })
}
fn denied() -> StoreError {
    PolicyError::PermissionDenied.into()
}
fn increment(value: u64) -> Result<u64> {
    value
        .checked_add(1)
        // Reserve the final signed SQLite value for terminal revocation.
        .filter(|v| *v < i64::MAX as u64)
        .ok_or_else(|| StoreError::Limit("Host revision limit reached".into()))
}
fn terminal_increment(value: u64) -> u64 {
    // Also fence a legacy/corrupted nonterminal row already at MAX: clearing its
    // credential is more important than advancing an exhausted counter.
    value.saturating_add(1).min(i64::MAX as u64)
}
fn capability_increment(value: u64) -> Result<u64> {
    value
        .checked_add(1)
        .filter(|v| *v <= i64::MAX as u64)
        .ok_or_else(|| StoreError::Limit("Host capability revision limit reached".into()))
}
fn secret(prefix: &str) -> String {
    auth::random_token().replacen("kst_", prefix, 1)
}
fn valid_secret(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|tail| {
        tail.len() == 43
            && tail
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    })
}
fn definition_json(definition: &HostDefinition) -> Result<String> {
    definition
        .validate()
        .map_err(|e| StoreError::Limit(format!("{}: {}", e.path, e.message)))?;
    let raw = serde_json::to_string(definition)?;
    if raw.len() > dunlin_domain::hosts::MAX_HOST_DEFINITION_BYTES {
        return Err(StoreError::Limit("Host definition is too large".into()));
    }
    Ok(raw)
}
fn capabilities_json(capabilities: &HostCapabilities) -> Result<String> {
    capabilities
        .validate()
        .map_err(|e| StoreError::Limit(format!("{}: {}", e.path, e.message)))?;
    let raw = serde_json::to_string(capabilities)?;
    if raw.len() > dunlin_domain::hosts::MAX_HOST_CAPABILITIES_BYTES {
        return Err(StoreError::Limit("Host report is too large".into()));
    }
    Ok(raw)
}
impl Store {
    pub(super) fn worker_host_by_credential(&self, hash: &str) -> Result<Option<Host>> {
        self.connection.query_row(
            &format!("SELECT {COLUMNS} FROM team_hosts WHERE credential_hash=?1 AND status='approved'"),
            [hash],raw,
        ).optional()?.map(decoded).transpose()
    }
    pub(super) fn worker_host_row(
        &self,
        id: &str,
        hash: &str,
        epoch: u64,
        credential_revision: u64,
    ) -> Result<Option<Host>> {
        let epoch = i64::try_from(epoch).map_err(|_| denied())?;
        let credential_revision = i64::try_from(credential_revision).map_err(|_| denied())?;
        self.connection.query_row(
            &format!("SELECT {COLUMNS} FROM team_hosts WHERE id=?1 AND credential_hash=?2 AND enrollment_epoch=?3 AND credential_revision=?4 AND status='approved'"),
            params![id,hash,epoch,credential_revision],raw,
        ).optional()?.map(decoded).transpose()
    }
    fn host_manage(&self, actor: &str, workspace: &str) -> Result<()> {
        self.team_context(workspace, actor)?
            .authorize_workspace(workspace, Permission::ManageHosts)?;
        Ok(())
    }
    fn host_row(&self, workspace: &str, id: &str) -> Result<Option<Host>> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM team_hosts WHERE workspace_id=?1 AND id=?2"),
                params![workspace, id],
                raw,
            )
            .optional()?
            .map(decoded)
            .transpose()
    }
    fn host_cas(&self, workspace: &str, id: &str, expected: u64) -> Result<Host> {
        self.host_row(workspace, id)?
            .filter(|host| host.revision == expected)
            .ok_or_else(|| {
                StoreError::Conflict("Host changed or is unavailable; reload before editing".into())
            })
    }
    fn host_audit(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        action: &str,
        revision: u64,
    ) -> Result<()> {
        self.team_audit_capacity()?;
        self.connection.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,?3,?4,?5,?6)",params![workspace,actor,action,revision as i64,now_secs(),id])?;
        Ok(())
    }
    pub fn hosts(&self, actor: &str, workspace: &str) -> Result<Vec<Host>> {
        self.host_manage(actor, workspace)?;
        let mut stmt = self.connection.prepare(&format!(
            "SELECT {COLUMNS} FROM team_hosts WHERE workspace_id=?1 ORDER BY created_at,id LIMIT 64"
        ))?;
        let hosts = stmt
            .query_map([workspace], raw)?
            .map(|row| decoded(row?))
            .collect::<Result<Vec<_>>>()?;
        Ok(hosts)
    }
    pub fn host(&self, actor: &str, workspace: &str, id: &str) -> Result<Option<Host>> {
        self.host_manage(actor, workspace)?;
        self.host_row(workspace, id)
    }
    pub fn host_revocation(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
    ) -> Result<Option<HostRevocation>> {
        self.host_manage(actor, workspace)?;
        self.connection.query_row(
            "SELECT host_id,workspace_id,actor_id,revision,enrollment_epoch,credential_revision,created_at FROM team_host_revocations WHERE workspace_id=?1 AND host_id=?2",
            params![workspace,id], |row| Ok(HostRevocation {
                host_id:row.get(0)?,workspace_id:row.get(1)?,actor_id:row.get(2)?,
                revision:super::positive_revision(row,3)?,enrollment_epoch:super::positive_revision(row,4)?,
                credential_revision:super::positive_revision(row,5)?,created_at:row.get(6)?,
            }),
        ).optional().map_err(Into::into)
    }
    pub fn create_host(
        &self,
        actor: &str,
        workspace: &str,
        definition: HostDefinition,
    ) -> Result<HostEnrollment> {
        let tx = self.connection.unchecked_transaction()?;
        self.host_manage(actor, workspace)?;
        let definition_raw = definition_json(&definition)?;
        let count: i64 = tx.query_row(
            "SELECT count(*) FROM team_hosts WHERE workspace_id=?1",
            [workspace],
            |r| r.get(0),
        )?;
        if count >= 64 {
            return Err(StoreError::Limit("Workspace host limit reached".into()));
        }
        let duplicate:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM team_hosts WHERE workspace_id=?1 AND (node_id=?2 OR serve_origin=?3))",params![workspace,definition.node_id,definition.serve_origin],|r|r.get(0))?;
        if duplicate {
            return Err(StoreError::Conflict(
                "Host node or Serve origin is already registered; use its enrollment rotation"
                    .into(),
            ));
        }
        let id = format!("host-{}", uuid::Uuid::now_v7());
        let now = now_secs();
        let ticket = secret("dhe_");
        let expires_at = now + 600;
        tx.execute("INSERT INTO team_hosts(id,workspace_id,node_id,serve_origin,definition,status,revision,enrollment_epoch,credential_revision,drain,created_at,updated_at,ticket_hash,ticket_expires) VALUES(?1,?2,?3,?4,?5,'pending',1,1,1,0,?6,?6,?7,?8)",params![id,workspace,definition.node_id,definition.serve_origin,definition_raw,now,auth::hash_token(&ticket),expires_at])?;
        self.host_audit(actor, workspace, &id, "host_created", 1)?;
        let host = self.host_row(workspace, &id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(HostEnrollment {
            host,
            enrollment: EnrollmentTicket { ticket, expires_at },
        })
    }
    pub fn rotate_host_enrollment(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        expected: u64,
    ) -> Result<HostEnrollment> {
        let tx = self.connection.unchecked_transaction()?;
        self.host_manage(actor, workspace)?;
        let host = self.host_cas(workspace, id, expected)?;
        let revision = increment(host.revision)?;
        let epoch = increment(host.enrollment_epoch)?;
        let credential_revision = increment(host.credential_revision)?;
        let ticket = secret("dhe_");
        let expires_at = now_secs() + 600;
        tx.execute("UPDATE team_hosts SET status='pending',revision=?1,enrollment_epoch=?2,credential_revision=?3,capabilities=NULL,last_seen=NULL,ticket_hash=?4,ticket_expires=?5,credential_hash=NULL,updated_at=?6 WHERE id=?7",params![revision as i64,epoch as i64,credential_revision as i64,auth::hash_token(&ticket),expires_at,now_secs(),id])?;
        self.host_audit(actor, workspace, id, "host_enrollment_rotated", revision)?;
        let host = self.host_row(workspace, id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(HostEnrollment {
            host,
            enrollment: EnrollmentTicket { ticket, expires_at },
        })
    }
    pub fn approve_host(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        expected: u64,
    ) -> Result<Host> {
        let tx = self.connection.unchecked_transaction()?;
        self.host_manage(actor, workspace)?;
        let host = self.host_cas(workspace, id, expected)?;
        if host.status != HostStatus::Claimed {
            return Err(StoreError::Conflict(
                "Only a claimed host can be approved".into(),
            ));
        }
        let revision = increment(host.revision)?;
        tx.execute(
            "UPDATE team_hosts SET status='approved',revision=?1,updated_at=?2 WHERE id=?3",
            params![revision as i64, now_secs(), id],
        )?;
        self.host_audit(actor, workspace, id, "host_approved", revision)?;
        let host = self.host_row(workspace, id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(host)
    }
    pub fn drain_host(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        expected: u64,
        drain: bool,
    ) -> Result<Host> {
        let tx = self.connection.unchecked_transaction()?;
        self.host_manage(actor, workspace)?;
        let host = self.host_cas(workspace, id, expected)?;
        if host.status == HostStatus::Revoked {
            return Err(StoreError::Conflict(
                "Revoked host cannot change drain state".into(),
            ));
        }
        let revision = increment(host.revision)?;
        tx.execute(
            "UPDATE team_hosts SET drain=?1,revision=?2,updated_at=?3 WHERE id=?4",
            params![drain, revision as i64, now_secs(), id],
        )?;
        self.host_audit(actor, workspace, id, "host_drain_changed", revision)?;
        let host = self.host_row(workspace, id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(host)
    }
    pub fn revoke_host(
        &self,
        actor: &str,
        workspace: &str,
        id: &str,
        expected: u64,
    ) -> Result<Host> {
        let tx = self.connection.unchecked_transaction()?;
        self.host_manage(actor, workspace)?;
        let host = self.host_cas(workspace, id, expected)?;
        if host.status == HostStatus::Revoked {
            tx.commit()?;
            return Ok(host);
        }
        let revision = terminal_increment(host.revision);
        let epoch = terminal_increment(host.enrollment_epoch);
        let credential_revision = terminal_increment(host.credential_revision);
        tx.execute("UPDATE team_hosts SET status='revoked',drain=1,revision=?1,enrollment_epoch=?2,credential_revision=?3,ticket_hash=NULL,ticket_expires=NULL,credential_hash=NULL,updated_at=?4 WHERE id=?5",params![revision as i64,epoch as i64,credential_revision as i64,now_secs(),id])?;
        // Revocation must remain possible when ordinary audit is full. This
        // mandatory receipt and credential invalidation share one transaction;
        // receipt failures roll back both. Positive enrollment still requires
        // ordinary audit capacity, preventing cycles after that quota is full.
        tx.execute("INSERT INTO team_host_revocations(host_id,workspace_id,actor_id,revision,enrollment_epoch,credential_revision,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(host_id) DO UPDATE SET workspace_id=excluded.workspace_id,actor_id=excluded.actor_id,revision=excluded.revision,enrollment_epoch=excluded.enrollment_epoch,credential_revision=excluded.credential_revision,created_at=excluded.created_at",
            params![id,workspace,actor,revision as i64,epoch as i64,credential_revision as i64,now_secs()])?;
        match self.team_audit_capacity() {
            Ok(()) => self.host_audit(actor, workspace, id, "host_revoked", revision)?,
            Err(StoreError::Limit(message))
                if message == "Team audit limit reached; administrator maintenance required" => {} // The dedicated receipt is mandatory.
            Err(error) => return Err(error),
        }
        let host = self.host_row(workspace, id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(host)
    }
    pub fn enroll_host(
        &self,
        ticket: &str,
        capabilities: HostCapabilities,
    ) -> Result<WorkerCredential> {
        if !valid_secret(ticket, "dhe_") {
            return Err(denied());
        }
        let report = capabilities_json(&capabilities)?;
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let host=tx.query_row(&format!("SELECT {COLUMNS} FROM team_hosts WHERE ticket_hash=?1 AND ticket_expires>?2 AND status='pending'"),params![auth::hash_token(ticket),now_secs()],raw).optional()?.map(decoded).transpose()?.ok_or_else(denied)?;
        let credential = secret("dhw_");
        let revision = increment(host.revision)?;
        let capability_revision = capability_increment(host.capability_revision)?;
        let changed=tx.execute("UPDATE team_hosts SET status='claimed',revision=?1,capability_revision=?7,ticket_hash=NULL,ticket_expires=NULL,credential_hash=?2,capabilities=?3,last_seen=?4,updated_at=?4 WHERE id=?5 AND status='pending' AND ticket_hash=?6",params![revision as i64,auth::hash_token(&credential),report,now_secs(),host.id,auth::hash_token(ticket),capability_revision as i64])?;
        if changed != 1 {
            return Err(denied());
        }
        tx.commit()?;
        self.catalog_changed();
        Ok(WorkerCredential {
            host_id: host.id,
            workspace_id: host.workspace_id,
            enrollment_epoch: host.enrollment_epoch,
            credential_revision: host.credential_revision,
            credential,
        })
    }
    pub fn report_host(
        &self,
        credential: &str,
        id: &str,
        epoch: u64,
        credential_revision: u64,
        capabilities: HostCapabilities,
    ) -> Result<Host> {
        if !valid_secret(credential, "dhw_")
            || epoch == 0
            || epoch > i64::MAX as u64
            || credential_revision == 0
            || credential_revision > i64::MAX as u64
        {
            return Err(denied());
        }
        let report = capabilities_json(&capabilities)?;
        let tx = self.connection.unchecked_transaction()?;
        let host=tx.query_row(&format!("SELECT {COLUMNS} FROM team_hosts WHERE id=?1 AND credential_hash=?2 AND enrollment_epoch=?3 AND credential_revision=?4 AND status IN ('claimed','approved')"),params![id,auth::hash_token(credential),epoch as i64,credential_revision as i64],raw).optional()?.map(decoded).transpose()?.ok_or_else(denied)?;
        let capability_revision = capability_increment(host.capability_revision)?;
        tx.execute("UPDATE team_hosts SET capabilities=?1,last_seen=?2,updated_at=?2,capability_revision=?3 WHERE id=?4",params![report,now_secs(),capability_revision as i64,id])?;
        let host = self.host_row(&host.workspace_id, id)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(host)
    }
}
#[cfg(test)]
#[path = "hosts_tests.rs"]
mod tests;
