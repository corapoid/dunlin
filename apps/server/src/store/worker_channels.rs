//! Durable diagnostic worker channel fences. These leases grant no execution,
//! user privileges, network verification or access to sessions/projects.
use super::{Result, Store, StoreError, auth, hosts::Host};
use dunlin_domain::collaboration::PolicyError;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
pub(crate) const WORKER_CHANNEL_SCHEMA:&str="
CREATE TABLE team_worker_channels (
 host_id TEXT PRIMARY KEY REFERENCES team_hosts(id), epoch INTEGER NOT NULL CHECK(epoch>0),
 channel_id TEXT NOT NULL UNIQUE, controller_boot_id TEXT NOT NULL, worker_boot_id TEXT NOT NULL,
 phase TEXT NOT NULL CHECK(phase IN ('active','offline')), expires_at_ms INTEGER NOT NULL CHECK(expires_at_ms>=0),
 last_sequence INTEGER NOT NULL CHECK(last_sequence>=0)
);
";
/// Constructible only after worker credential authentication. Never serializable
/// or printable: the captured hash is an exact credential-epoch pin.
#[derive(Clone)]
pub(crate) struct WorkerIdentity {
    host: Host,
    hash: String,
}
impl WorkerIdentity {
    pub fn host_id(&self) -> &str {
        &self.host.id
    }
    pub fn workspace_id(&self) -> &str {
        &self.host.workspace_id
    }
    /// Metadata captured at authentication; revalidate before every operation.
    pub fn host(&self) -> &Host {
        &self.host
    }
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChannelLease {
    pub host_id: String,
    pub workspace_id: String,
    pub channel_id: String,
    pub controller_boot_id: String,
    pub worker_boot_id: String,
    pub epoch: u64,
    pub expires_at_ms: i64,
    pub last_sequence: u64,
}
/// Persisted metadata only. An `active` row may already be expired or belong to
/// an older controller boot; callers must not treat this as health or authority.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct ChannelDiagnostic {
    pub lease: ChannelLease,
    pub phase: String,
}
struct Channel {
    lease: ChannelLease,
    phase: String,
}
fn denied() -> StoreError {
    PolicyError::PermissionDenied.into()
}
fn conflict() -> StoreError {
    StoreError::Conflict("Worker channel is stale, expired or unavailable".into())
}
fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}
fn deadline(now: i64, ttl: u64) -> Result<i64> {
    if now < 0 || !(5000..=15000).contains(&ttl) {
        return Err(StoreError::Limit(
            "Invalid worker lease clock or TTL".into(),
        ));
    }
    now.checked_add(ttl as i64)
        .ok_or_else(|| StoreError::Limit("Worker lease deadline limit reached".into()))
}
fn same(current: &ChannelLease, expected: &ChannelLease) -> bool {
    current.host_id == expected.host_id
        && current.workspace_id == expected.workspace_id
        && current.channel_id == expected.channel_id
        && current.controller_boot_id == expected.controller_boot_id
        && current.worker_boot_id == expected.worker_boot_id
        && current.epoch == expected.epoch
}
impl Store {
    pub(crate) fn worker_channel_diagnostic(
        &self,
        actor: &str,
        workspace: &str,
        host_id: &str,
    ) -> Result<Option<ChannelDiagnostic>> {
        let tx = self.connection.unchecked_transaction()?;
        // host() checks fresh workspace ManageHosts and exact host scope.
        self.host(actor, workspace, host_id)?.ok_or_else(denied)?;
        let diagnostic = self
            .worker_channel_row(host_id)?
            .map(|channel| ChannelDiagnostic {
                lease: channel.lease,
                phase: channel.phase,
            });
        tx.commit()?;
        Ok(diagnostic)
    }
    pub(crate) fn authenticate_worker_token(&self, token: &str) -> Result<WorkerIdentity> {
        let valid = token.strip_prefix("dhw_").is_some_and(|tail| {
            tail.len() == 43
                && tail
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        });
        if !valid {
            return Err(denied());
        }
        let hash = auth::hash_token(token);
        let host = self.worker_host_by_credential(&hash)?.ok_or_else(denied)?;
        Ok(WorkerIdentity { host, hash })
    }
    pub(crate) fn authenticate_worker(
        &self,
        token: &str,
        host_id: &str,
        enrollment_epoch: u64,
        credential_revision: u64,
    ) -> Result<WorkerIdentity> {
        if !identifier(host_id) || enrollment_epoch == 0 || credential_revision == 0 {
            return Err(denied());
        }
        let identity = self.authenticate_worker_token(token)?;
        if identity.host_id() != host_id
            || identity.host.enrollment_epoch != enrollment_epoch
            || identity.host.credential_revision != credential_revision
        {
            return Err(denied());
        }
        Ok(identity)
    }
    pub(crate) fn validate_worker_identity(&self, identity: &WorkerIdentity) -> Result<Host> {
        self.worker_host_row(
            identity.host_id(),
            &identity.hash,
            identity.host.enrollment_epoch,
            identity.host.credential_revision,
        )?
        .filter(|host| host.workspace_id == identity.workspace_id())
        .ok_or_else(denied)
    }
    fn worker_channel_row(&self, host: &str) -> Result<Option<Channel>> {
        self.connection.query_row("SELECT c.host_id,h.workspace_id,c.channel_id,c.controller_boot_id,c.worker_boot_id,c.epoch,c.expires_at_ms,c.last_sequence,c.phase FROM team_worker_channels c JOIN team_hosts h ON h.id=c.host_id WHERE c.host_id=?1",[host],|row|Ok(Channel{lease:ChannelLease{host_id:row.get(0)?,workspace_id:row.get(1)?,channel_id:row.get(2)?,controller_boot_id:row.get(3)?,worker_boot_id:row.get(4)?,epoch:super::positive_revision(row,5)?,expires_at_ms:row.get(6)?,last_sequence:u64::try_from(row.get::<_,i64>(7)?).map_err(|_|rusqlite::Error::InvalidQuery)?},phase:row.get(8)?})).optional().map_err(Into::into)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open_worker_channel(
        &self,
        identity: &WorkerIdentity,
        controller_boot_id: &str,
        worker_boot_id: &str,
        channel_id: &str,
        now_ms: i64,
        ttl_ms: u64,
    ) -> Result<ChannelLease> {
        let expires_at_ms = deadline(now_ms, ttl_ms)?;
        if ![controller_boot_id, worker_boot_id, channel_id]
            .into_iter()
            .all(identifier)
        {
            return Err(StoreError::Limit(
                "Invalid worker channel identifiers".into(),
            ));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let host = self.validate_worker_identity(identity)?;
        let previous = self.worker_channel_row(&host.id)?;
        let epoch = previous.map_or(Ok(1), |old| {
            old.lease
                .epoch
                .checked_add(1)
                .filter(|v| *v <= i64::MAX as u64)
                .ok_or_else(|| StoreError::Limit("Worker channel epoch exhausted".into()))
        })?;
        let active:i64=tx.query_row("SELECT count(*) FROM team_worker_channels WHERE phase='active' AND expires_at_ms>?1 AND host_id<>?2",params![now_ms,host.id],|r|r.get(0))?;
        if active >= 64 {
            return Err(StoreError::Limit(
                "Active worker channel capacity reached".into(),
            ));
        }
        let collision: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM team_worker_channels WHERE channel_id=?1)",
            [channel_id],
            |r| r.get(0),
        )?;
        if collision {
            return Err(StoreError::Conflict(
                "Worker channel identifier was already used".into(),
            ));
        }
        tx.execute("INSERT INTO team_worker_channels(host_id,epoch,channel_id,controller_boot_id,worker_boot_id,phase,expires_at_ms,last_sequence) VALUES(?1,?2,?3,?4,?5,'active',?6,0) ON CONFLICT(host_id) DO UPDATE SET epoch=excluded.epoch,channel_id=excluded.channel_id,controller_boot_id=excluded.controller_boot_id,worker_boot_id=excluded.worker_boot_id,phase='active',expires_at_ms=excluded.expires_at_ms,last_sequence=0",params![host.id,epoch as i64,channel_id,controller_boot_id,worker_boot_id,expires_at_ms])?;
        tx.commit()?;
        Ok(ChannelLease {
            host_id: host.id,
            workspace_id: host.workspace_id,
            channel_id: channel_id.into(),
            controller_boot_id: controller_boot_id.into(),
            worker_boot_id: worker_boot_id.into(),
            epoch,
            expires_at_ms,
            last_sequence: 0,
        })
    }
    pub(crate) fn renew_worker_channel(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        sequence: u64,
        now_ms: i64,
        ttl_ms: u64,
    ) -> Result<ChannelLease> {
        let expires_at_ms = deadline(now_ms, ttl_ms)?;
        if sequence == 0 || sequence > i64::MAX as u64 {
            return Err(conflict());
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        self.validate_worker_identity(identity)?;
        let channel = self
            .worker_channel_row(identity.host_id())?
            .filter(|c| {
                same(&c.lease, lease) && c.phase == "active" && c.lease.expires_at_ms > now_ms
            })
            .ok_or_else(conflict)?;
        if sequence == channel.lease.last_sequence {
            tx.commit()?;
            return Ok(channel.lease);
        }
        if channel.lease.last_sequence.checked_add(1) != Some(sequence) {
            return Err(conflict());
        }
        tx.execute(
            "UPDATE team_worker_channels SET last_sequence=?1,expires_at_ms=?2 WHERE host_id=?3",
            params![sequence as i64, expires_at_ms, identity.host_id()],
        )?;
        let mut result = channel.lease;
        result.last_sequence = sequence;
        result.expires_at_ms = expires_at_ms;
        tx.commit()?;
        Ok(result)
    }
    pub(crate) fn validate_worker_channel(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_boot_id: &str,
        now_ms: i64,
    ) -> Result<ChannelLease> {
        let tx = self.connection.unchecked_transaction()?;
        let lease = self.validate_worker_channel_in_transaction(
            identity,
            lease,
            controller_boot_id,
            now_ms,
        )?;
        tx.commit()?;
        Ok(lease)
    }
    /// Requires the caller's current SQLite transaction; no nested transaction.
    pub(crate) fn validate_worker_channel_in_transaction(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
        controller_boot_id: &str,
        now_ms: i64,
    ) -> Result<ChannelLease> {
        if self.connection.is_autocommit() {
            return Err(StoreError::Conflict(
                "A worker channel authority transaction is required".into(),
            ));
        }
        if now_ms < 0 || !identifier(controller_boot_id) {
            return Err(conflict());
        }
        self.validate_worker_identity(identity)?;
        let channel = self
            .worker_channel_row(identity.host_id())?
            .filter(|c| {
                same(&c.lease, lease)
                    && c.lease.controller_boot_id == controller_boot_id
                    && c.phase == "active"
                    && c.lease.expires_at_ms > now_ms
            })
            .ok_or_else(conflict)?;
        Ok(channel.lease)
    }
    pub(crate) fn close_worker_channel(
        &self,
        identity: &WorkerIdentity,
        lease: &ChannelLease,
    ) -> Result<()> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        // Closing is decreasing-authority cleanup, including after this exact
        // credential was revoked. The opaque identity still pins its original
        // host/scope, and the full lease fence must match the current row. It
        // cannot affect a replacement opened under a later transport epoch.
        if identity.host_id() != lease.host_id || identity.workspace_id() != lease.workspace_id {
            return Err(conflict());
        }
        let channel = self
            .worker_channel_row(identity.host_id())?
            .filter(|c| same(&c.lease, lease))
            .ok_or_else(conflict)?;
        if channel.phase != "offline" {
            tx.execute(
                "UPDATE team_worker_channels SET phase='offline' WHERE host_id=?1",
                [identity.host_id()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
#[cfg(test)]
#[path = "worker_channels_tests.rs"]
mod tests;
