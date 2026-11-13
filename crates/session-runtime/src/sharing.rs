//! Explicit session readers. Current owner authority is supplied by the store,
//! never inferred from workspace administration or native filesystem access.
use super::*;
use dunlin_domain::collaboration::SessionSharePolicy;
use std::collections::{BTreeMap, BTreeSet};

pub const SHARING_SCHEMA:&str="
CREATE TABLE interactive_session_sharing (
 session TEXT PRIMARY KEY REFERENCES interactive_sessions(id), revision INTEGER NOT NULL CHECK(revision>0)
);
CREATE TABLE interactive_session_readers (
 session TEXT NOT NULL REFERENCES interactive_sessions(id), user_id TEXT NOT NULL REFERENCES users(id),
 recipient_stamp TEXT NOT NULL, owner_stamp TEXT NOT NULL, grant_revision INTEGER NOT NULL CHECK(grant_revision>0),
 PRIMARY KEY(session,user_id)
);
CREATE TABLE interactive_session_share_audit (
 session TEXT NOT NULL REFERENCES interactive_sessions(id), revision INTEGER NOT NULL,
 actor_id TEXT NOT NULL, added TEXT NOT NULL, removed TEXT NOT NULL, refreshed TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(session,revision)
);
";
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedReader {
    pub user_id: String,
    pub grant_revision: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareAudit {
    pub revision: u64,
    pub actor_id: String,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub refreshed: Vec<String>,
    pub created_at: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareState {
    pub revision: u64,
    pub owner_id: String,
    pub readers: Vec<SharedReader>,
    pub audit: Vec<ShareAudit>,
}
struct ReaderGrant {
    recipient: AuthorityStamp,
    owner: AuthorityStamp,
    revision: u64,
}
impl<'a> Journal<'a> {
    pub fn validating_shared_owner(
        mut self,
        validator: impl Fn(&AuthorityStamp) -> Result<()> + 'a,
    ) -> Self {
        self.shared_owner_validator = Some(Box::new(validator));
        self
    }
    fn sharing_present(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='interactive_session_readers')",
            [],
            |r| r.get(0),
        )?)
    }
    fn reader_grant(&self, id: &str, user: &str) -> Result<Option<ReaderGrant>> {
        if !self.sharing_present()? {
            return Ok(None);
        }
        let row=self.connection.query_row("SELECT recipient_stamp,owner_stamp,grant_revision FROM interactive_session_readers WHERE session=?1 AND user_id=?2",params![id,user],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,nonnegative(r,2)?))).optional()?;
        row.map(|(recipient, owner, revision)| {
            Ok(ReaderGrant {
                recipient: serde_json::from_str(&recipient)?,
                owner: serde_json::from_str(&owner)?,
                revision,
            })
        })
        .transpose()
    }
    pub(super) fn authorize_shared_reader(
        &self,
        context: &AccessContext,
        session: &Session,
    ) -> Result<()> {
        if context.principal != PrincipalKind::Human {
            return Err(PolicyError::PermissionDenied.into());
        }
        let grant = self
            .reader_grant(&session.id, &context.membership.user_id)?
            .ok_or(PolicyError::PermissionDenied)?;
        if grant.owner.user_id != session.owner
            || grant.owner.workspace_id != session.workspace
            || grant.owner.project_id != session.project
        {
            return Err(PolicyError::ScopeMismatch.into());
        }
        let policy = SessionSharePolicy {
            workspace_id: session.workspace.clone(),
            project_id: session.project.clone(),
            owner_id: session.owner.clone(),
            reader_ids: BTreeSet::from([context.membership.user_id.clone()]),
        };
        policy.authorize_read(context)?;
        context.revalidate(&grant.recipient, Permission::ViewSharedSession)?;
        self.shared_owner_validator
            .as_ref()
            .ok_or(PolicyError::PermissionDenied)?(&grant.owner)?;
        Ok(())
    }
    pub fn read_access_key(&self, context: &AccessContext, id: &str) -> Result<String> {
        let session = self.authorized(context, id, Permission::ViewProject)?;
        let current = context.authorize_project(
            &session.workspace,
            &session.project,
            Permission::ViewProject,
        )?;
        let material = if session.owner == context.membership.user_id {
            serde_json::json!(["owner", id, current])
        } else {
            let grant = self
                .reader_grant(id, &context.membership.user_id)?
                .ok_or(PolicyError::PermissionDenied)?;
            serde_json::json!([
                "shared",
                id,
                current,
                grant.recipient,
                grant.owner,
                grant.revision
            ])
        };
        Ok(hex::encode(Sha256::digest(serde_json::to_vec(&material)?)))
    }
    fn share_state(&self, id: &str, owner: &str) -> Result<ShareState> {
        let revision = self
            .connection
            .query_row(
                "SELECT revision FROM interactive_session_sharing WHERE session=?1",
                [id],
                |r| nonnegative(r, 0),
            )
            .optional()?
            .unwrap_or(0);
        let mut q=self.connection.prepare("SELECT user_id,grant_revision FROM interactive_session_readers WHERE session=?1 ORDER BY user_id LIMIT 33")?;
        let readers = q
            .query_map([id], |r| {
                Ok(SharedReader {
                    user_id: r.get(0)?,
                    grant_revision: nonnegative(r, 1)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        if readers.len() > 32 {
            return Err(Error::Limit("Session reader limit"));
        }
        let mut q=self.connection.prepare("SELECT revision,actor_id,added,removed,refreshed,created_at FROM interactive_session_share_audit WHERE session=?1 ORDER BY revision DESC LIMIT 20")?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    nonnegative(r, 0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let audit = rows
            .into_iter()
            .map(
                |(revision, actor_id, added, removed, refreshed, created_at)| {
                    Ok(ShareAudit {
                        revision,
                        actor_id,
                        added: serde_json::from_str(&added)?,
                        removed: serde_json::from_str(&removed)?,
                        refreshed: serde_json::from_str(&refreshed)?,
                        created_at,
                    })
                },
            )
            .collect::<Result<_>>()?;
        Ok(ShareState {
            revision,
            owner_id: owner.into(),
            readers,
            audit,
        })
    }
    pub fn sharing(&self, owner: &AccessContext, id: &str) -> Result<ShareState> {
        let tx = self.connection.unchecked_transaction()?;
        let session = self.authorized(owner, id, Permission::ShareOwnSession)?;
        let result = self.share_state(id, &session.owner)?;
        tx.commit()?;
        Ok(result)
    }
    pub fn replace_readers(
        &self,
        owner: &AccessContext,
        id: &str,
        expected_revision: u64,
        recipients: &[AccessContext],
    ) -> Result<ShareState> {
        if recipients.len() > 32 {
            return Err(Error::Limit("Session reader limit"));
        }
        let tx = self.connection.unchecked_transaction()?;
        let session = self.authorized(owner, id, Permission::ShareOwnSession)?;
        let revision = self
            .connection
            .query_row(
                "SELECT revision FROM interactive_session_sharing WHERE session=?1",
                [id],
                |r| nonnegative(r, 0),
            )
            .optional()?
            .unwrap_or(0);
        if revision != expected_revision {
            return Err(Error::Conflict("Session sharing revision changed"));
        }
        let policy = SessionSharePolicy {
            workspace_id: session.workspace.clone(),
            project_id: session.project.clone(),
            owner_id: session.owner.clone(),
            reader_ids: recipients
                .iter()
                .map(|c| c.membership.user_id.clone())
                .collect(),
        };
        if policy.reader_ids.len() != recipients.len() || policy.reader_ids.contains(&session.owner)
        {
            return Err(Error::Conflict("Duplicate or owner session recipient"));
        }
        let mut requested = BTreeMap::new();
        for recipient in recipients {
            let owner_stamp = policy.authorize_share(owner, recipient)?;
            policy.authorize_read(recipient)?;
            let recipient_stamp = recipient.authorize_project(
                &session.workspace,
                &session.project,
                Permission::ViewSharedSession,
            )?;
            requested.insert(
                recipient.membership.user_id.clone(),
                (recipient_stamp, owner_stamp),
            );
        }
        let mut q=self.connection.prepare("SELECT user_id,recipient_stamp,owner_stamp,grant_revision FROM interactive_session_readers WHERE session=?1 ORDER BY user_id")?;
        let rows = q
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    nonnegative(r, 3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(q);
        if rows.len() > 32 {
            return Err(Error::Limit("Session reader limit"));
        }
        let previous = rows
            .into_iter()
            .map(|(user, r, o, revision)| {
                Ok((
                    user,
                    ReaderGrant {
                        recipient: serde_json::from_str(&r)?,
                        owner: serde_json::from_str(&o)?,
                        revision,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>>>()?;
        let added: Vec<_> = requested
            .keys()
            .filter(|u| !previous.contains_key(*u))
            .cloned()
            .collect();
        let removed: Vec<_> = previous
            .keys()
            .filter(|u| !requested.contains_key(*u))
            .cloned()
            .collect();
        let refreshed: Vec<_> = requested
            .iter()
            .filter(|(user, (r, o))| {
                previous
                    .get(*user)
                    .is_some_and(|p| &p.recipient != r || &p.owner != o)
            })
            .map(|(user, _)| user.clone())
            .collect();
        if added.is_empty() && removed.is_empty() && refreshed.is_empty() {
            let result = self.share_state(id, &session.owner)?;
            tx.commit()?;
            return Ok(result);
        }
        let own: i64 = self.connection.query_row(
            "SELECT count(*) FROM interactive_session_share_audit WHERE session=?1",
            [id],
            |r| r.get(0),
        )?;
        // Revocation keeps a reserved row per possible reader even after the
        // ordinary capacity fills. No positive authority can use this reserve.
        if (own >= 1024 && (!added.is_empty() || !refreshed.is_empty()))
            || own >= 1056
            || revision >= 9_007_199_254_740_991
        {
            return Err(Error::Limit("Session sharing audit capacity"));
        }
        let next = revision + 1;
        for user in &removed {
            self.connection.execute(
                "DELETE FROM interactive_session_readers WHERE session=?1 AND user_id=?2",
                params![id, user],
            )?;
        }
        for (user, (recipient, owner_stamp)) in requested {
            let grant_revision = previous
                .get(&user)
                .filter(|p| p.recipient == recipient && p.owner == owner_stamp)
                .map(|p| p.revision)
                .unwrap_or(next);
            self.connection.execute("INSERT INTO interactive_session_readers VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session,user_id) DO UPDATE SET recipient_stamp=excluded.recipient_stamp,owner_stamp=excluded.owner_stamp,grant_revision=excluded.grant_revision",params![id,user,serde_json::to_string(&recipient)?,serde_json::to_string(&owner_stamp)?,sql_integer(grant_revision)?])?;
        }
        self.connection.execute("INSERT INTO interactive_session_sharing VALUES(?1,?2) ON CONFLICT(session) DO UPDATE SET revision=excluded.revision",params![id,sql_integer(next)?])?;
        self.connection.execute("INSERT INTO interactive_session_share_audit(session,revision,actor_id,added,removed,refreshed) VALUES(?1,?2,?3,?4,?5,?6)",params![id,sql_integer(next)?,owner.membership.user_id,serde_json::to_string(&added)?,serde_json::to_string(&removed)?,serde_json::to_string(&refreshed)?])?;
        let result = self.share_state(id, &session.owner)?;
        tx.commit()?;
        self.changed();
        Ok(result)
    }
}
#[cfg(test)]
mod tests;
