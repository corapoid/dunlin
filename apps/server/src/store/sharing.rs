//! Explicit read sharing of interactive transcripts. No native grants or writes.
use super::*;
use dunlin_domain::collaboration::{Permission, PolicyError};
use dunlin_session_runtime::{ShareAudit, ShareState};
use serde::Serialize;

fn runtime_error(error: dunlin_session_runtime::Error) -> StoreError {
    match error {
        dunlin_session_runtime::Error::Policy(e) => StoreError::Policy(e),
        dunlin_session_runtime::Error::Conflict(e) => StoreError::Conflict(e.into()),
        dunlin_session_runtime::Error::Limit(e) => StoreError::Limit(e.into()),
        _ => StoreError::Limit("Session sharing is unavailable".into()),
    }
}
#[derive(Serialize)]
pub(crate) struct ShareUser {
    pub user_id: String,
    pub name: String,
}
#[derive(Serialize)]
pub(crate) struct ReaderView {
    pub user_id: String,
    pub name: String,
    pub active: bool,
}
#[derive(Serialize)]
pub(crate) struct SharingView {
    pub session_id: String,
    pub owner_id: String,
    pub revision: u64,
    pub readers: Vec<ReaderView>,
    pub eligible_readers: Vec<ShareUser>,
    pub eligible_readers_truncated: bool,
    pub audit: Vec<ShareAudit>,
}
impl Store {
    pub(crate) fn interactive_identity(&self, id: &str) -> Result<(String, String, String)> {
        self.connection
            .query_row(
                "SELECT workspace,project,owner FROM interactive_sessions WHERE id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or(PolicyError::PermissionDenied.into())
    }
    fn share_name(&self, id: &str) -> Result<String> {
        Ok(self
            .connection
            .query_row(
                "SELECT substr(name,1,128) FROM users WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_else(|| id.into()))
    }
    fn sharing_view_from(&self, id: &str, state: ShareState) -> Result<SharingView> {
        let (workspace, project, _) = self.interactive_identity(id)?;
        let mut candidates = {
            let mut q=self.connection.prepare("SELECT m.user_id,substr(u.name,1,128) FROM team_memberships m JOIN users u ON u.id=m.user_id JOIN team_project_grants g ON g.workspace_id=m.workspace_id AND g.user_id=m.user_id AND g.project_id=?2 WHERE m.workspace_id=?1 AND m.active=1 AND m.user_id<>?3 ORDER BY u.name,m.user_id LIMIT 257")?;
            q.query_map(params![workspace, project, state.owner_id], |r| {
                Ok(ShareUser {
                    user_id: r.get(0)?,
                    name: r.get(1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        let eligible_readers_truncated = candidates.len() > 256;
        candidates.truncate(256);
        let mut eligible_readers = Vec::new();
        for user in candidates {
            if self
                .team_project_context(&workspace, &project, &user.user_id)
                .and_then(|c| {
                    Ok(c.authorize_project(&workspace, &project, Permission::ViewSharedSession)?)
                })
                .is_ok()
            {
                eligible_readers.push(user);
            }
        }
        let mut readers = Vec::new();
        for entry in state.readers {
            readers.push(ReaderView {
                active: self
                    .ahp_access(&entry.user_id, &format!("ahp-chat:/{id}"))
                    .is_ok(),
                name: self.share_name(&entry.user_id)?,
                user_id: entry.user_id,
            });
        }
        let view = SharingView {
            session_id: id.into(),
            owner_id: state.owner_id,
            revision: state.revision,
            readers,
            eligible_readers,
            eligible_readers_truncated,
            audit: state.audit,
        };
        if serde_json::to_vec(&view)?.len() > 256 * 1024 {
            return Err(StoreError::Limit(
                "Sharing view exceeds its response budget".into(),
            ));
        }
        Ok(view)
    }
    pub(crate) fn session_sharing(&self, user: &str, id: &str) -> Result<SharingView> {
        let (workspace, project, _) = self.interactive_identity(id)?;
        let context = self.team_project_context(&workspace, &project, user)?;
        let state = self
            .interactive_journal()
            .sharing(&context, id)
            .map_err(runtime_error)?;
        self.sharing_view_from(id, state)
    }
    pub(crate) fn replace_session_readers(
        &self,
        user: &str,
        id: &str,
        revision: u64,
        readers: &[String],
    ) -> Result<SharingView> {
        if readers.len() > 32
            || readers
                .iter()
                .any(|s| s.is_empty() || s.len() > 128 || s.chars().any(char::is_control))
        {
            return Err(StoreError::Limit("Invalid reader list".into()));
        }
        let (workspace, project, _) = self.interactive_identity(id)?;
        let owner = self.team_project_context(&workspace, &project, user)?;
        let current = self
            .interactive_journal()
            .sharing(&owner, id)
            .map_err(runtime_error)?;
        if current.revision != revision {
            return Err(StoreError::Conflict(
                "Session sharing changed; reload before saving".into(),
            ));
        }
        let recipients = readers
            .iter()
            .map(|u| self.team_project_context(&workspace, &project, u))
            .collect::<Result<Vec<_>>>()?;
        let state = self
            .interactive_journal()
            .replace_readers(&owner, id, revision, &recipients)
            .map_err(runtime_error)?;
        self.sharing_view_from(id, state)
    }
}
