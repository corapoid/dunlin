//! One-use, expiring server-owned plans. No caller-supplied root, shell command,
//! force flags or client-authored snapshots. A disconnected execute is not retried.
use super::native_io::NativeGitIo;
use super::*;
use crate::store::{NativeGrantStamp, local_metadata::MetadataPermission};
use dunlin_workspace::git::{
    blame,
    operations::{self, Action, Outcome, Plan},
};
pub(super) struct Pending {
    pub user: String,
    pub device: String,
    pub project: history::GrantedProject,
    pub worktree_grant: Option<history::GrantedProject>,
    pub plan: Plan,
    pub expires: Instant,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Prepare {
    project: String,
    action: Action,
}
#[derive(Serialize, Deserialize)]
pub(super) struct Prepared {
    pub id: String,
    pub plan: Plan,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Execute {
    project: String,
    id: String,
}
fn uncertain_operation() -> ApiError {
    ApiError::conflict(
        "Authority changed during the operation; its result may be partial. Refresh before a new action; do not automatically retry.",
    )
}
fn authorized(
    state: &AppState,
    user: &str,
    device: &str,
    primary: &history::GrantedProject,
    additional: Option<&history::GrantedProject>,
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    store.native_checkout_transaction(|store| {
        for proof in std::iter::once(primary).chain(additional) {
            if proof.git.user != user || proof.git.device != device {
                return Err(ApiError::forbidden());
            }
            proof.git.validate_in_transaction(state, store)?;
        }
        Ok(())
    })
}
#[derive(Clone)]
struct CoveringProject {
    project: Project,
    grant: NativeGrantStamp,
}
fn worktree_grant(
    action: &Action,
    grants: &[CoveringProject],
) -> Result<Option<CoveringProject>, ApiError> {
    if let Action::WorktreeRemove { path } = action {
        let target = std::fs::canonicalize(path).map_err(|_| ApiError::forbidden())?;
        let grant = grants
            .iter()
            .find(|scope| {
                std::fs::canonicalize(&scope.project.path).is_ok_and(|root| target == root)
            })
            .ok_or_else(ApiError::forbidden)?;
        return Ok(Some(grant.clone()));
    }
    Ok(None)
}
fn target_matches(
    action: &Action,
    additional: Option<&history::GrantedProject>,
) -> Result<(), ApiError> {
    if let Action::WorktreeRemove { path } = action {
        let target = std::fs::canonicalize(path).map_err(|_| ApiError::forbidden())?;
        if additional
            .is_none_or(|proof| target != std::path::Path::new(proof.git.authority().root()))
        {
            return Err(ApiError::forbidden());
        }
    }
    Ok(())
}
fn history_permit(state: &AppState) -> Result<tokio::sync::OwnedSemaphorePermit, ApiError> {
    state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy"))
}
pub(super) async fn prepare(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(input): Json<Prepare>,
) -> Result<Json<Prepared>, ApiError> {
    let project = history::capture_project_for(
        &state,
        &user.id,
        &device.id,
        &input.project,
        MetadataPermission::Write,
    )
    .await?;
    let additional = if matches!(&input.action, Action::WorktreeRemove { .. }) {
        let grants = {
            let store = state
                .store
                .lock()
                .map_err(|_| ApiError::internal("Store unavailable"))?;
            store
                .projects_for_user(&user.id)?
                .into_iter()
                .map(|project| {
                    let grant = store
                        .capture_native_grant(&project.id, &user.id)
                        .map_err(|_| ApiError::forbidden())?;
                    Ok(CoveringProject { project, grant })
                })
                .collect::<Result<Vec<_>, ApiError>>()?
        };
        let action = input.action.clone();
        let permit = history_permit(&state)?;
        let original = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            worktree_grant(&action, &grants)
        })
        .await
        .map_err(|_| ApiError::internal("Worktree scope worker failed"))??
        .ok_or_else(ApiError::forbidden)?;
        let selected = history::capture_project_for(
            &state,
            &user.id,
            &device.id,
            &original.project.id,
            MetadataPermission::Write,
        )
        .await?;
        if selected.grant != original.grant
            || selected.git.original_project_path() != original.project.path
        {
            return Err(ApiError::forbidden());
        }
        Some(selected)
    } else {
        None
    };
    let permit = history_permit(&state)?;
    let worker_project = project.clone();
    let worker_additional = additional.clone();
    let worker_state = state.clone();
    let plan = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let primary = NativeGitIo::resume_blocking(&worker_state, &worker_project.git)?;
        let extra = worker_additional
            .as_ref()
            .map(|proof| NativeGitIo::resume_blocking(&worker_state, &proof.git))
            .transpose()?;
        target_matches(&input.action, worker_additional.as_ref())?;
        let outcome = operations::preflight(primary.authority().root(), input.action);
        primary.validate_blocking()?;
        if let Some(extra) = extra {
            extra.validate_blocking()?;
        }
        outcome.map_err(ApiError::bad_request)
    })
    .await
    .map_err(|_| ApiError::internal("Preflight worker failed"))??;
    authorized(&state, &user.id, &device.id, &project, additional.as_ref())?;
    let id = uuid::Uuid::now_v7().to_string();
    let mut plans = state
        .git_plans
        .lock()
        .map_err(|_| ApiError::internal("Plan store unavailable"))?;
    plans.retain(|_, p| {
        p.expires > Instant::now()
            && !(p.user == user.id
                && p.device == device.id
                && p.project.project.id == project.project.id)
    });
    if plans.len() >= 64 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pending Git confirmations",
        ));
    }
    plans.insert(
        id.clone(),
        Pending {
            user: user.id,
            device: device.id,
            project,
            worktree_grant: additional,
            plan: plan.clone(),
            expires: Instant::now() + Duration::from_secs(300),
        },
    );
    Ok(Json(Prepared { id, plan }))
}
pub(super) async fn execute(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(input): Json<Execute>,
) -> Result<Json<Outcome>, ApiError> {
    state.project_for_user(&user.id, &input.project)?;
    if input.id.len() > 64 {
        return Err(ApiError::bad_request("Invalid plan"));
    }
    let pending = {
        let mut plans = state
            .git_plans
            .lock()
            .map_err(|_| ApiError::internal("Plan store unavailable"))?;
        let p = plans.get(&input.id).ok_or_else(|| {
            ApiError::conflict(
                "Plan expired, consumed or missing. Do not retry a mutation; refresh state.",
            )
        })?;
        if p.user != user.id || p.device != device.id || p.project.project.id != input.project {
            return Err(ApiError::forbidden());
        }
        plans.remove(&input.id).ok_or_else(ApiError::forbidden)?
    };
    if pending.expires <= Instant::now() {
        return Err(ApiError::conflict("Plan expired; prepare again"));
    }
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy; prepare again"))?;
    let post_state = state.clone();
    let post_user = user.id.clone();
    let post_device = device.id.clone();
    let post_project = pending.project.clone();
    let post_additional = pending.worktree_grant.clone();
    let (started, result) = tokio::task::spawn_blocking(move || {
        let mut started = false;
        let outcome = (|| -> Result<Json<Outcome>, ApiError> {
            let _permit = permit;
            let primary = NativeGitIo::resume_blocking(&state, &pending.project.git)?;
            let additional = pending
                .worktree_grant
                .as_ref()
                .map(|proof| NativeGitIo::resume_blocking(&state, &proof.git))
                .transpose()?;
            target_matches(&pending.plan.action, pending.worktree_grant.as_ref())?;
            authorized(
                &state,
                &user.id,
                &device.id,
                &pending.project,
                pending.worktree_grant.as_ref(),
            )?;
            started = true;
            let result = operations::execute(primary.authority().root(), &pending.plan);
            primary.after_blocking(true)?;
            if let Some(extra) = additional {
                // Only a successful removal of this exact prechecked root may
                // skip its destructive filesystem postcheck. SQL remains fresh.
                let removed = matches!(&pending.plan.action, Action::WorktreeRemove { .. })
                    && result.as_ref().is_ok_and(|outcome| outcome.success)
                    && pending.plan.worktree_target.as_deref() == Some(extra.authority().root())
                    && std::fs::symlink_metadata(extra.authority().root())
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound);
                if !removed {
                    extra.after_blocking(true)?;
                }
            }
            authorized(
                &state,
                &user.id,
                &device.id,
                &pending.project,
                pending.worktree_grant.as_ref(),
            )
            .map_err(|_| uncertain_operation())?;
            Ok(Json(result.map_err(ApiError::conflict)?))
        })();
        (started, outcome)
    })
    .await
    .map_err(|_| ApiError::internal("Git worker failed; refresh state before any new action"))?;
    if started {
        authorized(
            &post_state,
            &post_user,
            &post_device,
            &post_project,
            post_additional.as_ref(),
        )
        .map_err(|_| uncertain_operation())?;
    }
    result
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct BlameQuery {
    project: String,
    path: String,
    scope: Option<String>,
    head: Option<String>,
}
pub(super) async fn blame(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(input): Query<BlameQuery>,
) -> Result<Json<git::history::HistoryResponse<blame::Blame>>, ApiError> {
    let project = history::capture_project(&state, &user.id, &device.id, &input.project).await?;
    history::read(
        state,
        user.id,
        device.id,
        project,
        input.scope,
        move |root| blame::load(root, &input.path, input.head.as_deref()),
    )
    .await
}
