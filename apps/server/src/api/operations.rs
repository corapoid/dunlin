//! One-use, expiring server-owned plans. No caller-supplied root, shell command,
//! force flags or client-authored snapshots. A disconnected execute is not retried.
use super::*;
use dunlin_workspace::git::{
    blame,
    operations::{self, Action, Outcome, Plan},
};
pub(super) struct Pending {
    pub user: String,
    pub device: String,
    pub project: Project,
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
fn authorized(
    state: &AppState,
    user: &str,
    device: &str,
    original: &Project,
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    if !store
        .list_devices(user)?
        .iter()
        .any(|d| d.id == device && d.revoked_at.is_none())
    {
        return Err(ApiError::unauthorized());
    }
    let project = store
        .project_for_user(&original.id, user)?
        .ok_or_else(ApiError::forbidden)?;
    if project.path != original.path {
        return Err(ApiError::conflict("Project changed; prepare again"));
    }
    Ok(())
}
fn worktree_grant(action: &Action, grants: &[Project]) -> Result<(), ApiError> {
    if let Action::WorktreeRemove { path } = action {
        let target = std::fs::canonicalize(path).map_err(|_| ApiError::forbidden())?;
        if !grants.iter().any(|project| {
            std::fs::canonicalize(&project.path).is_ok_and(|root| target.starts_with(root))
        }) {
            return Err(ApiError::forbidden());
        }
    }
    Ok(())
}
pub(super) async fn prepare(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(input): Json<Prepare>,
) -> Result<Json<Prepared>, ApiError> {
    let project = state.project_for_user(&user.id, &input.project)?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy"))?;
    let path = project.path.clone();
    let grants = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?
        .projects_for_user(&user.id)?;
    let plan = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        worktree_grant(&input.action, &grants)?;
        operations::preflight(&path, input.action).map_err(ApiError::bad_request)
    })
    .await
    .map_err(|_| ApiError::internal("Preflight worker failed"))??;
    authorized(&state, &user.id, &device.id, &project)?;
    let id = uuid::Uuid::now_v7().to_string();
    let mut plans = state
        .git_plans
        .lock()
        .map_err(|_| ApiError::internal("Plan store unavailable"))?;
    plans.retain(|_, p| {
        p.expires > Instant::now()
            && !(p.user == user.id && p.device == device.id && p.project.id == project.id)
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
        if p.user != user.id || p.device != device.id || p.project.id != input.project {
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
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        authorized(&state, &user.id, &device.id, &pending.project)?;
        let grants = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?
            .projects_for_user(&user.id)?;
        worktree_grant(&pending.plan.action, &grants)?;
        let outcome = operations::execute(&pending.project.path, &pending.plan)
            .map_err(ApiError::conflict)?;
        authorized(&state, &user.id, &device.id, &pending.project)?;
        Ok(Json(outcome))
    })
    .await
    .map_err(|_| ApiError::internal("Git worker failed; refresh state before any new action"))?
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
    let project = state.project_for_user(&user.id, &input.project)?;
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
