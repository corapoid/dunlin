//! Versioned local filesystem authority. Revocation preserves tombstones so an
//! old actor/plan cannot revive when the same project is granted again.
use super::*;
use serde::{Deserialize, Serialize};

pub(super) const MIGRATION: &str = "
ALTER TABLE project_grants ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>=1 AND revision<=9007199254740991);
ALTER TABLE project_grants ADD COLUMN active INTEGER NOT NULL DEFAULT 1 CHECK(active IN (0,1));
";
const MAX_REVISION: u64 = 9_007_199_254_740_991;
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeGrantStamp {
    pub project_id: String,
    pub user_id: String,
    pub revision: u64,
}
impl Store {
    fn native_grant_row(&self, project: &str, user: &str) -> Result<Option<(u64, bool)>> {
        Ok(self
            .connection
            .query_row(
                "SELECT revision,active FROM project_grants WHERE project_id=?1 AND user_id=?2",
                params![project, user],
                |r| Ok((positive_revision(r, 0)?, r.get(1)?)),
            )
            .optional()?)
    }
    /// Current persisted epoch, including revoked tombstones, for trusted CAS
    /// administration. This lookup grants no filesystem authority.
    pub fn native_grant_revision(&self, project_id: &str, user_id: &str) -> Result<Option<u64>> {
        Ok(self
            .native_grant_row(project_id, user_id)?
            .map(|(revision, _)| revision))
    }

    pub fn capture_native_grant(
        &self,
        project_id: &str,
        user_id: &str,
    ) -> Result<NativeGrantStamp> {
        let (revision, active) = self
            .native_grant_row(project_id, user_id)?
            .ok_or(dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        if !active {
            return Err(dunlin_domain::collaboration::PolicyError::PermissionDenied.into());
        }
        if revision > MAX_REVISION {
            return Err(StoreError::Limit("Native grant revision limit".into()));
        }
        Ok(NativeGrantStamp {
            project_id: project_id.into(),
            user_id: user_id.into(),
            revision,
        })
    }
    pub fn revalidate_native_grant(&self, stamp: &NativeGrantStamp) -> Result<()> {
        if !(1..=MAX_REVISION).contains(&stamp.revision)
            || self.native_grant_row(&stamp.project_id, &stamp.user_id)?
                != Some((stamp.revision, true))
        {
            return Err(dunlin_domain::collaboration::PolicyError::StaleAuthority.into());
        }
        Ok(())
    }
    /// Trusted server/local administration only. HTTP callers must establish
    /// their administrator/host policy boundary before invoking this mutation.
    pub fn set_native_grant(
        &self,
        project: &str,
        user: &str,
        active: bool,
        expected_revision: Option<u64>,
    ) -> Result<u64> {
        let tx = self.connection.unchecked_transaction()?;
        let previous = self.native_grant_row(project, user)?;
        if previous.map(|(revision, _)| revision) != expected_revision {
            return Err(StoreError::Conflict(
                "Native grant changed; reload before saving".into(),
            ));
        }
        if let Some((revision, current)) = previous
            && current == active
        {
            tx.commit()?;
            return Ok(revision);
        }
        let current_revision = previous.map(|(r, _)| r).unwrap_or(0);
        // Keep the last representable epoch exclusively for terminal denial.
        // Imported active MAX rows can still be revoked without resurrecting
        // that positive epoch; inactive MAX can never be granted again.
        let revision = if !active && current_revision == MAX_REVISION {
            MAX_REVISION
        } else {
            current_revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or_else(|| StoreError::Limit("Native grant revision limit".into()))?
        };
        if active && revision == MAX_REVISION {
            return Err(StoreError::Limit("Native grant revision limit".into()));
        }
        self.connection.execute("INSERT INTO project_grants(project_id,user_id,revision,active) VALUES(?1,?2,?3,?4) ON CONFLICT(project_id,user_id) DO UPDATE SET revision=excluded.revision,active=excluded.active",params![project,user,revision as i64,active])?;
        tx.commit()?;
        self.catalog_changed();
        Ok(revision)
    }
    /// Compatibility for existing trusted provisioning paths. Re-granting a
    /// revoked row increments its epoch instead of deleting its history.
    pub fn grant(&self, project_id: &str, user_id: &str) -> Result<()> {
        let previous = self.native_grant_row(project_id, user_id)?.map(|(r, _)| r);
        self.set_native_grant(project_id, user_id, true, previous)
            .map(|_| ())
    }
    pub fn revoke_grant(&self, project_id: &str, user_id: &str) -> Result<()> {
        let Some((revision, _)) = self.native_grant_row(project_id, user_id)? else {
            return Ok(());
        };
        self.set_native_grant(project_id, user_id, false, Some(revision))
            .map(|_| ())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Store, String, String) {
        let s = Store::open_in_memory().unwrap();
        let user = s.create_user("native-grantee", "member").unwrap();
        let p = s.create_project("native", "/tmp/native").unwrap();
        (s, p.id, user.id)
    }
    #[test]
    fn cas_noop_revocation_and_aba_never_revive_a_stamp() {
        let (s, p, u) = fixture();
        assert!(s.capture_native_grant(&p, &u).is_err());
        assert_eq!(s.set_native_grant(&p, &u, true, None).unwrap(), 1);
        let first = s.capture_native_grant(&p, &u).unwrap();
        assert_eq!(s.set_native_grant(&p, &u, true, Some(1)).unwrap(), 1);
        assert!(s.set_native_grant(&p, &u, false, None).is_err());
        assert!(s.revalidate_native_grant(&first).is_ok());
        assert_eq!(s.set_native_grant(&p, &u, false, Some(1)).unwrap(), 2);
        assert!(s.capture_native_grant(&p, &u).is_err());
        assert!(s.project_for_user(&p, &u).unwrap().is_none());
        assert!(s.projects_for_user(&u).unwrap().is_empty());
        assert!(s.revalidate_native_grant(&first).is_err());
        assert_eq!(s.set_native_grant(&p, &u, false, Some(2)).unwrap(), 2);
        assert!(s.set_native_grant(&p, &u, true, Some(1)).is_err());
        s.grant(&p, &u).unwrap();
        let next = s.capture_native_grant(&p, &u).unwrap();
        assert_eq!(next.revision, 3);
        assert!(s.revalidate_native_grant(&first).is_err());
        assert!(s.revalidate_native_grant(&next).is_ok());
        s.grant(&p, &u).unwrap();
        assert_eq!(s.capture_native_grant(&p, &u).unwrap(), next);
        s.revoke_grant(&p, &u).unwrap();
        assert_eq!(s.native_grant_row(&p, &u).unwrap(), Some((4, false)));
        s.revoke_grant(&p, &u).unwrap();
        assert_eq!(s.native_grant_row(&p, &u).unwrap(), Some((4, false)));
    }
    #[test]
    fn only_committed_changes_notify_and_insert_failure_keeps_no_authority() {
        let (s, p, u) = fixture();
        let mut changes = s.subscribe_catalog();
        s.connection.execute_batch("CREATE TRIGGER fail_grant BEFORE INSERT ON project_grants BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(s.set_native_grant(&p, &u, true, None).is_err());
        assert!(s.capture_native_grant(&p, &u).is_err());
        assert!(changes.try_recv().is_err());
        s.connection
            .execute_batch("DROP TRIGGER fail_grant")
            .unwrap();
        s.grant(&p, &u).unwrap();
        assert!(changes.try_recv().is_ok());
        s.grant(&p, &u).unwrap();
        assert!(changes.try_recv().is_err());
        assert!(s.set_native_grant(&p, &u, false, Some(99)).is_err());
        assert!(changes.try_recv().is_err());
        s.connection.execute_batch("CREATE TRIGGER fail_revoke BEFORE UPDATE ON project_grants BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let stamp = s.capture_native_grant(&p, &u).unwrap();
        assert!(s.revoke_grant(&p, &u).is_err());
        assert!(s.revalidate_native_grant(&stamp).is_ok());
        assert!(changes.try_recv().is_err());
    }
    #[test]
    fn migration_backfills_existing_grants_and_tombstones_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("db");
        let s = Store::open(&path).unwrap();
        let u = s.create_user("legacy", "member").unwrap();
        let p = s.create_project("legacy", "/tmp/legacy").unwrap();
        s.grant(&p.id, &u.id).unwrap();
        s.connection.execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; ALTER TABLE project_grants DROP COLUMN active; ALTER TABLE project_grants DROP COLUMN revision; ALTER TABLE relay_jobs DROP COLUMN checkout_grant; ALTER TABLE interactive_ahp_clients DROP COLUMN views; PRAGMA user_version=24;").unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        let first = s.capture_native_grant(&p.id, &u.id).unwrap();
        assert_eq!(first.revision, 1);
        assert!(s.project_for_user(&p.id, &u.id).unwrap().is_some());
        s.revoke_grant(&p.id, &u.id).unwrap();
        drop(s);
        let s = Store::open(&path).unwrap();
        assert_eq!(s.native_grant_row(&p.id, &u.id).unwrap(), Some((2, false)));
        assert!(s.capture_native_grant(&p.id, &u.id).is_err());
        s.grant(&p.id, &u.id).unwrap();
        assert_eq!(s.capture_native_grant(&p.id, &u.id).unwrap().revision, 3);
        assert!(s.revalidate_native_grant(&first).is_err());
        assert_eq!(
            s.connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::store::SCHEMA_VERSION
        );
    }
    #[test]
    fn terminal_revision_reserves_revocation_and_never_reissues_positive_authority() {
        let (s, p, u) = fixture();
        s.grant(&p, &u).unwrap();
        s.connection
            .execute(
                "UPDATE project_grants SET revision=?3 WHERE project_id=?1 AND user_id=?2",
                params![p, u, (MAX_REVISION - 1) as i64],
            )
            .unwrap();
        let old = s.capture_native_grant(&p, &u).unwrap();
        assert_eq!(
            s.set_native_grant(&p, &u, false, Some(MAX_REVISION - 1))
                .unwrap(),
            MAX_REVISION
        );
        assert!(s.revalidate_native_grant(&old).is_err());
        assert!(
            s.set_native_grant(&p, &u, true, Some(MAX_REVISION))
                .is_err()
        );
        assert_eq!(
            s.native_grant_row(&p, &u).unwrap(),
            Some((MAX_REVISION, false))
        );
        assert_eq!(
            s.set_native_grant(&p, &u, false, Some(MAX_REVISION))
                .unwrap(),
            MAX_REVISION
        );

        let (s, p, u) = fixture();
        s.grant(&p, &u).unwrap();
        s.connection
            .execute(
                "UPDATE project_grants SET revision=?3,active=0 WHERE project_id=?1 AND user_id=?2",
                params![p, u, (MAX_REVISION - 1) as i64],
            )
            .unwrap();
        assert!(
            s.set_native_grant(&p, &u, true, Some(MAX_REVISION - 1))
                .is_err()
        );
        assert_eq!(
            s.native_grant_row(&p, &u).unwrap(),
            Some((MAX_REVISION - 1, false))
        );

        let (s, p, u) = fixture();
        s.grant(&p, &u).unwrap();
        s.connection
            .execute(
                "UPDATE project_grants SET revision=?3 WHERE project_id=?1 AND user_id=?2",
                params![p, u, MAX_REVISION as i64],
            )
            .unwrap();
        let imported = s.capture_native_grant(&p, &u).unwrap();
        let mut changes = s.subscribe_catalog();
        assert_eq!(
            s.set_native_grant(&p, &u, true, Some(MAX_REVISION))
                .unwrap(),
            MAX_REVISION
        );
        assert!(changes.try_recv().is_err());
        s.revoke_grant(&p, &u).unwrap();
        assert!(
            changes.try_recv().is_ok(),
            "Terminal revocation changes state even when its epoch cannot increase"
        );
        assert_eq!(
            s.native_grant_row(&p, &u).unwrap(),
            Some((MAX_REVISION, false))
        );
        assert!(s.revalidate_native_grant(&imported).is_err());
        assert!(s.grant(&p, &u).is_err());
        assert!(changes.try_recv().is_err());
        assert!(
            s.revalidate_native_grant(&NativeGrantStamp {
                revision: 0,
                ..imported
            })
            .is_err()
        );
    }
}
