//! Operator-approved linked Git metadata. Capture is not authority; caller grants
//! and native epochs are checked again after filesystem validation.
use super::{NativeGrantStamp, Result, Store, StoreError, now_secs};
use crate::checkout_identity::{CapturedCheckout, LinkedRoots, RepoKind};
use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
const MAX: u64 = 9_007_199_254_740_991;
const AUDIT: i64 = 4096;
pub(crate) const SCHEMA: &str = "
CREATE TABLE native_metadata_approvals(project_id TEXT PRIMARY KEY REFERENCES projects(id),path TEXT NOT NULL,checkout TEXT NOT NULL,identity TEXT NOT NULL,settings TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),active INTEGER NOT NULL CHECK(active IN(0,1)));
CREATE TABLE native_metadata_grants(project_id TEXT NOT NULL REFERENCES native_metadata_approvals(project_id),user_id TEXT NOT NULL REFERENCES users(id),approval_revision INTEGER NOT NULL,native_revision INTEGER NOT NULL,permissions TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision BETWEEN 1 AND 9007199254740991),active INTEGER NOT NULL CHECK(active IN(0,1)),PRIMARY KEY(project_id,user_id));
CREATE TABLE native_metadata_audit(id INTEGER PRIMARY KEY,project_id TEXT NOT NULL REFERENCES native_metadata_approvals(project_id),subject TEXT NOT NULL,action TEXT NOT NULL,revision INTEGER NOT NULL,at INTEGER NOT NULL);
CREATE TABLE native_metadata_revocations(project_id TEXT NOT NULL REFERENCES native_metadata_approvals(project_id),subject TEXT NOT NULL,revision INTEGER NOT NULL,at INTEGER NOT NULL,PRIMARY KEY(project_id,subject));
";
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MetadataPermission {
    Read,
    Write,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataApproval {
    pub project_id: String,
    pub revision: u64,
    pub active: bool,
    pub checkout_identity_digest: String,
    pub settings_digest: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataGrant {
    pub project_id: String,
    pub user_id: String,
    pub revision: u64,
    pub active: bool,
    pub approval_revision: u64,
    pub native_grant_revision: u64,
    pub permissions: Vec<MetadataPermission>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MetadataStamp {
    pub approval: MetadataApproval,
    pub grant: MetadataGrant,
}
pub(crate) struct MetadataPreparation {
    project: String,
    path: String,
    settings: String,
}
pub(crate) struct PreparedMetadata {
    preparation: MetadataPreparation,
    checkout: CapturedCheckout,
}
impl MetadataPreparation {
    pub(crate) fn capture(self, git_dir: &Path, common_dir: &Path) -> Result<PreparedMetadata> {
        let checkout = CapturedCheckout::capture(
            Path::new(&self.path),
            &self.settings,
            Some(&LinkedRoots {
                git_dir: git_dir.into(),
                common_dir: common_dir.into(),
            }),
        )
        .map_err(checkout_error)?;
        if checkout.repo_kind() != RepoKind::LinkedWorktree {
            return Err(conflict());
        }
        Ok(PreparedMetadata {
            preparation: self,
            checkout,
        })
    }
}
#[derive(Clone)]
pub(crate) struct MetadataCandidate {
    stamp: MetadataStamp,
    native: NativeGrantStamp,
    path: String,
    checkout: CapturedCheckout,
    permission: MetadataPermission,
}
pub(crate) struct ValidatedMetadata(MetadataCandidate);
#[derive(Clone)]
pub(crate) struct MetadataAuthority(MetadataCandidate);
impl MetadataCandidate {
    pub(crate) fn validate_filesystem(self) -> Result<ValidatedMetadata> {
        self.checkout
            .revalidate_at(Path::new(&self.path))
            .map_err(checkout_error)?;
        Ok(ValidatedMetadata(self))
    }
}
impl MetadataAuthority {
    pub(crate) fn summary(&self) -> MetadataStamp {
        self.0.stamp.clone()
    }
    pub(crate) fn validate_filesystem(&self) -> Result<()> {
        self.0
            .checkout
            .revalidate_at(Path::new(&self.0.path))
            .map_err(checkout_error)
    }
}
fn conflict() -> StoreError {
    StoreError::Conflict("Local Git metadata approval or grant changed".into())
}
fn denied() -> StoreError {
    dunlin_domain::collaboration::PolicyError::PermissionDenied.into()
}
fn checkout_error(e: crate::checkout_identity::Error) -> StoreError {
    match e {
        crate::checkout_identity::Error::Invalid(v) => StoreError::Limit(v.into()),
        crate::checkout_identity::Error::Conflict(v) => StoreError::Conflict(v.into()),
        crate::checkout_identity::Error::Io(v) => v.into(),
        crate::checkout_identity::Error::Json(v) => v.into(),
    }
}
fn number(r: &rusqlite::Row<'_>, i: usize) -> rusqlite::Result<u64> {
    u64::try_from(r.get::<_, i64>(i)?)
        .ok()
        .filter(|v| (1..=MAX).contains(v))
        .ok_or(rusqlite::Error::InvalidQuery)
}
fn sql(v: u64) -> Result<i64> {
    if !(1..=MAX).contains(&v) {
        return Err(conflict());
    }
    i64::try_from(v).map_err(|_| conflict())
}
fn next(v: u64, positive: bool) -> Result<u64> {
    if !positive && v == MAX {
        return Ok(MAX);
    }
    v.checked_add(1)
        .filter(|v| *v <= MAX && (!positive || *v < MAX))
        .ok_or_else(conflict)
}
impl Store {
    fn metadata_path(&self, project: &str) -> Result<String> {
        self.connection
            .query_row("SELECT path FROM projects WHERE id=?1", [project], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(denied)
    }
    fn metadata_settings(&self, project: &str) -> Result<String> {
        Ok(super::auth::hash_token(
            &self
                .project_settings(project)?
                .unwrap_or_else(|| "{}".into()),
        ))
    }
    pub(crate) fn prepare_local_metadata(&self, project: &str) -> Result<MetadataPreparation> {
        let tx = self.connection.unchecked_transaction()?;
        let value = MetadataPreparation {
            project: project.into(),
            path: self.metadata_path(project)?,
            settings: self.metadata_settings(project)?,
        };
        tx.commit()?;
        Ok(value)
    }
    fn metadata_row(
        &self,
        project: &str,
    ) -> Result<Option<(MetadataApproval, String, CapturedCheckout)>> {
        self.connection.query_row("SELECT revision,active,identity,settings,path,checkout FROM native_metadata_approvals WHERE project_id=?1",[project],|r|Ok((number(r,0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get::<_,String>(5)?))).optional()?.map(|(revision,active,checkout_identity_digest,settings_digest,path,raw)|Ok((MetadataApproval{project_id:project.into(),revision,active,checkout_identity_digest,settings_digest},path,CapturedCheckout::decoded(raw).map_err(checkout_error)?))).transpose()
    }
    pub(crate) fn local_metadata(&self, project: &str) -> Result<Option<MetadataApproval>> {
        self.connection.query_row("SELECT revision,active,identity,settings FROM native_metadata_approvals WHERE project_id=?1",[project],|r|Ok(MetadataApproval{project_id:project.into(),revision:number(r,0)?,active:r.get(1)?,checkout_identity_digest:r.get(2)?,settings_digest:r.get(3)?})).optional().map_err(Into::into)
    }
    fn metadata_grant(&self, project: &str, user: &str) -> Result<Option<MetadataGrant>> {
        self.connection.query_row("SELECT approval_revision,native_revision,permissions,revision,active FROM native_metadata_grants WHERE project_id=?1 AND user_id=?2",params![project,user],|r|Ok((number(r,0)?,number(r,1)?,r.get::<_,String>(2)?,number(r,3)?,r.get(4)?))).optional()?.map(|(approval_revision,native_grant_revision,raw,revision,active)|Ok(MetadataGrant{project_id:project.into(),user_id:user.into(),approval_revision,native_grant_revision,permissions:serde_json::from_str(&raw)?,revision,active})).transpose()
    }
    pub(crate) fn local_metadata_grants(&self, project: &str) -> Result<Vec<MetadataGrant>> {
        let tx = self.connection.unchecked_transaction()?;
        let mut q=self.connection.prepare("SELECT user_id FROM native_metadata_grants WHERE project_id=?1 ORDER BY user_id LIMIT 64")?;
        let ids = q
            .query_map([project], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(q);
        let result = ids
            .into_iter()
            .map(|u| self.metadata_grant(project, &u)?.ok_or_else(denied))
            .collect::<Result<Vec<_>>>()?;
        tx.commit()?;
        Ok(result)
    }
    fn metadata_audit(
        &self,
        p: &str,
        subject: &str,
        action: &str,
        rev: u64,
        terminal: bool,
    ) -> Result<()> {
        if terminal {
            self.connection.execute("INSERT INTO native_metadata_revocations VALUES(?1,?2,?3,?4) ON CONFLICT(project_id,subject) DO UPDATE SET revision=excluded.revision,at=excluded.at",params![p,subject,sql(rev)?,now_secs()])?;
        }
        let count: i64 =
            self.connection
                .query_row("SELECT count(*) FROM native_metadata_audit", [], |r| {
                    r.get(0)
                })?;
        if count >= AUDIT {
            return if terminal {
                Ok(())
            } else {
                Err(StoreError::Limit(
                    "Local metadata audit capacity reached".into(),
                ))
            };
        }
        self.connection.execute("INSERT INTO native_metadata_audit(project_id,subject,action,revision,at) VALUES(?1,?2,?3,?4,?5)",params![p,subject,action,sql(rev)?,now_secs()])?;
        Ok(())
    }
    pub(crate) fn commit_local_metadata(
        &self,
        prepared: PreparedMetadata,
        expected: Option<u64>,
    ) -> Result<MetadataApproval> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let p = &prepared.preparation;
        if self.metadata_path(&p.project)? != p.path
            || self.metadata_settings(&p.project)? != p.settings
        {
            return Err(conflict());
        }
        let old = self.local_metadata(&p.project)?;
        if old.as_ref().map(|v| v.revision) != expected {
            return Err(conflict());
        }
        if old.is_none() {
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM native_metadata_approvals",
                [],
                |r| r.get(0),
            )?;
            if count >= 1024 {
                return Err(StoreError::Limit(
                    "Local metadata approval capacity reached".into(),
                ));
            }
        }
        let rev = next(expected.unwrap_or(0), true)?;
        self.connection.execute("INSERT INTO native_metadata_approvals VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(project_id) DO UPDATE SET path=excluded.path,checkout=excluded.checkout,identity=excluded.identity,settings=excluded.settings,revision=excluded.revision,active=1",params![p.project,p.path,prepared.checkout.encoded().map_err(checkout_error)?,prepared.checkout.identity_digest().map_err(checkout_error)?,p.settings,sql(rev)?])?;
        self.metadata_audit(&p.project, "approval", "approved", rev, false)?;
        let result = self.local_metadata(&p.project)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(result)
    }
    pub(crate) fn revoke_local_metadata(
        &self,
        project: &str,
        expected: u64,
    ) -> Result<MetadataApproval> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut old = self.local_metadata(project)?.ok_or_else(denied)?;
        if old.revision != expected {
            return Err(conflict());
        }
        if !old.active {
            tx.commit()?;
            return Ok(old);
        }
        old.revision = next(old.revision, false)?;
        old.active = false;
        self.connection.execute(
            "UPDATE native_metadata_approvals SET revision=?2,active=0 WHERE project_id=?1",
            params![project, sql(old.revision)?],
        )?;
        self.metadata_audit(project, "approval", "revoked", old.revision, true)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(old)
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn set_local_metadata_grant(
        &self,
        project: &str,
        user: &str,
        approval_revision: u64,
        native_grant_revision: u64,
        permissions: &[MetadataPermission],
        active: bool,
        expected: Option<u64>,
        ack_repo_write: bool,
    ) -> Result<MetadataGrant> {
        if !active {
            return self.revoke_local_metadata_grant(project, user, expected.ok_or_else(conflict)?);
        }
        let mut permissions = permissions.to_vec();
        permissions.sort();
        if permissions.is_empty()
            || permissions.len() > 2
            || permissions.windows(2).any(|w| w[0] == w[1])
            || !permissions.contains(&MetadataPermission::Read)
            || (permissions.contains(&MetadataPermission::Write) && !ack_repo_write)
        {
            return Err(StoreError::Limit(
                "Explicit Read and repository-wide Write acknowledgment required".into(),
            ));
        }
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let approval = self
            .local_metadata(project)?
            .filter(|a| a.active && a.revision == approval_revision)
            .ok_or_else(denied)?;
        if self.metadata_path(project)? != self.metadata_row(project)?.ok_or_else(denied)?.1
            || self.metadata_settings(project)? != approval.settings_digest
        {
            return Err(conflict());
        }
        let native = self.capture_native_grant(project, user)?;
        if native.revision != native_grant_revision {
            return Err(conflict());
        }
        let old = self.metadata_grant(project, user)?;
        if old.as_ref().map(|g| g.revision) != expected {
            return Err(conflict());
        }
        if let Some(g) = &old
            && g.active
            && g.approval_revision == approval_revision
            && g.native_grant_revision == native_grant_revision
            && g.permissions == permissions
        {
            tx.commit()?;
            return Ok(g.clone());
        }
        if old.is_none() {
            let count: i64 = self.connection.query_row(
                "SELECT count(*) FROM native_metadata_grants WHERE project_id=?1",
                [project],
                |r| r.get(0),
            )?;
            if count >= 64 {
                return Err(StoreError::Limit(
                    "Local metadata grant capacity reached".into(),
                ));
            }
        }
        let revision = next(expected.unwrap_or(0), true)?;
        self.connection.execute("INSERT INTO native_metadata_grants VALUES(?1,?2,?3,?4,?5,?6,1) ON CONFLICT(project_id,user_id) DO UPDATE SET approval_revision=excluded.approval_revision,native_revision=excluded.native_revision,permissions=excluded.permissions,revision=excluded.revision,active=1",params![project,user,sql(approval_revision)?,sql(native_grant_revision)?,serde_json::to_string(&permissions)?,sql(revision)?])?;
        self.metadata_audit(project, &format!("user:{user}"), "granted", revision, false)?;
        let result = self.metadata_grant(project, user)?.ok_or_else(denied)?;
        tx.commit()?;
        self.catalog_changed();
        Ok(result)
    }
    pub(crate) fn revoke_local_metadata_grant(
        &self,
        project: &str,
        user: &str,
        expected: u64,
    ) -> Result<MetadataGrant> {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let mut old = self.connection.query_row("SELECT approval_revision,native_revision,revision,active FROM native_metadata_grants WHERE project_id=?1 AND user_id=?2",params![project,user],|r|Ok(MetadataGrant{project_id:project.into(),user_id:user.into(),approval_revision:number(r,0)?,native_grant_revision:number(r,1)?,permissions:Vec::new(),revision:number(r,2)?,active:r.get(3)?})).optional()?.ok_or_else(denied)?;
        if old.revision != expected {
            return Err(conflict());
        }
        if !old.active {
            tx.commit()?;
            return Ok(old);
        }
        old.revision = next(old.revision, false)?;
        old.active = false;
        self.connection.execute("UPDATE native_metadata_grants SET revision=?3,active=0,permissions='[]' WHERE project_id=?1 AND user_id=?2",params![project,user,sql(old.revision)?])?;
        self.metadata_audit(
            project,
            &format!("user:{user}"),
            "revoked",
            old.revision,
            true,
        )?;
        tx.commit()?;
        self.catalog_changed();
        Ok(old)
    }
    fn metadata_candidate_snapshot(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<MetadataCandidate> {
        let (approval, path, checkout) = self
            .metadata_row(project)?
            .filter(|v| v.0.active)
            .ok_or_else(denied)?;
        if checkout.repo_kind() != RepoKind::LinkedWorktree
            || checkout.settings_digest() != approval.settings_digest
            || checkout.identity_digest().map_err(checkout_error)?
                != approval.checkout_identity_digest
        {
            return Err(conflict());
        }
        let grant = self
            .metadata_grant(project, user)?
            .filter(|g| {
                g.active
                    && g.approval_revision == approval.revision
                    && g.permissions.contains(&permission)
            })
            .ok_or_else(denied)?;
        let native = self.capture_native_grant(project, user)?;
        if native.revision != grant.native_grant_revision
            || self.metadata_path(project)? != path
            || self.metadata_settings(project)? != approval.settings_digest
        {
            return Err(denied());
        }
        Ok(MetadataCandidate {
            stamp: MetadataStamp { approval, grant },
            native,
            path,
            checkout,
            permission,
        })
    }
    pub(crate) fn local_metadata_candidate(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<MetadataCandidate> {
        let tx = self.connection.unchecked_transaction()?;
        let c = self.local_metadata_candidate_in_transaction(project, user, permission)?;
        tx.commit()?;
        Ok(c)
    }
    pub(super) fn local_metadata_candidate_in_transaction(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<MetadataCandidate> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        self.metadata_candidate_snapshot(project, user, permission)
    }
    pub(crate) fn authorize_local_metadata(
        &self,
        validated: ValidatedMetadata,
    ) -> Result<MetadataAuthority> {
        let tx = self.connection.unchecked_transaction()?;
        let a = self.authorize_local_metadata_in_transaction(validated)?;
        tx.commit()?;
        Ok(a)
    }
    pub(super) fn authorize_local_metadata_in_transaction(
        &self,
        validated: ValidatedMetadata,
    ) -> Result<MetadataAuthority> {
        let a = MetadataAuthority(validated.0);
        self.revalidate_local_metadata_in_transaction(&a)?;
        Ok(a)
    }
    pub(crate) fn revalidate_local_metadata(&self, a: &MetadataAuthority) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        self.revalidate_local_metadata_in_transaction(a)?;
        tx.commit()?;
        Ok(())
    }
    pub(super) fn revalidate_local_metadata_in_transaction(
        &self,
        a: &MetadataAuthority,
    ) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let old = &a.0;
        let current = self.metadata_candidate_snapshot(
            &old.stamp.approval.project_id,
            &old.stamp.grant.user_id,
            old.permission,
        )?;
        if current.stamp != old.stamp || current.native != old.native || current.path != old.path {
            return Err(conflict());
        }
        Ok(())
    }
}
impl MetadataAuthority {
    pub(super) fn captured_checkout(&self) -> &CapturedCheckout {
        &self.0.checkout
    }
    pub(super) fn stamp_ref(&self) -> &MetadataStamp {
        &self.0.stamp
    }
}
#[cfg(test)]
mod tests;

impl MetadataCandidate {
    pub(super) fn captured_checkout(&self) -> &CapturedCheckout {
        &self.checkout
    }
    pub(super) fn stamp_ref(&self) -> &MetadataStamp {
        &self.stamp
    }
}
