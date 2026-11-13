//! Store-owned identity and visibility for the AHP observer transport.
use super::*;
use ahp_types::state::SessionSummary;
use dunlin_domain::collaboration::{AccessContext, Permission};
use std::collections::BTreeMap;
pub(crate) struct AhpAccess {
    pub id: String,
    pub chat: bool,
    pub context: AccessContext,
    pub read_key: String,
}
pub(crate) struct AhpClient {
    pub version: String,
    pub boot: String,
    pub visibility: BTreeMap<String, String>,
    pub views: BTreeMap<String, usize>,
}
fn validate_views(views: &BTreeMap<String, usize>) -> Result<()> {
    if views.len() > 32
        || views.iter().any(|(channel, turns)| {
            *turns > 64
                || !channel.strip_prefix("ahp-chat:/").is_some_and(|id| {
                    !id.is_empty()
                        && id.len() <= 128
                        && id
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                })
        })
    {
        return Err(StoreError::Limit("Invalid AHP subscription views".into()));
    }
    Ok(())
}
impl Store {
    pub(crate) fn ahp_access(&self, user: &str, channel: &str) -> Result<AhpAccess> {
        let (id, chat) = if let Some(id) = channel.strip_prefix("ahp-chat:/") {
            (id, true)
        } else if let Some(id) = channel.strip_prefix("ahp-session:/") {
            (id, false)
        } else {
            return Err(StoreError::Limit("Unsupported AHP channel".into()));
        };
        if id.is_empty() || id.len() > 128 || id.contains('/') {
            return Err(StoreError::Limit("Invalid AHP channel".into()));
        }
        let (workspace, project) = self.interactive_scope(id)?;
        let context = self.team_project_context(&workspace, &project, user)?;
        context.authorize_project(&workspace, &project, Permission::ViewProject)?;
        // Ownership is checked by the journal in addition to current team grants.
        self.interactive_journal()
            .authorize_channel(&context, id)
            .map_err(|_| dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        let read_key = self
            .interactive_journal()
            .read_access_key(&context, id)
            .map_err(|_| dunlin_domain::collaboration::PolicyError::PermissionDenied)?;
        Ok(AhpAccess {
            id: id.into(),
            chat,
            context,
            read_key,
        })
    }
    pub(crate) fn ahp_summaries(&self, user: &str) -> Result<Vec<SessionSummary>> {
        let ids = {
            let mut q = self.connection.prepare(
                "SELECT id FROM interactive_sessions WHERE owner=?1 OR EXISTS(SELECT 1 FROM interactive_session_readers r WHERE r.session=interactive_sessions.id AND r.user_id=?1) ORDER BY id LIMIT 257",
            )?;
            q.query_map([user], |r| r.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if ids.len() > 256 {
            return Err(StoreError::Limit("AHP session catalog limit".into()));
        }
        let mut summaries = Vec::new();
        for id in ids {
            if let Ok(access) = self.ahp_access(user, &format!("ahp-session:/{id}"))
                && let Ok(mut summary) = self
                    .interactive_journal()
                    .channel_summary(&access.context, &id)
            {
                let (workspace, project, owner) = self.interactive_identity(&id)?;
                let meta = summary.meta.get_or_insert_with(Default::default);
                meta.insert("dunlin.shared".into(), serde_json::json!(owner != user));
                meta.insert(
                    "dunlin.canShare".into(),
                    serde_json::json!(
                        owner == user
                            && access
                                .context
                                .authorize_project(
                                    &workspace,
                                    &project,
                                    Permission::ShareOwnSession
                                )
                                .is_ok()
                    ),
                );
                meta.insert("dunlin.ownerId".into(), serde_json::json!(owner));
                meta.insert("dunlin.workspaceId".into(), serde_json::json!(workspace));
                meta.insert("dunlin.projectId".into(), serde_json::json!(project));
                meta.insert("dunlin.readKey".into(), serde_json::json!(access.read_key));
                summaries.push(summary);
            }
        }
        summaries.sort_by(|a, b| {
            b.modified_at
                .cmp(&a.modified_at)
                .then_with(|| a.resource.cmp(&b.resource))
        });
        Ok(summaries)
    }
    pub(crate) fn ahp_register_client(
        &self,
        device: &str,
        client: &str,
        version: &str,
        boot: &str,
        visibility: &BTreeMap<String, String>,
    ) -> Result<()> {
        self.connection.execute(
            "DELETE FROM interactive_ahp_clients WHERE updated<unixepoch()-28800",
            [],
        )?;
        let exists: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM interactive_ahp_clients WHERE device=?1 AND client=?2)",
            params![device, client],
            |r| r.get(0),
        )?;
        if !exists {
            let (all, own): (i64, i64) = self.connection.query_row(
                "SELECT count(*),coalesce(sum(device=?1),0) FROM interactive_ahp_clients",
                [device],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if all >= 1024 || own >= 32 {
                return Err(StoreError::Limit("AHP client registry limit".into()));
            }
        }
        self.connection.execute("INSERT INTO interactive_ahp_clients(device,client,version,boot,visibility) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(device,client) DO UPDATE SET version=excluded.version,boot=excluded.boot,visibility=excluded.visibility,updated=unixepoch()",params![device,client,version,boot,serde_json::to_string(visibility)?])?;
        Ok(())
    }
    pub(crate) fn ahp_set_visibility(
        &self,
        device: &str,
        client: &str,
        visibility: &BTreeMap<String, String>,
    ) -> Result<()> {
        let updated=self.connection.execute("UPDATE interactive_ahp_clients SET visibility=?3,updated=unixepoch() WHERE device=?1 AND client=?2",params![device,client,serde_json::to_string(visibility)?])?;
        if updated != 1 {
            return Err(StoreError::Conflict("Initialize this client first".into()));
        }
        Ok(())
    }
    pub(crate) fn ahp_set_views(
        &self,
        device: &str,
        client: &str,
        views: &BTreeMap<String, usize>,
    ) -> Result<()> {
        validate_views(views)?;
        let updated = self.connection.execute("UPDATE interactive_ahp_clients SET views=?3,updated=unixepoch() WHERE device=?1 AND client=?2 AND updated>=unixepoch()-28800",params![device,client,serde_json::to_string(views)?])?;
        if updated != 1 {
            return Err(StoreError::Conflict("Initialize this client first".into()));
        }
        Ok(())
    }
    pub(crate) fn ahp_client(&self, device: &str, client: &str) -> Result<Option<AhpClient>> {
        let row=self.connection.query_row("SELECT version,boot,visibility,views FROM interactive_ahp_clients WHERE device=?1 AND client=?2 AND updated>=unixepoch()-28800",params![device,client],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?))).optional()?;
        row.map(|(version, boot, raw, views)| {
            let views = serde_json::from_str(&views)?;
            validate_views(&views)?;
            Ok(AhpClient {
                version,
                boot,
                visibility: serde_json::from_str(&raw)?,
                views,
            })
        })
        .transpose()
    }
}

#[cfg(test)]
mod view_tests {
    use super::*;
    #[test]
    fn requested_history_views_survive_reopen_and_are_device_client_scoped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("views.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("views", "admin").unwrap();
        let device = store.create_device(&user.id, "one").unwrap().0.id;
        let other = store.create_device(&user.id, "two").unwrap().0.id;
        let visibility = BTreeMap::from([("ahp-chat:/one".into(), "read-epoch".into())]);
        for (device, client) in [(&device, "a"), (&device, "b"), (&other, "a")] {
            store
                .ahp_register_client(device, client, "1.0.0", "boot-a", &visibility)
                .unwrap();
        }
        let views = BTreeMap::from([("ahp-chat:/one".into(), 20), ("ahp-chat:/two".into(), 0)]);
        store.ahp_set_views(&device, "a", &views).unwrap();
        // Re-registering negotiation must not silently erase requested views.
        store
            .ahp_register_client(&device, "a", "1.0.0", "boot-b", &visibility)
            .unwrap();
        assert!(
            store
                .ahp_client(&other, "a")
                .unwrap()
                .unwrap()
                .views
                .is_empty()
        );
        assert!(
            store
                .ahp_client(&device, "b")
                .unwrap()
                .unwrap()
                .views
                .is_empty()
        );
        assert!(store.ahp_set_views(&device, "missing", &views).is_err());
        for invalid in [
            BTreeMap::from([("ahp-root://".into(), 1)]),
            BTreeMap::from([("ahp-chat:/".into(), 1)]),
            BTreeMap::from([("ahp-chat:/one".into(), 65)]),
            (0..33).map(|n| (format!("ahp-chat:/{n}"), 1)).collect(),
        ] {
            assert!(store.ahp_set_views(&device, "a", &invalid).is_err());
            assert_eq!(
                store.ahp_client(&device, "a").unwrap().unwrap().views,
                views
            );
        }
        drop(store);
        let store = Store::open(&path).unwrap();
        let client = store.ahp_client(&device, "a").unwrap().unwrap();
        assert_eq!(client.boot, "boot-b");
        assert_eq!(client.visibility, visibility);
        assert_eq!(client.views, views);
        store.ahp_set_views(&device, "a", &BTreeMap::new()).unwrap();
        assert!(
            store
                .ahp_client(&device, "a")
                .unwrap()
                .unwrap()
                .views
                .is_empty()
        );
        store.connection.execute("UPDATE interactive_ahp_clients SET updated=unixepoch()-28801 WHERE device=?1 AND client='a'",[&device]).unwrap();
        assert!(store.ahp_set_views(&device, "a", &views).is_err());
        assert!(store.ahp_client(&device, "a").unwrap().is_none());
    }
    #[test]
    fn version_26_clients_upgrade_to_empty_views_without_losing_negotiation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("upgrade.db");
        let store = Store::open(&path).unwrap();
        let user = store.create_user("upgrade-views", "admin").unwrap();
        let device = store.create_device(&user.id, "device").unwrap().0.id;
        let visibility = BTreeMap::from([("ahp-chat:/kept".into(), "read-epoch".into())]);
        store
            .ahp_register_client(&device, "client", "1.0.0", "boot", &visibility)
            .unwrap();
        store
            .connection
            .execute_batch(
                "DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; ALTER TABLE interactive_ahp_clients DROP COLUMN views; PRAGMA user_version=26;",
            )
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let client = store.ahp_client(&device, "client").unwrap().unwrap();
        assert_eq!(client.version, "1.0.0");
        assert_eq!(client.boot, "boot");
        assert_eq!(client.visibility, visibility);
        assert!(client.views.is_empty());
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::store::SCHEMA_VERSION
        );
    }
}

#[cfg(test)]
mod admission_migration_tests {
    use super::*;
    #[test]
    fn version_27_upgrade_preserves_views_and_sessions_and_durable_admission_reopens() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("admissions.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("admission-upgrade", "admin").unwrap();
        let device = store.create_device(&owner.id, "browser").unwrap().0.id;
        let workspace = store.create_workspace(&owner, "kept-workspace").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "kept-project")
            .unwrap();
        let context = store
            .team_project_context(&workspace.id, &project.id, &owner.id)
            .unwrap();
        let legacy = store
            .interactive_journal()
            .create(&context, &workspace.id, &project.id)
            .unwrap();
        let channel = format!("ahp-chat:/{}", legacy.id);
        let visibility = BTreeMap::from([(channel.clone(), "kept-read-epoch".into())]);
        let views = BTreeMap::from([(channel, 20)]);
        store
            .ahp_register_client(&device, "client", "1.0.0", "old-boot", &visibility)
            .unwrap();
        store.ahp_set_views(&device, "client", &views).unwrap();
        store
            .connection
            .execute_batch("DROP TABLE IF EXISTS scoped_secret_grants; DROP TABLE IF EXISTS scoped_secret_references; DROP TABLE IF EXISTS scoped_credentials; ALTER TABLE team_audit DROP COLUMN resource_id; DROP TABLE IF EXISTS interactive_mcp_inputs; DROP TABLE mcp_oauth_secret_references; DROP TABLE mcp_oauth_connections; DROP TABLE controller_remote_session_stream; DROP TABLE controller_remote_session_turns; DROP TABLE controller_remote_sessions; DROP TABLE controller_remote_execution_outcomes; DROP TABLE controller_remote_executions; ALTER TABLE relay_jobs DROP COLUMN checkout_stamp; ALTER TABLE team_local_bindings DROP COLUMN metadata_approval_revision; DROP TABLE team_local_binding_revocations; DROP TABLE native_metadata_grants; DROP TABLE native_metadata_audit; DROP TABLE native_metadata_revocations; DROP TABLE native_metadata_approvals; DROP TABLE team_remote_binding_revocations; DROP TABLE team_remote_bindings; DROP TABLE team_worker_channels; DROP TABLE team_host_revocations; DROP TABLE team_hosts; DROP TABLE interactive_session_profiles; DROP TABLE team_agent_versions; DROP TABLE team_agent_drafts; DROP TABLE interactive_session_admissions; PRAGMA user_version=27;")
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        let client = store.ahp_client(&device, "client").unwrap().unwrap();
        assert_eq!(client.views, views);
        assert_eq!(client.visibility, visibility);
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT count(*) FROM interactive_session_admissions",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let context = store
            .team_project_context(&workspace.id, &project.id, &owner.id)
            .unwrap();
        assert_eq!(
            store
                .interactive_journal()
                .snapshot(&context, &legacy.id)
                .unwrap()
                .0
                .phase,
            dunlin_session_runtime::SessionPhase::Pending
        );
        let fingerprint = "b".repeat(64);
        let accepted = store
            .interactive_journal()
            .create_admitted(
                &context,
                &workspace.id,
                &project.id,
                &device,
                "migration-admission-0001",
                &fingerprint,
                "original-runtime",
            )
            .unwrap();
        assert!(accepted.lease.is_some());
        let response = serde_json::to_value(&accepted.session).unwrap();
        assert!(response.get("runtime").is_none() && response.get("authority").is_none());
        drop(store);
        let store = Store::open(&path).unwrap();
        let context = store
            .team_project_context(&workspace.id, &project.id, &owner.id)
            .unwrap();
        let retry = store
            .interactive_journal()
            .create_admitted(
                &context,
                &workspace.id,
                &project.id,
                &device,
                "migration-admission-0001",
                &fingerprint,
                "never-acquired",
            )
            .unwrap();
        assert!(retry.lease.is_none());
        assert_eq!(serde_json::to_value(&retry.session).unwrap(), response);
        assert_eq!(
            store
                .connection
                .query_row("SELECT count(*) FROM interactive_sessions", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .connection
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
                .unwrap(),
            crate::store::SCHEMA_VERSION
        );
        assert_eq!(
            store.ahp_client(&device, "client").unwrap().unwrap().views,
            views
        );
    }
}
