//! HTTP and WebSocket API for the relay.

use std::collections::{HashMap, VecDeque};
use std::path::{Path as FsPath, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State, ws::WebSocketUpgrade};
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use axum::http::{HeaderName, HeaderValue, StatusCode, header::AUTHORIZATION, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dunlin_domain::{ProjectSettings, ProviderUsage, validate_project_environment};
use dunlin_workspace::checks;
use dunlin_workspace::fs as workspace_fs;
use dunlin_workspace::git;
use dunlin_workspace::impact;
use dunlin_workspace::search;
use dunlin_workspace::tree::FileTree;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

mod history;
mod operations;

use crate::config::Config;
use crate::store::{ArtifactRow, Device, Project, Store, StoreError, User};

pub type UsageCache = Arc<Mutex<Option<(Instant, Vec<ProviderUsage>)>>>;

#[derive(Clone)]
pub struct AppState {
    pub(crate) management: Arc<Mutex<crate::management::Management>>,
    pub store: Arc<Mutex<Store>>,
    pub config: Arc<Config>,
    pub(crate) vault: Option<Arc<crate::vault::RelayVault>>,
    pub(crate) coordination: Option<Arc<dunlin_coordination::service::Service>>,
    pub(crate) artifact_dir: PathBuf,
    pub leases: Arc<Mutex<HashMap<String, Lease>>>,
    pub usage_cache: UsageCache,
    history_reads: Arc<tokio::sync::Semaphore>,
    fs_reads: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    terminal_sessions: Arc<Semaphore>,
    pair_attempts: Arc<Mutex<VecDeque<Instant>>>,
    git_plans: Arc<Mutex<HashMap<String, operations::Pending>>>,
    review_plans: Arc<Mutex<HashMap<String, PendingRestore>>>,
    compare_plans: Arc<Mutex<HashMap<String, PendingCompare>>>,
    checks: Arc<Mutex<HashMap<String, Arc<std::sync::atomic::AtomicBool>>>>,
}

struct PendingRestore {
    user_id: String,
    project_id: String,
    prepared: Instant,
    plan: dunlin_workspace::task_review::Restore,
}

struct PendingCompare {
    user_id: String,
    project_id: String,
    prepared: Instant,
    plan: dunlin_workspace::comparison::Prepared,
}

#[derive(Clone)]
pub struct Lease {
    pub device_id: String,
    pub connection_id: String,
    pub close_tx: mpsc::Sender<()>,
    pub expires_at: Instant,
}

impl AppState {
    #[allow(dead_code)]
    pub fn new(store: Store, config: Config) -> Self {
        Self::new_with_vault(store, config, None)
    }

    pub(crate) fn new_with_vault(
        store: Store,
        config: Config,
        vault: Option<Arc<crate::vault::RelayVault>>,
    ) -> Self {
        Self {
            management: Arc::new(Mutex::new(crate::management::Management::default())),
            store: Arc::new(Mutex::new(store)),
            artifact_dir: config
                .db
                .parent()
                .unwrap_or(FsPath::new("."))
                .join("artifacts"),
            config: Arc::new(config),
            vault,
            coordination: None,
            leases: Arc::new(Mutex::new(HashMap::new())),
            usage_cache: Arc::new(Mutex::new(None)),
            history_reads: Arc::new(tokio::sync::Semaphore::new(4)),
            fs_reads: Arc::new(Semaphore::new(8)),
            connections: Arc::new(Semaphore::new(32)),
            terminal_sessions: Arc::new(Semaphore::new(8)),
            pair_attempts: Arc::new(Mutex::new(VecDeque::new())),
            git_plans: Arc::new(Mutex::new(HashMap::new())),
            review_plans: Arc::new(Mutex::new(HashMap::new())),
            compare_plans: Arc::new(Mutex::new(HashMap::new())),
            checks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn project_for_user(&self, user_id: &str, project_id: &str) -> Result<Project, ApiError> {
        self.store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?
            .project_for_user(project_id, user_id)
            .map_err(ApiError::from)?
            .ok_or_else(ApiError::forbidden)
    }

    fn fs_permit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        self.fs_reads.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Workspace file reads are busy; try again",
            )
        })
    }

    fn connection_permit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        let _installation = self.installation_guard()?;
        self.connections.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many active ACP connections; close one and retry",
            )
        })
    }

    pub(crate) fn terminal_permit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        let _installation = self.installation_guard()?;
        self.terminal_sessions
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                ApiError::new(
                    StatusCode::TOO_MANY_REQUESTS,
                    "Too many active relay terminals; close one and retry",
                )
            })
    }

    pub(crate) fn tools_busy(&self) -> bool {
        self.connections.available_permits() < 32 || self.terminal_sessions.available_permits() < 8
    }

    fn installation_guard(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, crate::management::Management>, ApiError> {
        let management = self
            .management
            .lock()
            .map_err(|_| ApiError::internal("Management lock poisoned"))?;
        if management.jobs.values().any(|running| {
            running
                .job
                .lock()
                .map(|job| job.kind.starts_with("install:") && !job.finished)
                .unwrap_or(true)
        }) {
            return Err(ApiError::conflict(
                "Server tools are being installed; wait for the installation to finish",
            ));
        }
        Ok(management)
    }

    fn allow_pair_attempt(&self) -> bool {
        let Ok(mut attempts) = self.pair_attempts.lock() else {
            return false;
        };
        let now = Instant::now();
        while attempts
            .front()
            .is_some_and(|at| now.duration_since(*at) > Duration::from_secs(60))
        {
            attempts.pop_front();
        }
        if attempts.len() >= 30 {
            return false;
        }
        attempts.push_back(now);
        true
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, message)
    }

    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "Invalid or expired token")
    }

    pub fn forbidden() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            "Project is not available to this user",
        )
    }

    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "Not found")
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, message)
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(json!({ "error": self.message }))).into_response()
    }
}

impl From<StoreError> for ApiError {
    fn from(error: StoreError) -> Self {
        ApiError::internal(error.to_string())
    }
}

impl From<String> for ApiError {
    fn from(message: String) -> Self {
        ApiError::bad_request(message)
    }
}

pub struct AuthUser {
    pub user: User,
    pub device: Device,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let header = parts
            .headers
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(ApiError::unauthorized)?;
        let token = header
            .strip_prefix("Bearer ")
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .ok_or_else(ApiError::unauthorized)?;
        let auth = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?
            .authenticate(token)
            .map_err(ApiError::from)?
            .ok_or_else(ApiError::unauthorized)?;
        if auth.0.role != "admin" && crate::management::host_accounts(&state.config) {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "This relay uses host-level Git accounts and permits administrators only. Use an isolated relay service for member access.",
            ));
        }
        Ok(Self {
            user: auth.0,
            device: auth.1,
        })
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route(
            "/v1/server",
            get(crate::management::info).put(crate::management::update),
        )
        .route("/v1/server/actions", post(crate::management::start))
        .route("/v1/server/clone", post(crate::management::clone_project))
        .route(
            "/v1/server/jobs/{id}",
            get(crate::management::job).delete(crate::management::cancel),
        )
        .route("/v1/server/copies", post(crate::management::copy::begin))
        .route(
            "/v1/server/copies/{id}/chunk",
            post(crate::management::copy::chunk),
        )
        .route(
            "/v1/server/copies/{id}/finish",
            post(crate::management::copy::finish),
        )
        .route(
            "/v1/server/copies/{id}",
            delete(crate::management::copy::cancel),
        )
        .route("/v1/git/operation/prepare", post(operations::prepare))
        .route("/v1/git/operation/execute", post(operations::execute))
        .route("/v1/git/blame", get(operations::blame))
        .route("/v1/pair", post(pair))
        .route("/v1/me", get(me))
        .route(
            "/v1/sessions/{id}/transcript/revision",
            get(session_transcript_revision),
        )
        .route("/v1/usage", get(usage))
        .route(
            "/v1/billing/evidence",
            get(billing_evidence_get).post(billing_evidence_put),
        )
        .route("/v1/projects", get(projects))
        .route("/v1/coordination", post(crate::coordination::request))
        .route(
            "/v1/project/settings",
            get(project_settings_get).put(project_settings_put),
        )
        .route("/v1/checks/run", post(check_run))
        .route("/v1/checks/cancel", post(check_cancel))
        .route("/v1/impact/build", post(impact_build))
        .route("/v1/impact/verify", post(impact_verify))
        .route("/v1/github/metadata", post(github_metadata))
        .route("/v1/review/capture", post(review_capture))
        .route("/v1/review/changes", post(review_changes))
        .route("/v1/review/restore/prepare", post(review_restore_prepare))
        .route("/v1/review/restore/execute", post(review_restore_execute))
        .route("/v1/compare/prepare", post(compare_prepare))
        .route("/v1/compare/create", post(compare_create))
        .route("/v1/compare/changes", post(compare_changes))
        .route("/v1/compare/validate", post(compare_validate))
        .route("/v1/context", get(remote_context))
        .route("/v1/artifacts", get(artifacts_list).post(artifact_upload))
        .route(
            "/v1/artifacts/{id}",
            get(artifact_download).delete(artifact_delete),
        )
        .route(
            "/v1/artifacts/{id}/share",
            post(artifact_share).delete(artifact_revoke_share),
        )
        .route("/a/{token}", get(public_artifact))
        .route("/v1/sessions", get(sessions))
        .route("/v1/sessions/{id}", delete(delete_session))
        .route("/v1/sessions/{id}/takeover", post(takeover))
        .route(
            "/v1/sessions/{id}/transcript",
            get(session_transcript).put(session_transcript_save),
        )
        .route("/acp", get(acp_ws))
        .route("/v1/terminal", get(crate::terminal::ws))
        .route("/v1/fs/tree", get(fs_tree))
        .route("/v1/fs/storage", get(fs_storage))
        .route("/v1/fs/file", get(fs_file).put(fs_write))
        .route("/v1/audits", get(audit_runs))
        .route("/v1/audits/file", get(audit_file))
        .route("/v1/fs/create", post(fs_create))
        .route("/v1/fs/mkdir", post(fs_mkdir))
        .route("/v1/fs/move", post(fs_move))
        .route("/v1/fs/rename", post(fs_rename))
        .route("/v1/fs/delete", post(fs_delete))
        .route("/v1/fs/preview", get(fs_preview))
        .route("/v1/git/init", post(git_init))
        .route("/v1/fs/upload", post(fs_upload))
        .route("/v1/fs/stat", get(fs_stat))
        .route("/v1/search", get(search_project))
        .route("/v1/git/status", get(git_status))
        .route("/v1/git/history", get(history::list))
        .route("/v1/git/history/commit", get(history::commit))
        .route("/v1/git/history/patch", get(history::patch))
        .route("/v1/git/history/branch", post(history::create_branch))
        .route("/v1/git/overview", get(history::overview))
        .route("/v1/git/diff", get(git_diff))
        .route("/v1/git/stage", post(git_stage))
        .route("/v1/git/unstage", post(git_unstage))
        .route("/v1/git/commit", post(git_commit))
        .route("/v1/git/amend", post(git_amend))
        .route("/v1/git/discard", post(git_discard))
        .route("/v1/git/pull", post(git_pull))
        .route("/v1/git/push", post(git_push))
        .route("/v1/git/branches", get(git_branches))
        .route("/v1/git/switch", post(git_switch))
        .route("/v1/git/branch-context", get(git_branch_context))
        .route("/v1/git/switch-safe", post(git_switch_safe))
        // Checkpoint review payloads carry bounded base64 overlays (16 MiB
        // encoded) plus restore previews; individual handlers still enforce
        // their domain limits.
        .layer(DefaultBodyLimit::max(64 * 1024 * 1024))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
}

const MAX_ARTIFACT_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize)]
struct ArtifactUpload {
    project_id: String,
    kind: String,
    title: String,
    mime: String,
    data: String,
}

#[derive(Deserialize)]
struct ArtifactQuery {
    project: String,
}

#[derive(Deserialize)]
struct ShareRequest {
    #[serde(default)]
    ttl_secs: Option<u64>,
}

async fn artifacts_list(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Query(query): Query<ArtifactQuery>,
) -> Result<Json<Vec<ArtifactRow>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let rows = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifacts_for_project(&user.id, &project.id)
        .map_err(ApiError::from)?;
    Ok(Json(rows))
}

async fn artifact_upload(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Json(request): Json<ArtifactUpload>,
) -> Result<Json<ArtifactRow>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project_id)?;
    let bytes = STANDARD
        .decode(request.data.as_bytes())
        .map_err(|_| ApiError::bad_request("Artifact data is not valid base64"))?;
    if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_BYTES {
        return Err(ApiError::bad_request(format!(
            "Artifact must be between 1 byte and {} MiB",
            MAX_ARTIFACT_BYTES / 1024 / 1024
        )));
    }
    let kind = clean_label(&request.kind, 40, "artifact")?;
    let title = clean_label(&request.title, 240, "Untitled artifact")?;
    let mime = clean_mime(&request.mime)?;
    let id = uuid::Uuid::now_v7().to_string();
    let sha256 = sha256_hex(&bytes);
    let directory = state.artifact_dir.join(&project.id);
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| ApiError::internal("Could not create artifact directory"))?;
    let blob_path = directory.join(&id);
    tokio::fs::write(&blob_path, &bytes)
        .await
        .map_err(|_| ApiError::internal("Could not write artifact"))?;
    let row = ArtifactRow {
        id,
        project_id: project.id,
        user_id: user.id,
        kind,
        title,
        mime,
        bytes: bytes.len() as i64,
        sha256,
        blob_path: blob_path.to_string_lossy().to_string(),
        created_at: crate::store::now_secs(),
    };
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .insert_artifact(&row)
        .map_err(ApiError::from)?;
    Ok(Json(row))
}

async fn artifact_download(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    state.project_for_user(&user.id, &row.project_id)?;
    let bytes = tokio::fs::read(&row.blob_path)
        .await
        .map_err(|_| ApiError::internal("Could not read artifact"))?;
    Ok(Json(json!({
        "artifact": row,
        "data": STANDARD.encode(bytes),
    })))
}

async fn artifact_delete(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    state.project_for_user(&user.id, &row.project_id)?;
    if row.user_id != user.id {
        return Err(ApiError::forbidden());
    }
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .delete_artifact(&id)
        .map_err(ApiError::from)?;
    let _ = tokio::fs::remove_file(&row.blob_path).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn artifact_share(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Path(id): Path<String>,
    Json(request): Json<ShareRequest>,
) -> Result<Json<Value>, ApiError> {
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    state.project_for_user(&user.id, &row.project_id)?;
    if row.user_id != user.id {
        return Err(ApiError::forbidden());
    }
    let ttl = request
        .ttl_secs
        .unwrap_or(7 * 24 * 60 * 60)
        .clamp(60, 30 * 24 * 60 * 60);
    let token = format!(
        "{}{}",
        uuid::Uuid::now_v7().simple(),
        uuid::Uuid::now_v7().simple()
    );
    let token_hash = sha256_hex(token.as_bytes());
    let expires_at = crate::store::now_secs() + ttl as i64;
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .create_artifact_share(&id, &token_hash, expires_at)
        .map_err(ApiError::from)?;
    Ok(Json(json!({
        "token": token,
        "expires_at": expires_at,
        "url": format!("/a/{token}"),
    })))
}

async fn artifact_revoke_share(
    State(state): State<AppState>,
    AuthUser { user, .. }: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    state.project_for_user(&user.id, &row.project_id)?;
    if row.user_id != user.id {
        return Err(ApiError::forbidden());
    }
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .revoke_artifact_shares(&id)
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn public_artifact(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::not_found());
    }
    let token_hash = sha256_hex(token.as_bytes());
    let share = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact_share_by_token(&token_hash)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    let now = crate::store::now_secs();
    if share.revoked_at.is_some() || share.expires_at <= now {
        return Err(ApiError::not_found());
    }
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .artifact(&share.artifact_id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    let bytes = tokio::fs::read(&row.blob_path)
        .await
        .map_err(|_| ApiError::internal("Could not read artifact"))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, public_mime(&row.mime))
        .header(CONTENT_DISPOSITION, "inline; filename=\"dunlin-artifact\"")
        .header("X-Content-Type-Options", "nosniff")
        .header("Content-Security-Policy", "sandbox; default-src 'none'")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from(bytes))
        .map_err(|_| ApiError::internal("Could not build artifact response"))
}

fn clean_label(value: &str, max: usize, fallback: &str) -> Result<String, ApiError> {
    let value = value.trim();
    if value.len() > max || value.chars().any(char::is_control) {
        return Err(ApiError::bad_request("Artifact label is invalid"));
    }
    if value.is_empty() {
        Ok(fallback.into())
    } else {
        Ok(value.to_string())
    }
}

fn clean_mime(value: &str) -> Result<String, ApiError> {
    let value = value.trim().to_ascii_lowercase();
    let allowed = value.starts_with("text/")
        || value.starts_with("image/")
        || matches!(
            value.as_str(),
            "application/json" | "application/pdf" | "application/octet-stream"
        );
    if !allowed || value.len() > 120 || value.chars().any(char::is_control) {
        return Err(ApiError::bad_request("Artifact MIME type is not allowed"));
    }
    Ok(value)
}

fn public_mime(value: &str) -> &'static str {
    match value {
        "text/markdown" => "text/markdown; charset=utf-8",
        "application/json" => "application/json",
        "application/pdf" => "application/pdf",
        "image/png" => "image/png",
        "image/jpeg" => "image/jpeg",
        "image/gif" => "image/gif",
        "image/webp" => "image/webp",
        _ => "text/plain; charset=utf-8",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
    device_name: String,
}

#[derive(Serialize)]
struct PairResponse {
    token: String,
    user: User,
    device_id: String,
}

async fn pair(
    State(state): State<AppState>,
    Json(request): Json<PairRequest>,
) -> Result<Json<PairResponse>, ApiError> {
    if !state.allow_pair_attempt() {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many pairing attempts; try again later",
        ));
    }
    let device_name = request.device_name.trim();
    let device_name: String = if device_name.is_empty() {
        "Dunlin device".into()
    } else {
        device_name.chars().take(120).collect()
    };
    if request.code.len() > 64 {
        return Err(ApiError::bad_request("Invalid pairing code"));
    }
    let (user, device, token) = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        let user = store
            .consume_pairing_code(&request.code)
            .map_err(ApiError::from)?
            .ok_or_else(ApiError::unauthorized)?;
        let (device, token) = store
            .create_device(&user.id, &device_name)
            .map_err(ApiError::from)?;
        (user, device, token)
    };
    Ok(Json(PairResponse {
        token,
        user,
        device_id: device.id,
    }))
}

async fn me(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Value>, ApiError> {
    let devices = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .list_devices(&user.id)
        .map_err(ApiError::from)?;
    Ok(Json(json!({
        "user": user,
        "device": device,
        "devices": devices,
    })))
}

#[derive(Deserialize)]
struct ImpactRequest {
    project: String,
}

#[derive(Deserialize)]
struct ImpactVerifyRequest {
    project: String,
    index: impact::Index,
}

async fn impact_build(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ImpactRequest>,
) -> Result<Json<impact::Index>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Impact indexing is busy; try again",
            )
        })?;
    let index = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let root =
            git::history::repository_root(&project.path, None).map_err(ApiError::bad_request)?;
        impact::build(&root, None).map_err(ApiError::bad_request)
    })
    .await
    .map_err(|_| ApiError::internal("Impact worker failed"))??;
    Ok(Json(index))
}

async fn impact_verify(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ImpactVerifyRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    tokio::task::spawn_blocking(move || {
        let root =
            git::history::repository_root(&project.path, None).map_err(ApiError::bad_request)?;
        if request.index.root != root {
            return Err(ApiError::conflict(
                "Impact index belongs to another repository",
            ));
        }
        impact::verify(&request.index).map_err(ApiError::bad_request)
    })
    .await
    .map_err(|_| ApiError::internal("Impact worker failed"))??;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct GithubLoadRequest {
    project: String,
}

async fn github_metadata(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<GithubLoadRequest>,
) -> Result<Json<dunlin_workspace::github::Metadata>, ApiError> {
    if user.role != "admin" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Relay GitHub metadata uses the host account and requires an administrator",
        ));
    }
    let project = state.project_for_user(&user.id, &request.project)?;
    Ok(Json(
        run_blocking(move || dunlin_workspace::github::metadata(&project.path)).await?,
    ))
}

#[derive(Deserialize)]
struct ReviewCaptureRequest {
    project: String,
    label: String,
}

#[derive(Deserialize)]
struct ReviewChangesRequest {
    project: String,
    checkpoint: dunlin_workspace::task_review::Checkpoint,
}

#[derive(Deserialize)]
struct ReviewRestorePrepareRequest {
    project: String,
    checkpoint: dunlin_workspace::task_review::Checkpoint,
    path: String,
}

#[derive(Deserialize)]
struct ReviewRestoreExecuteRequest {
    project: String,
    id: String,
}

#[derive(Serialize)]
struct ReviewRestorePreview {
    id: String,
    change: dunlin_workspace::task_review::Change,
}

fn checkpoint_root(
    project: &Project,
    checkpoint: &dunlin_workspace::task_review::Checkpoint,
) -> Result<(), String> {
    let root = git::history::repository_root(&project.path, None)?;
    if checkpoint.root != root {
        return Err("Checkpoint belongs to a different repository".into());
    }
    Ok(())
}

async fn review_capture(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewCaptureRequest>,
) -> Result<Json<dunlin_workspace::task_review::Checkpoint>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    let point =
        run_blocking(move || dunlin_workspace::task_review::capture(&project.path, &request.label))
            .await?;
    Ok(Json(point))
}

async fn review_changes(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewChangesRequest>,
) -> Result<Json<Vec<dunlin_workspace::task_review::Change>>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    checkpoint_root(&project, &request.checkpoint).map_err(ApiError::bad_request)?;
    let changes =
        run_blocking(move || dunlin_workspace::task_review::changes(&request.checkpoint)).await?;
    Ok(Json(changes))
}

async fn review_restore_prepare(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewRestorePrepareRequest>,
) -> Result<Json<ReviewRestorePreview>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    checkpoint_root(&project, &request.checkpoint).map_err(ApiError::bad_request)?;
    if state
        .review_plans
        .lock()
        .map(|plans| plans.len())
        .unwrap_or(0)
        >= 64
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many prepared restores; execute or wait for expiry",
        ));
    }
    let plan = run_blocking(move || {
        dunlin_workspace::task_review::prepare_restore(&request.checkpoint, &request.path)
    })
    .await?;
    let change = plan.change.clone();
    let id = uuid::Uuid::now_v7().to_string();
    state
        .review_plans
        .lock()
        .map_err(|_| ApiError::internal("Restore plan registry lock poisoned"))?
        .insert(
            id.clone(),
            PendingRestore {
                user_id: user.id,
                project_id: request.project,
                prepared: Instant::now(),
                plan,
            },
        );
    Ok(Json(ReviewRestorePreview { id, change }))
}

async fn review_restore_execute(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewRestoreExecuteRequest>,
) -> Result<StatusCode, ApiError> {
    state.project_for_user(&user.id, &request.project)?;
    let pending = state
        .review_plans
        .lock()
        .map_err(|_| ApiError::internal("Restore plan registry lock poisoned"))?
        .remove(&request.id)
        .ok_or_else(ApiError::not_found)?;
    if pending.user_id != user.id
        || pending.project_id != request.project
        || pending.prepared.elapsed() > Duration::from_secs(300)
    {
        return Err(ApiError::conflict(
            "Restore preview expired or belongs to another session",
        ));
    }
    run_blocking(move || dunlin_workspace::task_review::restore(pending.plan)).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ComparePrepareRequest {
    project: String,
}

#[derive(Deserialize)]
struct CompareCreateRequest {
    project: String,
    id: String,
}

#[derive(Deserialize)]
struct CompareChangesRequest {
    project: String,
    checkpoint: dunlin_workspace::task_review::Checkpoint,
}

#[derive(Deserialize)]
struct CompareValidateRequest {
    project: String,
    head: String,
}

#[derive(Serialize)]
struct ComparePreparedResponse {
    id: String,
    root: String,
    head: String,
    paths: Vec<String>,
}

#[derive(Serialize)]
struct CompareCreateResponse {
    trees: dunlin_workspace::comparison::Trees,
    projects: Vec<Project>,
}

async fn compare_prepare(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ComparePrepareRequest>,
) -> Result<Json<ComparePreparedResponse>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    if state
        .compare_plans
        .lock()
        .map(|plans| plans.len())
        .unwrap_or(0)
        >= 32
    {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many prepared comparisons; create or wait for expiry",
        ));
    }
    let plan = run_blocking(move || dunlin_workspace::comparison::prepare(&project.path)).await?;
    let id = plan.id.clone();
    let response = ComparePreparedResponse {
        id: id.clone(),
        root: plan.root.clone(),
        head: plan.head.clone(),
        paths: plan.paths.to_vec(),
    };
    state
        .compare_plans
        .lock()
        .map_err(|_| ApiError::internal("Comparison plan registry lock poisoned"))?
        .insert(
            id,
            PendingCompare {
                user_id: user.id,
                project_id: request.project,
                prepared: Instant::now(),
                plan,
            },
        );
    Ok(Json(response))
}

async fn compare_create(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareCreateRequest>,
) -> Result<Json<CompareCreateResponse>, ApiError> {
    let source = state.project_for_user(&user.id, &request.project)?;
    let pending = state
        .compare_plans
        .lock()
        .map_err(|_| ApiError::internal("Comparison plan registry lock poisoned"))?
        .remove(&request.id)
        .ok_or_else(ApiError::not_found)?;
    if pending.user_id != user.id
        || pending.project_id != request.project
        || pending.prepared.elapsed() > Duration::from_secs(300)
    {
        return Err(ApiError::conflict(
            "Comparison confirmation expired or belongs to another session",
        ));
    }
    let head = pending.plan.head.clone();
    let id = pending.plan.id.clone();
    let trees =
        tokio::task::spawn_blocking(move || dunlin_workspace::comparison::create(pending.plan))
            .await
            .map_err(|_| ApiError::internal("Comparison worker failed"))?;
    let mut projects = Vec::new();
    {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        for (index, path) in trees.paths.iter().enumerate() {
            let name = format!(
                "{} · compare {} · {}",
                source.name,
                &id[..id.len().min(8)],
                if index == 0 { "A" } else { "B" }
            );
            if let Ok(project) = store.create_project(&name, path) {
                if let Err(error) = store.grant(&project.id, &user.id) {
                    eprintln!("[relay] comparison grant rejected · {error}");
                    continue;
                }
                projects.push(project);
            }
        }
    }
    let mut trees = trees;
    if projects.len() != trees.paths.len() && trees.error.is_none() {
        trees.error = Some(
            "Comparison trees were created but one or more relay project grants could not be registered"
                .into(),
        );
    }
    if trees.error.is_none()
        && !projects
            .iter()
            .all(|project| project.path.starts_with(&trees.root))
    {
        trees.error = Some(
            "Comparison worktree escaped the source repository; no agent start is authorized"
                .into(),
        );
    }
    if head.is_empty() || trees.head != head {
        trees.error = Some("Comparison baseline commit changed while creating worktrees".into());
    }
    Ok(Json(CompareCreateResponse { trees, projects }))
}

async fn compare_changes(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareChangesRequest>,
) -> Result<Json<Vec<dunlin_workspace::task_review::Change>>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    checkpoint_root(&project, &request.checkpoint).map_err(ApiError::bad_request)?;
    let changes =
        run_blocking(move || dunlin_workspace::task_review::changes(&request.checkpoint)).await?;
    Ok(Json(changes))
}

async fn compare_validate(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareValidateRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || dunlin_workspace::comparison::validate_tree(&project.path, &request.head))
        .await?;
    Ok(StatusCode::OK)
}

async fn remote_context(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Value>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let settings = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .project_settings(&project.id)
        .map_err(ApiError::from)?
        .as_deref()
        .map(serde_json::from_str::<ProjectSettings>)
        .transpose()
        .map_err(|_| ApiError::internal("stored project settings are invalid"))?
        .unwrap_or_default();
    let mut settings = serde_json::to_value(settings)
        .map_err(|_| ApiError::internal("project settings could not be serialized"))?;
    if let Some(environment) = settings
        .get_mut("environment")
        .and_then(serde_json::Value::as_object_mut)
    {
        for value in environment.values_mut() {
            *value = json!("[hidden]");
        }
    }
    Ok(Json(json!({
        "project": project,
        "settings": settings,
        "relay": {
            "version": env!("CARGO_PKG_VERSION"),
            "harness": state.config.harness,
            "sandbox": state.config.sandbox,
            "store_transcripts": state.config.store_transcripts,
            "max_frame_bytes": state.config.max_frame_bytes,
        },
        "server_time_ms": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0),
        "context_boundary": "The relay can report its own configuration and stored project defaults. Harness-owned system prompts, discovered instructions, loaded skills and model history remain unknown.",
    })))
}

#[derive(Deserialize)]
struct CheckRequest {
    project: String,
    argv: Vec<String>,
}

#[derive(Deserialize)]
struct CheckCancelRequest {
    project: String,
    id: String,
}

async fn check_run(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CheckRequest>,
) -> Result<Json<checks::Report>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    let plan = tokio::task::spawn_blocking(move || checks::prepare(&project.path, request.argv))
        .await
        .map_err(|_| ApiError::internal("Check worker failed"))?
        .map_err(ApiError::bad_request)?;
    let id = uuid::Uuid::now_v7().to_string();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    state
        .checks
        .lock()
        .map_err(|_| ApiError::internal("check registry lock poisoned"))?
        .insert(id.clone(), cancel.clone());
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Checks are busy; try again"))?;
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        checks::run(plan, cancel)
    })
    .await
    .map_err(|_| ApiError::internal("Check worker failed"))?;
    state
        .checks
        .lock()
        .map_err(|_| ApiError::internal("check registry lock poisoned"))?
        .remove(&id);
    Ok(Json(result.map_err(ApiError::bad_request)?))
}

async fn check_cancel(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CheckCancelRequest>,
) -> Result<StatusCode, ApiError> {
    state.project_for_user(&user.id, &request.project)?;
    if let Some(cancel) = state
        .checks
        .lock()
        .map_err(|_| ApiError::internal("check registry lock poisoned"))?
        .get(&request.id)
    {
        cancel.store(true, std::sync::atomic::Ordering::Release);
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ProjectSettingsQuery {
    project: String,
}

#[derive(Deserialize)]
struct ProjectSettingsBody {
    project: String,
    settings: ProjectSettings,
}

async fn project_settings_get(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectSettingsQuery>,
) -> Result<Json<ProjectSettings>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let payload = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .project_settings(&project.id)
        .map_err(ApiError::from)?;
    let settings = payload
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| ApiError::internal("stored project settings are invalid"))?
        .unwrap_or_default();
    Ok(Json(settings))
}

async fn project_settings_put(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ProjectSettingsBody>,
) -> Result<StatusCode, ApiError> {
    if user.role != "admin" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Project runtime settings are restricted to relay administrators",
        ));
    }
    let project = state.project_for_user(&user.id, &body.project)?;
    validate_project_environment(&body.settings.environment).map_err(ApiError::bad_request)?;
    let payload = serde_json::to_string(&body.settings)
        .map_err(|_| ApiError::bad_request("Project settings could not be serialized"))?;
    if payload.len() > 128 * 1024 {
        return Err(ApiError::bad_request("Project settings exceed 128 KiB"));
    }
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .save_project_settings(&project.id, &payload)
        .map_err(ApiError::from)?;
    Ok(StatusCode::OK)
}

async fn projects(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<Project>>, ApiError> {
    let projects = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .projects_for_user(&user.id)
        .map_err(ApiError::from)?;
    Ok(Json(projects))
}

async fn usage(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderUsage>>, ApiError> {
    if user.role != "admin" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Provider usage is restricted to relay administrators",
        ));
    }
    if let Ok(cache) = state.usage_cache.lock()
        && let Some((at, usages)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(60)
    {
        return Ok(Json(usages.clone()));
    }
    let usages = run_blocking(|| Ok(dunlin_usage::probe_all())).await?;
    if let Ok(mut cache) = state.usage_cache.lock() {
        *cache = Some((Instant::now(), usages.clone()));
    }
    Ok(Json(usages))
}

#[derive(Serialize)]
struct BillingEvidenceResponse {
    user_id: String,
    user_name: String,
    device_id: String,
    device_name: String,
    updated_at: i64,
    evidence: dunlin_usage::billing::Evidence,
}

async fn billing_evidence_put(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(evidence): Json<dunlin_usage::billing::Evidence>,
) -> Result<StatusCode, ApiError> {
    evidence.validate().map_err(ApiError::bad_request)?;
    let payload = serde_json::to_string(&evidence)
        .map_err(|_| ApiError::bad_request("Billing evidence could not be serialized"))?;
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .save_billing_evidence(&user.id, &device.id, &payload)
        .map_err(ApiError::from)?;
    Ok(StatusCode::OK)
}

async fn billing_evidence_get(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<BillingEvidenceResponse>>, ApiError> {
    if user.role != "admin" {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Aggregated billing evidence is restricted to relay administrators",
        ));
    }
    let rows = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .billing_evidence(None)
        .map_err(ApiError::from)?;
    let mut response = Vec::with_capacity(rows.len());
    for row in rows {
        let evidence = serde_json::from_str(&row.payload).map_err(|_| {
            ApiError::internal("Stored billing evidence is invalid; ask the device to sync again")
        })?;
        response.push(BillingEvidenceResponse {
            user_id: row.user_id,
            user_name: row.user_name,
            device_id: row.device_id,
            device_name: row.device_name,
            updated_at: row.updated_at,
            evidence,
        });
    }
    Ok(Json(response))
}

#[derive(Deserialize)]
struct ProjectQuery {
    project: String,
}

#[derive(Deserialize)]
struct AcpQuery {
    project: String,
    /// Comma-separated relay project ids explicitly granted for a group
    /// conversation. Only paths from these grants may enter the sandbox.
    roots: Option<String>,
}

#[derive(Serialize)]
struct SessionInfo {
    id: String,
    title: String,
    updated_at: i64,
    last_device_id: Option<String>,
    active_device_id: Option<String>,
}

async fn sessions(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let rows = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .list_sessions(&user.id, &project.id)
        .map_err(ApiError::from)?;
    let sessions = rows
        .into_iter()
        .map(|row| {
            let active = active_lease(&state, &row.id).map(|lease| lease.device_id);
            SessionInfo {
                id: row.id,
                title: row.title,
                updated_at: row.updated_at,
                last_device_id: row.last_device_id,
                active_device_id: active,
            }
        })
        .collect();
    Ok(Json(sessions))
}

async fn delete_session(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let owner = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .session_owner(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    if owner.user_id != user.id {
        return Err(ApiError::not_found());
    }
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .delete_session(&id)
        .map_err(ApiError::from)?;
    if let Some(lease) = state
        .leases
        .lock()
        .map_err(|_| ApiError::internal("lease lock poisoned"))?
        .remove(&id)
    {
        let _ = lease.close_tx.try_send(());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn takeover(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    owned_session(&state, &user, &id)?;
    if let Some(lease) = state
        .leases
        .lock()
        .map_err(|_| ApiError::internal("lease lock poisoned"))?
        .remove(&id)
    {
        let _ = lease.close_tx.try_send(());
    }
    Ok(StatusCode::OK)
}

fn owned_session(state: &AppState, user: &User, id: &str) -> Result<(), ApiError> {
    let owner = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .session_owner(id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    if owner.user_id != user.id {
        return Err(ApiError::not_found());
    }
    Ok(())
}

#[derive(Deserialize)]
struct TranscriptRequest {
    payload: Value,
    #[serde(default)]
    expected_revision: Option<i64>,
}

async fn session_transcript_revision(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    owned_session(&state, &user, &id)?;
    let revision = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .transcript_revision(&id)
        .map_err(ApiError::from)?;
    Ok(Json(json!({ "revision": revision })))
}

async fn session_transcript_save(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<TranscriptRequest>,
) -> Result<StatusCode, ApiError> {
    if !state.config.store_transcripts {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "Transcript storage is disabled on this relay",
        ));
    }
    owned_session(&state, &user, &id)?;
    let payload = serde_json::to_string(&request.payload)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if payload.len() > 4 * 1024 * 1024 {
        return Err(ApiError::bad_request("Transcript is larger than 4 MiB"));
    }
    let saved = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?;
        match request.expected_revision {
            Some(revision) => store.save_transcript_if_revision(&id, &payload, revision),
            None => store.save_transcript(&id, &payload),
        }
        .map_err(ApiError::from)?
    };
    if !saved {
        return Err(ApiError::conflict(
            "The relay has a newer transcript revision; the stale snapshot was not saved",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn session_transcript(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    owned_session(&state, &user, &id)?;
    let payload = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .load_transcript(&id)
        .map_err(ApiError::from)?
        .ok_or_else(ApiError::not_found)?;
    let value: Value = serde_json::from_str(&payload)
        .map_err(|error| ApiError::internal(format!("Stored transcript is invalid: {error}")))?;
    Ok(Json(value))
}

async fn acp_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Query(query): Query<AcpQuery>,
) -> Result<Response, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let mut roots = Vec::new();
    if let Some(value) = query.roots.as_deref().filter(|value| !value.is_empty()) {
        if value.len() > 4096 {
            return Err(ApiError::bad_request("Too many relay workspace roots"));
        }
        for id in value.split(',') {
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
            {
                return Err(ApiError::bad_request("Invalid relay workspace root id"));
            }
            let root = state.project_for_user(&user.id, id)?;
            if root.id != project.id && !roots.iter().any(|entry: &Project| entry.id == root.id) {
                roots.push(root);
            }
        }
        if roots.len() > 31 {
            return Err(ApiError::bad_request(
                "A relay group workspace supports at most 32 roots",
            ));
        }
    }
    let connection_id = format!("conn-{}", uuid::Uuid::now_v7());
    let permit = state.connection_permit()?;
    let header_id = connection_id.clone();
    let mut response = ws.on_upgrade(move |socket| {
        crate::acp::bridge(
            socket,
            state,
            user,
            device,
            project,
            roots,
            connection_id,
            permit,
        )
    });
    if let Ok(value) = HeaderValue::from_str(&header_id) {
        response
            .headers_mut()
            .insert(HeaderName::from_static("acp-connection-id"), value);
    }
    Ok(response)
}

pub fn active_lease(state: &AppState, session_id: &str) -> Option<Lease> {
    let mut leases = state.leases.lock().ok()?;
    if let Some(lease) = leases.get(session_id) {
        if lease.expires_at > Instant::now() {
            return Some(lease.clone());
        }
        leases.remove(session_id);
    }
    None
}

async fn fs_tree(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<FileTree>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let permit = state.fs_permit()?;
    let tree = run_blocking(move || {
        let _permit = permit;
        Ok(FileTree::load(project.path))
    })
    .await?;
    Ok(Json(tree))
}

async fn fs_storage(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Option<u64>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let permit = state.fs_permit()?;
    Ok(Json(
        run_blocking(move || {
            let _permit = permit;
            Ok(dunlin_workspace::tree::storage_size(std::path::Path::new(
                &project.path,
            )))
        })
        .await?,
    ))
}

#[derive(Deserialize)]
struct FileQuery {
    project: String,
    path: String,
}

async fn fs_file(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<workspace_fs::FileContents>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let permit = state.fs_permit()?;
    let file = run_blocking(move || {
        let _permit = permit;
        workspace_fs::read_file(&project.path, &query.path)
    })
    .await?;
    Ok(Json(file))
}

#[derive(Deserialize)]
struct AuditRunsQuery {
    project: String,
}

#[derive(Deserialize)]
struct AuditFileQuery {
    project: String,
    run: String,
    name: String,
}

#[derive(Serialize)]
struct AuditRunInfo {
    id: String,
    files: Vec<String>,
}

fn safe_audit_run(value: &str) -> bool {
    value.starts_with("run-")
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn safe_audit_file(value: &str) -> bool {
    matches!(
        value,
        "dunlin-run.json"
            | "run-metadata.json"
            | "scan-manifest.json"
            | "findings.json"
            | "coverage-ledger.json"
            | "coverage.json"
            | "REPORT.md"
            | "report.md"
            | "FINDINGS-DETAIL.md"
            | "NEEDS-VALIDATION.md"
            | "architecture.md"
            | "verification.json"
            | "results.sarif"
            | "exports/results.sarif"
            | "dunlin-results.sarif"
            | "dunlin-findings.json"
            | "dunlin-findings.csv"
            | "dunlin-report.md"
    ) || (value.starts_with("verification-")
        && value.ends_with(".json")
        && value.len() <= 160
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')))
}

async fn audit_runs(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<AuditRunsQuery>,
) -> Result<Json<Vec<AuditRunInfo>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let permit = state.fs_permit()?;
    let runs = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let root = FsPath::new(&project.path).join(".dunlin/audits");
        let Ok(read) = std::fs::read_dir(&root) else {
            return Vec::new();
        };
        let mut runs = read
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let metadata = std::fs::symlink_metadata(entry.path()).ok()?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return None;
                }
                let id = entry.file_name().to_str()?.to_string();
                if !safe_audit_run(&id) {
                    return None;
                }
                let mut files = std::fs::read_dir(entry.path())
                    .ok()?
                    .filter_map(Result::ok)
                    .filter_map(|file| {
                        let name = file.file_name().to_str()?.to_string();
                        safe_audit_file(&name).then_some(name)
                    })
                    .take(256)
                    .collect::<Vec<_>>();
                if entry.path().join("exports/results.sarif").is_file() {
                    files.push("exports/results.sarif".into());
                }
                Some(AuditRunInfo { id, files })
            })
            .take(101)
            .collect::<Vec<_>>();
        runs.sort_by(|a, b| a.id.cmp(&b.id));
        runs.truncate(100);
        runs
    })
    .await
    .map_err(|_| ApiError::internal("audit list worker failed"))?;
    Ok(Json(runs))
}

async fn audit_file(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<AuditFileQuery>,
) -> Result<Json<workspace_fs::FileContents>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    if !safe_audit_run(&query.run) || !safe_audit_file(&query.name) {
        return Err(ApiError::bad_request("Invalid audit artifact path"));
    }
    let permit = state.fs_permit()?;
    let relative = format!(".dunlin/audits/{}/{}", query.run, query.name);
    let file = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        workspace_fs::read_file(&project.path, &relative)
    })
    .await
    .map_err(|_| ApiError::internal("audit read worker failed"))?
    .map_err(ApiError::bad_request)?;
    Ok(Json(file))
}

#[derive(Deserialize)]
struct WriteFileRequest {
    project: String,
    path: String,
    contents: String,
    expected_mtime_ms: Option<u64>,
}

async fn fs_write(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<WriteFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    let stat = tokio::task::spawn_blocking(move || {
        workspace_fs::write_file_checked(
            &project.path,
            &request.path,
            &request.contents,
            request.expected_mtime_ms,
        )
    })
    .await
    .map_err(|error| ApiError::internal(error.to_string()))?
    .map_err(|error| {
        if error.starts_with("conflict:") {
            ApiError::conflict(error)
        } else {
            ApiError::bad_request(error)
        }
    })?;
    Ok(Json(stat))
}

#[derive(Deserialize)]
struct CreateFileRequest {
    project: String,
    path: String,
}

async fn fs_create(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    let stat =
        run_blocking(move || workspace_fs::create_file(&project.path, &request.path)).await?;
    Ok(Json(stat))
}

async fn fs_mkdir(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    Ok(Json(
        run_blocking(move || {
            dunlin_workspace::transfer::create_directory(&project.path, &request.path)
        })
        .await?,
    ))
}

#[derive(Deserialize)]
struct RenameRequest {
    project: String,
    path: String,
    name: String,
}
async fn fs_rename(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<dunlin_workspace::transfer::Transferred>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    Ok(Json(
        run_blocking(move || {
            dunlin_workspace::transfer::rename_entry(&project.path, &request.path, &request.name)
        })
        .await?,
    ))
}
async fn fs_delete(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<Value>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || dunlin_workspace::transfer::delete_entry(&project.path, &request.path))
        .await?;
    Ok(Json(json!({"ok": true})))
}
async fn fs_preview(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let permit = state.fs_permit()?;
    let bytes = run_blocking(move || {
        let _permit = permit;
        dunlin_workspace::preview::read(&project.path, &query.path)
    })
    .await?;
    if bytes.len() > 18 * 1024 * 1024 {
        return Err(ApiError::bad_request(
            "Preview response is larger than 24 MiB; open a smaller image",
        ));
    }
    let encoded = STANDARD.encode(bytes);
    if encoded.len() > 24 * 1024 * 1024 {
        return Err(ApiError::bad_request(
            "Preview response is larger than 24 MiB; open a smaller image",
        ));
    }
    Ok(Json(json!({"contents": encoded})))
}

#[derive(Deserialize)]
struct MoveRequest {
    project: String,
    sources: Vec<String>,
    target: String,
}

async fn fs_move(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<MoveRequest>,
) -> Result<Json<dunlin_workspace::transfer::TransferReport>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    if request.sources.len() > 1000 {
        return Err(ApiError::bad_request("Too many paths"));
    }
    let report = run_blocking(move || {
        dunlin_workspace::transfer::move_entries(&project.path, &request.sources, &request.target)
    })
    .await?;
    Ok(Json(report))
}

async fn git_init(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<Json<Value>, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::initialize(&project.path)).await?;
    Ok(Json(json!({"ok": true})))
}

async fn fs_stat(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let stat = run_blocking(move || workspace_fs::stat_file(&project.path, &query.path)).await?;
    Ok(Json(stat))
}

#[derive(Deserialize)]
struct UploadRequest {
    project: String,
    name: String,
    contents: String,
}

#[derive(Serialize)]
struct UploadResponse {
    path: String,
}

/// Create a project-relative directory without following a symlink in any
/// component. Used for the attachment namespace, which is written by the relay
/// process rather than by the project-scoped editor.
fn safe_project_directory(root: &str, relative: &str) -> Result<PathBuf, ApiError> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| ApiError::internal(format!("Project is unavailable: {error}")))?;
    let mut current = root;
    for part in relative
        .split('/')
        .filter(|part| !part.is_empty() && *part != ".")
    {
        if part == ".." {
            return Err(ApiError::bad_request("Invalid attachment path"));
        }
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(ApiError::bad_request(
                    "Attachment directory is a symlink or not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| {
                    ApiError::internal(format!("Could not create attachment directory: {error}"))
                })?;
            }
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "Could not inspect attachment directory: {error}"
                )));
            }
        }
    }
    Ok(current)
}

fn has_symlink_component(path: &FsPath) -> bool {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        if std::fs::symlink_metadata(&current)
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
        {
            return true;
        }
    }
    false
}

/// Store a locally attached file inside the project so the remote agent can
/// read it. Attachments are namespaced under `.dunlin/attachments`.
async fn fs_upload(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<UploadRequest>,
) -> Result<Json<UploadResponse>, ApiError> {
    const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
    let project = state.project_for_user(&user.id, &request.project)?;
    let bytes = STANDARD
        .decode(request.contents.as_bytes())
        .map_err(|_| ApiError::bad_request("Attachment is not valid base64"))?;
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::bad_request("Attachment is larger than 8 MiB"));
    }
    let name = sanitize_attachment_name(&request.name);
    let relative = format!(".dunlin/attachments/{}-{name}", uuid::Uuid::now_v7());
    let directory = safe_project_directory(&project.path, ".dunlin/attachments")?;
    let mut attachment_count = 0usize;
    let mut attachment_bytes = 0u64;
    if let Ok(entries) = std::fs::read_dir(&directory) {
        for entry in entries.flatten() {
            if let Ok(metadata) = std::fs::symlink_metadata(entry.path())
                && metadata.is_file()
                && !metadata.file_type().is_symlink()
            {
                attachment_count += 1;
                attachment_bytes = attachment_bytes.saturating_add(metadata.len());
            }
        }
    }
    if attachment_count >= 1000 || attachment_bytes >= 1024 * 1024 * 1024 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Attachment quota reached; remove old attachments first",
        ));
    }
    let full = directory.join(
        FsPath::new(&relative)
            .file_name()
            .ok_or_else(|| ApiError::bad_request("Invalid attachment name"))?,
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&full)
        .map_err(|error| ApiError::internal(format!("Could not store attachment: {error}")))?;
    use std::io::Write as _;
    file.write_all(&bytes)
        .map_err(|error| ApiError::internal(format!("Could not store attachment: {error}")))?;
    exclude_attachments_from_git(&project.path);
    Ok(Json(UploadResponse { path: relative }))
}

/// Keep uploaded attachments out of the user's `git status` without touching a
/// tracked `.gitignore`.
fn exclude_attachments_from_git(project_path: &str) {
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(project_path)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
    else {
        return;
    };
    if !output.status.success() {
        return;
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if path.is_empty() {
        return;
    }
    let path = FsPath::new(&path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        FsPath::new(project_path).join(path)
    };
    if has_symlink_component(&path) {
        return;
    }
    if let Some(parent) = path.parent() {
        if has_symlink_component(parent) {
            return;
        }
        let _ = std::fs::create_dir_all(parent);
    }
    if has_symlink_component(&path) {
        return;
    }
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.lines().any(|line| line.trim() == ".dunlin/") {
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(".dunlin/\n");
        let _ = std::fs::write(path, next);
    }
}

fn sanitize_attachment_name(name: &str) -> String {
    let base = FsPath::new(name)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("attachment");
    let cleaned: String = base
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "attachment".into()
    } else {
        cleaned.chars().take(120).collect()
    }
}

#[derive(Deserialize)]
struct SearchQuery {
    project: String,
    q: String,
    limit: Option<usize>,
}

async fn search_project(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<search::SearchHit>>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    if query.q.len() > 512 {
        return Err(ApiError::bad_request(
            "Search query is limited to 512 bytes",
        ));
    }
    let limit = query.limit.unwrap_or(80).clamp(1, 500);
    let permit = state.fs_permit()?;
    let hits = run_blocking(move || {
        let _permit = permit;
        Ok(search::search_project(&project.path, &query.q, limit))
    })
    .await?;
    Ok(Json(hits))
}

async fn git_status(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<git::GitStatus>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let status = run_blocking(move || Ok(git::load(&project.path))).await?;
    Ok(Json(status))
}

#[derive(Deserialize)]
struct DiffQuery {
    project: String,
    file: String,
    staged: Option<bool>,
}

#[derive(Serialize)]
struct PatchResponse {
    patch: String,
}

async fn git_diff(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<PatchResponse>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let staged = query.staged.unwrap_or(false);
    let patch = run_blocking(move || git::file_diff(&project.path, &query.file, staged)).await?;
    Ok(Json(PatchResponse { patch }))
}

#[derive(Deserialize)]
struct StageRequest {
    project: String,
    file: Option<String>,
}

async fn git_stage(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<StageRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || {
        if let Some(file) = request.file {
            git::stage_file(&project.path, &file)
        } else {
            git::stage_all(&project.path)
        }
    })
    .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct FileRequest {
    project: String,
    file: String,
}

async fn git_unstage(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<FileRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::unstage_file(&project.path, &request.file)).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct CommitRequest {
    project: String,
    message: String,
}

async fn git_commit(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    if request.message.len() > 1024 * 1024 {
        return Err(ApiError::bad_request("Commit message is too large"));
    }
    run_blocking(move || git::commit(&project.path, &request.message)).await?;
    Ok(StatusCode::OK)
}

async fn git_amend(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    if request.message.len() > 1024 * 1024 {
        return Err(ApiError::bad_request("Commit message is too large"));
    }
    run_blocking(move || git::amend(&project.path, &request.message)).await?;
    Ok(StatusCode::OK)
}

async fn git_discard(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<FileRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::discard_file(&project.path, &request.file)).await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ProjectRequest {
    project: String,
}

async fn git_pull(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::pull(&project.path)).await?;
    Ok(StatusCode::OK)
}

async fn git_push(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::push(&project.path)).await?;
    Ok(StatusCode::OK)
}

#[derive(Serialize)]
struct BranchesResponse {
    branches: Vec<String>,
}

async fn git_branches(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<BranchesResponse>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let branches = run_blocking(move || git::branches(&project.path)).await?;
    Ok(Json(BranchesResponse { branches }))
}

#[derive(Deserialize)]
struct BranchRequest {
    project: String,
    branch: String,
}

async fn git_branch_context(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<git::BranchContext>, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    Ok(Json(
        run_blocking(move || git::branch_context(&project.path)).await?,
    ))
}

#[derive(Deserialize)]
struct SafeBranchRequest {
    project: String,
    branch: String,
    expected: git::BranchContext,
    keep_changes: bool,
}
async fn git_switch_safe(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<SafeBranchRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || {
        git::switch_branch_checked(
            &project.path,
            &request.branch,
            &request.expected,
            request.keep_changes,
        )
    })
    .await?;
    Ok(StatusCode::OK)
}

async fn git_switch(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<BranchRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
    run_blocking(move || git::switch_branch(&project.path, &request.branch)).await?;
    Ok(StatusCode::OK)
}

pub(crate) async fn run_blocking<T, F>(operation: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tokio::task::spawn_blocking(operation)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?
        .map_err(ApiError::bad_request)
}

#[cfg(test)]
mod tests;
