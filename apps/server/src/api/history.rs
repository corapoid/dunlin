//! Authenticated, project-granted, bounded read-only Git history. The caller
//! supplies no root/path/command; every operation derives its root from the grant.
use super::native_io::{NativeGitIo, NativeGitProof};
use super::{ApiError, AppState, AuthUser, Json, Query, State, StatusCode};
use crate::store::{NativeGrantStamp, Project, local_metadata::MetadataPermission};
use dunlin_workspace::git::history::{self, Commit, CommitDetails, CommitPatch, HistoryResponse};
use serde::Deserialize;
use sha2::{Digest, Sha256};

fn default_limit() -> usize {
    500
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListQuery {
    project: String,
    #[serde(default)]
    revision: String,
    #[serde(default = "default_limit")]
    limit: usize,
    scope: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverviewQuery {
    project: String,
    scope: Option<String>,
}

pub(super) async fn overview(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<OverviewQuery>,
) -> Result<Json<HistoryResponse<dunlin_workspace::git::overview::Overview>>, ApiError> {
    let project = capture_project(&state, &user.id, &device.id, &query.project).await?;
    read(
        state,
        user.id,
        device.id,
        project,
        query.scope,
        dunlin_workspace::git::overview::load,
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CommitQuery {
    project: String,
    scope: String,
    oid: String,
    #[serde(default)]
    parent: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PatchQuery {
    project: String,
    scope: String,
    oid: String,
    #[serde(default)]
    parent: usize,
    /// Index into the immutable comparison's file list; never a filesystem path.
    file: usize,
}

#[derive(Clone)]
pub(super) struct GrantedProject {
    pub project: Project,
    pub grant: NativeGrantStamp,
    pub git: NativeGitProof,
}
pub(super) async fn capture_project(
    state: &AppState,
    user: &str,
    device: &str,
    id: &str,
) -> Result<GrantedProject, ApiError> {
    capture_project_for(state, user, device, id, MetadataPermission::Read).await
}
pub(super) async fn capture_project_for(
    state: &AppState,
    user: &str,
    device: &str,
    id: &str,
    permission: MetadataPermission,
) -> Result<GrantedProject, ApiError> {
    let io = NativeGitIo::capture(state, user, device, id, permission).await?;
    let git = io.proof();
    Ok(GrantedProject {
        project: git.project.clone(),
        grant: git.grant_stamp().clone(),
        git,
    })
}
/// Final response cut is SQL-only. All filesystem validation runs in the bounded
/// worker before this check, never while holding Store or on the async reactor.
pub(super) fn revalidate_project(
    state: &AppState,
    user: &str,
    device: &str,
    original: &GrantedProject,
) -> Result<(), ApiError> {
    if original.git.user != user || original.git.device != device {
        return Err(ApiError::forbidden());
    }
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    store.native_checkout_transaction(|store| original.git.validate_in_transaction(state, store))
}

fn scope(granted: &GrantedProject, root: &str) -> Result<String, ApiError> {
    let project = &granted.project;
    let mut hash = Sha256::new();
    let revision = granted.grant.revision.to_string();
    for part in [
        project.id.as_str(),
        granted.git.original_project_path(),
        root,
        &granted.grant.user_id,
        &revision,
    ] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    if let Some(metadata) = granted.git.authority().metadata_stamp() {
        hash.update(
            serde_json::to_vec(metadata)
                .map_err(|_| ApiError::internal("Metadata scope unavailable"))?,
        );
        hash.update([0]);
    }
    Ok(hex::encode(hash.finalize()))
}

pub(super) async fn read<T, F>(
    state: AppState,
    user: String,
    device: String,
    project: GrantedProject,
    expected: Option<String>,
    operation: F,
) -> Result<Json<HistoryResponse<T>>, ApiError>
where
    T: Send + 'static,
    F: FnOnce(&str) -> Result<T, String> + Send + 'static,
{
    if expected
        .as_ref()
        .is_some_and(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(ApiError::bad_request("Invalid history scope"));
    }
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Git history is busy; try again",
            )
        })?;
    let original = project.clone();
    let worker_state = state.clone();
    let result = tokio::task::spawn_blocking(move || {
        // Keep the permit with the blocking work, even if HTTP is cancelled.
        let _permit = permit;
        let io = NativeGitIo::resume_blocking(&worker_state, &project.git)?;
        let root =
            history::repository_root(io.authority().root(), None).map_err(ApiError::bad_request)?;
        let scope = scope(&project, &root)?;
        if expected.as_ref().is_some_and(|expected| expected != &scope) {
            return Err(ApiError::conflict(
                "The repository scope changed. Close and reopen Git history.",
            ));
        }
        let outcome = operation(&root);
        io.after_blocking(false)?;
        let value = outcome.map_err(ApiError::bad_request)?;
        history::repository_root(io.authority().root(), Some(&root))
            .map_err(|_| ApiError::conflict("The repository root changed during the read"))?;
        Ok(HistoryResponse {
            version: history::PROTOCOL_VERSION,
            scope,
            value,
        })
    })
    .await
    .map_err(|_| ApiError::internal("Git history worker failed"))?;
    // A grant/device may be revoked or a project relocated while Git runs. Never
    // return data merely because authorization succeeded before the blocking job.
    revalidate_project(&state, &user, &device, &original)?;
    result.map(Json)
}

fn validate_commit(oid: &str, parent: usize) -> Result<(), ApiError> {
    if !matches!(oid.len(), 40 | 64)
        || !oid.bytes().all(|b| b.is_ascii_hexdigit())
        || parent >= 2000
    {
        return Err(ApiError::bad_request(
            "Select a full commit ID and a valid parent index",
        ));
    }
    Ok(())
}

pub(super) async fn list(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<HistoryResponse<Vec<Commit>>>, ApiError> {
    let project = capture_project(&state, &user.id, &device.id, &query.project).await?;
    if query.limit == 0
        || query.limit > 2000
        || query.revision.len() > 512
        || query.revision.contains('\0')
    {
        return Err(ApiError::bad_request(
            "History requires 1–2,000 commits and a revision of at most 512 bytes",
        ));
    }
    read(
        state,
        user.id,
        device.id,
        project,
        query.scope,
        move |root| history::load_from(root, &query.revision, query.limit),
    )
    .await
}

pub(super) async fn commit(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<CommitQuery>,
) -> Result<Json<HistoryResponse<CommitDetails>>, ApiError> {
    let project = capture_project(&state, &user.id, &device.id, &query.project).await?;
    validate_commit(&query.oid, query.parent)?;
    read(
        state,
        user.id,
        device.id,
        project,
        Some(query.scope),
        move |root| history::details(root, &query.oid, query.parent),
    )
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BranchCreateRequest {
    project: String,
    scope: String,
    oid: String,
    name: String,
}

/// Create one ref from an exact commit. This is the only history mutation:
/// it never checks out, fetches, moves HEAD or writes the index.
pub(super) async fn create_branch(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<BranchCreateRequest>,
) -> Result<StatusCode, ApiError> {
    let project = capture_project_for(
        &state,
        &user.id,
        &device.id,
        &request.project,
        MetadataPermission::Write,
    )
    .await?;
    validate_commit(&request.oid, 0)?;
    if request.scope.len() != 64
        || !request.scope.bytes().all(|b| b.is_ascii_hexdigit())
        || request.name.is_empty()
        || request.name.len() > 240
        || request.name.starts_with('-')
        || request.name == "HEAD"
        || request.name.trim() != request.name
    {
        return Err(ApiError::bad_request(
            "Branch creation requires a valid scope, full commit ID and explicit name",
        ));
    }
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Git history is busy; try again",
            )
        })?;
    let original = project.clone();
    let scope = request.scope;
    let oid = request.oid;
    let name = request.name;
    let worker_state = state.clone();
    let worker_user = user.id.clone();
    let worker_device = device.id.clone();
    let (started, result) = tokio::task::spawn_blocking(move || {
        let mut started = false;
        let result = (|| -> Result<(), ApiError> {
            let _permit = permit;
            let io = NativeGitIo::resume_blocking(&worker_state, &project.git)?;
            let root = history::repository_root(io.authority().root(), None)
                .map_err(ApiError::bad_request)?;
            if scope != self::scope(&project, &root)? {
                return Err(ApiError::conflict(
                    "The repository scope changed. Close and reopen Git history.",
                ));
            }
            revalidate_project(&worker_state, &worker_user, &worker_device, &project)?;
            started = true;
            let outcome = history::create_branch(&root, &name, &oid);
            io.after_blocking(true)?;
            outcome.map_err(ApiError::bad_request)?;
            history::repository_root(io.authority().root(), Some(&root)).map_err(|_| {
                ApiError::conflict("The repository root changed during branch creation")
            })?;
            Ok(())
        })();
        (started, result)
    })
    .await
    .map_err(|_| ApiError::internal("Git history worker failed"))?;
    if started {
        revalidate_project(&state, &user.id, &device.id, &original).map_err(|_|ApiError::conflict("Authority changed during branch creation; its outcome may be partial. Refresh before a new action; do not automatically retry."))?;
    }
    result?;
    Ok(StatusCode::OK)
}

pub(super) async fn patch(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<PatchQuery>,
) -> Result<Json<HistoryResponse<CommitPatch>>, ApiError> {
    let project = capture_project(&state, &user.id, &device.id, &query.project).await?;
    validate_commit(&query.oid, query.parent)?;
    if query.file >= 2000 {
        return Err(ApiError::bad_request(
            "Select a file from the commit comparison",
        ));
    }
    read(
        state,
        user.id,
        device.id,
        project,
        Some(query.scope),
        move |root| {
            let detail = history::details(root, &query.oid, query.parent)?;
            let file = detail
                .files
                .get(query.file)
                .ok_or("Select a file from the commit comparison")?
                .clone();
            let patch = history::file_patch(root, &query.oid, query.parent, &file)?;
            Ok(CommitPatch {
                oid: detail.commit.oid,
                parent: query.parent,
                file,
                patch,
            })
        },
    )
    .await
}
