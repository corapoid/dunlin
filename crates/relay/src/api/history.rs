//! Authenticated, project-granted, bounded read-only Git history. The caller
//! supplies no root/path/command; every operation derives its root from the grant.
use super::{ApiError, AppState, AuthUser, Json, Query, State, StatusCode};
use crate::store::Project;
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
    let project = state.project_for_user(&user.id, &query.project)?;
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

fn scope(project: &Project, root: &str) -> String {
    let mut hash = Sha256::new();
    for part in [&project.id, &project.path, root] {
        hash.update(part.as_bytes());
        hash.update([0]);
    }
    format!("{:x}", hash.finalize())
}

pub(super) async fn read<T, F>(
    state: AppState,
    user: String,
    device: String,
    project: Project,
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
    let result = tokio::task::spawn_blocking(move || {
        // Keep the permit with the blocking work, even if HTTP is cancelled.
        let _permit = permit;
        let root = history::repository_root(&project.path, None).map_err(ApiError::bad_request)?;
        let scope = scope(&project, &root);
        if expected.as_ref().is_some_and(|expected| expected != &scope) {
            return Err(ApiError::conflict(
                "The repository scope changed. Close and reopen Git history.",
            ));
        }
        let value = operation(&root).map_err(ApiError::bad_request)?;
        history::repository_root(&project.path, Some(&root))
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
    let active_device = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .list_devices(&user)
        .map_err(ApiError::from)?
        .into_iter()
        .any(|entry| entry.id == device && entry.revoked_at.is_none());
    if !active_device {
        return Err(ApiError::unauthorized());
    }
    let current = state.project_for_user(&user, &original.id)?;
    if current.path != original.path {
        return Err(ApiError::conflict("The project changed during the read"));
    }
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
    let project = state.project_for_user(&user.id, &query.project)?;
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
    let project = state.project_for_user(&user.id, &query.project)?;
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

pub(super) async fn patch(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<PatchQuery>,
) -> Result<Json<HistoryResponse<CommitPatch>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
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
