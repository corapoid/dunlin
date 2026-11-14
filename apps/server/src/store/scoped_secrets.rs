//! Scoped credential publication ledger. Contains metadata and write receipts,
//! never values or credential policies. Unknown references are never reclaimed.
use super::Store;
mod grants;
mod local;
mod retention;
use crate::mcp_oauth::BrowserProof;
use dunlin_domain::{
    collaboration::{AuthorityStamp, Permission},
    hosts::HostStatus,
};
use dunlin_vault::scoped::{Binding, Candidate, Error, Result, Version, WriteClaim};
pub(super) use grants::SCHEMA as GRANTS_SCHEMA;
pub(crate) use grants::{Grant, GrantRequest, Principal};
pub(super) use local::migrate as migrate_local;
pub(super) use retention::MIGRATION as RETENTION_MIGRATION;
pub(crate) use retention::Retention;
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};
use serde::Serialize;

const CREDENTIALS_V48: &str = "
CREATE TABLE scoped_credentials (
 workspace TEXT NOT NULL, project TEXT NOT NULL, host TEXT NOT NULL,
 credential TEXT NOT NULL CHECK(length(credential) BETWEEN 1 AND 128),
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 state TEXT NOT NULL CHECK(state IN ('installing','active','disabled')),
 active_reference TEXT, staged_reference TEXT, updated_at INTEGER NOT NULL,
 last_disable TEXT CHECK(last_disable IS NULL OR length(last_disable)<=4096),
 PRIMARY KEY(workspace,project,host,credential),
 FOREIGN KEY(workspace,project) REFERENCES team_projects(workspace_id,id),
 FOREIGN KEY(host) REFERENCES team_hosts(id),
 CHECK((state='installing' AND staged_reference IS NOT NULL)
    OR (state='active' AND active_reference IS NOT NULL AND staged_reference IS NULL)
    OR (state='disabled' AND staged_reference IS NULL))
);
";
const REFERENCES_SCHEMA: &str = "CREATE TABLE scoped_secret_references (
 reference TEXT PRIMARY KEY CHECK(length(reference)=78 AND substr(reference,1,14)='scoped-secret:'),
 workspace TEXT NOT NULL, project TEXT NOT NULL, host TEXT NOT NULL, credential TEXT NOT NULL,
 revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),
 sha256 TEXT NOT NULL CHECK(length(sha256)=64),
 writer_user TEXT NOT NULL REFERENCES users(id), writer_device TEXT NOT NULL REFERENCES devices(id),
 browser_hash TEXT NOT NULL, membership_revision INTEGER NOT NULL CHECK(membership_revision>0),
 grant_revision INTEGER NOT NULL CHECK(grant_revision>0), host_revision INTEGER NOT NULL CHECK(host_revision>0),
 write_ack INTEGER NOT NULL DEFAULT 0 CHECK(write_ack IN (0,1)),
 activated INTEGER NOT NULL DEFAULT 0 CHECK(activated IN (0,1)), created_at INTEGER NOT NULL,
 FOREIGN KEY(workspace,project,host,credential) REFERENCES scoped_credentials(workspace,project,host,credential),
 UNIQUE(workspace,project,host,credential,revision)
);
CREATE INDEX scoped_references_scope ON scoped_secret_references(workspace,project,host,credential);
";
pub(super) fn create_v48(tx: &Transaction<'_>) -> rusqlite::Result<()> {
    tx.execute_batch(CREDENTIALS_V48)?;
    tx.execute_batch(REFERENCES_SCHEMA)
}

#[derive(Clone)]
pub(crate) struct Proof {
    pub browser: BrowserProof,
    pub stamp: AuthorityStamp,
    pub binding: Binding,
    pub host_revision: u64,
}
#[derive(Serialize)]
pub(crate) struct Status {
    pub credential: String,
    pub revision: u64,
    pub state: String,
}
fn error(error: super::StoreError) -> Error {
    match error {
        super::StoreError::Policy(_) => Error::Denied,
        super::StoreError::Conflict(_) => Error::Conflict,
        super::StoreError::Limit(_) => Error::Limit,
        _ => Error::Storage,
    }
}
fn storage(_: rusqlite::Error) -> Error {
    Error::Storage
}
fn token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
}
fn hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid(candidate: &Candidate) -> Result<()> {
    if ![
        candidate.credential.as_str(),
        &candidate.binding.workspace,
        &candidate.binding.project,
        &candidate.binding.host,
    ]
    .into_iter()
    .all(token)
        || !candidate
            .reference
            .strip_prefix("scoped-secret:")
            .is_some_and(hex)
        || !hex(&candidate.sha256)
        || candidate.expected_revision >= 9_007_199_254_740_991
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
impl Store {
    fn scoped_secret_authorize(
        &self,
        actor: &str,
        binding: &Binding,
    ) -> Result<(AuthorityStamp, u64, HostStatus)> {
        let stamp = self
            .authorize_team_project(
                actor,
                &binding.workspace,
                &binding.project,
                Permission::ManageProject,
            )
            .map_err(error)?;
        if binding.host == "local" {
            self.team_context(&binding.workspace, actor)
                .map_err(error)?
                .authorize_workspace(&binding.workspace, Permission::ManageHosts)
                .map_err(|_| Error::Denied)?;
            let local = self
                .local_binding_row(&binding.workspace, &binding.project)
                .map_err(error)?
                .ok_or(Error::Denied)?;
            let ready = self
                .local_binding_candidate(
                    actor,
                    &binding.workspace,
                    &binding.project,
                    Permission::ManageProject,
                )
                .is_ok_and(|current| current.is_some_and(|(b, _)| b.revision == local.revision));
            return Ok((
                stamp,
                local.revision,
                if ready {
                    HostStatus::Approved
                } else {
                    HostStatus::Revoked
                },
            ));
        }
        let host = self
            .host(actor, &binding.workspace, &binding.host)
            .map_err(error)?
            .ok_or(Error::Denied)?;
        Ok((stamp, host.revision, host.status))
    }
    pub(crate) fn scoped_secret_proof(
        &self,
        browser: BrowserProof,
        binding: Binding,
    ) -> Result<Proof> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        let (stamp, host_revision, _) = self.scoped_secret_authorize(&browser.user, &binding)?;
        tx.commit().map_err(storage)?;
        Ok(Proof {
            browser,
            stamp,
            binding,
            host_revision,
        })
    }
    fn scoped_secret_cut<'a>(
        &'a self,
        proof: &Proof,
        installing: bool,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Transaction<'a>> {
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        identity(self)?;
        let (stamp, host_revision, status) =
            self.scoped_secret_authorize(&proof.browser.user, &proof.binding)?;
        if stamp != proof.stamp
            || host_revision != proof.host_revision
            || (installing && status != HostStatus::Approved)
        {
            return Err(Error::Denied);
        }
        Ok(tx)
    }
    pub(crate) fn scoped_secret_reserve(
        &self,
        proof: &Proof,
        candidate: &Candidate,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<WriteClaim> {
        valid(candidate)?;
        if candidate.binding != proof.binding {
            return Err(Error::Denied);
        }
        let tx = self.scoped_secret_cut(proof, true, identity)?;
        let b = &candidate.binding;
        let current:Option<i64>=tx.query_row("SELECT revision FROM scoped_credentials WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4",params![b.workspace,b.project,b.host,candidate.credential],|r|r.get(0)).optional().map_err(storage)?;
        if current.unwrap_or(0) as u64 != candidate.expected_revision {
            return Err(Error::Conflict);
        }
        let references: i64 = tx
            .query_row(
                "SELECT count(*) FROM scoped_secret_references WHERE workspace=?1 AND cleanup_state!='deleted'",
                [&b.workspace],
                |r| r.get(0),
            )
            .map_err(storage)?;
        let own:i64=tx.query_row("SELECT count(*) FROM scoped_secret_references WHERE workspace=?1 AND project=?2 AND host=?3 AND credential=?4 AND cleanup_state!='deleted'",params![b.workspace,b.project,b.host,candidate.credential],|r|r.get(0)).map_err(storage)?;
        let credentials: i64 = tx
            .query_row(
                "SELECT count(*) FROM scoped_credentials WHERE workspace=?1 AND project=?2",
                params![b.workspace, b.project],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if references >= 4096 || own >= 256 || (current.is_none() && credentials >= 128) {
            return Err(Error::Limit);
        }
        let duplicate: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM scoped_secret_references WHERE reference=?1)",
                [&candidate.reference],
                |r| r.get(0),
            )
            .map_err(storage)?;
        if duplicate {
            return Err(Error::Conflict);
        }
        let revision = candidate.expected_revision + 1;
        tx.execute("INSERT INTO scoped_credentials(workspace,project,host,credential,revision,state,staged_reference,updated_at,remote_host,local_workspace,local_project) VALUES(?1,?2,?3,?4,?5,'installing',?6,?7,CASE WHEN ?3='local' THEN NULL ELSE ?3 END,CASE WHEN ?3='local' THEN ?1 ELSE NULL END,CASE WHEN ?3='local' THEN ?2 ELSE NULL END) ON CONFLICT(workspace,project,host,credential) DO UPDATE SET revision=excluded.revision,state='installing',staged_reference=excluded.staged_reference,updated_at=excluded.updated_at",params![b.workspace,b.project,b.host,candidate.credential,revision as i64,candidate.reference,super::now_secs()]).map_err(storage)?;
        tx.execute("INSERT INTO scoped_secret_references(reference,workspace,project,host,credential,revision,sha256,writer_user,writer_device,browser_hash,membership_revision,grant_revision,host_revision,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",params![candidate.reference,b.workspace,b.project,b.host,candidate.credential,revision as i64,candidate.sha256,proof.browser.user,proof.browser.device,proof.browser.session_hash,proof.stamp.membership_revision as i64,proof.stamp.grant_revision as i64,proof.host_revision as i64,super::now_secs()]).map_err(storage)?;
        self.scoped_secret_audit(
            proof,
            &candidate.credential,
            "scoped_secret_reserved",
            revision,
        )?;
        let claim = WriteClaim::reserved(candidate.clone(), revision)?;
        tx.commit().map_err(storage)?;
        Ok(claim)
    }
    /// Preserve a factual write ACK even after logout/grant revocation. The
    /// original private writer proof and exact immutable reference must match.
    pub(crate) fn scoped_secret_acknowledge(
        &self,
        proof: &Proof,
        claim: &WriteClaim,
    ) -> Result<()> {
        let c = claim.candidate();
        valid(c)?;
        if c.binding != proof.binding {
            return Err(Error::Denied);
        }
        let tx = Transaction::new_unchecked(&self.connection, TransactionBehavior::Immediate)
            .map_err(storage)?;
        let changed=tx.execute("UPDATE scoped_secret_references SET write_ack=1 WHERE reference=?1 AND workspace=?2 AND project=?3 AND host=?4 AND credential=?5 AND revision=?6 AND sha256=?7 AND writer_user=?8 AND writer_device=?9 AND browser_hash=?10 AND membership_revision=?11 AND grant_revision=?12 AND host_revision=?13",params![c.reference,c.binding.workspace,c.binding.project,c.binding.host,c.credential,claim.revision() as i64,c.sha256,proof.browser.user,proof.browser.device,proof.browser.session_hash,proof.stamp.membership_revision as i64,proof.stamp.grant_revision as i64,proof.host_revision as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.commit().map_err(storage)
    }
    pub(crate) fn scoped_secret_activate(
        &self,
        proof: &Proof,
        claim: WriteClaim,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Version> {
        let c = claim.candidate();
        valid(c)?;
        if c.binding != proof.binding {
            return Err(Error::Denied);
        }
        let tx = self.scoped_secret_cut(proof, true, identity)?;
        let acknowledged:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM scoped_secret_references WHERE reference=?1 AND revision=?2 AND sha256=?3 AND write_ack=1 AND cleanup_state='retained' AND writer_user=?4 AND writer_device=?5 AND browser_hash=?6 AND membership_revision=?7 AND grant_revision=?8 AND host_revision=?9)",params![c.reference,claim.revision() as i64,c.sha256,proof.browser.user,proof.browser.device,proof.browser.session_hash,proof.stamp.membership_revision as i64,proof.stamp.grant_revision as i64,proof.host_revision as i64],|r|r.get(0)).map_err(storage)?;
        if !acknowledged {
            return Err(Error::Conflict);
        }
        let changed=tx.execute("UPDATE scoped_credentials SET state='active',active_reference=?1,staged_reference=NULL,updated_at=?2 WHERE workspace=?3 AND project=?4 AND host=?5 AND credential=?6 AND revision=?7 AND state='installing' AND staged_reference=?1",params![c.reference,super::now_secs(),c.binding.workspace,c.binding.project,c.binding.host,c.credential,claim.revision() as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        tx.execute(
            "UPDATE scoped_secret_references SET activated=1 WHERE reference=?1",
            [&c.reference],
        )
        .map_err(storage)?;
        self.scoped_secret_audit(
            proof,
            &c.credential,
            "scoped_secret_activated",
            claim.revision(),
        )?;
        let version = claim.activated();
        tx.commit().map_err(storage)?;
        Ok(version)
    }
    fn scoped_secret_audit(
        &self,
        proof: &Proof,
        credential: &str,
        action: &str,
        revision: u64,
    ) -> Result<()> {
        self.team_audit_capacity().map_err(error)?;
        self.connection.execute("INSERT INTO team_audit(workspace_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?2,?3,?4,?5,?6)",params![proof.binding.workspace,proof.browser.user,action,revision as i64,super::now_secs(),format!("scoped-secret:{}:{credential}",proof.binding.host)]).map_err(storage)?;
        Ok(())
    }
    pub(crate) fn scoped_secret_validate(
        &self,
        proof: &Proof,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<()> {
        self.scoped_secret_cut(proof, false, identity)?
            .commit()
            .map_err(storage)
    }
    pub(crate) fn scoped_secret_list(
        &self,
        proof: &Proof,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Vec<Status>> {
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let b = &proof.binding;
        let rows = {
            let mut stmt=tx.prepare("SELECT credential,revision,state FROM scoped_credentials WHERE workspace=?1 AND project=?2 AND host=?3 ORDER BY credential LIMIT 128").map_err(storage)?;
            stmt.query_map(params![b.workspace, b.project, b.host], |r| {
                Ok(Status {
                    credential: r.get(0)?,
                    revision: super::positive_revision(r, 1)?,
                    state: r.get(2)?,
                })
            })
            .map_err(storage)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(storage)?
        };
        tx.commit().map_err(storage)?;
        Ok(rows)
    }
    pub(crate) fn scoped_secret_disable(
        &self,
        proof: &Proof,
        credential: &str,
        expected: u64,
        identity: &dyn Fn(&Store) -> Result<()>,
    ) -> Result<Status> {
        if !token(credential) || expected == 0 || expected > 9_007_199_254_740_991 {
            return Err(Error::Invalid);
        }
        let tx = self.scoped_secret_cut(proof, false, identity)?;
        let b = &proof.binding;
        let revision = (expected + 1).min(9_007_199_254_740_991);
        let receipt=serde_json::json!({"actor":proof.browser.user,"device":proof.browser.device,
            "membership_revision":proof.stamp.membership_revision,"grant_revision":proof.stamp.grant_revision,
            "host_revision":proof.host_revision,"revision":revision,"terminal":revision==expected,"at":super::now_secs()}).to_string();
        let changed=tx.execute("UPDATE scoped_credentials SET revision=?8,state='disabled',staged_reference=NULL,updated_at=?1,last_disable=?7 WHERE workspace=?2 AND project=?3 AND host=?4 AND credential=?5 AND revision=?6",params![super::now_secs(),b.workspace,b.project,b.host,credential,expected as i64,receipt,revision as i64]).map_err(storage)?;
        if changed != 1 {
            return Err(Error::Conflict);
        }
        // The bounded receipt above is mandatory. Global audit exhaustion or
        // a rejected audit insert cannot prevent this negative authority cut.
        match self.team_audit_capacity() {
            Ok(()) => {
                let _ =
                    self.scoped_secret_audit(proof, credential, "scoped_secret_disabled", revision);
            }
            Err(super::StoreError::Limit(_)) => {}
            Err(other) => return Err(error(other)),
        }
        tx.commit().map_err(storage)?;
        Ok(Status {
            credential: credential.into(),
            revision,
            state: "disabled".into(),
        })
    }
}
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) use tests::scope as testing_scope;
