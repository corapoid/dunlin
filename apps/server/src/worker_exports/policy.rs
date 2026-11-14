//! Operator-owned execution policy snapshots. These do not enable execution.
use super::{Error, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
const MAX_BYTES: usize = 32 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutionPolicy {
    schema_version: u16,
    harnesses: Vec<String>,
    sandbox_required: bool,
    credential_mode: CredentialMode,
    max_seconds: u64,
    max_output_bytes: u64,
    max_parallel: u16,
    #[serde(default)]
    acknowledge_repository_wide_metadata_write: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_mcp_broker"
    )]
    mcp_broker: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "explicit_recipes"
    )]
    recipes: Option<Vec<RecipeProgram>>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeProgram {
    program: String,
    template: dunlin_relay_protocol::worker::recipe::RecipeTemplate,
}
fn explicit_recipes<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<Vec<RecipeProgram>>, D::Error> {
    Vec::<RecipeProgram>::deserialize(deserializer).map(Some)
}
fn explicit_mcp_broker<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<bool>, D::Error> {
    bool::deserialize(deserializer).map(Some)
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CredentialMode {
    Isolated,
}
/// No Deserialize: only bounded validated raw bytes construct a snapshot.
pub(crate) struct PolicySnapshot {
    raw: Vec<u8>,
    digest: String,
}
impl PolicySnapshot {
    fn parse(raw: Vec<u8>) -> Result<Self> {
        if raw.is_empty() || raw.len() > MAX_BYTES {
            return Err(Error::Invalid("Execution policy exceeds its byte limit"));
        }
        let policy: ExecutionPolicy = serde_json::from_slice(&raw)?;
        let unique: std::collections::BTreeSet<_> = policy.harnesses.iter().collect();
        if !matches!(
            (
                policy.schema_version,
                policy.mcp_broker,
                policy.recipes.as_ref()
            ),
            (1, None, None) | (2, Some(true), None) | (3, Some(true), Some(_))
        ) || !policy.sandbox_required
            || policy.harnesses.is_empty()
            || policy.harnesses.len() > 3
            || unique.len() != policy.harnesses.len()
            || policy
                .harnesses
                .iter()
                .any(|h| !matches!(h.as_str(), "opencode" | "codex" | "claude"))
            || !(1..=3600).contains(&policy.max_seconds)
            || !(1024..=16_777_216).contains(&policy.max_output_bytes)
            || !(1..=8).contains(&policy.max_parallel)
        {
            return Err(Error::Invalid("Unsupported execution policy"));
        }
        if let Some(recipes) = &policy.recipes {
            let names: std::collections::BTreeSet<_> =
                recipes.iter().map(|r| &r.template.name).collect();
            if recipes.is_empty()
                || recipes.len() > 16
                || names.len() != recipes.len()
                || recipes.iter().any(|r| {
                    r.template.timeout_secs > policy.max_seconds
                        || r.template.validate_program(&r.program).is_err()
                })
            {
                return Err(Error::Invalid("Invalid operator recipe templates"));
            }
        }
        let digest = hex::encode(Sha256::digest(&raw));
        Ok(Self { raw, digest })
    }
    pub(super) fn validate_checkout(&self, checkout: &super::CapturedCheckout) -> Result<()> {
        let parsed: ExecutionPolicy = serde_json::from_slice(&self.raw)?;
        if checkout.repo_kind() == super::RepoKind::LinkedWorktree
            && !parsed.acknowledge_repository_wide_metadata_write
        {
            return Err(Error::Invalid(
                "Linked execution policy requires explicit repository-wide metadata write acknowledgement",
            ));
        }
        if parsed.recipes.as_ref().is_some_and(|recipes| {
            recipes.iter().any(|r| {
                let program = Path::new(&r.program);
                program.starts_with(checkout.canonical_root())
                    || std::fs::canonicalize(program)
                        .is_ok_and(|p| p.starts_with(checkout.canonical_root()))
            })
        }) {
            return Err(Error::Invalid(
                "Operator recipe programs must be outside the exported checkout",
            ));
        }
        if self.digest != checkout.settings_digest() {
            return Err(Error::Invalid(
                "Execution policy digest does not match checkout settings",
            ));
        }
        Ok(())
    }
    pub fn digest(&self) -> &str {
        &self.digest
    }
    pub(super) fn permits_mcp_broker(&self) -> Result<bool> {
        let policy: ExecutionPolicy = serde_json::from_slice(&self.raw)?;
        Ok(matches!(policy.schema_version, 2 | 3) && policy.mcp_broker == Some(true))
    }
    #[cfg(unix)]
    pub fn load(path: &Path) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        if !path.is_absolute() {
            return Err(Error::Invalid("Execution policy path must be absolute"));
        }
        use std::os::{
            fd::{AsRawFd, FromRawFd, OwnedFd},
            unix::ffi::OsStrExt,
        };
        // Walk directory descriptors: no parent symlink can be swapped in between
        // a path check and the final open. The final descriptor owns the bytes read.
        let mut directory = std::fs::File::open("/")?;
        let components: Vec<_> = path
            .components()
            .filter(|c| !matches!(c, std::path::Component::RootDir))
            .collect();
        if components.is_empty() {
            return Err(Error::Invalid("Execution policy must be a file"));
        }
        let mut opened = None;
        for (index, part) in components.iter().enumerate() {
            let std::path::Component::Normal(name) = part else {
                return Err(Error::Invalid(
                    "Execution policy path must contain only normal components",
                ));
            };
            let name = std::ffi::CString::new(name.as_bytes())
                .map_err(|_| Error::Invalid("Invalid execution policy path"))?;
            let last = index + 1 == components.len();
            let flags = libc::O_RDONLY
                | libc::O_NOFOLLOW
                | libc::O_CLOEXEC
                | if last {
                    libc::O_NONBLOCK
                } else {
                    libc::O_DIRECTORY
                };
            let descriptor = unsafe { libc::openat(directory.as_raw_fd(), name.as_ptr(), flags) };
            if descriptor < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            let fd = unsafe { OwnedFd::from_raw_fd(descriptor) };
            let file = std::fs::File::from(fd);
            if last {
                opened = Some(file);
            } else {
                directory = file;
            }
        }
        let file = opened.ok_or(Error::Invalid("Execution policy must be a file"))?;
        let before = file.metadata()?;
        if !before.is_file()
            || before.nlink() != 1
            || before.uid() != unsafe { libc::geteuid() }
            || before.mode() & 0o022 != 0
            || before.len() > MAX_BYTES as u64
        {
            return Err(Error::Invalid(
                "Execution policy must be an owned non-writable regular file with one link",
            ));
        }
        let mut raw = Vec::new();
        let mut file = file;
        (&mut file)
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut raw)?;
        let after = file.metadata()?;
        if before.dev() != after.dev()
            || before.ino() != after.ino()
            || before.len() != after.len()
            || before.mtime() != after.mtime()
            || before.mtime_nsec() != after.mtime_nsec()
            || before.ctime() != after.ctime()
            || before.ctime_nsec() != after.ctime_nsec()
        {
            return Err(Error::Conflict("Execution policy changed while reading"));
        }
        Self::parse(raw)
    }
    #[cfg(not(unix))]
    pub fn load(_path: &Path) -> Result<Self> {
        Err(Error::Invalid(
            "Execution policy loading requires macOS or Linux",
        ))
    }
}

pub(super) const SCHEMA: &str = "
CREATE TABLE export_execution_policies(export_id TEXT PRIMARY KEY REFERENCES worker_exports(id), export_revision INTEGER NOT NULL CHECK(export_revision BETWEEN 1 AND 9007199254740991), raw BLOB NOT NULL CHECK(length(raw) BETWEEN 1 AND 32768), digest TEXT NOT NULL CHECK(length(digest)=64));
PRAGMA user_version=2;
";
impl super::WorkerStore {
    pub(super) fn write_policy(
        &self,
        id: &str,
        settings_digest: &str,
        policy: Option<&PolicySnapshot>,
    ) -> Result<()> {
        self.connection.execute(
            "DELETE FROM export_execution_policies WHERE export_id=?1",
            [id],
        )?;
        if let Some(policy) = policy {
            if policy.digest != settings_digest {
                return Err(Error::Invalid(
                    "Execution policy digest does not match checkout settings",
                ));
            }
            self.connection.execute("INSERT INTO export_execution_policies SELECT id,revision,?2,?3 FROM worker_exports WHERE id=?1", rusqlite::params![id,policy.raw,policy.digest])?;
        }
        Ok(())
    }
    pub(super) fn policy_snapshot(
        &self,
        id: &str,
        settings_digest: &str,
    ) -> Result<Option<PolicySnapshot>> {
        use rusqlite::OptionalExtension;
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version == 1 {
            return Ok(None);
        }
        let stored: Option<(Vec<u8>, String, i64, i64, String)> = self.connection.query_row("SELECT p.raw,p.digest,p.export_revision,e.revision,e.repo_kind FROM export_execution_policies p JOIN worker_exports e ON e.id=p.export_id WHERE p.export_id=?1", [id], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?))).optional()?;
        stored
            .map(
                |(raw, digest, policy_revision, export_revision, repo_kind)| {
                    let snapshot = PolicySnapshot::parse(raw)?;
                    if snapshot.digest != digest
                        || digest != settings_digest
                        || policy_revision != export_revision
                    {
                        return Err(Error::Denied);
                    }
                    let repo_kind: super::RepoKind = serde_json::from_str(&repo_kind)?;
                    let parsed: ExecutionPolicy = serde_json::from_slice(&snapshot.raw)?;
                    if repo_kind == super::RepoKind::LinkedWorktree
                        && !parsed.acknowledge_repository_wide_metadata_write
                    {
                        return Err(Error::Denied);
                    }
                    Ok(snapshot)
                },
            )
            .transpose()
    }
}
#[cfg(test)]
mod tests;
