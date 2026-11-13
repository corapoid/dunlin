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
use dunlin_relay_protocol::{PairRequest, PairResponse, SessionInfo};
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
mod native_io;
mod operations;
mod presence;
mod web;

use crate::config::Config;
use crate::store::{ArtifactRow, Device, Project, Store, StoreError, User};

pub type UsageCache = Arc<Mutex<Option<(Instant, Vec<ProviderUsage>)>>>;

#[derive(Clone)]
pub struct AppState {
    pub(crate) jobs: Arc<crate::jobs::Service>,
    pub(crate) interactive: Arc<crate::interactive::Service>,
    pub(crate) remote_sessions: Arc<std::sync::OnceLock<Arc<crate::remote_sessions::Service>>>,
    pub(crate) worker_channels: Arc<crate::worker_channel::Service>,
    pub(crate) linear: Arc<crate::linear::Service>,
    pub(crate) mcp_oauth: Arc<crate::mcp_oauth::Service>,
    pub(crate) scoped_secrets: Arc<crate::scoped_secrets::Service>,
    presence: Arc<Mutex<presence::Registry>>,
    pub(crate) management: Arc<Mutex<crate::management::Management>>,
    pub store: Arc<Mutex<Store>>,
    pub config: Arc<Config>,
    pub(crate) vault: Option<Arc<crate::vault::RelayVault>>,
    pub(crate) coordination: Option<Arc<dunlin_coordination::service::Service>>,
    pub(crate) artifact_dir: PathBuf,
    pub leases: Arc<Mutex<HashMap<String, Lease>>>,
    pub usage_cache: UsageCache,
    browser_sessions: Arc<web::BrowserSessions>,
    oidc: Arc<crate::oidc::Service>,
    history_reads: Arc<tokio::sync::Semaphore>,
    fs_reads: Arc<Semaphore>,
    connections: Arc<Semaphore>,
    terminal_sessions: Arc<Semaphore>,
    pair_attempts: Arc<Mutex<VecDeque<Instant>>>,
    git_plans: Arc<Mutex<HashMap<String, operations::Pending>>>,
    review_plans: Arc<Mutex<HashMap<String, PendingRestore>>>,
    compare_plans: Arc<Mutex<HashMap<String, PendingCompare>>>,
    checks: Arc<Mutex<HashMap<String, RunningCheck>>>,
}

struct PendingRestore {
    proof: native_io::NativeGitProof,
    prepared: Instant,
    plan: dunlin_workspace::task_review::Restore,
}

struct PendingCompare {
    proof: native_io::NativeGitProof,
    prepared: Instant,
    plan: dunlin_workspace::comparison::Prepared,
}

struct RunningCheck {
    proof: native_io::NativeGitProof,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
struct CheckCancellation(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CheckCancellation {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
struct CheckCleanup {
    state: AppState,
    id: String,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}
impl Drop for CheckCleanup {
    fn drop(&mut self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Release);
        if let Ok(mut checks) = self.state.checks.lock()
            && checks
                .get(&self.id)
                .is_some_and(|entry| Arc::ptr_eq(&entry.cancel, &self.cancel))
        {
            checks.remove(&self.id);
        }
    }
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
            jobs: Arc::new(crate::jobs::Service::default()),
            interactive: Arc::new(crate::interactive::Service::default()),
            remote_sessions: Arc::new(std::sync::OnceLock::new()),
            worker_channels: Arc::new(crate::worker_channel::Service::default()),
            linear: Arc::new(crate::linear::Service::default()),
            mcp_oauth: Arc::new(crate::mcp_oauth::Service::default()),
            scoped_secrets: Arc::new(crate::scoped_secrets::Service::default()),
            presence: Arc::new(Mutex::new(presence::Registry::default())),
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
            browser_sessions: Arc::new(web::BrowserSessions::default()),
            oidc: Arc::new(crate::oidc::Service::default()),
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

    pub(crate) fn fs_permit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        self.fs_reads.clone().try_acquire_owned().map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Workspace file reads are busy; try again",
            )
        })
    }

    pub(crate) fn connection_permit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
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
        let (user, device) = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("store lock poisoned"))?
            .authenticate(token)?
            .ok_or_else(ApiError::unauthorized)?;
        check_host_account_policy(state, &user)?;
        Ok(Self { user, device })
    }
}

pub(crate) fn check_host_account_policy(state: &AppState, user: &User) -> Result<(), ApiError> {
    if user.role != "admin" && crate::management::host_accounts(&state.config) {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "This relay uses host-level Git accounts and permits administrators only. Use an isolated relay service for member access.",
        ));
    }
    Ok(())
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secret-tool", get(web::scoped_secrets::tool_catalog).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/secret-bindings", get(web::scoped_secrets::available).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets/{credential}/grants", get(web::scoped_secrets::grants).post(web::scoped_secrets::set_grant).layer(DefaultBodyLimit::max(4096)).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets/{credential}/grants/{grant}/revoke", post(web::scoped_secrets::revoke_grant).layer(DefaultBodyLimit::max(1024)).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets", get(web::scoped_secrets::list).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets/{credential}", post(web::scoped_secrets::install).layer(DefaultBodyLimit::max(80*1024)).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/projects/{project}/hosts/{host}/scoped-secrets/{credential}/disable", post(web::scoped_secrets::disable).layer(DefaultBodyLimit::max(1024)).layer(axum::middleware::map_response(web::scoped_secrets::private_headers)))
        .route("/v1/web/workspaces/{id}/mcp-providers", get(web::mcp_oauth::providers))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}/inspect", post(web::mcp_oauth::inspect).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/workspaces/{id}/mcp-providers/{provider}/authorize", post(web::mcp_oauth::start).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/mcp/oauth/{provider}/callback", get(web::mcp_oauth::callback).layer(axum::middleware::map_response(web::mcp_oauth::callback_headers)))
        .route("/v1/web/workspaces/{id}/mcp-connections", get(web::mcp_oauth::list))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}", get(web::mcp_oauth::get))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}/disconnect", post(web::mcp_oauth::disconnect).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}/revoke", post(web::mcp_oauth::revoke).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}/reconcile-revocation", post(web::mcp_oauth::reconcile).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/workspaces/{id}/mcp-connections/{connection}/cleanup", post(web::mcp_oauth::cleanup).layer(DefaultBodyLimit::max(1024)))
        .route("/v1/web/workspaces/{id}/projects/{project}/interactive-sessions", post(web::interactive::create).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/interactive-sessions/{id}", get(web::interactive::get))
        .route("/v1/web/ahp",get(web::ahp::upgrade))
        .route("/v1/web/interactive-sessions/{id}/sharing", get(web::sharing::get).put(web::sharing::replace).layer(DefaultBodyLimit::max(8192)))
        .route("/v1/web/interactive-sessions/{id}/stop", post(web::interactive::stop).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/binding", get(web::bindings::get).put(web::bindings::set).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/remote-bindings", get(web::remote_bindings::list).post(web::remote_bindings::create).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/remote-bindings/resolve", post(web::remote_bindings::resolve).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/remote-bindings/{binding}", axum::routing::put(web::remote_bindings::save).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/remote-bindings/{binding}/revoke", post(web::remote_bindings::revoke).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/projects/{project}/binding/revoke", post(web::bindings::revoke).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/service-principals", get(web::service_principals::list).post(web::service_principals::create).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/service-principals/{principal}/revoke", post(web::service_principals::revoke).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/service/workspaces/{id}/projects/{project}/workflows/{workflow}/versions/{version}", get(web::service_principals::workflow))
        .route("/v1/web/oidc/config", get(web::oidc::configuration))
        .route("/v1/web/oidc/start", post(web::oidc::start))
        .route("/v1/web/oidc/callback", get(web::oidc::callback))
        .route(
            "/v1/web/pair",
            post(web::pair).layer(DefaultBodyLimit::max(4096)),
        )
        .route("/v1/web/session", get(web::session))
        .route("/v1/web/logout", post(web::logout))
        .route("/v1/web/live", get(web::upgrade))
        .route(
            "/v1/web/workspaces",
            get(web::list_workspaces)
                .post(web::create_workspace)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/v1/web/workspaces/{id}/members",
            get(web::team_members)
                .put(web::update_team_member)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route("/v1/web/workspaces/{id}/audit", get(web::team_audit))
        .route(
            "/v1/web/workspaces/{id}/invitations",
            post(web::invite_member),
        )
        .route(
            "/v1/web/workspaces/{id}/invitations/{recipient}",
            delete(web::revoke_invitations),
        )
        .route("/v1/web/invitations/accept", post(web::accept_invitation))
        .route(
            "/v1/web/workspaces/{id}/projects",
            get(web::list_projects)
                .post(web::create_project)
                .layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/grants",
            axum::routing::put(web::update_project_grant).layer(DefaultBodyLimit::max(4096)),
        )
        .route("/v1/worker/channel", get(crate::worker_channel::upgrade))
        .route("/v1/worker/enroll", post(web::hosts::worker_enroll).layer(DefaultBodyLimit::max(20 * 1024)))
        .route("/v1/worker/report", post(web::hosts::worker_report).layer(DefaultBodyLimit::max(20 * 1024)))
        .route("/v1/web/workspaces/{id}/hosts", get(web::hosts::list).post(web::hosts::create).layer(DefaultBodyLimit::max(16 * 1024)))
        .route("/v1/web/workspaces/{id}/hosts/{host}", get(web::hosts::get))
        .route("/v1/web/workspaces/{id}/hosts/{host}/channel", get(web::hosts::channel))
        .route("/v1/web/workspaces/{id}/hosts/{host}/health", get(web::hosts::health))
        .route("/v1/web/workspaces/{id}/hosts/{host}/health/live", get(web::hosts::live_health))
        .route("/v1/web/workspaces/{id}/hosts/{host}/revocation", get(web::hosts::revocation))
        .route("/v1/web/workspaces/{id}/hosts/{host}/enrollment", post(web::hosts::enrollment).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/hosts/{host}/approve", post(web::hosts::approve).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/hosts/{host}/drain", post(web::hosts::drain).layer(DefaultBodyLimit::max(4096)))
        .route("/v1/web/workspaces/{id}/hosts/{host}/revoke", post(web::hosts::revoke).layer(DefaultBodyLimit::max(4096)))
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/agents",
            get(web::agents::list).post(web::agents::create)
                .layer(DefaultBodyLimit::max(72 * 1024)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/agents/{agent}",
            get(web::agents::get).put(web::agents::save)
                .layer(DefaultBodyLimit::max(72 * 1024)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/agents/{agent}/publish",
            post(web::agents::publish).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/agents/{agent}/versions/{version}",
            get(web::agents::version),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/agent-profiles",
            get(web::agents::profiles),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/workflows",
            post(web::create_workflow).layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/workflows/{workflow}",
            get(web::get_workflow)
                .put(web::save_workflow)
                .layer(DefaultBodyLimit::max(1024 * 1024)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/workflows/{workflow}/publish",
            post(web::publish_workflow).layer(DefaultBodyLimit::max(4096)),
        )
        .route(
            "/v1/web/workspaces/{id}/projects/{project}/workflows/{workflow}/versions/{version}",
            get(web::workflow_version),
        )
        .route("/health", get(health))
        .route("/v1/presence", post(presence::heartbeat))
        .route(
            "/v1/jobs",
            get(crate::jobs::list)
                .post(crate::jobs::enqueue)
                .layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/v1/jobs/queue", post(crate::jobs::queue_action))
        .route(
            "/v1/jobs/{id}",
            get(crate::jobs::get).post(crate::jobs::action),
        )
        .route(
            "/v1/integrations/linear",
            get(crate::linear::inbox)
                .put(crate::linear::connect)
                .delete(crate::linear::disconnect)
                .layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route("/v1/integrations/linear/sync", post(crate::linear::sync))
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
        .fallback(web::asset)
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

struct ArtifactStage {
    path: PathBuf,
    published: bool,
}
impl Drop for ArtifactStage {
    fn drop(&mut self) {
        if !self.published {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}
fn private_artifact(
    state: &AppState,
    user: &str,
    device: &str,
    id: &str,
) -> Result<(ArtifactRow, native_io::NativeIo), ApiError> {
    let row = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?
        .artifact(id)?
        .ok_or_else(ApiError::not_found)?;
    let io = native_io::NativeIo::capture(state, user, device, &row.project_id)?;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        io.validate_in_store(&store)?;
        current_artifact(&store, &row)?;
    }
    Ok((row, io))
}
fn current_artifact(store: &Store, original: &ArtifactRow) -> Result<ArtifactRow, ApiError> {
    let current = store
        .artifact(&original.id)?
        .ok_or_else(ApiError::not_found)?;
    if current.project_id != original.project_id
        || current.user_id != original.user_id
        || current.blob_path != original.blob_path
        || current.sha256 != original.sha256
    {
        return Err(ApiError::conflict(
            "Artifact changed; refresh before continuing",
        ));
    }
    Ok(current)
}
fn mutate_owned_artifact<T>(
    state: &AppState,
    io: &native_io::NativeIo,
    row: &ArtifactRow,
    operation: impl FnOnce(&Store, &ArtifactRow) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    io.validate_in_store(&store)?;
    let current = current_artifact(&store, row)?;
    if current.project_id != io.project.id || current.user_id != io.proof().user {
        return Err(ApiError::forbidden());
    }
    operation(&store, &current)
}
fn publish_artifact(
    state: &AppState,
    io: &native_io::NativeIo,
    row: &ArtifactRow,
    stage: &mut ArtifactStage,
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    io.validate_in_store(&store)?;
    if row.project_id != io.project.id || row.user_id != io.proof().user {
        return Err(ApiError::forbidden());
    }
    // Both paths are server-generated in the same private directory. hard_link
    // publishes without replacing an existing file; authorization and DB insert
    // share the Store cut while the large byte transfer happened before it.
    std::fs::hard_link(&stage.path, &row.blob_path)
        .map_err(|_| ApiError::internal("Could not publish artifact"))?;
    let old = std::mem::replace(&mut stage.path, PathBuf::from(&row.blob_path));
    let _ = std::fs::remove_file(old);
    store.insert_artifact(row)?;
    stage.published = true;
    Ok(())
}

async fn artifacts_list(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Query(query): Query<ArtifactQuery>,
) -> Result<Json<Vec<ArtifactRow>>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let rows = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        io.validate_in_store(&store)?;
        store.artifacts_for_project(&user.id, &io.project.id)?
    };
    io.validate()?;
    Ok(Json(rows))
}

async fn artifact_upload(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Json(request): Json<ArtifactUpload>,
) -> Result<Json<ArtifactRow>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project_id)?;
    let publisher = io.clone();
    let kind = clean_label(&request.kind, 40, "artifact")?;
    let title = clean_label(&request.title, 240, "Untitled artifact")?;
    let mime = clean_mime(&request.mime)?;
    let permit = state.fs_permit()?;
    let worker_state = state.clone();
    let row = io
        .mutate(move |project| {
            let _permit = permit;
            let bytes = STANDARD
                .decode(request.data.as_bytes())
                .map_err(|_| "Artifact data is not valid base64")?;
            if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_BYTES {
                return Err(format!(
                    "Artifact must be between 1 byte and {} MiB",
                    MAX_ARTIFACT_BYTES / 1024 / 1024
                ));
            }
            let id = uuid::Uuid::now_v7().to_string();
            let directory = worker_state.artifact_dir.join(&project.id);
            publisher
                .validate()
                .map_err(|error| format!("conflict:{}", error.message))?;
            std::fs::create_dir_all(&directory)
                .map_err(|_| "internal:Could not create artifact directory")?;
            let pending = directory.join(format!(".pending-{id}"));
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&pending)
                .map_err(|_| "internal:Could not stage artifact")?;
            let mut stage = ArtifactStage {
                path: pending,
                published: false,
            };
            file.write_all(&bytes)
                .map_err(|_| "internal:Could not stage artifact")?;
            drop(file);
            let row = ArtifactRow {
                id: id.clone(),
                project_id: project.id.clone(),
                user_id: user.id,
                kind,
                title,
                mime,
                bytes: bytes.len() as i64,
                sha256: sha256_hex(&bytes),
                blob_path: directory.join(id).to_string_lossy().into_owned(),
                created_at: crate::store::now_secs(),
            };
            publish_artifact(&worker_state, &publisher, &row, &mut stage)
                .map_err(|error| format!("conflict:{}", error.message))?;
            Ok(row)
        })
        .await?;
    Ok(Json(row))
}

async fn artifact_download(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let (row, io) = private_artifact(&state, &user.id, &device.id, &id)?;
    let guard = io.clone();
    let original = row.clone();
    let permit = state.fs_permit()?;
    let bytes = io
        .read(move |_| {
            let _permit = permit;
            use std::io::Read as _;
            let file = std::fs::File::open(&original.blob_path)
                .map_err(|_| "internal:Could not read artifact")?;
            let mut bytes = Vec::new();
            file.take((MAX_ARTIFACT_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|_| "internal:Could not read artifact")?;
            if bytes.len() > MAX_ARTIFACT_BYTES {
                return Err("Artifact exceeds its read budget".into());
            }
            Ok(bytes)
        })
        .await?;
    let encoded = STANDARD.encode(bytes);
    {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        guard.validate_in_store(&store)?;
        current_artifact(&store, &row)?;
    }
    guard.validate()?;
    Ok(Json(json!({"artifact":row,"data":encoded})))
}

async fn artifact_delete(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (row, io) = private_artifact(&state, &user.id, &device.id, &id)?;
    if row.user_id != user.id {
        return Err(ApiError::forbidden());
    }
    let guard = io.clone();
    let permit = state.fs_permit()?;
    io.mutate(move|_| {
        let _permit=permit;
        Ok((|| -> Result<(),ApiError> {
            mutate_owned_artifact(&state,&guard,&row,|store,current|{store.delete_artifact(&current.id)?;Ok(())})?;
            guard.validate().map_err(|_|ApiError::conflict("Artifact metadata was removed but file cleanup may be incomplete; inspect state and do not automatically retry"))?;
            let _=std::fs::remove_file(&row.blob_path);
            Ok(())
        })())
    }).await??;
    Ok(StatusCode::NO_CONTENT)
}

async fn artifact_share(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Path(id): Path<String>,
    Json(request): Json<ShareRequest>,
) -> Result<Json<Value>, ApiError> {
    let (row, io) = private_artifact(&state, &user.id, &device.id, &id)?;
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
    mutate_owned_artifact(&state, &io, &row, |store, current| {
        store.create_artifact_share(&current.id, &token_hash, expires_at)?;
        Ok(())
    })?;
    io.after(true)?;
    Ok(Json(
        json!({"token":token,"expires_at":expires_at,"url":format!("/a/{token}")}),
    ))
}

async fn artifact_revoke_share(
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let (row, io) = private_artifact(&state, &user.id, &device.id, &id)?;
    mutate_owned_artifact(&state, &io, &row, |store, current| {
        store.revoke_artifact_shares(&current.id)?;
        Ok(())
    })?;
    io.after(true)?;
    Ok(StatusCode::NO_CONTENT)
}

// Public links intentionally use their token capability, not the uploader's
// current native grant. Revocation/expiry is checked again after slow I/O.
fn live_public_artifact(state: &AppState, token_hash: &str) -> Result<ArtifactRow, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    let share = store
        .artifact_share_by_token(token_hash)?
        .ok_or_else(ApiError::not_found)?;
    if share.revoked_at.is_some() || share.expires_at <= crate::store::now_secs() {
        return Err(ApiError::not_found());
    }
    store
        .artifact(&share.artifact_id)?
        .ok_or_else(ApiError::not_found)
}

async fn public_artifact(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Response, ApiError> {
    if token.is_empty() || token.len() > 128 {
        return Err(ApiError::not_found());
    }
    let token_hash = sha256_hex(token.as_bytes());
    let row = live_public_artifact(&state, &token_hash)?;
    let original = row.clone();
    let permit = state.fs_permit()?;
    let bytes = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        use std::io::Read as _;
        let file = std::fs::File::open(&original.blob_path)
            .map_err(|_| ApiError::internal("Could not read artifact"))?;
        let mut bytes = Vec::new();
        file.take((MAX_ARTIFACT_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| ApiError::internal("Could not read artifact"))?;
        if bytes.len() > MAX_ARTIFACT_BYTES {
            return Err(ApiError::bad_request("Artifact exceeds its read budget"));
        }
        Ok::<_, ApiError>(bytes)
    })
    .await
    .map_err(|_| ApiError::internal("Public artifact reader stopped"))??;
    let current = live_public_artifact(&state, &token_hash)?;
    if current.id != row.id || current.blob_path != row.blob_path || current.sha256 != row.sha256 {
        return Err(ApiError::not_found());
    }
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ImpactRequest>,
) -> Result<Json<impact::Index>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
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
    let index = io
        .read(move |project| {
            let _permit = permit;
            let root = git::history::repository_root(&project.path, None)?;
            impact::build(&root, None)
        })
        .await?;
    Ok(Json(index))
}

async fn impact_verify(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ImpactVerifyRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Impact verification is busy; try again",
            )
        })?;
    io.read(move |project| {
        let _permit = permit;
        let root = git::history::repository_root(&project.path, None)?;
        if request.index.root != root {
            return Err("conflict:Impact index belongs to another repository".into());
        }
        impact::verify(&request.index)
    })
    .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct GithubLoadRequest {
    project: String,
}

async fn github_metadata(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<GithubLoadRequest>,
) -> Result<Json<dunlin_workspace::github::Metadata>, ApiError> {
    current_metadata_admin(&state, &user.id, &device.id)?;
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "GitHub metadata is busy; try again",
            )
        })?;
    let worker_state = state.clone();
    let worker_user = user.id.clone();
    let worker_device = device.id.clone();
    let result = io
        .read(move |project| {
            let _permit = permit;
            Ok((|| -> Result<_, ApiError> {
                current_metadata_admin(&worker_state, &worker_user, &worker_device)?;
                let result = dunlin_workspace::github::metadata(&project.path)
                    .map_err(ApiError::bad_request);
                current_metadata_admin(&worker_state, &worker_user, &worker_device)?;
                result
            })())
        })
        .await?;
    current_metadata_admin(&state, &user.id, &device.id)?;
    Ok(Json(result?))
}

fn current_metadata_admin(state: &AppState, user: &str, device: &str) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    current_admin_in_store(
        state,
        &store,
        user,
        device,
        "Relay GitHub metadata uses the host account and requires a current administrator",
    )
}
fn current_admin_in_store(
    state: &AppState,
    store: &Store,
    user: &str,
    device: &str,
    message: &str,
) -> Result<(), ApiError> {
    let current = store
        .active_device_user(device)?
        .filter(|current| current.id == user)
        .ok_or_else(ApiError::unauthorized)?;
    if current.role != "admin" {
        return Err(ApiError::new(StatusCode::FORBIDDEN, message));
    }
    check_host_account_policy(state, &current)?;
    if !store.oidc_device_allowed(device, state.config.oidc.as_ref())? {
        return Err(ApiError::unauthorized());
    }
    Ok(())
}
fn settings_admin(state: &AppState, user: &str, device: &str) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    current_admin_in_store(
        state,
        &store,
        user,
        device,
        "Project runtime settings are restricted to relay administrators",
    )
}
fn read_project_settings(
    state: &AppState,
    io: &native_io::NativeIo,
) -> Result<Option<String>, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    io.validate_in_store(&store)?;
    Ok(store.project_settings(&io.project.id)?)
}
fn write_project_settings(
    state: &AppState,
    io: &native_io::NativeIo,
    payload: &str,
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    io.validate_in_store(&store)?;
    let proof = io.proof();
    current_admin_in_store(
        state,
        &store,
        &proof.user,
        &proof.device,
        "Project runtime settings are restricted to relay administrators",
    )?;
    store.save_project_settings(&io.project.id, payload)?;
    Ok(())
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewCaptureRequest>,
) -> Result<Json<dunlin_workspace::task_review::Checkpoint>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Review capture is busy; try again",
            )
        })?;
    let point = io
        .mutate(move |project| {
            let _permit = permit;
            dunlin_workspace::task_review::capture(&project.path, &request.label)
        })
        .await?;
    Ok(Json(point))
}

async fn review_changes(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewChangesRequest>,
) -> Result<Json<Vec<dunlin_workspace::task_review::Change>>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Review changes are busy; try again",
            )
        })?;
    let changes = io
        .read(move |project| {
            let _permit = permit;
            checkpoint_root(project, &request.checkpoint)?;
            dunlin_workspace::task_review::changes(&request.checkpoint)
        })
        .await?;
    Ok(Json(changes))
}

async fn review_restore_prepare(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewRestorePrepareRequest>,
) -> Result<Json<ReviewRestorePreview>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    let guard = io.clone();
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy; try again"))?;
    let plan = io
        .read(move |project| {
            let _permit = permit;
            checkpoint_root(project, &request.checkpoint)?;
            dunlin_workspace::task_review::prepare_restore(&request.checkpoint, &request.path)
        })
        .await?;
    let change = plan.change.clone();
    let id = uuid::Uuid::now_v7().to_string();
    guard.validate().await?;
    let mut plans = state
        .review_plans
        .lock()
        .map_err(|_| ApiError::internal("Restore plan registry unavailable"))?;
    plans.retain(|_, entry| entry.prepared.elapsed() < Duration::from_secs(300));
    if plans.len() >= 64 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many prepared restores",
        ));
    }
    plans.insert(
        id.clone(),
        PendingRestore {
            proof: guard.proof(),
            prepared: Instant::now(),
            plan,
        },
    );
    Ok(Json(ReviewRestorePreview { id, change }))
}

async fn review_restore_execute(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ReviewRestoreExecuteRequest>,
) -> Result<StatusCode, ApiError> {
    let pending = {
        let mut plans = state
            .review_plans
            .lock()
            .map_err(|_| ApiError::internal("Restore plan registry unavailable"))?;
        let entry = plans.get(&request.id).ok_or_else(ApiError::not_found)?;
        if entry.proof.user != user.id
            || entry.proof.device != device.id
            || entry.proof.project.id != request.project
        {
            return Err(ApiError::forbidden());
        }
        plans.remove(&request.id).ok_or_else(ApiError::not_found)?
    };
    if pending.prepared.elapsed() > Duration::from_secs(300) {
        return Err(ApiError::conflict("Restore preview expired; prepare again"));
    }
    let io = native_io::NativeGitIo::resume(&state, pending.proof).await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy; prepare again"))?;
    io.mutate(move |_| {
        let _permit = permit;
        dunlin_workspace::task_review::restore(pending.plan)
    })
    .await?;
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
    git_access: &'static str,
}

async fn compare_prepare(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ComparePrepareRequest>,
) -> Result<Json<ComparePreparedResponse>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    let guard = io.clone();
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy; try again"))?;
    let plan = io
        .read(move |project| {
            let _permit = permit;
            dunlin_workspace::comparison::prepare(&project.path)
        })
        .await?;
    let response = ComparePreparedResponse {
        id: plan.id.clone(),
        root: plan.root.clone(),
        head: plan.head.clone(),
        paths: plan.paths.to_vec(),
    };
    guard.validate().await?;
    let mut plans = state
        .compare_plans
        .lock()
        .map_err(|_| ApiError::internal("Comparison registry unavailable"))?;
    plans.retain(|_, entry| entry.prepared.elapsed() < Duration::from_secs(300));
    if plans.len() >= 32 {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many prepared comparisons",
        ));
    }
    plans.insert(
        plan.id.clone(),
        PendingCompare {
            proof: guard.proof(),
            prepared: Instant::now(),
            plan,
        },
    );
    Ok(Json(response))
}

async fn compare_create(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareCreateRequest>,
) -> Result<Json<CompareCreateResponse>, ApiError> {
    let pending = {
        let mut plans = state
            .compare_plans
            .lock()
            .map_err(|_| ApiError::internal("Comparison registry unavailable"))?;
        let entry = plans.get(&request.id).ok_or_else(ApiError::not_found)?;
        if entry.proof.user != user.id
            || entry.proof.device != device.id
            || entry.proof.project.id != request.project
        {
            return Err(ApiError::forbidden());
        }
        plans.remove(&request.id).ok_or_else(ApiError::not_found)?
    };
    if pending.prepared.elapsed() > Duration::from_secs(300) {
        return Err(ApiError::conflict(
            "Comparison confirmation expired; prepare again",
        ));
    }
    let io = native_io::NativeGitIo::resume(&state, pending.proof).await?;
    let guard = io.clone();
    let head = pending.plan.head.clone();
    let id = pending.plan.id.clone();
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Git is busy; prepare again"))?;
    let mut trees = io
        .mutate(move |_| {
            let _permit = permit;
            Ok(dunlin_workspace::comparison::create(pending.plan))
        })
        .await?;
    if head.is_empty() || trees.head != head {
        return Err(ApiError::conflict(
            "Comparison baseline changed; inspect partial worktrees before any new operation; do not automatically retry",
        ));
    }
    let projects = register_comparison_projects(&state, &guard, &trees, &id).await?;
    if projects.len() != trees.paths.len() && trees.error.is_none() {
        trees.error=Some("Comparison trees created but registration was incomplete; inspect state before retrying".into());
    }
    guard.after(true).await?;
    Ok(Json(CompareCreateResponse {
        trees,
        projects,
        git_access: "requires_operator_approval",
    }))
}

async fn register_comparison_projects(
    state: &AppState,
    source: &native_io::NativeGitIo,
    trees: &dunlin_workspace::comparison::Trees,
    id: &str,
) -> Result<Vec<Project>, ApiError> {
    if trees.paths.len() > 2 {
        return Err(ApiError::conflict(
            "Comparison registration exceeds its bounded target count; inspect worktrees before retrying",
        ));
    }
    // The worker keeps its slot until filesystem and SQL registration settle,
    // including when the request future is dropped.
    let permit = state.fs_permit().map_err(|_| ApiError::conflict(
        "Comparison worktrees may already exist but registration is busy. Inspect state before a new operation; do not automatically retry."))?;
    let state = state.clone();
    let source = source.clone();
    let root = trees.root.clone();
    let paths = trees.paths.clone();
    let id = id.to_owned();
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let uncertain = || {
            ApiError::conflict(
                "Comparison worktrees may already exist but source authority changed or registration could not be verified. Inspect state before a new operation; do not automatically retry.",
            )
        };
        source.validate_blocking().map_err(|_| uncertain())?;
        let root = std::fs::canonicalize(&root).map_err(|_| uncertain())?;
        for path in &paths {
            let canonical = std::fs::canonicalize(path).map_err(|_| uncertain())?;
            if !canonical.starts_with(&root) {
                return Err(uncertain());
            }
        }
        source.validate_blocking().map_err(|_| uncertain())?;
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        let projects = paths
            .iter()
            .enumerate()
            .map(|(index, path)| {
                (
                    format!(
                        "{} · compare {} · {}",
                        source.project.name,
                        &id[..id.len().min(8)],
                        if index == 0 { "A" } else { "B" }
                    ),
                    path.clone(),
                )
            })
            .collect::<Vec<_>>();
        let proof = source.proof();
        store
            .register_native_checkout_projects(source.authority(), &proof.user, &projects, |store| {
                proof.validate_in_transaction(&state, store)
            })
            .map_err(|_: ApiError| uncertain())
    }).await.map_err(|_| ApiError::conflict(
        "Comparison registration worker stopped; inspect worktrees before a new operation; do not automatically retry."))?
}

async fn compare_changes(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareChangesRequest>,
) -> Result<Json<Vec<dunlin_workspace::task_review::Change>>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Comparison changes are busy; try again",
            )
        })?;
    let changes = io
        .read(move |project| {
            let _permit = permit;
            checkpoint_root(project, &request.checkpoint)?;
            dunlin_workspace::task_review::changes(&request.checkpoint)
        })
        .await?;
    Ok(Json(changes))
}

async fn compare_validate(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CompareValidateRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Comparison validation is busy; try again",
            )
        })?;
    io.read(move |project| {
        let _permit = permit;
        dunlin_workspace::comparison::validate_tree(&project.path, &request.head)
    })
    .await?;
    Ok(StatusCode::OK)
}

async fn remote_context(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Value>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let payload = read_project_settings(&state, &io)?;
    let settings = payload
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
    io.validate()?;
    Ok(Json(json!({
        "project": io.project,
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CheckRequest>,
) -> Result<Json<checks::Report>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::new(StatusCode::TOO_MANY_REQUESTS, "Checks are busy; try again"))?;
    let id = uuid::Uuid::now_v7().to_string();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _request_cancel = CheckCancellation(cancel.clone());
    {
        let mut entries = state
            .checks
            .lock()
            .map_err(|_| ApiError::internal("Check registry unavailable"))?;
        if entries.len() >= 64 {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many active checks",
            ));
        }
        entries.insert(
            id.clone(),
            RunningCheck {
                proof: io.proof(),
                cancel: cancel.clone(),
            },
        );
    }
    let cleanup = CheckCleanup {
        state: state.clone(),
        id: id.clone(),
        cancel: cancel.clone(),
    };
    let worker_io = io.clone();
    let worker_cancel = cancel.clone();
    let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let worker_started = started.clone();
    let mut worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let _cleanup = cleanup;
        worker_io.validate_blocking()?;
        let plan = checks::prepare(&worker_io.project.path, request.argv)
            .map_err(ApiError::bad_request)?;
        worker_io.validate_blocking()?;
        worker_started.store(true, std::sync::atomic::Ordering::Release);
        let outcome = checks::run(plan, worker_cancel);
        worker_io.after_blocking(true)?;
        let mut report = outcome.map_err(ApiError::bad_request)?;
        report.id = id;
        Ok::<_, ApiError>(report)
    });
    // This detached supervisor owns the worker lifecycle. Dropping the HTTP
    // future signals cancellation, while the worker retains its slot and cleanup.
    let supervisor = tokio::spawn(async move {
        loop {
            tokio::select! {
                result=&mut worker => {
                    let result=result.map_err(|_|ApiError::internal("Check worker stopped; inspect state before retrying"))?;
                    io.after(started.load(std::sync::atomic::Ordering::Acquire)).await?;
                    return result;
                }
                _=tokio::time::sleep(Duration::from_millis(100)) => {
                    if io.validate().await.is_err() {cancel.store(true,std::sync::atomic::Ordering::Release);}
                }
            }
        }
    });
    Ok(Json(supervisor.await.map_err(|_| {
        ApiError::internal("Check supervisor stopped; inspect state before retrying")
    })??))
}

async fn check_cancel(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CheckCancelRequest>,
) -> Result<StatusCode, ApiError> {
    {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        crate::management::authority::current_user_locked(&state, &store, &user.id, &device.id)?;
    }
    let entry = {
        let entries = state
            .checks
            .lock()
            .map_err(|_| ApiError::internal("Check registry unavailable"))?;
        entries
            .get(&request.id)
            .map(|entry| (entry.proof.clone(), entry.cancel.clone()))
    };
    if let Some((proof, cancel)) = entry {
        if proof.user != user.id || proof.device != device.id || proof.project.id != request.project
        {
            return Err(ApiError::forbidden());
        }
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectSettingsQuery>,
) -> Result<Json<ProjectSettings>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let payload = read_project_settings(&state, &io)?;
    let settings = payload
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| ApiError::internal("stored project settings are invalid"))?
        .unwrap_or_default();
    io.validate()?;
    Ok(Json(settings))
}

async fn project_settings_put(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ProjectSettingsBody>,
) -> Result<StatusCode, ApiError> {
    settings_admin(&state, &user.id, &device.id)?;
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &body.project)?;
    validate_project_environment(&body.settings.environment).map_err(ApiError::bad_request)?;
    let payload = serde_json::to_string(&body.settings)
        .map_err(|_| ApiError::bad_request("Project settings could not be serialized"))?;
    if payload.len() > 128 * 1024 {
        return Err(ApiError::bad_request("Project settings exceed 128 KiB"));
    }
    write_project_settings(&state, &io, &payload)?;
    io.after(true)?;
    settings_admin(&state,&user.id,&device.id).map_err(|_|ApiError::conflict("Administrator authority changed during settings update; inspect current settings before a new operation; do not automatically retry"))?;
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

struct UsageRequestCancellation(Arc<std::sync::atomic::AtomicBool>);
impl Drop for UsageRequestCancellation {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
}
async fn usage_with_probe<F>(
    auth: AuthUser,
    state: AppState,
    probe: F,
) -> Result<Json<Vec<ProviderUsage>>, ApiError>
where
    F: FnOnce(&dyn Fn() -> bool) -> Vec<ProviderUsage> + Send + 'static,
{
    let authority = crate::management::authority::AdminAuthority::capture(&state, &auth)?;
    let cached = read_usage_cache(&state, &authority)?;
    if let Some(cached) = cached {
        authority.validate(&state)?;
        return Ok(Json(cached));
    }
    let permit = state
        .history_reads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "Usage probes are busy; try again",
            )
        })?;
    let interrupted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let _request = UsageRequestCancellation(interrupted.clone());
    let worker_authority = authority.clone();
    let worker_state = state.clone();
    let worker = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        let allowed = || {
            !interrupted.load(std::sync::atomic::Ordering::Acquire)
                && worker_authority.validate(&worker_state).is_ok()
        };
        worker_authority.validate(&worker_state)?;
        if !allowed() {
            return Err(ApiError::conflict("Usage request was interrupted"));
        }
        let result = probe(&allowed);
        worker_authority.validate(&worker_state)?;
        if !allowed() {
            return Err(ApiError::conflict("Usage request was interrupted"));
        }
        Ok::<_, ApiError>(result)
    });
    let usages = tokio::time::timeout(Duration::from_secs(45), worker)
        .await
        .map_err(|_| {
            ApiError::new(
                StatusCode::GATEWAY_TIMEOUT,
                "Usage probe timed out; no account data was returned",
            )
        })?
        .map_err(|_| ApiError::internal("Usage probe worker stopped"))??;
    authority.validate(&state)?;
    publish_usage_cache(&state, &authority, &usages)?;
    authority.validate(&state)?;
    Ok(Json(usages))
}
fn read_usage_cache(
    state: &AppState,
    authority: &crate::management::authority::AdminAuthority,
) -> Result<Option<Vec<ProviderUsage>>, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    authority.validate_locked(state, &store)?;
    let cached = state
        .usage_cache
        .lock()
        .map_err(|_| ApiError::internal("Usage cache unavailable"))?;
    Ok(cached
        .as_ref()
        .filter(|(at, _)| at.elapsed() < Duration::from_secs(60))
        .map(|(_, usages)| usages.clone()))
}
fn publish_usage_cache(
    state: &AppState,
    authority: &crate::management::authority::AdminAuthority,
    usages: &[ProviderUsage],
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    authority.validate_locked(state, &store)?;
    *state
        .usage_cache
        .lock()
        .map_err(|_| ApiError::internal("Usage cache unavailable"))? =
        Some((Instant::now(), usages.to_vec()));
    Ok(())
}
/// Own-device billing submission is a different authority from administrator
/// aggregation. This proof is state-free and never trusts the cached role.
#[derive(Clone)]
struct BillingDeviceAuthority {
    user: String,
    device: String,
}
impl BillingDeviceAuthority {
    fn capture(state: &AppState, auth: &AuthUser) -> Result<Self, ApiError> {
        let proof = Self {
            user: auth.user.id.clone(),
            device: auth.device.id.clone(),
        };
        proof.validate(state)?;
        Ok(proof)
    }
    fn validate_locked(&self, state: &AppState, store: &Store) -> Result<(), ApiError> {
        crate::management::authority::current_user_locked(state, store, &self.user, &self.device)?;
        Ok(())
    }
    fn validate(&self, state: &AppState) -> Result<(), ApiError> {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        self.validate_locked(state, &store)
    }
}
fn save_billing_payload(
    state: &AppState,
    proof: &BillingDeviceAuthority,
    payload: &str,
) -> Result<(), ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    proof.validate_locked(state, &store)?;
    store.save_billing_evidence(&proof.user, &proof.device, payload)?;
    Ok(())
}
fn aggregate_billing_rows(
    state: &AppState,
    proof: &crate::management::authority::AdminAuthority,
) -> Result<Vec<crate::store::BillingEvidenceRow>, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    proof.validate_locked(state, &store)?;
    Ok(store.billing_evidence(None)?)
}

async fn usage(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<ProviderUsage>>, ApiError> {
    usage_with_probe(auth, state, guarded_host_usage).await
}
fn guarded_host_usage(authorized: &dyn Fn() -> bool) -> Vec<ProviderUsage> {
    dunlin_usage::probe_all_guarded(authorized)
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
    auth: AuthUser,
    State(state): State<AppState>,
    Json(evidence): Json<dunlin_usage::billing::Evidence>,
) -> Result<StatusCode, ApiError> {
    // Submission is deliberately available to every authenticated human device;
    // administrator authority is required only for the cross-device aggregate.
    let proof = BillingDeviceAuthority::capture(&state, &auth)?;
    evidence.validate().map_err(ApiError::bad_request)?;
    let payload = serde_json::to_string(&evidence)
        .map_err(|_| ApiError::bad_request("Billing evidence could not be serialized"))?;
    save_billing_payload(&state, &proof, &payload)?;
    proof.validate(&state).map_err(|_|ApiError::conflict("Device authority changed during billing evidence submission; inspect synced evidence before a new operation; do not automatically retry"))?;
    Ok(StatusCode::OK)
}

async fn billing_evidence_get(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<BillingEvidenceResponse>>, ApiError> {
    let proof = crate::management::authority::AdminAuthority::capture(&state, &auth)?;
    let rows = aggregate_billing_rows(&state, &proof)?;
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
    proof.validate(&state)?;
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

async fn sessions(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Vec<SessionInfo>>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let rows = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        io.validate_in_store(&store)?;
        store.list_sessions(&user.id, &io.project.id)?
    };
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
    io.validate()?;
    Ok(Json(sessions))
}

async fn delete_session(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let io = owned_session(&state, &user.id, &device.id, &id)?;
    with_owned_session(&state, &io, &id, |store| {
        // Lock order is Store -> leases; no lease holder acquires Store.
        let mut leases = state
            .leases
            .lock()
            .map_err(|_| ApiError::internal("Lease store unavailable"))?;
        store.delete_session(&id)?;
        if let Some(lease) = leases.remove(&id) {
            let _ = lease.close_tx.try_send(());
        }
        Ok(())
    })?;
    io.after(true)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn takeover(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let io = owned_session(&state, &user.id, &device.id, &id)?;
    let changed = with_owned_session(&state, &io, &id, |_| {
        if let Some(lease) = state
            .leases
            .lock()
            .map_err(|_| ApiError::internal("Lease store unavailable"))?
            .remove(&id)
        {
            let _ = lease.close_tx.try_send(());
            Ok(true)
        } else {
            Ok(false)
        }
    })?;
    io.after(changed)?;
    Ok(StatusCode::OK)
}

fn owned_session(
    state: &AppState,
    user: &str,
    device: &str,
    id: &str,
) -> Result<native_io::NativeIo, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    let owner = store.session_owner(id)?.ok_or_else(ApiError::not_found)?;
    if owner.user_id != user {
        return Err(ApiError::not_found());
    }
    native_io::NativeIo::capture_in_store(state, &store, user, device, &owner.project_id)
}
fn with_owned_session<T>(
    state: &AppState,
    io: &native_io::NativeIo,
    id: &str,
    operation: impl FnOnce(&Store) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    let store = state
        .store
        .lock()
        .map_err(|_| ApiError::internal("Store unavailable"))?;
    io.validate_in_store(&store)?;
    let owner = store.session_owner(id)?.ok_or_else(ApiError::not_found)?;
    if owner.user_id != io.proof().user || owner.project_id != io.project.id {
        return Err(ApiError::not_found());
    }
    operation(&store)
}

#[derive(Deserialize)]
struct TranscriptRequest {
    payload: Value,
    #[serde(default)]
    expected_revision: Option<i64>,
}

async fn session_transcript_revision(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let io = owned_session(&state, &user.id, &device.id, &id)?;
    let revision =
        with_owned_session(
            &state,
            &io,
            &id,
            |store| Ok(store.transcript_revision(&id)?),
        )?;
    with_owned_session(&state, &io, &id, |_| Ok(()))?;
    Ok(Json(json!({"revision":revision})))
}

async fn session_transcript_save(
    AuthUser { user, device }: AuthUser,
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
    let io = owned_session(&state, &user.id, &device.id, &id)?;
    let payload = serde_json::to_string(&request.payload)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if payload.len() > 4 * 1024 * 1024 {
        return Err(ApiError::bad_request("Transcript is larger than 4 MiB"));
    }
    let saved = with_owned_session(&state, &io, &id, |store| {
        Ok(match request.expected_revision {
            Some(revision) => store.save_transcript_if_revision(&id, &payload, revision),
            None => store.save_transcript(&id, &payload),
        }?)
    })?;
    if !saved {
        return Err(ApiError::conflict(
            "The relay has a newer transcript revision; the stale snapshot was not saved",
        ));
    }
    io.after(true)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn session_transcript(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let io = owned_session(&state, &user.id, &device.id, &id)?;
    let payload = with_owned_session(&state, &io, &id, |store| {
        store.load_transcript(&id)?.ok_or_else(ApiError::not_found)
    })?;
    let value: Value = serde_json::from_str(&payload)
        .map_err(|error| ApiError::internal(format!("Stored transcript is invalid: {error}")))?;
    with_owned_session(&state, &io, &id, |_| Ok(()))?;
    Ok(Json(value))
}

async fn acp_ws(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    AuthUser { user, device }: AuthUser,
    Query(query): Query<AcpQuery>,
) -> Result<Response, ApiError> {
    // Pin every root's native authority in the same Store cut as its path.
    let (project, roots, native_grants) = {
        let store = state
            .store
            .lock()
            .map_err(|_| ApiError::internal("Store unavailable"))?;
        let project = store
            .project_for_user(&query.project, &user.id)?
            .ok_or_else(ApiError::forbidden)?;
        let mut roots = Vec::new();
        let mut grants = vec![store.capture_native_grant(&project.id, &user.id)?];
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
                let root = store
                    .project_for_user(id, &user.id)?
                    .ok_or_else(ApiError::forbidden)?;
                if root.id != project.id && !roots.iter().any(|entry: &Project| entry.id == root.id)
                {
                    grants.push(store.capture_native_grant(&root.id, &user.id)?);
                    roots.push(root);
                }
                if roots.len() > 31 {
                    return Err(ApiError::bad_request(
                        "A relay group workspace supports at most 32 roots",
                    ));
                }
            }
        }
        (project, roots, grants)
    };
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
            native_grants,
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<FileTree>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let permit = state.fs_permit()?;
    let tree = io
        .read(move |project| {
            let _permit = permit;
            Ok(FileTree::load(&project.path))
        })
        .await?;
    Ok(Json(tree))
}

async fn fs_storage(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<Option<u64>>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let permit = state.fs_permit()?;
    Ok(Json(
        io.read(move |project| {
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<workspace_fs::FileContents>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let permit = state.fs_permit()?;
    let file = io
        .read(move |project| {
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<AuditRunsQuery>,
) -> Result<Json<Vec<AuditRunInfo>>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let permit = state.fs_permit()?;
    let runs = io
        .read(move |project| {
            let _permit = permit;
            let root = FsPath::new(&project.path).join(".dunlin/audits");
            let Ok(read) = std::fs::read_dir(&root) else {
                return Ok(Vec::new());
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
            Ok(runs)
        })
        .await?;
    Ok(Json(runs))
}

async fn audit_file(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<AuditFileQuery>,
) -> Result<Json<workspace_fs::FileContents>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    if !safe_audit_run(&query.run) || !safe_audit_file(&query.name) {
        return Err(ApiError::bad_request("Invalid audit artifact path"));
    }
    let permit = state.fs_permit()?;
    let relative = format!(".dunlin/audits/{}/{}", query.run, query.name);
    let file = io
        .read(move |project| {
            let _permit = permit;
            workspace_fs::read_file(&project.path, &relative)
        })
        .await?;
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<WriteFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    let stat = io
        .mutate(move |project| {
            workspace_fs::write_file_checked(
                &project.path,
                &request.path,
                &request.contents,
                request.expected_mtime_ms,
            )
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    let stat = io
        .mutate(move |project| workspace_fs::create_file(&project.path, &request.path))
        .await?;
    Ok(Json(stat))
}

async fn fs_mkdir(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    Ok(Json(
        io.mutate(move |project| {
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<RenameRequest>,
) -> Result<Json<dunlin_workspace::transfer::Transferred>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    Ok(Json(
        io.mutate(move |project| {
            dunlin_workspace::transfer::rename_entry(&project.path, &request.path, &request.name)
        })
        .await?,
    ))
}
async fn fs_delete(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CreateFileRequest>,
) -> Result<Json<Value>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    io.mutate(move |project| {
        dunlin_workspace::transfer::delete_entry(&project.path, &request.path)
    })
    .await?;
    Ok(Json(json!({"ok": true})))
}
async fn fs_preview(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<Value>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let permit = state.fs_permit()?;
    let guard = io.clone();
    let bytes = io
        .read(move |project| {
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
    guard.validate()?;
    Ok(Json(json!({"contents": encoded})))
}

#[derive(Deserialize)]
struct MoveRequest {
    project: String,
    sources: Vec<String>,
    target: String,
}

async fn fs_move(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<MoveRequest>,
) -> Result<Json<dunlin_workspace::transfer::TransferReport>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    if request.sources.len() > 1000 {
        return Err(ApiError::bad_request("Too many paths"));
    }
    let report = io
        .mutate(move |project| {
            dunlin_workspace::transfer::move_entries(
                &project.path,
                &request.sources,
                &request.target,
            )
        })
        .await?;
    Ok(Json(report))
}

async fn git_init(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<Json<Value>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.initialize(move |project| git::initialize(&project.path))
        .await?;
    Ok(Json(json!({"ok": true})))
}

async fn fs_stat(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<FileQuery>,
) -> Result<Json<workspace_fs::FileStat>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    let stat = io
        .read(move |project| workspace_fs::stat_file(&project.path, &query.path))
        .await?;
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
fn safe_project_directory(
    root: &str,
    relative: &str,
    guard: &native_io::NativeIo,
) -> Result<PathBuf, ApiError> {
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
                guard.validate()?;
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

/// Store a locally attached file inside the project so the remote agent can
/// read it. Attachments are namespaced under `.dunlin/attachments`.
async fn fs_upload(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<UploadRequest>,
) -> Result<Json<UploadResponse>, ApiError> {
    const MAX_UPLOAD_BYTES: usize = 8 * 1024 * 1024;
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &request.project)?;
    let bytes = STANDARD
        .decode(request.contents.as_bytes())
        .map_err(|_| ApiError::bad_request("Attachment is not valid base64"))?;
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(ApiError::bad_request("Attachment is larger than 8 MiB"));
    }
    // This optional repository convenience never grants metadata access to the
    // workspace upload. Unapproved metadata simply receives no ignore entry.
    let git = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await
    .ok();
    let name = sanitize_attachment_name(&request.name);
    let relative = format!(".dunlin/attachments/{}-{name}", uuid::Uuid::now_v7());
    let guard = io.clone();
    let relative = io
        .mutate(move |project| {
            let directory = safe_project_directory(&project.path, ".dunlin/attachments", &guard)
                .map_err(|error| {
                    if error.status.is_server_error() {
                        format!("internal:{}", error.message)
                    } else {
                        error.message
                    }
                })?;
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
                return Err("quota:Attachment quota reached; remove old attachments first".into());
            }
            let full = directory.join(
                FsPath::new(&relative)
                    .file_name()
                    .ok_or("Invalid attachment name")?,
            );
            guard
                .validate()
                .map_err(|error| format!("conflict:{}", error.message))?;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&full)
                .map_err(|error| format!("internal:Could not store attachment: {error}"))?;
            use std::io::Write as _;
            file.write_all(&bytes)
                .map_err(|error| format!("internal:Could not store attachment: {error}"))?;
            if let Some(git) = git {
                exclude_attachments_from_git(&git)?;
            }
            Ok(relative)
        })
        .await?;
    Ok(Json(UploadResponse { path: relative }))
}

/// Keep uploaded attachments out of the user's `git status` without touching a
/// tracked `.gitignore`.
fn exclude_attachments_from_git(guard: &native_io::NativeGitIo) -> Result<(), String> {
    use std::io::{Read as _, Write as _};
    let conflict = |error: ApiError| format!("conflict:{}", error.message);
    guard.validate_blocking().map_err(conflict)?;
    // The authority opens one exact approved Git metadata file using directory
    // descriptors. No Git subprocess or returned path expands browser FS scope.
    let Some(mut file) = guard
        .authority()
        .open_git_exclude()
        .map_err(|_| "conflict:Git exclude authority could not be verified".to_string())?
    else {
        return Ok(());
    };
    let mut existing = Vec::new();
    (&mut file)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut existing)
        .map_err(|_| {
            "conflict:Git exclude read failed; inspect attachment before retrying".to_string()
        })?;
    if existing.len() > 1024 * 1024 {
        return Err(
            "conflict:Git exclude exceeds its bounded read; inspect attachment before retrying"
                .into(),
        );
    }
    if !String::from_utf8_lossy(&existing)
        .lines()
        .any(|line| line.trim() == ".dunlin/")
    {
        guard.validate_blocking().map_err(conflict)?;
        let line = if existing.is_empty() || existing.ends_with(b"\n") {
            b".dunlin/\n".as_slice()
        } else {
            b"\n.dunlin/\n".as_slice()
        };
        if existing.len().saturating_add(line.len()) > 1024 * 1024 {
            return Err("conflict:Git exclude has no bounded append capacity; inspect attachment before retrying".into());
        }
        file.write_all(line).map_err(|_| {
            "conflict:Git exclude append may be partial; inspect attachment before retrying"
                .to_string()
        })?;
    }
    guard.after_blocking(true).map_err(conflict)
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<search::SearchHit>>, ApiError> {
    let io = native_io::NativeIo::capture(&state, &user.id, &device.id, &query.project)?;
    if query.q.len() > 512 {
        return Err(ApiError::bad_request(
            "Search query is limited to 512 bytes",
        ));
    }
    let limit = query.limit.unwrap_or(80).clamp(1, 500);
    let permit = state.fs_permit()?;
    let hits = io
        .read(move |project| {
            let _permit = permit;
            Ok(search::search_project(&project.path, &query.q, limit))
        })
        .await?;
    Ok(Json(hits))
}

async fn git_status(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<git::GitStatus>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &query.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let status = io.read(move |project| Ok(git::load(&project.path))).await?;
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<PatchResponse>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &query.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let staged = query.staged.unwrap_or(false);
    let patch = io
        .read(move |project| git::file_diff(&project.path, &query.file, staged))
        .await?;
    Ok(Json(PatchResponse { patch }))
}

#[derive(Deserialize)]
struct StageRequest {
    project: String,
    file: Option<String>,
}

async fn git_stage(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<StageRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| {
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<FileRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| git::unstage_file(&project.path, &request.file))
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct CommitRequest {
    project: String,
    message: String,
}

async fn git_commit(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    if request.message.len() > 1024 * 1024 {
        return Err(ApiError::bad_request("Commit message is too large"));
    }
    io.mutate(move |project| git::commit(&project.path, &request.message))
        .await?;
    Ok(StatusCode::OK)
}

async fn git_amend(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<CommitRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    if request.message.len() > 1024 * 1024 {
        return Err(ApiError::bad_request("Commit message is too large"));
    }
    io.mutate(move |project| git::amend(&project.path, &request.message))
        .await?;
    Ok(StatusCode::OK)
}

async fn git_discard(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<FileRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| git::discard_file(&project.path, &request.file))
        .await?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
struct ProjectRequest {
    project: String,
}

async fn git_pull(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| git::pull(&project.path)).await?;
    Ok(StatusCode::OK)
}

async fn git_push(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<ProjectRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| git::push(&project.path)).await?;
    Ok(StatusCode::OK)
}

#[derive(Serialize)]
struct BranchesResponse {
    branches: Vec<String>,
}

async fn git_branches(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<BranchesResponse>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &query.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    let branches = io.read(move |project| git::branches(&project.path)).await?;
    Ok(Json(BranchesResponse { branches }))
}

#[derive(Deserialize)]
struct BranchRequest {
    project: String,
    branch: String,
}

async fn git_branch_context(
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<ProjectQuery>,
) -> Result<Json<git::BranchContext>, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &query.project,
        crate::store::local_metadata::MetadataPermission::Read,
    )
    .await?;
    Ok(Json(
        io.read(move |project| git::branch_context(&project.path))
            .await?,
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<SafeBranchRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| {
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
    AuthUser { user, device }: AuthUser,
    State(state): State<AppState>,
    Json(request): Json<BranchRequest>,
) -> Result<StatusCode, ApiError> {
    let io = native_io::NativeGitIo::capture(
        &state,
        &user.id,
        &device.id,
        &request.project,
        crate::store::local_metadata::MetadataPermission::Write,
    )
    .await?;
    io.mutate(move |project| git::switch_branch(&project.path, &request.branch))
        .await?;
    Ok(StatusCode::OK)
}

#[cfg(test)]
mod tests;
