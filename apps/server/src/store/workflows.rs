//! Immutable published definitions and CAS drafts. This module never executes a
//! graph or claims a successful run merely because its definition is valid.
use dunlin_domain::{collaboration::Permission, workflow::WorkflowDefinition};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Result, Store, StoreError, now_secs};

pub(super) const MIGRATION: &str = "
ALTER TABLE team_audit ADD COLUMN project_id TEXT REFERENCES team_projects(id);
CREATE TABLE team_workflow_drafts (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, project_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0), definition TEXT NOT NULL,
    updated_by TEXT NOT NULL REFERENCES users(id), updated_at INTEGER NOT NULL,
    FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id)
);
CREATE TABLE team_workflow_versions (
    workflow_id TEXT NOT NULL REFERENCES team_workflow_drafts(id),
    version INTEGER NOT NULL CHECK(version>0), draft_revision INTEGER NOT NULL,
    definition TEXT NOT NULL, digest TEXT NOT NULL,
    published_by TEXT NOT NULL REFERENCES users(id), published_at INTEGER NOT NULL,
    PRIMARY KEY(workflow_id,version), UNIQUE(workflow_id,draft_revision)
);
PRAGMA user_version=11;
";

const MAX_DEFINITION_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowDraft {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub revision: u64,
    pub definition: WorkflowDefinition,
    pub updated_by: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct WorkflowVersion {
    pub workflow_id: String,
    pub version: u64,
    pub draft_revision: u64,
    pub definition: WorkflowDefinition,
    pub digest: String,
    pub published_by: String,
    pub published_at: i64,
}

fn encoded(definition: &WorkflowDefinition) -> Result<String> {
    definition
        .validate()
        .map_err(|error| StoreError::Limit(format!("{}: {}", error.path, error.message)))?;
    let raw = serde_json::to_string(definition)?;
    if raw.len() > MAX_DEFINITION_BYTES {
        return Err(StoreError::Limit(
            "Workflow definition exceeds 1 MiB".into(),
        ));
    }
    Ok(raw)
}

impl Store {
    fn workflow_draft_row(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<WorkflowDraft>> {
        let row = self.connection.query_row(
            "SELECT revision,definition,updated_by,updated_at FROM team_workflow_drafts WHERE workspace_id=?1 AND project_id=?2 AND id=?3",
            params![workspace,project,id], |row| Ok((super::positive_revision(row, 0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        ).optional()?;
        row.map(|(revision, raw, updated_by, updated_at)| {
            Ok(WorkflowDraft {
                id: id.into(),
                workspace_id: workspace.into(),
                project_id: project.into(),
                revision,
                definition: serde_json::from_str(&raw)?,
                updated_by,
                updated_at,
            })
        })
        .transpose()
    }

    pub fn workflow_draft(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<WorkflowDraft>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        self.workflow_draft_row(workspace, project, id)
    }

    pub fn create_workflow(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowDraft> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::EditWorkflow)?;
        self.team_audit_capacity()?;
        let raw = encoded(&definition)?;
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM team_workflow_drafts WHERE project_id=?1",
            [project],
            |row| row.get(0),
        )?;
        if count >= 256 {
            return Err(StoreError::Limit("Project workflow limit reached".into()));
        }
        let id = format!("wfl-{}", uuid::Uuid::now_v7());
        let updated_at = now_secs();
        transaction.execute("INSERT INTO team_workflow_drafts(id,workspace_id,project_id,revision,definition,updated_by,updated_at) VALUES(?1,?2,?3,1,?4,?5,?6)", params![id,workspace,project,raw,actor,updated_at])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,?3,'workflow_created',1,?4)", params![workspace,project,actor,updated_at])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(WorkflowDraft {
            id,
            workspace_id: workspace.into(),
            project_id: project.into(),
            revision: 1,
            definition,
            updated_by: actor.into(),
            updated_at,
        })
    }

    pub fn save_workflow(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
        definition: WorkflowDefinition,
    ) -> Result<WorkflowDraft> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::EditWorkflow)?;
        self.team_audit_capacity()?;
        let raw = encoded(&definition)?;
        let revision = expected_revision
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Workflow revision limit reached".into()))?;
        let updated_at = now_secs();
        let changed = transaction.execute("UPDATE team_workflow_drafts SET revision=?1,definition=?2,updated_by=?3,updated_at=?4 WHERE id=?5 AND workspace_id=?6 AND project_id=?7 AND revision=?8",
            params![revision as i64,raw,actor,updated_at,id,workspace,project,expected_revision as i64])?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "Workflow changed or is unavailable; reload before editing".into(),
            ));
        }
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,?3,'workflow_draft_saved',?4,?5)", params![workspace,project,actor,revision as i64,updated_at])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(WorkflowDraft {
            id: id.into(),
            workspace_id: workspace.into(),
            project_id: project.into(),
            revision,
            definition,
            updated_by: actor.into(),
            updated_at,
        })
    }

    pub fn publish_workflow(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
    ) -> Result<WorkflowVersion> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::Publish)?;
        let draft = self
            .workflow_draft_row(workspace, project, id)?
            .filter(|draft| draft.revision == expected_revision)
            .ok_or_else(|| {
                StoreError::Conflict(
                    "Workflow changed or is unavailable; review before publishing".into(),
                )
            })?;
        let raw = encoded(&draft.definition)?;
        let digest = hex::encode(Sha256::digest(raw.as_bytes()));
        // Duplicate delivery returns the existing immutable version; never republish.
        let existing: Option<u64> = transaction.query_row("SELECT version FROM team_workflow_versions WHERE workflow_id=?1 AND draft_revision=?2", params![id,expected_revision as i64], |row| super::positive_revision(row, 0)).optional()?;
        if let Some(version) = existing {
            return self
                .workflow_version(actor, workspace, project, id, version)?
                .ok_or_else(|| StoreError::Conflict("Published workflow is unavailable".into()));
        }
        self.team_audit_capacity()?;
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM team_workflow_versions WHERE workflow_id=?1",
            [id],
            |row| row.get(0),
        )?;
        if count >= 1024 {
            return Err(StoreError::Limit("Workflow version limit reached".into()));
        }
        let version = (count + 1) as u64;
        let published_at = now_secs();
        transaction.execute("INSERT INTO team_workflow_versions(workflow_id,version,draft_revision,definition,digest,published_by,published_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![id,version as i64,expected_revision as i64,raw,digest,actor,published_at])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at) VALUES(?1,?2,?3,?3,'workflow_published',?4,?5)", params![workspace,project,actor,version as i64,published_at])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(WorkflowVersion {
            workflow_id: id.into(),
            version,
            draft_revision: expected_revision,
            definition: draft.definition,
            digest,
            published_by: actor.into(),
            published_at,
        })
    }

    pub fn workflow_version(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        version: u64,
    ) -> Result<Option<WorkflowVersion>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        self.load_workflow_version(workspace, project, id, version)
    }

    pub(super) fn load_workflow_version(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
        version: u64,
    ) -> Result<Option<WorkflowVersion>> {
        let row = self.connection.query_row(
            "SELECT v.draft_revision,v.definition,v.digest,v.published_by,v.published_at FROM team_workflow_versions v JOIN team_workflow_drafts d ON d.id=v.workflow_id WHERE d.workspace_id=?1 AND d.project_id=?2 AND v.workflow_id=?3 AND v.version=?4",
            params![workspace,project,id,i64::try_from(version).map_err(|_| StoreError::Limit("Invalid workflow version".into()))?],
            |row| Ok((super::positive_revision(row, 0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?)),
        ).optional()?;
        row.map(
            |(draft_revision, raw, digest, published_by, published_at)| {
                Ok(WorkflowVersion {
                    workflow_id: id.into(),
                    version,
                    draft_revision,
                    definition: serde_json::from_str(&raw)?,
                    digest,
                    published_by,
                    published_at,
                })
            },
        )
        .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_domain::collaboration::WorkspaceRole;
    use serde_json::json;

    fn definition(name: &str) -> WorkflowDefinition {
        serde_json::from_value(json!({"schema_version":1,"name":name,"limits":{"max_duration_seconds":60,"max_agent_turns":1,"max_step_attempts":1,"max_concurrent_steps":1},
            "nodes":[{"id":"start","name":"start","kind":{"type":"trigger","source":{"type":"manual"}},"inputs":{},"outputs":{}},{"id":"end","name":"end","kind":{"type":"complete"},"inputs":{},"outputs":{}}],
            "edges":[{"from":"start","to":"end","outcome":"always"}],"bindings":[],"outputs":{}})).unwrap()
    }

    #[test]
    fn draft_cas_and_immutable_published_versions_survive_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("relay.db");
        let store = Store::open(&path).unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "project")
            .unwrap();
        let draft = store
            .create_workflow(&owner.id, &workspace.id, &project.id, definition("first"))
            .unwrap();
        let first = store
            .publish_workflow(&owner.id, &workspace.id, &project.id, &draft.id, 1)
            .unwrap();
        assert_eq!(
            store
                .publish_workflow(&owner.id, &workspace.id, &project.id, &draft.id, 1)
                .unwrap()
                .version,
            first.version
        );
        store
            .save_workflow(
                &owner.id,
                &workspace.id,
                &project.id,
                &draft.id,
                1,
                definition("second"),
            )
            .unwrap();
        assert!(matches!(
            store.save_workflow(
                &owner.id,
                &workspace.id,
                &project.id,
                &draft.id,
                1,
                definition("stale")
            ),
            Err(StoreError::Conflict(_))
        ));
        assert!(
            store
                .publish_workflow(&owner.id, &workspace.id, &project.id, &draft.id, 1)
                .is_err()
        );
        let second = store
            .publish_workflow(&owner.id, &workspace.id, &project.id, &draft.id, 2)
            .unwrap();
        assert_eq!(second.version, 2);
        assert_ne!(first.digest, second.digest);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .workflow_version(&owner.id, &workspace.id, &project.id, &draft.id, 1)
                .unwrap()
                .unwrap()
                .definition
                .name,
            "first"
        );
        assert_eq!(
            store
                .workflow_draft(&owner.id, &workspace.id, &project.id, &draft.id)
                .unwrap()
                .unwrap()
                .definition
                .name,
            "second"
        );
    }

    #[test]
    fn membership_role_and_grant_revalidated_for_every_workflow_operation() {
        let store = Store::open_in_memory().unwrap();
        let owner = store.create_user("owner", "admin").unwrap();
        let viewer = store.create_user("viewer", "member").unwrap();
        let workspace = store.create_workspace(&owner, "team").unwrap();
        let project = store
            .create_team_project(&owner.id, &workspace.id, "project")
            .unwrap();
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &viewer.id,
                WorkspaceRole::Viewer,
                true,
                None,
            )
            .unwrap();
        store
            .set_team_project_grant(
                &owner.id,
                &workspace.id,
                &project.id,
                &viewer.id,
                super::super::team_projects::PROJECT_PERMISSIONS
                    .iter()
                    .copied()
                    .collect(),
                None,
            )
            .unwrap();
        let draft = store
            .create_workflow(&owner.id, &workspace.id, &project.id, definition("first"))
            .unwrap();
        assert!(
            store
                .workflow_draft(&viewer.id, &workspace.id, &project.id, &draft.id)
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .create_workflow(&viewer.id, &workspace.id, &project.id, definition("denied"))
                .is_err()
        );
        assert!(
            store
                .publish_workflow(&viewer.id, &workspace.id, &project.id, &draft.id, 1)
                .is_err()
        );
        store
            .set_team_member(
                &owner.id,
                &workspace.id,
                &viewer.id,
                WorkspaceRole::Viewer,
                false,
                Some(1),
            )
            .unwrap();
        assert!(
            store
                .workflow_draft(&viewer.id, &workspace.id, &project.id, &draft.id)
                .is_err()
        );
        let mut invalid = definition("invalid");
        invalid.edges.clear();
        assert!(
            store
                .create_workflow(&owner.id, &workspace.id, &project.id, invalid)
                .is_err()
        );
    }
}
