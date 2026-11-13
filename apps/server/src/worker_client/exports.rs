//! Metadata jobs never own the socket. Blocking filesystem work is bounded even
//! after its asynchronous waiter times out; its permit lives in the closure.
use super::WorkerConfig;
use crate::worker_exports as local;
use dunlin_relay_protocol::worker as wire;
use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{sync::Semaphore, task::JoinSet};
const JOB_TIMEOUT: Duration = Duration::from_secs(5);
type Outcome = Result<wire::ExportDescriptor, wire::ExportErrorCode>;
pub(super) struct Requests {
    scope: local::ExportScope,
    directory: Option<std::path::PathBuf>,
    permits: Arc<Semaphore>,
    closed: Arc<AtomicBool>,
    ids: BTreeSet<String>,
    tasks: JoinSet<(String, Outcome)>,
}
impl Requests {
    pub fn new(config: &WorkerConfig) -> Self {
        Self {
            scope: local::ExportScope {
                controller_node_id: config.controller.node_id.clone(),
                worker_node_id: config.network.node_id.clone(),
                workspace_id: config.workspace_id.clone(),
                host_id: config.host_id.clone(),
                enrollment_epoch: config.enrollment_epoch,
                credential_revision: config.credential_revision,
            },
            directory: config.exports_state_dir.clone(),
            permits: Arc::new(Semaphore::new(2)),
            closed: Arc::new(AtomicBool::new(false)),
            ids: BTreeSet::new(),
            tasks: JoinSet::new(),
        }
    }
    pub fn pending(&self) -> bool {
        !self.ids.is_empty()
    }
    /// Duplicate in-flight identities are a protocol error, not a second reply
    /// which could accidentally resolve the original controller request.
    pub fn enqueue(
        &mut self,
        id: String,
        export: String,
        principal: wire::ExportPrincipal,
        expected: Option<u64>,
    ) -> Result<Option<wire::ExportErrorCode>, &'static str> {
        if self.ids.contains(&id) {
            return Err("worker_duplicate_export_request");
        }
        if self.ids.len() >= 8 {
            return Ok(Some(wire::ExportErrorCode::Busy));
        }
        let Some(directory) = self.directory.clone() else {
            return Ok(Some(wire::ExportErrorCode::Unavailable));
        };
        self.ids.insert(id.clone());
        let scope = self.scope.clone();
        let permits = self.permits.clone();
        let closed = self.closed.clone();
        self.tasks.spawn(async move {
            let deadline = Instant::now() + JOB_TIMEOUT;
            let result = tokio::time::timeout(JOB_TIMEOUT, async move {
                let permit = permits
                    .acquire_owned()
                    .await
                    .map_err(|_| wire::ExportErrorCode::Unavailable)?;
                tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    describe(
                        &directory, scope, export, principal, expected, &closed, deadline,
                    )
                })
                .await
                .map_err(|_| wire::ExportErrorCode::Unavailable)?
            })
            .await
            .unwrap_or(Err(wire::ExportErrorCode::Busy));
            (id, result)
        });
        Ok(None)
    }
    pub async fn completed(&mut self) -> Result<(String, Outcome), &'static str> {
        let (id, outcome) = self
            .tasks
            .join_next()
            .await
            .ok_or("worker_export_task_missing")?
            .map_err(|_| "worker_export_task_failed")?;
        self.ids.remove(&id);
        Ok((id, outcome))
    }
}
impl Drop for Requests {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
    }
}
fn current(closed: &AtomicBool, deadline: Instant) -> Result<(), wire::ExportErrorCode> {
    if closed.load(Ordering::Acquire) || Instant::now() >= deadline {
        Err(wire::ExportErrorCode::Busy)
    } else {
        Ok(())
    }
}
fn error(error: local::Error) -> wire::ExportErrorCode {
    match error {
        local::Error::Denied => wire::ExportErrorCode::Forbidden,
        local::Error::Conflict(_) => wire::ExportErrorCode::StaleExport,
        _ => wire::ExportErrorCode::Unavailable,
    }
}
fn describe(
    directory: &std::path::Path,
    scope: local::ExportScope,
    id: String,
    principal: wire::ExportPrincipal,
    expected: Option<u64>,
    closed: &AtomicBool,
    deadline: Instant,
) -> Outcome {
    current(closed, deadline)?;
    let store = local::WorkerStore::open_existing(directory, scope.clone()).map_err(error)?;
    let principal = local::ExportPrincipal {
        kind: match principal.kind {
            wire::PrincipalKind::Human => local::PrincipalKind::Human,
            wire::PrincipalKind::Service => local::PrincipalKind::Service,
        },
        id: principal.id,
    };
    let candidate = store
        .candidate(&scope, &id, &principal, local::ExportPermission::Read)
        .map_err(error)?;
    current(closed, deadline)?;
    let validated = candidate.validate_filesystem().map_err(error)?;
    current(closed, deadline)?;
    let authority = store
        .authorize_validated(&scope, validated)
        .map_err(error)?;
    // A second filesystem pass catches changes during the SQL authorization;
    // the final SQL cut also fences operator revoke/rebind from another process.
    authority.validate_filesystem().map_err(error)?;
    current(closed, deadline)?;
    store
        .revalidate_authority(&scope, &authority)
        .map_err(error)?;
    let stamp = authority.export_stamp();
    let grant = authority.grant_stamp().grant;
    if expected.is_some_and(|value| value != stamp.export.revision) {
        return Err(wire::ExportErrorCode::StaleExport);
    }
    let export = stamp.export;
    let descriptor = wire::ExportDescriptor {
        scope: wire::ExportScope {
            controller_node_id: scope.controller_node_id,
            worker_node_id: scope.worker_node_id,
            workspace_id: scope.workspace_id,
            host_id: scope.host_id,
            enrollment_epoch: scope.enrollment_epoch,
            credential_revision: scope.credential_revision,
        },
        export: wire::ExportSummary {
            id: export.id,
            name: export.name,
            repo_kind: match export.repo_kind {
                local::RepoKind::Directory => wire::RepoKind::Directory,
                local::RepoKind::Git => wire::RepoKind::Git,
                local::RepoKind::LinkedWorktree => wire::RepoKind::LinkedWorktree,
            },
            checkout_identity_digest: export.checkout_identity_digest,
            settings_digest: export.settings_digest,
            revision: export.revision,
            active: export.active,
        },
        grant: wire::ExportGrant {
            principal: wire::ExportPrincipal {
                kind: match grant.principal.kind {
                    local::PrincipalKind::Human => wire::PrincipalKind::Human,
                    local::PrincipalKind::Service => wire::PrincipalKind::Service,
                },
                id: grant.principal.id,
            },
            permissions: grant
                .permissions
                .into_iter()
                .map(|p| match p {
                    local::ExportPermission::Read => wire::ExportPermission::Read,
                    local::ExportPermission::Execute => wire::ExportPermission::Execute,
                })
                .collect(),
            revision: grant.revision,
            export_revision: grant.export_revision,
            active: grant.active,
        },
    };
    descriptor
        .validate()
        .map_err(|_| wire::ExportErrorCode::Unavailable)?;
    current(closed, deadline)?;
    Ok(descriptor)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, WorkerConfig, String) {
        let dir = tempfile::tempdir().unwrap();
        let mut config = super::super::tests::config();
        config.exports_state_dir = Some(dir.path().join("state"));
        let root = dir.path().join("checkout");
        std::fs::create_dir(&root).unwrap();
        let scope = Requests::new(&config).scope.clone();
        let store =
            local::WorkerStore::open(config.exports_state_dir.as_ref().unwrap(), scope.clone())
                .unwrap();
        let checkout = local::CapturedCheckout::capture(&root, &"a".repeat(64), None).unwrap();
        let export = store
            .register_captured(&scope, "Operator export", &checkout)
            .unwrap();
        store
            .set_grant(
                &scope,
                &export.id,
                &local::ExportPrincipal {
                    kind: local::PrincipalKind::Human,
                    id: "reader".into(),
                },
                &[local::ExportPermission::Read],
                true,
                None,
            )
            .unwrap();
        (dir, config, export.id)
    }
    fn principal() -> wire::ExportPrincipal {
        wire::ExportPrincipal {
            kind: wire::PrincipalKind::Human,
            id: "reader".into(),
        }
    }
    #[tokio::test]
    async fn descriptor_is_path_free_fresh_and_explicitly_scoped() {
        let (_dir, config, id) = fixture();
        let mut requests = Requests::new(&config);
        assert_eq!(
            requests
                .enqueue("request".into(), id.clone(), principal(), Some(1))
                .unwrap(),
            None
        );
        let (request, descriptor) = requests.completed().await.unwrap();
        assert_eq!(request, "request");
        let descriptor = descriptor.unwrap();
        descriptor.validate().unwrap();
        assert_eq!(descriptor.export.id, id);
        assert_eq!(descriptor.scope.worker_node_id, config.network.node_id);
        assert_eq!(descriptor.grant.principal, principal());
        let json = serde_json::to_string(&descriptor).unwrap();
        assert!(!json.contains("checkout\""));
        assert!(!json.contains(config.exports_state_dir.as_ref().unwrap().to_str().unwrap()));
        assert_eq!(
            requests
                .enqueue("wrongrevision".into(), id.clone(), principal(), Some(2))
                .unwrap(),
            None
        );
        assert_eq!(
            requests.completed().await.unwrap().1,
            Err(wire::ExportErrorCode::StaleExport)
        );
        let store = local::WorkerStore::open_existing(
            config.exports_state_dir.as_ref().unwrap(),
            requests.scope.clone(),
        )
        .unwrap();
        store.revoke_export(&requests.scope, &id, 1).unwrap();
        requests
            .enqueue("revoked".into(), id, principal(), None)
            .unwrap();
        assert_eq!(
            requests.completed().await.unwrap().1,
            Err(wire::ExportErrorCode::Forbidden)
        );
        let mut changed = config.clone();
        changed.enrollment_epoch += 1;
        let mut requests = Requests::new(&changed);
        requests
            .enqueue("scope".into(), descriptor.export.id, principal(), None)
            .unwrap();
        assert_eq!(
            requests.completed().await.unwrap().1,
            Err(wire::ExportErrorCode::StaleExport)
        );
    }
    #[tokio::test]
    async fn requests_bound_pending_work_and_never_discover_or_initialize_catalogs() {
        let config = super::super::tests::config();
        let mut requests = Requests::new(&config);
        assert_eq!(
            requests
                .enqueue("missing".into(), "export".into(), principal(), None)
                .unwrap(),
            Some(wire::ExportErrorCode::Unavailable)
        );
        assert!(!requests.pending());
        let (dir, mut config, id) = fixture();
        let mut requests = Requests::new(&config);
        let held = requests
            .permits
            .clone()
            .acquire_many_owned(2)
            .await
            .unwrap();
        for i in 0..8 {
            assert_eq!(
                requests
                    .enqueue(format!("request{i}"), id.clone(), principal(), None)
                    .unwrap(),
                None
            );
        }
        assert!(
            requests
                .enqueue("request0".into(), id.clone(), principal(), None)
                .is_err()
        );
        assert_eq!(
            requests
                .enqueue("overflow".into(), id, principal(), None)
                .unwrap(),
            Some(wire::ExportErrorCode::Busy)
        );
        drop(requests);
        drop(held);
        config.exports_state_dir = Some(dir.path().join("not-created"));
        let mut requests = Requests::new(&config);
        requests
            .enqueue("absent".into(), "export".into(), principal(), None)
            .unwrap();
        assert_eq!(
            requests.completed().await.unwrap().1,
            Err(wire::ExportErrorCode::Unavailable)
        );
        assert!(!config.exports_state_dir.as_ref().unwrap().exists());
    }
    #[tokio::test]
    async fn filesystem_change_and_explicit_service_principal_cannot_reuse_human_grant() {
        let (dir, config, id) = fixture();
        let mut requests = Requests::new(&config);
        let service = wire::ExportPrincipal {
            kind: wire::PrincipalKind::Service,
            id: "reader".into(),
        };
        requests
            .enqueue("service".into(), id.clone(), service, None)
            .unwrap();
        assert_eq!(
            requests.completed().await.unwrap().1,
            Err(wire::ExportErrorCode::Forbidden)
        );
        std::fs::rename(dir.path().join("checkout"), dir.path().join("old")).unwrap();
        std::fs::create_dir(dir.path().join("checkout")).unwrap();
        requests
            .enqueue("changed".into(), id, principal(), None)
            .unwrap();
        assert!(requests.completed().await.unwrap().1.is_err());
    }
}
