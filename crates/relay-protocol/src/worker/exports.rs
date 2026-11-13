//! Path-free, bounded metadata from an operator-approved worker export. A
//! descriptor is an observation, never a filesystem grant or execution lease.
use super::{Result, identifier, invalid};
use serde::{Deserialize, Serialize};
pub const MAX_EXPORT_REVISION: u64 = 9_007_199_254_740_991;
fn revision(value: u64, path: &str) -> Result<()> {
    if !(1..=MAX_EXPORT_REVISION).contains(&value) {
        return Err(invalid(
            path,
            "A positive safe integer revision is required",
        ));
    }
    Ok(())
}
fn digest(value: &str, path: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(invalid(path, "A lowercase SHA-256 digest is required"));
    }
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrincipalKind {
    Human,
    Service,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportPrincipal {
    pub kind: PrincipalKind,
    pub id: String,
}
impl ExportPrincipal {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.id, "principal.id")
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportPermission {
    Read,
    Execute,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoKind {
    Directory,
    Git,
    LinkedWorktree,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportScope {
    pub controller_node_id: String,
    pub worker_node_id: String,
    pub workspace_id: String,
    pub host_id: String,
    pub enrollment_epoch: u64,
    pub credential_revision: u64,
}
impl ExportScope {
    pub fn validate(&self) -> Result<()> {
        for value in [
            &self.controller_node_id,
            &self.worker_node_id,
            &self.workspace_id,
            &self.host_id,
        ] {
            identifier(value, "scope")?;
        }
        if self.controller_node_id == self.worker_node_id {
            return Err(invalid(
                "scope",
                "Distinct worker and controller nodes are required",
            ));
        }
        revision(self.enrollment_epoch, "enrollment_epoch")?;
        revision(self.credential_revision, "credential_revision")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportSummary {
    pub id: String,
    pub name: String,
    pub repo_kind: RepoKind,
    pub checkout_identity_digest: String,
    pub settings_digest: String,
    pub revision: u64,
    pub active: bool,
}
impl ExportSummary {
    pub fn validate(&self) -> Result<()> {
        identifier(&self.id, "export.id")?;
        if self.name.trim().is_empty()
            || self.name.len() > 128
            || self.name.chars().any(char::is_control)
        {
            return Err(invalid("export.name", "A bounded export name is required"));
        }
        if !self.active {
            return Err(invalid(
                "export.active",
                "Only active exports may be described",
            ));
        }
        digest(&self.checkout_identity_digest, "checkout_identity_digest")?;
        digest(&self.settings_digest, "settings_digest")?;
        revision(self.revision, "export.revision")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportGrant {
    pub principal: ExportPrincipal,
    pub permissions: Vec<ExportPermission>,
    pub revision: u64,
    pub export_revision: u64,
    pub active: bool,
}
impl ExportGrant {
    pub fn validate(&self) -> Result<()> {
        self.principal.validate()?;
        if !self.active
            || !(1..=2).contains(&self.permissions.len())
            || !self.permissions.contains(&ExportPermission::Read)
            || self
                .permissions
                .iter()
                .enumerate()
                .any(|(i, p)| self.permissions[..i].contains(p))
        {
            return Err(invalid(
                "grant",
                "A current explicit read grant is required",
            ));
        }
        revision(self.revision, "grant.revision")?;
        revision(self.export_revision, "grant.export_revision")
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportDescriptor {
    pub scope: ExportScope,
    pub export: ExportSummary,
    pub grant: ExportGrant,
}
impl ExportDescriptor {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        self.export.validate()?;
        self.grant.validate()?;
        if self.export.revision != self.grant.export_revision {
            return Err(invalid(
                "grant.export_revision",
                "Export and grant revisions differ",
            ));
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportErrorCode {
    Unavailable,
    Forbidden,
    StaleExport,
    Busy,
}
pub(super) fn request(
    export_id: &str,
    principal: &ExportPrincipal,
    expected: Option<u64>,
) -> Result<()> {
    identifier(export_id, "export_id")?;
    principal.validate()?;
    if let Some(expected) = expected {
        revision(expected, "expected_export_revision")?;
    }
    Ok(())
}
