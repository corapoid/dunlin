//! HTTP and WebSocket API for the relay.

use std::collections::HashMap;
use std::path::Path as FsPath;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, FromRequestParts, Path, Query, State, ws::WebSocketUpgrade};
use axum::http::{HeaderName, HeaderValue, StatusCode, header::AUTHORIZATION, request::Parts};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use dunlin_domain::ProviderUsage;
use dunlin_workspace::fs as workspace_fs;
use dunlin_workspace::git;
use dunlin_workspace::search;
use dunlin_workspace::tree::FileTree;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::config::Config;
use crate::store::{Device, Project, Store, StoreError, User};

pub type UsageCache = Arc<Mutex<Option<(Instant, Vec<ProviderUsage>)>>>;

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub config: Arc<Config>,
    pub leases: Arc<Mutex<HashMap<String, Lease>>>,
    pub usage_cache: UsageCache,
}

#[derive(Clone)]
pub struct Lease {
    pub device_id: String,
    pub connection_id: String,
    pub close_tx: mpsc::Sender<()>,
    pub expires_at: Instant,
}

impl AppState {
    pub fn new(store: Store, config: Config) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
            config: Arc::new(config),
            leases: Arc::new(Mutex::new(HashMap::new())),
            usage_cache: Arc::new(Mutex::new(None)),
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
        Ok(Self {
            user: auth.0,
            device: auth.1,
        })
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/v1/pair", post(pair))
        .route("/v1/me", get(me))
        .route("/v1/usage", get(usage))
        .route("/v1/projects", get(projects))
        .route("/v1/sessions", get(sessions))
        .route("/v1/sessions/{id}", delete(delete_session))
        .route("/v1/sessions/{id}/takeover", post(takeover))
        .route(
            "/v1/sessions/{id}/transcript",
            get(session_transcript).put(session_transcript_save),
        )
        .route("/acp", get(acp_ws))
        .route("/v1/fs/tree", get(fs_tree))
        .route("/v1/fs/file", get(fs_file).put(fs_write))
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
        .layer(DefaultBodyLimit::max(12 * 1024 * 1024))
        .with_state(state)
}

async fn health() -> &'static str {
    "ok"
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
    let device_name = request.device_name.trim();
    let device_name = if device_name.is_empty() {
        "Dunlin device"
    } else {
        device_name
    };
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
            .create_device(&user.id, device_name)
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
    _auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderUsage>>, ApiError> {
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

#[derive(Deserialize)]
struct ProjectQuery {
    project: String,
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
    state
        .store
        .lock()
        .map_err(|_| ApiError::internal("store lock poisoned"))?
        .save_transcript(&id, &payload)
        .map_err(ApiError::from)?;
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
    Query(query): Query<ProjectQuery>,
) -> Result<Response, ApiError> {
    let project = state.project_for_user(&user.id, &query.project)?;
    let connection_id = format!("conn-{}", uuid::Uuid::now_v7());
    let header_id = connection_id.clone();
    let mut response = ws.on_upgrade(move |socket| {
        crate::acp::bridge(socket, state, user, device, project, connection_id)
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
    let tree = run_blocking(move || Ok(FileTree::load(project.path))).await?;
    Ok(Json(tree))
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
    let file = run_blocking(move || workspace_fs::read_file(&project.path, &query.path)).await?;
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
    if let Some(expected) = request.expected_mtime_ms {
        let current =
            workspace_fs::stat_file(&project.path, &request.path).map_err(ApiError::bad_request)?;
        if current.mtime_ms != expected {
            return Err(ApiError::conflict(
                "File changed on the relay since it was opened",
            ));
        }
    }
    let stat = run_blocking(move || {
        workspace_fs::write_file(&project.path, &request.path, &request.contents)
    })
    .await?;
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
    let bytes =
        run_blocking(move || dunlin_workspace::preview::read(&project.path, &query.path)).await?;
    Ok(Json(json!({"contents": STANDARD.encode(bytes)})))
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
    let full = FsPath::new(&project.path).join(&relative);
    if let Some(parent) = full.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| ApiError::internal(format!("Could not store attachment: {error}")))?;
    }
    std::fs::write(&full, bytes)
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
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
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
        cleaned
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
    let limit = query.limit.unwrap_or(80).clamp(1, 500);
    let hits =
        run_blocking(move || Ok(search::search_project(&project.path, &query.q, limit))).await?;
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
    run_blocking(move || git::commit(&project.path, &request.message)).await?;
    Ok(StatusCode::OK)
}

async fn git_amend(
    AuthUser { user, .. }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<StatusCode, ApiError> {
    let project = state.project_for_user(&user.id, &request.project)?;
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

async fn run_blocking<T, F>(operation: F) -> Result<T, ApiError>
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
