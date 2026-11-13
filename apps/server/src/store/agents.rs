//! Immutable published definitions and CAS drafts. This module never executes a
//! profile or starts a process merely because its definition is valid.
use dunlin_domain::{
    agent::{AgentDefinition, MAX_AGENT_DEFINITION_BYTES},
    collaboration::Permission,
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{Result, Store, StoreError, now_secs};

pub(crate) const AGENT_SCHEMA: &str = "
CREATE TABLE team_agent_drafts (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, project_id TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision>0), definition TEXT NOT NULL,
    updated_by TEXT NOT NULL REFERENCES users(id), updated_at INTEGER NOT NULL,
    FOREIGN KEY(workspace_id,project_id) REFERENCES team_projects(workspace_id,id)
);
CREATE TABLE team_agent_versions (
    agent_id TEXT NOT NULL REFERENCES team_agent_drafts(id),
    version INTEGER NOT NULL CHECK(version>0), draft_revision INTEGER NOT NULL,
    definition TEXT NOT NULL, digest TEXT NOT NULL,
    published_by TEXT NOT NULL REFERENCES users(id), published_at INTEGER NOT NULL,
    PRIMARY KEY(agent_id,version), UNIQUE(agent_id,draft_revision)
);
";

#[derive(Clone, Debug, Serialize)]
pub struct AgentDraft {
    pub id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub revision: u64,
    pub definition: AgentDefinition,
    pub updated_by: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AgentVersion {
    pub agent_id: String,
    pub version: u64,
    pub draft_revision: u64,
    pub definition: AgentDefinition,
    pub digest: String,
    pub published_by: String,
    pub published_at: i64,
}

fn encoded(definition: &AgentDefinition) -> Result<String> {
    definition
        .validate()
        .map_err(|error| StoreError::Limit(format!("{}: {}", error.path, error.message)))?;
    let raw = serde_json::to_string(definition)?;
    if raw.len() > MAX_AGENT_DEFINITION_BYTES {
        return Err(StoreError::Limit("Agent definition exceeds 64 KiB".into()));
    }
    Ok(raw)
}

impl Store {
    fn agent_draft_row(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<AgentDraft>> {
        let row = self.connection.query_row(
            "SELECT revision,definition,updated_by,updated_at FROM team_agent_drafts WHERE workspace_id=?1 AND project_id=?2 AND id=?3",
            params![workspace,project,id], |row| Ok((super::positive_revision(row, 0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        ).optional()?;
        row.map(|(revision, raw, updated_by, updated_at)| {
            Ok(AgentDraft {
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

    pub fn agent_drafts(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Vec<AgentDraft>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        let mut stmt = self.connection.prepare("SELECT id FROM team_agent_drafts WHERE workspace_id=?1 AND project_id=?2 ORDER BY updated_at DESC,id LIMIT 128")?;
        let ids = stmt
            .query_map(params![workspace, project], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                self.agent_draft_row(workspace, project, &id)?
                    .ok_or_else(|| StoreError::Conflict("Agent draft is unavailable".into()))
            })
            .collect()
    }
    pub fn agent_profiles(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
    ) -> Result<Vec<AgentVersion>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        let mut stmt=self.connection.prepare("SELECT d.id,MAX(v.version) FROM team_agent_drafts d JOIN team_agent_versions v ON v.agent_id=d.id WHERE d.workspace_id=?1 AND d.project_id=?2 GROUP BY d.id ORDER BY d.id LIMIT 128")?;
        let keys = stmt
            .query_map(params![workspace, project], |row| {
                Ok((row.get::<_, String>(0)?, super::positive_revision(row, 1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        keys.into_iter()
            .map(|(id, version)| {
                self.load_agent_version(workspace, project, &id, version)?
                    .ok_or_else(|| StoreError::Conflict("Published agent is unavailable".into()))
            })
            .collect()
    }

    pub fn agent_draft(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
    ) -> Result<Option<AgentDraft>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        self.agent_draft_row(workspace, project, id)
    }

    pub fn create_agent(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        definition: AgentDefinition,
    ) -> Result<AgentDraft> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        self.team_audit_capacity()?;
        let raw = encoded(&definition)?;
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM team_agent_drafts WHERE project_id=?1",
            [project],
            |row| row.get(0),
        )?;
        if count >= 128 {
            return Err(StoreError::Limit("Project agent limit reached".into()));
        }
        let id = format!("agt-{}", uuid::Uuid::now_v7());
        let updated_at = now_secs();
        transaction.execute("INSERT INTO team_agent_drafts(id,workspace_id,project_id,revision,definition,updated_by,updated_at) VALUES(?1,?2,?3,1,?4,?5,?6)", params![id,workspace,project,raw,actor,updated_at])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,?3,'agent_created',1,?4,?5)", params![workspace,project,actor,updated_at,id])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(AgentDraft {
            id,
            workspace_id: workspace.into(),
            project_id: project.into(),
            revision: 1,
            definition,
            updated_by: actor.into(),
            updated_at,
        })
    }

    pub fn save_agent(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
        definition: AgentDefinition,
    ) -> Result<AgentDraft> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        self.team_audit_capacity()?;
        let raw = encoded(&definition)?;
        if expected_revision == 0 {
            return Err(StoreError::Conflict("Invalid agent revision".into()));
        }
        let revision = expected_revision
            .checked_add(1)
            .filter(|value| *value <= i64::MAX as u64)
            .ok_or_else(|| StoreError::Limit("Agent revision limit reached".into()))?;
        let updated_at = now_secs();
        let changed = transaction.execute("UPDATE team_agent_drafts SET revision=?1,definition=?2,updated_by=?3,updated_at=?4 WHERE id=?5 AND workspace_id=?6 AND project_id=?7 AND revision=?8",
            params![revision as i64,raw,actor,updated_at,id,workspace,project,expected_revision as i64])?;
        if changed != 1 {
            return Err(StoreError::Conflict(
                "Agent changed or is unavailable; reload before editing".into(),
            ));
        }
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,?3,'agent_draft_saved',?4,?5,?6)", params![workspace,project,actor,revision as i64,updated_at,id])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(AgentDraft {
            id: id.into(),
            workspace_id: workspace.into(),
            project_id: project.into(),
            revision,
            definition,
            updated_by: actor.into(),
            updated_at,
        })
    }

    pub fn publish_agent(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        expected_revision: u64,
    ) -> Result<AgentVersion> {
        let transaction = self.connection.unchecked_transaction()?;
        self.authorize_team_project(actor, workspace, project, Permission::ManageProject)?;
        let draft = self
            .agent_draft_row(workspace, project, id)?
            .filter(|draft| draft.revision == expected_revision)
            .ok_or_else(|| {
                StoreError::Conflict(
                    "Agent changed or is unavailable; review before publishing".into(),
                )
            })?;
        let raw = encoded(&draft.definition)?;
        let digest = hex::encode(Sha256::digest(raw.as_bytes()));
        // Duplicate delivery returns the existing immutable version; never republish.
        let existing: Option<u64> = transaction
            .query_row(
                "SELECT version FROM team_agent_versions WHERE agent_id=?1 AND draft_revision=?2",
                params![id, expected_revision as i64],
                |row| super::positive_revision(row, 0),
            )
            .optional()?;
        if let Some(version) = existing {
            return self
                .load_agent_version(workspace, project, id, version)?
                .ok_or_else(|| StoreError::Conflict("Published agent is unavailable".into()));
        }
        self.team_audit_capacity()?;
        let count: i64 = transaction.query_row(
            "SELECT COUNT(*) FROM team_agent_versions WHERE agent_id=?1",
            [id],
            |row| row.get(0),
        )?;
        if count >= 128 {
            return Err(StoreError::Limit("Agent version limit reached".into()));
        }
        let version = (count + 1) as u64;
        let published_at = now_secs();
        transaction.execute("INSERT INTO team_agent_versions(agent_id,version,draft_revision,definition,digest,published_by,published_at) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![id,version as i64,expected_revision as i64,raw,digest,actor,published_at])?;
        transaction.execute("INSERT INTO team_audit(workspace_id,project_id,actor_id,target_id,action,revision,created_at,resource_id) VALUES(?1,?2,?3,?3,'agent_published',?4,?5,?6)", params![workspace,project,actor,version as i64,published_at,id])?;
        transaction.commit()?;
        self.catalog_changed();
        Ok(AgentVersion {
            agent_id: id.into(),
            version,
            draft_revision: expected_revision,
            definition: draft.definition,
            digest,
            published_by: actor.into(),
            published_at,
        })
    }

    pub fn agent_version(
        &self,
        actor: &str,
        workspace: &str,
        project: &str,
        id: &str,
        version: u64,
    ) -> Result<Option<AgentVersion>> {
        self.authorize_team_project(actor, workspace, project, Permission::ViewProject)?;
        self.load_agent_version(workspace, project, id, version)
    }

    pub(super) fn load_agent_version(
        &self,
        workspace: &str,
        project: &str,
        id: &str,
        version: u64,
    ) -> Result<Option<AgentVersion>> {
        let row = self.connection.query_row(
            "SELECT v.draft_revision,v.definition,v.digest,v.published_by,v.published_at FROM team_agent_versions v JOIN team_agent_drafts d ON d.id=v.agent_id WHERE d.workspace_id=?1 AND d.project_id=?2 AND v.agent_id=?3 AND v.version=?4",
            params![workspace,project,id,i64::try_from(version).map_err(|_| StoreError::Limit("Invalid agent version".into()))?],
            |row| Ok((super::positive_revision(row, 0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, i64>(4)?)),
        ).optional()?;
        row.map(
            |(draft_revision, raw, digest, published_by, published_at)| {
                Ok(AgentVersion {
                    agent_id: id.into(),
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
#[path = "agents_tests.rs"]
mod tests;
