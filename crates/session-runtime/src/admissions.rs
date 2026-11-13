//! Durable HTTP admission identity; retries never reacquire or restart a runtime.
use super::*;

pub const ADMISSION_SCHEMA: &str = "
CREATE TABLE interactive_session_admissions (
 device TEXT NOT NULL, request_key TEXT NOT NULL,
 owner TEXT NOT NULL REFERENCES users(id), workspace TEXT NOT NULL, project TEXT NOT NULL,
 fingerprint TEXT NOT NULL, session TEXT NOT NULL UNIQUE REFERENCES interactive_sessions(id),
 response TEXT NOT NULL,
 PRIMARY KEY(device,request_key)
);
";
/// Stored in the same transaction as admission, including failed startups.
pub const ADMISSION_PROFILE_SCHEMA: &str = "
CREATE TABLE interactive_session_profiles (
 session TEXT PRIMARY KEY REFERENCES interactive_sessions(id),
 agent_id TEXT NOT NULL, version INTEGER NOT NULL CHECK(version>0), digest TEXT NOT NULL
);
";
const MAX_ADMISSIONS: i64 = 1024;
const MAX_DEVICE_ADMISSIONS: i64 = 128;
#[derive(Debug)]
pub struct AdmissionResult {
    pub session: Session,
    /// Present only for a newly admitted runtime. Historical retries cannot
    /// obtain a new lease, including after interruption or recovery.
    pub lease: Option<Lease>,
}
// Public response fields only: captured runtime/authority are never recreated
// from historical JSON. Current authorization is always checked independently.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptedSession {
    id: String,
    workspace: String,
    project: String,
    owner: String,
    phase: SessionPhase,
    epoch: u64,
    sequence: u64,
}
fn identity(device: &str, key: &str, fingerprint: &str) -> Result<()> {
    if !crate::dispatch::identifier(device)
        || !(16..=128).contains(&key.len())
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        || fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::Conflict("Invalid session admission identity"));
    }
    Ok(())
}
impl Journal<'_> {
    /// The immutable admission pin remains available even if startup failed
    /// before a harness-ready event, and survives recovery without dispatch.
    pub fn session_profile(
        &self,
        context: &AccessContext,
        id: &str,
    ) -> Result<Option<dunlin_domain::agent::AgentProfileRef>> {
        self.authorized(context, id, Permission::ViewProject)?;
        Ok(self
            .connection
            .query_row(
                "SELECT agent_id,version,digest FROM interactive_session_profiles WHERE session=?1",
                [id],
                |row| {
                    Ok(dunlin_domain::agent::AgentProfileRef {
                        agent_id: row.get(0)?,
                        version: u64::try_from(row.get::<_, i64>(1)?)
                            .ok()
                            .filter(|version| *version > 0)
                            .ok_or(rusqlite::Error::InvalidQuery)?,
                        digest: row.get(2)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn admission_lookup(
        &self,
        context: &AccessContext,
        device: &str,
        key: &str,
        fingerprint: &str,
    ) -> Result<Option<Session>> {
        identity(device, key, fingerprint)?;
        self.admission_lookup_checked(context, device, key, |_, previous| {
            if previous != fingerprint {
                return Err(Error::Conflict(
                    "Admission key was used with another request",
                ));
            }
            Ok(())
        })
    }

    /// Read-only receipt lookup with a host-owned intent check. The callback
    /// runs only after current ownership/project authorization and cannot yield
    /// a runtime lease. It may validate immutable pins without a live worker.
    /// The callback must remain SQL-only when the caller owns a transaction.
    pub fn admission_lookup_checked(
        &self,
        context: &AccessContext,
        device: &str,
        key: &str,
        check: impl FnOnce(&str, &str) -> Result<()>,
    ) -> Result<Option<Session>> {
        identity(device, key, &"0".repeat(64))?;
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        let grant = context
            .project_grant
            .as_ref()
            .ok_or(PolicyError::PermissionDenied)?;
        context.authorize_project(
            &grant.workspace_id,
            &grant.project_id,
            Permission::CreateSession,
        )?;
        let other_owner: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM interactive_session_admissions WHERE device=?1 AND owner<>?2)",
            params![device, context.membership.user_id], |row| row.get(0),
        )?;
        if other_owner {
            return Err(PolicyError::PermissionDenied.into());
        }
        let row: Option<(String,String,String,String,String,String)> = self.connection.query_row(
            "SELECT owner,workspace,project,fingerprint,session,response FROM interactive_session_admissions WHERE device=?1 AND request_key=?2",
            params![device,key], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)),
        ).optional()?;
        let Some((owner, workspace, project, previous, id, raw)) = row else {
            return Ok(None);
        };
        if owner != context.membership.user_id {
            return Err(PolicyError::PermissionDenied.into());
        }
        if workspace != grant.workspace_id || project != grant.project_id {
            return Err(Error::Conflict(
                "Admission key was used with another request",
            ));
        }
        context.authorize_project(&workspace, &project, Permission::CreateSession)?;
        // Validate the current session's ownership/scope too, without reviving
        // its captured lease or requiring its historical authority to be fresh.
        let current = self.authorized(context, &id, Permission::CreateSession)?;
        let accepted: AcceptedSession = serde_json::from_str(&raw)?;
        if current.workspace != workspace
            || current.project != project
            || current.owner != owner
            || accepted.id != id
            || accepted.workspace != workspace
            || accepted.project != project
            || accepted.owner != owner
        {
            return Err(Error::Conflict("Invalid persisted session admission"));
        }
        identity(device, key, &previous)?;
        check(&id, &previous)?;
        Ok(Some(Session {
            id: accepted.id,
            workspace: accepted.workspace,
            project: accepted.project,
            owner: accepted.owner,
            phase: accepted.phase,
            epoch: accepted.epoch,
            sequence: accepted.sequence,
            runtime: None,
            authority: None,
        }))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn create_admitted(
        &self,
        context: &AccessContext,
        workspace: &str,
        project: &str,
        device: &str,
        key: &str,
        fingerprint: &str,
        instance: &str,
    ) -> Result<AdmissionResult> {
        self.create_admitted_profile(
            context,
            workspace,
            project,
            device,
            key,
            fingerprint,
            instance,
            None,
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn create_admitted_profile(
        &self,
        context: &AccessContext,
        workspace: &str,
        project: &str,
        device: &str,
        key: &str,
        fingerprint: &str,
        instance: &str,
        profile: Option<&dunlin_domain::agent::AgentProfileRef>,
    ) -> Result<AdmissionResult> {
        if let Some(profile) = profile
            && (profile.agent_id.is_empty()
                || profile.agent_id.len() > 128
                || !profile
                    .agent_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                || profile.version == 0
                || profile.version > i64::MAX as u64
                || profile.digest.len() != 64
                || !profile
                    .digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
        {
            return Err(Error::Conflict("Invalid agent profile pin"));
        }
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        context.authorize_project(workspace, project, Permission::CreateSession)?;
        let tx = rusqlite::Transaction::new_unchecked(
            self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )?;
        let admitted = self.create_admitted_profile_in_transaction(
            context,
            workspace,
            project,
            device,
            key,
            fingerprint,
            instance,
            profile,
        )?;
        tx.commit()?;
        if admitted.lease.is_some() {
            self.notify_committed_admission()?;
        }
        Ok(admitted)
    }
    /// Compose admission with controller-native authority under the caller's
    /// fresh SQL transaction. No notification is published before outer commit.
    #[allow(clippy::too_many_arguments)]
    pub fn create_admitted_profile_in_transaction(
        &self,
        context: &AccessContext,
        workspace: &str,
        project: &str,
        device: &str,
        key: &str,
        fingerprint: &str,
        instance: &str,
        profile: Option<&dunlin_domain::agent::AgentProfileRef>,
    ) -> Result<AdmissionResult> {
        if self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Admission requires a fresh SQL transaction",
            ));
        }
        if let Some(profile) = profile
            && (profile.agent_id.is_empty()
                || profile.agent_id.len() > 128
                || !profile
                    .agent_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
                || profile.version == 0
                || profile.version > i64::MAX as u64
                || profile.digest.len() != 64
                || !profile
                    .digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)))
        {
            return Err(Error::Conflict("Invalid agent profile pin"));
        }
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        context.authorize_project(workspace, project, Permission::CreateSession)?;
        if let Some(session) = self.admission_lookup(context, device, key, fingerprint)? {
            return Ok(AdmissionResult {
                session,
                lease: None,
            });
        }
        let (total, own): (i64, i64) = self.connection.query_row(
            "SELECT count(*),coalesce(sum(device=?1),0) FROM interactive_session_admissions",
            [device],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if total >= MAX_ADMISSIONS || own >= MAX_DEVICE_ADMISSIONS {
            return Err(Error::Limit("Session admission ledger capacity reached"));
        }
        let session = self.create_inner(context, workspace, project)?;
        let lease = self.acquire_inner(context, &session.id, 0, instance)?;
        let session = self.session(&session.id)?;
        self.connection.execute(
            "INSERT INTO interactive_session_admissions(device,request_key,owner,workspace,project,fingerprint,session,response) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![device,key,session.owner,workspace,project,fingerprint,session.id,serde_json::to_string(&session)?],
        )?;
        if let Some(profile) = profile {
            self.connection.execute("INSERT INTO interactive_session_profiles(session,agent_id,version,digest) VALUES(?1,?2,?3,?4)",
                params![session.id,profile.agent_id,profile.version as i64,profile.digest])?;
        }
        Ok(AdmissionResult {
            session,
            lease: Some(lease),
        })
    }
    /// The owner of an outer transaction calls this only after successful commit.
    pub fn notify_committed_admission(&self) -> Result<()> {
        if !self.connection.is_autocommit() {
            return Err(Error::Conflict(
                "Admission notification requires committed SQL",
            ));
        }
        self.changed();
        Ok(())
    }
}
#[cfg(test)]
mod tests;
