//! State-free native checkout authority. SQL candidates do not authorize roots;
//! filesystem capture and a fresh SQL cut are mandatory before construction.
use super::local_metadata::{
    MetadataAuthority, MetadataCandidate, MetadataPermission, MetadataStamp, ValidatedMetadata,
};
use super::{NativeGrantStamp, Result, Store, StoreError};
use crate::checkout_identity::{CapturedCheckout, RepoKind};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::path::Path;
fn conflict() -> StoreError {
    StoreError::Conflict("Native checkout authority changed".into())
}
fn checkout_error(e: crate::checkout_identity::Error) -> StoreError {
    match e {
        crate::checkout_identity::Error::Invalid(v) => StoreError::Limit(v.into()),
        crate::checkout_identity::Error::Conflict(v) => StoreError::Conflict(v.into()),
        crate::checkout_identity::Error::Io(v) => v.into(),
        crate::checkout_identity::Error::Json(v) => v.into(),
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeCheckoutStamp {
    pub native_grant: NativeGrantStamp,
    pub permission: MetadataPermission,
    pub settings_digest: String,
    pub native_path_digest: String,
    pub checkout_identity_digest: String,
    pub repo_kind: RepoKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetadataStamp>,
}
#[derive(Clone)]
struct Scope {
    permission: MetadataPermission,
    grant: NativeGrantStamp,
    path: String,
    settings: String,
}
pub(crate) struct NativeCheckoutCandidate {
    scope: Scope,
    metadata: Option<MetadataCandidate>,
}
pub(crate) struct ValidatedNativeCheckout {
    scope: Scope,
    checkout: Option<CapturedCheckout>,
    metadata: Option<ValidatedMetadata>,
}
/// Only sandbox constructors consume this wrapper. It is never serializable and
/// never expands workspace folders, browser file access, vaults or attachments.
#[derive(Clone, Default)]
pub(crate) struct SandboxMetadataRoots(Vec<(String, MetadataPermission)>);
impl SandboxMetadataRoots {
    /// Only an admitted worker lease with current operator execution policy can
    /// construct worker metadata write mounts. This does not expand workspace IO.
    pub(crate) fn from_worker_execution(
        lease: &crate::worker_exports::execution::ExecutionLease,
    ) -> Result<Self> {
        Self::captured_worker_metadata(lease.sandbox_only_checkout())
    }
    pub(crate) fn from_worker_session(
        lease: &crate::worker_exports::session::WorkerSessionLease,
    ) -> Result<Self> {
        Self::captured_worker_metadata(lease.sandbox_only_checkout())
    }
    fn captured_worker_metadata(checkout: &CapturedCheckout) -> Result<Self> {
        let roots = checkout
            .captured_metadata_paths()
            .into_iter()
            .map(|path| {
                Ok((
                    path.to_str().ok_or_else(conflict)?.to_owned(),
                    MetadataPermission::Write,
                ))
            })
            .collect::<Result<Vec<_>>>()?;
        if roots.len() > 128 {
            return Err(conflict());
        }
        Ok(Self(roots))
    }
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, MetadataPermission)> {
        self.0.iter().map(|(p, k)| (p.as_str(), *k))
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(crate) fn extend(&mut self, other: &Self) -> Result<()> {
        let mut merged = self.0.clone();
        for (path, permission) in &other.0 {
            if let Some((_, current)) = merged.iter_mut().find(|(p, _)| p == path) {
                if *permission == MetadataPermission::Write {
                    *current = MetadataPermission::Write;
                }
            } else {
                merged.push((path.clone(), *permission));
            }
        }
        if merged.len() > 128 {
            return Err(StoreError::Limit(
                "Sandbox metadata root capacity reached".into(),
            ));
        }
        merged.sort();
        self.0 = merged;
        Ok(())
    }
}
#[derive(Clone)]
pub(crate) struct NativeCheckoutAuthority {
    scope: Scope,
    checkout: CapturedCheckout,
    metadata: Option<MetadataAuthority>,
    stamp: NativeCheckoutStamp,
    root: String,
    roots: SandboxMetadataRoots,
}
impl NativeCheckoutCandidate {
    pub(crate) fn validate_filesystem(self) -> Result<ValidatedNativeCheckout> {
        let (checkout, metadata) = match self.metadata {
            Some(candidate) => (None, Some(candidate.validate_filesystem()?)),
            None => {
                let captured = CapturedCheckout::capture(
                    Path::new(&self.scope.path),
                    &self.scope.settings,
                    None,
                )
                .map_err(checkout_error)?;
                if captured.repo_kind() == RepoKind::LinkedWorktree {
                    return Err(conflict());
                }
                (Some(captured), None)
            }
        };
        Ok(ValidatedNativeCheckout {
            scope: self.scope,
            checkout,
            metadata,
        })
    }
}
impl NativeCheckoutAuthority {
    pub(crate) fn open_git_exclude(&self) -> Result<Option<std::fs::File>> {
        if self.scope.permission != MetadataPermission::Write {
            return Err(StoreError::Policy(
                dunlin_domain::collaboration::PolicyError::PermissionDenied,
            ));
        }
        self.validate_filesystem()?;
        self.checkout.open_git_exclude().map_err(checkout_error)
    }
    pub(crate) fn root(&self) -> &str {
        &self.root
    }
    pub(crate) fn stamp(&self) -> NativeCheckoutStamp {
        self.stamp.clone()
    }
    pub(crate) fn metadata_stamp(&self) -> Option<&MetadataStamp> {
        self.stamp.metadata.as_ref()
    }
    pub(crate) fn metadata_roots(&self) -> SandboxMetadataRoots {
        self.roots.clone()
    }
    /// Terminal postcondition for one explicit Git-init mutation only. This
    /// never returns an upgraded/reusable authority for the new repository.
    pub(crate) fn validate_initialized_git(&self) -> Result<()> {
        if self.metadata.is_some() {
            return Err(conflict());
        }
        self.checkout
            .validate_initialized_git_at(Path::new(&self.scope.path))
            .map_err(checkout_error)
    }
    pub(crate) fn validate_filesystem(&self) -> Result<()> {
        self.checkout
            .revalidate_at(Path::new(&self.scope.path))
            .map_err(checkout_error)
    }
}
impl Store {
    /// A fresh SQL cut for caller identity and checkout pins. The closure must
    /// perform SQL only; no filesystem validation or notification belongs here.
    pub(crate) fn native_checkout_transaction<T, E>(
        &self,
        operation: impl FnOnce(&Self) -> std::result::Result<T, E>,
    ) -> std::result::Result<T, E>
    where
        E: From<StoreError>,
    {
        let tx = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(StoreError::from)
        .map_err(E::from)?;
        let result = operation(self)?;
        tx.commit().map_err(StoreError::from).map_err(E::from)?;
        Ok(result)
    }
    fn native_checkout_scope(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<Scope> {
        let grant = self.capture_native_grant(project, user)?;
        let path = self
            .connection
            .query_row("SELECT path FROM projects WHERE id=?1", [project], |r| {
                r.get(0)
            })
            .optional()?
            .ok_or_else(conflict)?;
        let settings = super::auth::hash_token(
            &self
                .project_settings(project)?
                .unwrap_or_else(|| "{}".into()),
        );
        Ok(Scope {
            permission,
            grant,
            path,
            settings,
        })
    }
    pub(crate) fn native_checkout_candidate(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<NativeCheckoutCandidate> {
        let tx = self.connection.unchecked_transaction()?;
        let c = self.native_checkout_candidate_in_transaction(project, user, permission)?;
        tx.commit()?;
        Ok(c)
    }
    pub(crate) fn native_checkout_candidate_in_transaction(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
    ) -> Result<NativeCheckoutCandidate> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let scope = self.native_checkout_scope(project, user, permission)?;
        // Absence is a successful SQL result, never an error fallback. Any
        // persisted approval (including a tombstone) requires its own consent.
        let metadata = if self.local_metadata(project)?.is_some() {
            Some(self.local_metadata_candidate_in_transaction(project, user, permission)?)
        } else {
            None
        };
        Ok(NativeCheckoutCandidate { scope, metadata })
    }
    pub(crate) fn authorize_native_checkout(
        &self,
        validated: ValidatedNativeCheckout,
    ) -> Result<NativeCheckoutAuthority> {
        let tx = self.connection.unchecked_transaction()?;
        let authority = self.authorize_native_checkout_in_transaction(validated)?;
        tx.commit()?;
        Ok(authority)
    }
    pub(crate) fn authorize_native_checkout_in_transaction(
        &self,
        validated: ValidatedNativeCheckout,
    ) -> Result<NativeCheckoutAuthority> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let current = self.native_checkout_scope(
            &validated.scope.grant.project_id,
            &validated.scope.grant.user_id,
            validated.scope.permission,
        )?;
        if current.grant != validated.scope.grant
            || current.path != validated.scope.path
            || current.settings != validated.scope.settings
        {
            return Err(conflict());
        }
        let metadata = validated
            .metadata
            .map(|m| self.authorize_local_metadata_in_transaction(m))
            .transpose()?;
        // A newly added approval never silently upgrades an ordinary proof.
        if metadata.is_none() && self.local_metadata(&current.grant.project_id)?.is_some() {
            return Err(conflict());
        }
        let checkout = match (&metadata, validated.checkout) {
            (Some(m), None) => m.captured_checkout().clone(),
            (None, Some(c)) => c,
            _ => return Err(conflict()),
        };
        let root = checkout
            .canonical_root()
            .to_str()
            .ok_or_else(conflict)?
            .to_owned();
        let roots = SandboxMetadataRoots(
            checkout
                .captured_metadata_paths()
                .into_iter()
                .map(|p| {
                    Ok((
                        p.to_str().ok_or_else(conflict)?.to_owned(),
                        current.permission,
                    ))
                })
                .collect::<Result<Vec<_>>>()?,
        );
        let stamp = NativeCheckoutStamp {
            native_grant: current.grant.clone(),
            permission: current.permission,
            settings_digest: current.settings.clone(),
            native_path_digest: super::auth::hash_token(&current.path),
            checkout_identity_digest: checkout.identity_digest().map_err(checkout_error)?,
            repo_kind: checkout.repo_kind(),
            metadata: metadata.as_ref().map(|m| m.stamp_ref().clone()),
        };
        let a = NativeCheckoutAuthority {
            scope: current,
            checkout,
            metadata,
            stamp,
            root,
            roots,
        };
        Ok(a)
    }
    #[cfg(test)]
    pub(crate) fn revalidate_native_checkout(&self, a: &NativeCheckoutAuthority) -> Result<()> {
        self.revalidate_native_checkouts(std::slice::from_ref(a))
    }
    #[cfg(test)]
    pub(crate) fn revalidate_native_checkouts(
        &self,
        authorities: &[NativeCheckoutAuthority],
    ) -> Result<()> {
        let tx = self.connection.unchecked_transaction()?;
        for a in authorities {
            self.revalidate_native_checkout_in_transaction(a)?;
        }
        tx.commit()?;
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn revalidate_native_checkout_after_init(
        &self,
        a: &NativeCheckoutAuthority,
    ) -> Result<()> {
        self.native_checkout_transaction(|store| {
            store.revalidate_native_checkout_after_init_in_transaction(a)
        })
    }
    pub(crate) fn revalidate_native_checkout_after_init_in_transaction(
        &self,
        a: &NativeCheckoutAuthority,
    ) -> Result<()> {
        if a.checkout.repo_kind() != RepoKind::Directory || a.metadata.is_some() {
            return Err(conflict());
        }
        self.revalidate_native_checkout_in_transaction(a)
    }
    pub(crate) fn revalidate_native_checkout_in_transaction(
        &self,
        a: &NativeCheckoutAuthority,
    ) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let current = self.native_checkout_scope(
            &a.scope.grant.project_id,
            &a.scope.grant.user_id,
            a.scope.permission,
        )?;
        if current.grant != a.scope.grant
            || current.path != a.scope.path
            || current.settings != a.scope.settings
        {
            return Err(conflict());
        }
        match &a.metadata {
            Some(m) => self.revalidate_local_metadata_in_transaction(m)?,
            None => {
                if self.local_metadata(&current.grant.project_id)?.is_some() {
                    return Err(conflict());
                }
            }
        }
        Ok(())
    }
}
/// A new checkout is inspected before its native project/grant is published.
/// This object is not serializable and conveys no authority by itself.
#[derive(Clone)]
pub(crate) struct PreparedOrdinaryCheckout {
    path: String,
    settings: String,
    checkout: CapturedCheckout,
}
impl PreparedOrdinaryCheckout {
    pub(crate) fn capture(root: &Path, settings_digest: &str) -> Result<Self> {
        let checkout =
            CapturedCheckout::capture(root, settings_digest, None).map_err(checkout_error)?;
        if checkout.repo_kind() == RepoKind::LinkedWorktree {
            return Err(conflict());
        }
        let path = checkout
            .canonical_root()
            .to_str()
            .ok_or_else(conflict)?
            .to_owned();
        Ok(Self {
            path,
            settings: settings_digest.into(),
            checkout,
        })
    }
    pub(crate) fn root(&self) -> &str {
        &self.path
    }
    pub(crate) fn validate_filesystem(&self) -> Result<()> {
        self.checkout
            .revalidate_at(Path::new(&self.path))
            .map_err(checkout_error)
    }
}
impl Store {
    /// SQL pins only. A persisted stamp cannot construct roots or skip the
    /// mandatory filesystem validation before execution/status publication.
    pub(crate) fn validate_native_checkout_stamp_in_transaction(
        &self,
        stamp: &NativeCheckoutStamp,
    ) -> Result<()> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let current = self.native_checkout_scope(
            &stamp.native_grant.project_id,
            &stamp.native_grant.user_id,
            stamp.permission,
        )?;
        if current.grant != stamp.native_grant
            || current.settings != stamp.settings_digest
            || super::auth::hash_token(&current.path) != stamp.native_path_digest
        {
            return Err(conflict());
        }
        match &stamp.metadata {
            Some(expected) => {
                let candidate = self.local_metadata_candidate_in_transaction(
                    &current.grant.project_id,
                    &current.grant.user_id,
                    stamp.permission,
                )?;
                if candidate.stamp_ref() != expected
                    || candidate
                        .captured_checkout()
                        .identity_digest()
                        .map_err(checkout_error)?
                        != stamp.checkout_identity_digest
                    || candidate.captured_checkout().repo_kind() != stamp.repo_kind
                {
                    return Err(conflict());
                }
            }
            None => {
                if stamp.repo_kind == RepoKind::LinkedWorktree
                    || self.local_metadata(&current.grant.project_id)?.is_some()
                {
                    return Err(conflict());
                }
            }
        }
        Ok(())
    }
    pub(crate) fn authorize_new_native_checkout_in_transaction(
        &self,
        project: &str,
        user: &str,
        permission: MetadataPermission,
        prepared: PreparedOrdinaryCheckout,
    ) -> Result<NativeCheckoutAuthority> {
        if self.connection.is_autocommit() {
            return Err(conflict());
        }
        let scope = self.native_checkout_scope(project, user, permission)?;
        if scope.path != prepared.path
            || scope.settings != prepared.settings
            || self.local_metadata(project)?.is_some()
        {
            return Err(conflict());
        }
        self.authorize_native_checkout_in_transaction(ValidatedNativeCheckout {
            scope,
            checkout: Some(prepared.checkout),
            metadata: None,
        })
    }
}
#[cfg(test)]
mod tests;

// Derived project grants are published atomically with the captured source and
// caller identity check. The callback performs SQL only inside this fresh cut.
impl Store {
    pub(crate) fn register_native_checkout_projects<E>(
        &self,
        authority: &NativeCheckoutAuthority,
        user: &str,
        projects: &[(String, String)],
        authorize: impl FnOnce(&Store) -> std::result::Result<(), E>,
    ) -> std::result::Result<Vec<super::Project>, E>
    where
        E: From<StoreError>,
    {
        if projects.len() > 2
            || authority.stamp.permission != MetadataPermission::Write
            || authority.scope.grant.user_id != user
        {
            return Err(E::from(conflict()));
        }
        let result = self.native_checkout_transaction(|store| {
            authorize(store)?;
            store.revalidate_native_checkout_in_transaction(authority).map_err(E::from)?;
            let mut result = Vec::new();
            for (name, path) in projects {
                let path = path.trim();
                let path = if path == "/" { path.to_string() } else { path.trim_end_matches('/').to_string() };
                let project = super::Project { id: format!("prj-{}", uuid::Uuid::now_v7()), name:name.trim().to_string(), path };
                store.connection.execute("INSERT INTO projects(id,name,path,created_at) VALUES(?1,?2,?3,?4)", rusqlite::params![project.id,project.name,project.path,super::now_secs()]).map_err(StoreError::from).map_err(E::from)?;
                store.connection.execute("INSERT INTO project_grants(project_id,user_id,revision,active) VALUES(?1,?2,1,1)",rusqlite::params![project.id,user]).map_err(StoreError::from).map_err(E::from)?;
                result.push(project);
            }
            Ok::<_,E>(result)
        })?;
        if !result.is_empty() {
            self.catalog_changed();
        }
        Ok(result)
    }
}
