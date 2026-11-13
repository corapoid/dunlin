//! Server-owned ACP processes. Browser observers do not own child lifetimes.
use crate::api::{AppState, AuthUser};
use dunlin_domain::{
    ProjectSettings,
    collaboration::{AccessContext, Permission},
};
use dunlin_session_runtime::{HarnessInfo, Journal, Lease, Session, TurnPhase};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process::{Child, ChildStdin, ChildStdout},
    sync::{Semaphore, broadcast},
    time::Instant,
};

type Result<T> = std::result::Result<T, &'static str>;
pub(crate) mod mcp;
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Options {
    pub model: String,
    pub mode: String,
    pub max_seconds: u64,
    pub max_output_bytes: usize,
}
pub(crate) type ProfileRef = dunlin_domain::agent::AgentProfileRef;
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacyCreateRequest {
    pub model: String,
    pub mode: String,
    pub max_seconds: u64,
    pub max_output_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<dunlin_domain::binding::ProjectTargetRef>,
}
impl LegacyCreateRequest {
    fn options(&self) -> Options {
        Options {
            model: self.model.clone(),
            mode: self.mode.clone(),
            max_seconds: self.max_seconds,
            max_output_bytes: self.max_output_bytes,
        }
    }
}
#[cfg(test)]
impl From<Options> for LegacyCreateRequest {
    fn from(options: Options) -> Self {
        Self {
            model: options.model,
            mode: options.mode,
            max_seconds: options.max_seconds,
            max_output_bytes: options.max_output_bytes,
            target: None,
        }
    }
}
#[derive(Clone, Deserialize, serde::Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub(crate) enum CreateRequest {
    Profile {
        profile: ProfileRef,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        mcp_connections: Vec<mcp::Selection>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<dunlin_domain::binding::ProjectTargetRef>,
    },
    Legacy(LegacyCreateRequest),
}
impl CreateRequest {
    fn target(&self) -> Option<&dunlin_domain::binding::ProjectTargetRef> {
        match self {
            Self::Profile { target, .. } => target.as_ref(),
            Self::Legacy(request) => request.target.as_ref(),
        }
    }
    pub(crate) fn validate_target(&self) -> Result<()> {
        if let Some(target) = self.target() {
            target.validate().map_err(|_| "invalid_project_target")?;
            if matches!(
                target,
                dunlin_domain::binding::ProjectTargetRef::Remote { .. }
            ) {
                return Err("remote_execution_not_supported");
            }
        }
        Ok(())
    }
}
/// A profile may select only the administrator's current fixed harness. Custom
/// commands remain available to the legacy API, never inferred as catalog IDs.
pub(crate) fn configured_harness_id(config: &crate::config::Config) -> Option<&'static str> {
    harness_id_for_command(&crate::management::harness(config))
}
fn harness_id_for_command(command: &str) -> Option<&'static str> {
    dunlin_workspace::server::HARNESSES
        .iter()
        .find(|entry| entry.2 == command)
        .map(|entry| entry.0)
}
#[derive(Debug)]
pub(crate) struct CreateAcceptance {
    pub session: Session,
    pub replayed: bool,
}
pub(crate) fn valid_admission_key(key: &str) -> bool {
    (16..=128).contains(&key.len())
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}
fn admission_error(error: dunlin_session_runtime::Error) -> &'static str {
    match error {
        dunlin_session_runtime::Error::Conflict("Invalid session admission identity") => {
            "invalid_idempotency_key"
        }
        dunlin_session_runtime::Error::Conflict(_) => "idempotency_conflict",
        dunlin_session_runtime::Error::Policy(_) => "admission_authority_denied",
        dunlin_session_runtime::Error::Limit(_) => "session_admission_capacity_reached",
        _ => "session_admission_failed",
    }
}
struct CreateScope {
    authority: AccessContext,
    user: String,
    native: crate::store::Project,
    binding: u64,
    binding_snapshot: crate::store::Binding,
    fingerprint_identity: Value,
    harness: String,
    fingerprint: String,
    options: Options,
    profile: Option<ProfileRef>,
    instructions: String,
    mcp: Vec<mcp::BoundConnection>,
}
fn fresh_create_scope(
    state: &AppState,
    store: &crate::store::Store,
    auth: &AuthUser,
    workspace: &str,
    project: &str,
    request: &CreateRequest,
) -> Result<CreateScope> {
    request.validate_target()?;
    let current = store
        .active_device_user(&auth.device.id)
        .map_err(|_| "authority_unavailable")?
        .filter(|user| user.id == auth.user.id)
        .ok_or("authority_revoked")?;
    crate::api::check_host_account_policy(state, &current).map_err(|_| "host_policy_changed")?;
    if !store
        .oidc_device_allowed(&auth.device.id, state.config.oidc.as_ref())
        .map_err(|_| "authority_unavailable")?
    {
        return Err("identity_revoked");
    }
    let (binding, native) = store
        .local_binding_candidate(&current.id, workspace, project, Permission::CreateSession)
        .map_err(|_| "checkout_or_grant_unavailable")?
        .ok_or("binding_missing")?;
    if let Some(dunlin_domain::binding::ProjectTargetRef::Local { revision }) = request.target()
        && *revision != binding.revision
    {
        return Err("project_target_changed");
    }
    let native_grant = store
        .capture_native_grant(&native.id, &current.id)
        .map_err(|_| "native_grant_unavailable")?;
    let authority = store
        .team_project_context(workspace, project, &current.id)
        .map_err(|_| "authority_unavailable")?;
    authority
        .authorize_project(workspace, project, Permission::ViewProject)
        .map_err(|_| "view_grant_required")?;
    let stamp = authority
        .authorize_project(workspace, project, Permission::CreateSession)
        .map_err(|_| "create_grant_required")?;
    let harness = crate::management::harness(&state.config);
    let (options, profile, instructions, mcp) = match request {
        CreateRequest::Legacy(request) => (request.options(), None, String::new(), Vec::new()),
        CreateRequest::Profile {
            profile,
            mcp_connections,
            ..
        } => {
            if profile.agent_id.is_empty()
                || profile.agent_id.len() > 128
                || !profile
                    .agent_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
                || profile.version == 0
                || profile.version > i64::MAX as u64
                || profile.digest.len() != 64
                || !profile
                    .digest
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            {
                return Err("invalid_agent_profile");
            }
            let version = store
                .agent_version(
                    &current.id,
                    workspace,
                    project,
                    &profile.agent_id,
                    profile.version,
                )
                .map_err(|_| "agent_profile_unavailable")?
                .ok_or("agent_profile_unavailable")?;
            if version.digest != profile.digest {
                return Err("agent_profile_changed");
            }
            version
                .definition
                .validate()
                .map_err(|_| "invalid_agent_profile")?;
            let mcp = mcp::bind(
                state,
                store,
                &current.id,
                workspace,
                &version.definition,
                mcp_connections,
            )?;
            if harness_id_for_command(&harness) != Some(version.definition.harness.as_str()) {
                return Err("agent_profile_harness_unavailable");
            }
            let definition = version.definition;
            let instructions = definition.render_instructions();
            (
                Options {
                    model: definition.model,
                    mode: definition.mode,
                    max_seconds: definition.max_seconds,
                    max_output_bytes: definition.max_output_bytes,
                },
                Some(profile.clone()),
                instructions,
                mcp,
            )
        }
    };
    if options.model.is_empty()
        || options.model.len() > 256
        || options.mode.is_empty()
        || options.mode.len() > 128
        || !(1..=1800).contains(&options.max_seconds)
        || !(1024..=4 * 1024 * 1024).contains(&options.max_output_bytes)
    {
        return Err("invalid_runtime_budget_or_profile");
    }
    let mut request_identity = json!({
        "workspace": workspace,
        "project": project,
        "options": options,
        "authority": stamp,
        "binding": binding,
        "nativeGrant": native_grant,
        "nativePath": native.path,
        "harness": harness,
    });
    // Omitted/null targets preserve historical fingerprints byte-for-byte.
    // An explicit local pin is a distinct intent even when it resolves here.
    if let Some(target) = request.target() {
        request_identity["target"] = json!(target);
    }
    if let Some(profile) = &profile {
        request_identity["profile"] = json!(profile);
        request_identity["instructions"] = json!(instructions);
    }
    if !mcp.is_empty() {
        request_identity["mcpConnections"] = json!(
            mcp.iter()
                .map(mcp::BoundConnection::identity)
                .collect::<Vec<_>>()
        );
    }
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&request_identity).map_err(|_| "admission_fingerprint_failed")?,
    ));
    Ok(CreateScope {
        authority,
        user: current.id,
        native,
        binding: binding.revision,
        binding_snapshot: binding,
        fingerprint_identity: request_identity,
        harness,
        fingerprint,
        options,
        profile,
        instructions,
        mcp,
    })
}
/// The filesystem part is captured exactly once per explicit admission attempt.
/// It must never be reconstructed from a historical acceptance receipt.
struct PreparedCreate {
    scope: CreateScope,
    checkout: crate::store::native_checkout::NativeCheckoutAuthority,
    base_fingerprint: String,
}
struct CreateCutError(&'static str);
impl From<crate::store::StoreError> for CreateCutError {
    fn from(_: crate::store::StoreError) -> Self {
        Self("checkout_or_grant_changed")
    }
}
fn authority_cut<T>(
    store: &crate::store::Store,
    operation: impl FnOnce(&crate::store::Store) -> Result<T>,
) -> Result<T> {
    store
        .native_checkout_transaction(|store| operation(store).map_err(CreateCutError))
        .map_err(|error: CreateCutError| error.0)
}
fn prepare_create_scope(
    state: &AppState,
    auth: &AuthUser,
    workspace: &str,
    project: &str,
    request: &CreateRequest,
) -> Result<PreparedCreate> {
    let _fs_slot = state
        .interactive
        .fs_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| "checkout_validation_busy")?;
    let (mut scope, candidate) = {
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        let scope = fresh_create_scope(state, &store, auth, workspace, project, request)?;
        let candidate = store
            .native_checkout_candidate(
                &scope.native.id,
                &scope.user,
                crate::store::local_metadata::MetadataPermission::Write,
            )
            .map_err(|_| "checkout_or_grant_unavailable")?;
        (scope, candidate)
    };
    let validated = candidate
        .validate_filesystem()
        .map_err(|_| "checkout_identity_changed")?;
    let checkout = {
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        store
            .authorize_native_checkout(validated)
            .map_err(|_| "checkout_or_grant_changed")?
    };
    if scope
        .binding_snapshot
        .validate_native_checkout(&scope.native, &checkout)
        .map_err(|_| "checkout_identity_changed")?
        != checkout.root()
    {
        return Err("binding_changed");
    }
    let base_fingerprint = scope.fingerprint.clone();
    // The old local resolver used the canonical path in admission identities.
    // Keep that contract, including a native project registered via a symlink.
    // The raw SQL path remains in base_fingerprint for the final comparison.
    scope.fingerprint_identity["nativePath"] = json!(checkout.root());
    if let Some(metadata) = checkout.metadata_stamp() {
        scope.fingerprint_identity["nativeMetadata"] = json!(metadata);
    }
    scope.fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&scope.fingerprint_identity)
            .map_err(|_| "admission_fingerprint_failed")?,
    ));
    Ok(PreparedCreate {
        scope,
        checkout,
        base_fingerprint,
    })
}
/// SQL only; the caller owns a fresh IMMEDIATE transaction. No filesystem work
/// or notification may run between this cut and durable admission.
fn revalidate_create_scope(
    state: &AppState,
    store: &crate::store::Store,
    auth: &AuthUser,
    workspace: &str,
    project: &str,
    request: &CreateRequest,
    prepared: &PreparedCreate,
) -> std::result::Result<CreateScope, CreateCutError> {
    let fresh = fresh_create_scope(state, store, auth, workspace, project, request)
        .map_err(CreateCutError)?;
    if fresh.fingerprint != prepared.base_fingerprint
        || fresh.binding_snapshot != prepared.scope.binding_snapshot
    {
        return Err(CreateCutError("idempotency_conflict"));
    }
    store.revalidate_native_checkout_in_transaction(&prepared.checkout)?;
    Ok(fresh)
}
#[allow(clippy::too_many_arguments)]
fn lookup_prepared_receipt(
    state: &AppState,
    auth: &AuthUser,
    workspace: &str,
    project: &str,
    request: &CreateRequest,
    key: Option<&str>,
    prepared: &PreparedCreate,
) -> Result<Option<CreateAcceptance>> {
    let Some(key) = key else { return Ok(None) };
    let store = state.store.lock().map_err(|_| "store_unavailable")?;
    store
        .native_checkout_transaction(|store| {
            let fresh =
                revalidate_create_scope(state, store, auth, workspace, project, request, prepared)?;
            store
                .interactive_journal()
                .admission_lookup(
                    &fresh.authority,
                    &auth.device.id,
                    key,
                    &prepared.scope.fingerprint,
                )
                .map(|session| {
                    session.map(|session| CreateAcceptance {
                        session,
                        replayed: true,
                    })
                })
                .map_err(|e| CreateCutError(admission_error(e)))
        })
        .map_err(|e: CreateCutError| e.0)
}
struct Admission {
    user: String,
    device: String,
    workspace: String,
    project: String,
    checkout: crate::store::native_checkout::NativeCheckoutAuthority,
    binding_snapshot: crate::store::Binding,
    lease: Lease,
    harness: String,
}
struct ActiveTurn {
    id: String,
    started: Instant,
}
struct Handle {
    stop: Arc<AtomicBool>,
    events: broadcast::Sender<()>,
    abort: Mutex<Option<tokio::task::AbortHandle>>,
    admission: Mutex<Option<Admission>>,
    active_turn: Mutex<Option<ActiveTurn>>,
}
pub(crate) struct Service {
    actors: Mutex<HashMap<String, Arc<Handle>>>,
    shutting_down: AtomicBool,
    slots: Arc<Semaphore>,
    fs_slots: Arc<Semaphore>,
    #[cfg(test)]
    pub command: Option<(String, Vec<String>)>,
}
impl Default for Service {
    fn default() -> Self {
        Self {
            actors: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
            slots: Arc::new(Semaphore::new(8)),
            fs_slots: Arc::new(Semaphore::new(4)),
            #[cfg(test)]
            command: None,
        }
    }
}
#[derive(Clone)]
struct Task {
    id: String,
    user: String,
    device: String,
    workspace: String,
    project: String,
    native: String,
    checkout: crate::store::native_checkout::NativeCheckoutAuthority,
    binding_snapshot: crate::store::Binding,
    root: String,
    binding: u64,
    harness: String,
    lease: Lease,
    options: Options,
    profile: Option<ProfileRef>,
    instructions: String,
    mcp: Vec<mcp::BoundConnection>,
    handle: Arc<Handle>,
}
fn context(state: &AppState, store: &crate::store::Store, task: &Task) -> Result<AccessContext> {
    authority_cut(store, |store| context_in_transaction(state, store, task))
}
fn context_in_transaction(
    state: &AppState,
    store: &crate::store::Store,
    task: &Task,
) -> Result<AccessContext> {
    for connection in &task.mcp {
        connection.validate(state, store)?;
    }
    store
        .revalidate_native_checkout_in_transaction(&task.checkout)
        .map_err(|_| "checkout_or_grant_changed")?;
    let user = store
        .active_device_user(&task.device)
        .map_err(|_| "authority_unavailable")?
        .filter(|u| u.id == task.user)
        .ok_or("authority_revoked")?;
    crate::api::check_host_account_policy(state, &user).map_err(|_| "host_policy_changed")?;
    if !store
        .oidc_device_allowed(&task.device, state.config.oidc.as_ref())
        .map_err(|_| "authority_unavailable")?
    {
        return Err("identity_revoked");
    }
    let binding = store
        .local_binding_for_user(&task.user, &task.workspace, &task.project)
        .map_err(|_| "grant_revoked")?
        .ok_or("binding_removed")?;
    if binding != task.binding_snapshot {
        return Err("binding_changed");
    }
    store
        .team_project_context(&task.workspace, &task.project, &task.user)
        .map_err(|_| "authority_revoked")
}
fn journal<T>(
    state: &AppState,
    task: &Task,
    action: impl FnOnce(&Journal<'_>, &AccessContext) -> dunlin_session_runtime::Result<T>,
) -> Result<T> {
    let store = state.store.lock().map_err(|_| "store_unavailable")?;
    let context = context(state, &store, task)?;
    let journal = store.interactive_journal();
    let before = journal
        .sequence(&context, &task.id)
        .map_err(|_| "journal_unavailable")?;
    let result = action(&journal, &context).map_err(|_| "journal_or_authority_rejected")?;
    if journal
        .sequence(&context, &task.id)
        .map_err(|_| "journal_unavailable")?
        != before
    {
        let _ = task.handle.events.send(());
    }
    Ok(result)
}
fn guard_sync(state: &AppState, task: &Task, deadline: Instant) -> Result<()> {
    if task.handle.stop.load(Ordering::Acquire) {
        return Err("stop_requested");
    }
    if Instant::now() >= deadline {
        return Err("time_budget_exceeded");
    }
    if crate::management::harness(&state.config) != task.harness {
        return Err("harness_changed");
    }
    let (binding, native) = {
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        context(state, &store, task)?;
        store
            .local_binding_candidate(
                &task.user,
                &task.workspace,
                &task.project,
                Permission::CreateSession,
            )
            .map_err(|_| "checkout_or_policy_changed")?
            .ok_or("binding_removed")?
    };
    task.checkout
        .validate_filesystem()
        .map_err(|_| "checkout_identity_changed")?;
    if binding != task.binding_snapshot
        || binding
            .validate_native_checkout(&native, &task.checkout)
            .map_err(|_| "checkout_identity_changed")?
            != task.root
    {
        return Err("binding_changed");
    }
    {
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        context(state, &store, task)?;
        let (current, current_native) = store
            .local_binding_candidate(
                &task.user,
                &task.workspace,
                &task.project,
                Permission::CreateSession,
            )
            .map_err(|_| "checkout_or_policy_changed")?
            .ok_or("binding_removed")?;
        if current != task.binding_snapshot || current_native.path != native.path {
            return Err("binding_changed");
        }
    }

    Ok(())
}
async fn guard(state: &AppState, task: &Task, deadline: Instant) -> Result<()> {
    let acquire = state.interactive.fs_slots.clone().acquire_owned();
    tokio::pin!(acquire);
    let permit = loop {
        tokio::select! {permit=&mut acquire=>break permit.map_err(|_|"runtime_stopped")?,_=tokio::time::sleep(Duration::from_millis(200))=>{
         if task.handle.stop.load(Ordering::Acquire)||Instant::now()>=deadline{return Err("runtime_stopped_or_timed_out");}
        }}
    };
    let owned_state = state.clone();
    let owned_task = task.clone();
    let mut job = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        guard_sync(&owned_state, &owned_task, deadline)
    });
    loop {
        tokio::select! {result=&mut job=>return result.map_err(|_|"guard_unavailable")?,_=tokio::time::sleep(Duration::from_millis(200))=>{
         if task.handle.stop.load(Ordering::Acquire)||Instant::now()>=deadline{return Err("runtime_stopped_or_timed_out");}
        }}
    }
}
struct Exit {
    state: AppState,
    task: Task,
    reason: &'static str,
}
impl Drop for Exit {
    fn drop(&mut self) {
        if let Ok(store) = self.state.store.lock() {
            let _ =
                store
                    .interactive_journal()
                    .interrupt(&self.task.id, &self.task.lease, self.reason);
        }
        if let Ok(mut actors) = self.state.interactive.actors.lock()
            && actors
                .get(&self.task.id)
                .is_some_and(|current| Arc::ptr_eq(current, &self.task.handle))
        {
            actors.remove(&self.task.id);
        }
        let _ = self.task.handle.events.send(());
    }
}
struct Group(Option<u32>);
impl Group {
    fn terminate(&mut self) {
        if let Some(pid) = self.0.take() {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
            #[cfg(not(unix))]
            let _ = pid;
        }
    }
}
impl Drop for Group {
    fn drop(&mut self) {
        self.terminate();
    }
}
impl Service {
    /// Reject new work before stopping actors, including creates already validating FS.
    pub(crate) async fn shutdown(&self) {
        self.shutting_down.store(true, Ordering::Release);
        self.slots.close();
        let handles = {
            let actors = self.actors.lock().unwrap_or_else(|e| e.into_inner());
            actors.values().cloned().collect::<Vec<_>>()
        };
        for handle in &handles {
            handle.stop.store(true, Ordering::Release);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self
                .actors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_empty()
            {
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        for handle in handles {
            if let Some(abort) = handle
                .abort
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                abort.abort();
            }
        }
        // Let cancelled tasks drop their process-group and journal guards.
        tokio::task::yield_now().await;
    }
    #[cfg(test)]
    pub(crate) fn create(
        state: AppState,
        auth: AuthUser,
        workspace: String,
        project: String,
        options: Options,
    ) -> Result<Session> {
        Self::create_with_key(state, auth, workspace, project, options, None)
            .map(|accepted| accepted.session)
    }
    #[cfg(test)]
    pub(crate) fn create_with_key(
        state: AppState,
        auth: AuthUser,
        workspace: String,
        project: String,
        options: Options,
        key: Option<String>,
    ) -> Result<CreateAcceptance> {
        Self::create_request_with_key(
            state,
            auth,
            workspace,
            project,
            CreateRequest::Legacy(options.into()),
            key,
        )
    }
    pub(crate) fn create_request_with_key(
        state: AppState,
        auth: AuthUser,
        workspace: String,
        project: String,
        request: CreateRequest,
        key: Option<String>,
    ) -> Result<CreateAcceptance> {
        // An explicit remote intent must never resolve or replay a local one.
        request.validate_target()?;
        if key.as_deref().is_some_and(|key| !valid_admission_key(key)) {
            return Err("invalid_idempotency_key");
        }
        // Receipt lookup precedes scarce runtime capacity. Release Store before
        // connection_permit: it takes Management, whose ordering is Management ->
        // Store. Recheck all pins and the ledger after acquiring permits.
        let prepared = prepare_create_scope(&state, &auth, &workspace, &project, &request)?;
        if let Some(accepted) = lookup_prepared_receipt(
            &state,
            &auth,
            &workspace,
            &project,
            &request,
            key.as_deref(),
            &prepared,
        )? {
            return Ok(accepted);
        }
        if !state.config.experimental_interactive_runtime || !state.config.store_transcripts {
            return Err("interactive_runtime_disabled");
        }
        if state.interactive.shutting_down.load(Ordering::Acquire) {
            return Err("runtime_shutting_down");
        }
        #[cfg(not(test))]
        if !state.config.sandbox {
            return Err("interactive_runtime_requires_sandbox");
        }
        let runtime =
            tokio::runtime::Handle::try_current().map_err(|_| "runtime_executor_unavailable")?;
        let slot = match state.interactive.slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                if let Some(accepted) = lookup_prepared_receipt(
                    &state,
                    &auth,
                    &workspace,
                    &project,
                    &request,
                    key.as_deref(),
                    &prepared,
                )? {
                    return Ok(accepted);
                }
                return Err("runtime_capacity_reached");
            }
        };
        let connection = match state.connection_permit() {
            Ok(permit) => permit,
            Err(_) => {
                if let Some(accepted) = lookup_prepared_receipt(
                    &state,
                    &auth,
                    &workspace,
                    &project,
                    &request,
                    key.as_deref(),
                    &prepared,
                )? {
                    return Ok(accepted);
                }
                return Err("harness_management_busy");
            }
        };
        // Recheck immutable filesystem evidence immediately before the final
        // SQL cut; no Store mutex is held while inspecting checkout metadata.
        let _fs_slot = state
            .interactive
            .fs_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| "checkout_validation_busy")?;
        prepared
            .checkout
            .validate_filesystem()
            .map_err(|_| "checkout_identity_changed")?;
        prepared
            .scope
            .binding_snapshot
            .validate_native_checkout(&prepared.scope.native, &prepared.checkout)
            .map_err(|_| "checkout_identity_changed")?;
        drop(_fs_slot);
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        let handle = Arc::new(Handle {
            stop: Arc::new(AtomicBool::new(false)),
            events: broadcast::channel(64).0,
            abort: Mutex::new(None),
            admission: Mutex::new(None),
            active_turn: Mutex::new(None),
        });
        let mut admission = handle
            .admission
            .lock()
            .map_err(|_| "registry_unavailable")?;
        let mut abort = handle.abort.lock().map_err(|_| "registry_unavailable")?;
        // Store -> actors is also the ordering used by queue admission. Shutdown
        // releases actors before touching Store or waiting for workers.
        let mut actors = state
            .interactive
            .actors
            .lock()
            .map_err(|_| "registry_unavailable")?;
        if state.interactive.shutting_down.load(Ordering::Acquire) {
            return Err("runtime_shutting_down");
        }
        // Legacy unkeyed calls are each a distinct intent, never inferred retries.
        // A private random key gives them atomic create+lease without exposing a
        // recoverable idempotency identity or changing their compatibility rule.
        let request_key = key.unwrap_or_else(|| format!("legacy-{}", uuid::Uuid::now_v7()));
        let (accepted, settings, others) = store
            .native_checkout_transaction(|store| {
                let fresh = revalidate_create_scope(
                    &state, store, &auth, &workspace, &project, &request, &prepared,
                )?;
                if state.interactive.shutting_down.load(Ordering::Acquire) {
                    return Err(CreateCutError("runtime_shutting_down"));
                }
                let settings: ProjectSettings = serde_json::from_str(
                    &store
                        .project_settings(&fresh.native.id)?
                        .unwrap_or_else(|| "{}".into()),
                )
                .map_err(|_| CreateCutError("invalid_settings"))?;
                if settings.permission_mode == Some(dunlin_domain::PermissionMode::ReadOnly) {
                    return Err(CreateCutError("read_only_kernel_adapter_not_available"));
                }
                dunlin_domain::validate_project_environment(&settings.environment)
                    .map_err(|_| CreateCutError("invalid_environment"))?;
                if fresh.harness.split_whitespace().next().is_none() {
                    return Err(CreateCutError("harness_missing"));
                }
                let others = store
                    .list_projects()?
                    .into_iter()
                    .filter(|p| p.id != fresh.native.id)
                    .map(|p| p.path)
                    .collect::<Vec<_>>();
                let accepted = store
                    .interactive_journal()
                    .create_admitted_profile_in_transaction(
                        &fresh.authority,
                        &workspace,
                        &project,
                        &auth.device.id,
                        &request_key,
                        &prepared.scope.fingerprint,
                        &format!("actor-{}", uuid::Uuid::now_v7()),
                        prepared.scope.profile.as_ref(),
                    )
                    .map_err(|e| CreateCutError(admission_error(e)))?;
                Ok((accepted, settings, others))
            })
            .map_err(|e: CreateCutError| e.0)?;
        if accepted.lease.is_some() {
            store
                .interactive_journal()
                .notify_committed_admission()
                .map_err(admission_error)?;
        }
        let Some(lease) = accepted.lease else {
            return Ok(CreateAcceptance {
                session: accepted.session,
                replayed: true,
            });
        };
        let PreparedCreate {
            scope, checkout, ..
        } = prepared;
        let CreateScope {
            user,
            native,
            binding,
            binding_snapshot,
            harness,
            options,
            profile,
            instructions,
            mcp,
            ..
        } = scope;
        let session = accepted.session;
        let task = Task {
            id: session.id.clone(),
            user,
            device: auth.device.id,
            workspace,
            project,
            native: native.id,
            root: checkout.root().to_owned(),
            checkout,
            binding_snapshot,
            binding,
            harness,
            lease,
            options,
            profile,
            instructions,
            mcp,
            handle: handle.clone(),
        };
        *admission = Some(Admission {
            user: task.user.clone(),
            device: task.device.clone(),
            workspace: task.workspace.clone(),
            project: task.project.clone(),
            checkout: task.checkout.clone(),
            binding_snapshot: task.binding_snapshot.clone(),
            lease: task.lease.clone(),
            harness: task.harness.clone(),
        });
        actors.insert(session.id.clone(), handle.clone());
        drop(admission);
        drop(actors);
        drop(store);
        // No fallible locks or preparation remain after the durable acceptance.
        // A crash here leaves its receipt; recovery interrupts it, never restarts.
        // Exit must only be constructed after Store/actors guards were released.
        let exit = Exit {
            state: state.clone(),
            task: task.clone(),
            reason: "worker_dropped",
        };
        let owned_state = state.clone();
        let worker = runtime.spawn(async move {
            let mut exit = exit;
            let _slot = slot;
            let _connection = connection;
            if let Err(reason) = run(&owned_state, &task, &settings, &others).await {
                exit.reason = reason;
            }
        });
        *abort = Some(worker.abort_handle());
        Ok(CreateAcceptance {
            session,
            replayed: false,
        })
    }
    /// Called with the Store lock held; only an existing actor can accept work.
    /// Its original device/binding authority remains independent of the sender.
    pub(crate) fn queue_text_allowed(
        &self,
        state: &AppState,
        store: &crate::store::Store,
        id: &str,
        user: &str,
    ) -> bool {
        let check = || -> Result<()> {
            if self.shutting_down.load(Ordering::Acquire) {
                return Err("runtime_shutting_down");
            }
            let actors = self.actors.lock().map_err(|_| "registry_unavailable")?;
            let handle = actors.get(id).ok_or("runtime_unavailable")?;
            if handle.stop.load(Ordering::Acquire) {
                return Err("stop_requested");
            }
            let admission = handle
                .admission
                .lock()
                .map_err(|_| "registry_unavailable")?;
            let a = admission.as_ref().ok_or("runtime_unavailable")?;
            if a.user != user || crate::management::harness(&state.config) != a.harness {
                return Err("runtime_authority_changed");
            }
            store
                .revalidate_native_checkout_in_transaction(&a.checkout)
                .map_err(|_| "checkout_or_grant_changed")?;
            let owner = store
                .active_device_user(&a.device)
                .map_err(|_| "authority_unavailable")?
                .filter(|u| u.id == a.user)
                .ok_or("authority_revoked")?;
            crate::api::check_host_account_policy(state, &owner)
                .map_err(|_| "host_policy_changed")?;
            if !store
                .oidc_device_allowed(&a.device, state.config.oidc.as_ref())
                .map_err(|_| "authority_unavailable")?
            {
                return Err("identity_revoked");
            }
            let binding = store
                .local_binding_for_user(user, &a.workspace, &a.project)
                .map_err(|_| "authority_unavailable")?
                .ok_or("binding_removed")?;
            if binding != a.binding_snapshot {
                return Err("binding_changed");
            }
            let context = store
                .team_project_context(&a.workspace, &a.project, user)
                .map_err(|_| "authority_revoked")?;
            context
                .authorize_project(&a.workspace, &a.project, Permission::CreateSession)
                .map_err(|_| "authority_revoked")?;
            store
                .interactive_journal()
                .authorize_dispatch(&context, id, &a.lease)
                .map_err(|_| "runtime_fenced")?;
            Ok(())
        };
        authority_cut(store, |_| check()).is_ok()
    }
    /// Timing comes from the producer's monotonic clock, never client duration.
    pub(crate) fn turn_elapsed(&self, id: &str, turn: &str) -> Option<u64> {
        let actors = self.actors.lock().ok()?;
        let handle = actors.get(id)?;
        let current = handle.active_turn.lock().ok()?;
        let current = current.as_ref().filter(|a| a.id == turn)?;
        u64::try_from(current.started.elapsed().as_millis()).ok()
    }
    pub(crate) fn stop(
        &self,
        state: &AppState,
        auth: &AuthUser,
        id: &str,
        epoch: u64,
    ) -> Result<()> {
        let store = state.store.lock().map_err(|_| "store_unavailable")?;
        let (workspace, project) = store
            .interactive_scope(id)
            .map_err(|_| "session_unavailable")?;
        let authority = store
            .team_project_context(&workspace, &project, &auth.user.id)
            .map_err(|_| "authority_revoked")?;
        let (_, _, owner) = store
            .interactive_identity(id)
            .map_err(|_| "session_owner_required")?;
        if owner != auth.user.id {
            return Err("session_owner_required");
        }
        let (session, _) = store
            .interactive_journal()
            .snapshot(&authority, id)
            .map_err(|_| "session_unavailable")?;
        if session.epoch != epoch {
            return Err("runtime_epoch_changed");
        }
        if let Some(handle) = self
            .actors
            .lock()
            .map_err(|_| "registry_unavailable")?
            .get(id)
        {
            handle.stop.store(true, Ordering::Release);
        }
        Ok(())
    }
}
#[derive(Clone)]
struct PendingPermission {
    nonce: String,
    request_id: Value,
    deadline: Instant,
}
fn permission_sender_allowed(
    state: &AppState,
    task: &Task,
    decision: &dunlin_session_runtime::PermissionDecision,
) -> Result<()> {
    let store = state.store.lock().map_err(|_| "store_unavailable")?;
    let authority = context(state, &store, task)?;
    store
        .interactive_journal()
        .authorize_dispatch(&authority, &task.id, &task.lease)
        .map_err(|_| "permission_authority_changed")?;
    permission_device_allowed(state, &store, &task.user, decision)
}
fn permission_device_allowed(
    state: &AppState,
    store: &crate::store::Store,
    owner: &str,
    decision: &dunlin_session_runtime::PermissionDecision,
) -> Result<()> {
    let user = store
        .active_device_user(&decision.device_id)
        .map_err(|_| "authority_unavailable")?
        .filter(|u| u.id == owner)
        .ok_or("permission_device_revoked")?;
    crate::api::check_host_account_policy(state, &user).map_err(|_| "host_policy_changed")?;
    if !store
        .oidc_device_allowed(&decision.device_id, state.config.oidc.as_ref())
        .map_err(|_| "authority_unavailable")?
    {
        return Err("identity_revoked");
    }
    Ok(())
}
struct RequestReply {
    value: Value,
    // This flag is produced locally; agent-controlled JSON cannot forge evidence.
    cancellation_not_sent: bool,
}
impl std::ops::Deref for RequestReply {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.value
    }
}
struct Acp {
    input: ChildStdin,
    output: BufReader<ChildStdout>,
    partial: Vec<u8>,
    remote: String,
    turn: Option<String>,
    bytes: usize,
    frames: usize,
}
async fn frame(reader: &mut BufReader<ChildStdout>, partial: &mut Vec<u8>) -> Result<Value> {
    loop {
        let bytes = reader.fill_buf().await.map_err(|_| "agent_read_failed")?;
        if bytes.is_empty() {
            return Err("agent_closed");
        }
        let newline = bytes.iter().position(|b| *b == b'\n');
        let n = newline.map(|i| i + 1).unwrap_or(bytes.len());
        if partial.len() + n > 1024 * 1024 {
            return Err("agent_frame_limit");
        }
        partial.extend_from_slice(&bytes[..n]);
        reader.consume(n);
        if newline.is_some() {
            return serde_json::from_slice(&std::mem::take(partial))
                .map_err(|_| "agent_invalid_json");
        }
    }
}
impl Acp {
    async fn send(
        &mut self,
        state: &AppState,
        task: &Task,
        message: Value,
        deadline: Instant,
    ) -> Result<()> {
        self.send_checked(state, task, message, deadline, None)
            .await
    }
    async fn send_checked(
        &mut self,
        state: &AppState,
        task: &Task,
        message: Value,
        deadline: Instant,
        permission: Option<&dunlin_session_runtime::PermissionDecision>,
    ) -> Result<()> {
        guard(state, task, deadline).await?;
        if let Some(decision) = permission {
            permission_sender_allowed(state, task, decision)?;
            journal(state, task, |j, a| {
                j.validate_permission_delivery(a, &task.id, &task.lease, &decision.nonce)
            })?;
        }
        let mut bytes = serde_json::to_vec(&message).map_err(|_| "invalid_request")?;
        if bytes.len() > 1024 * 1024 {
            return Err("request_frame_limit");
        }
        bytes.push(b'\n');
        let write = async {
            self.input.write_all(&bytes).await?;
            self.input.flush().await
        };
        tokio::pin!(write);
        loop {
            tokio::select! {result=&mut write=>return result.map_err(|_|"agent_write_failed"),_=tokio::time::sleep(Duration::from_millis(200))=>{
                guard(state,task,deadline).await?;
                if let Some(decision)=permission {
                    permission_sender_allowed(state,task,decision)?;
                    journal(state,task,|j,a|j.validate_permission_delivery(a,&task.id,&task.lease,&decision.nonce))?;
                }
            }}
        }
    }
    async fn discard_permissions(
        &mut self,
        state: &AppState,
        task: &Task,
        pending: &mut Vec<PendingPermission>,
        reason: &str,
        deadline: Instant,
    ) -> Result<()> {
        for p in pending.drain(..) {
            journal(state, task, |j, a| {
                j.invalidate_permission(a, &task.id, &task.lease, &p.nonce, reason)
            })?;
            self.send(state,task,json!({"jsonrpc":"2.0","id":p.request_id,"result":{"outcome":{"outcome":"cancelled"}}}),deadline).await?;
        }
        Ok(())
    }
    async fn poll_permissions(
        &mut self,
        state: &AppState,
        task: &Task,
        pending: &mut Vec<PendingPermission>,
        cancel: bool,
        deadline: Instant,
    ) -> Result<()> {
        let mut index = 0;
        while index < pending.len() {
            let p = pending[index].clone();
            let status = journal(state, task, |j, a| {
                j.permission_status(a, &task.id, &task.lease, &p.nonce)
            })?;
            let mut consumed = false;
            let reject = if cancel || Instant::now() >= p.deadline {
                journal(state, task, |j, a| {
                    j.invalidate_permission(
                        a,
                        &task.id,
                        &task.lease,
                        &p.nonce,
                        if cancel {
                            "turn_cancelled"
                        } else {
                            "permission_expired"
                        },
                    )
                })?;
                true
            } else {
                match status {
                    dunlin_session_runtime::PermissionStatus::Pending => false,
                    dunlin_session_runtime::PermissionStatus::Invalid => {
                        journal(state, task, |j, a| {
                            j.invalidate_permission(
                                a,
                                &task.id,
                                &task.lease,
                                &p.nonce,
                                "permission_unavailable",
                            )
                        })?;
                        true
                    }
                    dunlin_session_runtime::PermissionStatus::Decided(decision) => {
                        if permission_sender_allowed(state, task, &decision).is_err() {
                            journal(state, task, |j, a| {
                                j.invalidate_permission(
                                    a,
                                    &task.id,
                                    &task.lease,
                                    &p.nonce,
                                    "permission_device_revoked",
                                )
                            })?;
                            true
                        } else {
                            let decision = journal(state, task, |j, a| {
                                j.consume_permission(a, &task.id, &task.lease, &p.nonce)
                            })?;
                            let outcome = match &decision.option_id {
                                Some(option) => json!({"outcome":"selected","optionId":option}),
                                None if !decision.approved => json!({"outcome":"cancelled"}),
                                None => return Err("invalid_permission_choice"),
                            };
                            self.send_checked(state,task,json!({"jsonrpc":"2.0","id":p.request_id,"result":{"outcome":outcome}}),deadline,Some(&decision)).await?;
                            journal(state, task, |j, a| {
                                j.permission_delivered(a, &task.id, &task.lease, &p.nonce)
                            })?;
                            consumed = true;
                            false
                        }
                    }
                }
            };
            if reject {
                self.send(state,task,json!({"jsonrpc":"2.0","id":p.request_id,"result":{"outcome":{"outcome":"cancelled"}}}),deadline).await?;
                pending.remove(index);
            } else if consumed {
                pending.remove(index);
            } else {
                index += 1;
            }
        }
        Ok(())
    }
    async fn request(
        &mut self,
        state: &AppState,
        task: &Task,
        id: u64,
        method: &str,
        params: Value,
        deadline: Instant,
    ) -> Result<RequestReply> {
        let mut deadline = deadline;
        let requested = |turn: &str| {
            journal(state, task, |j, a| {
                j.cancellation_requested(a, &task.id, &task.lease, turn)
            })
        };
        if method == "session/prompt"
            && let Some(turn) = &self.turn
            && requested(turn)?
        {
            return Ok(RequestReply {
                value: json!({"stopReason":"cancelled"}),
                cancellation_not_sent: true,
            });
        }
        self.send(
            state,
            task,
            json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
            deadline,
        )
        .await?;
        let mut cancel_sent = false;
        let mut permissions: Vec<PendingPermission> = Vec::new();
        loop {
            if cancel_sent && Instant::now() >= deadline {
                return Err("cancellation_unconfirmed");
            }
            guard(state, task, deadline).await?;
            let cancel = if let Some(turn) = &self.turn {
                requested(turn)?
            } else {
                false
            };
            self.poll_permissions(state, task, &mut permissions, cancel, deadline)
                .await?;
            if !cancel_sent && cancel {
                deadline = std::cmp::min(deadline, Instant::now() + Duration::from_secs(5));
                self.send(state, task, json!({"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":self.remote}}), deadline).await?;
                cancel_sent = true;
            }
            let value = tokio::select! {value=frame(&mut self.output,&mut self.partial)=>value?,_=tokio::time::sleep(Duration::from_millis(200))=>continue};
            self.bytes = self.bytes.saturating_add(value.to_string().len());
            self.frames += 1;
            if self.bytes > task.options.max_output_bytes || self.frames > 4096 {
                return Err("agent_output_budget");
            }
            if value["jsonrpc"] != "2.0" {
                return Err("agent_protocol_invalid");
            }
            if let Some(method) = value["method"].as_str() {
                if value["params"]["sessionId"] != self.remote || self.remote.is_empty() {
                    return Err("agent_session_scope_changed");
                }
                if let Some(request) = value.get("id") {
                    if method == "session/request_permission" {
                        let registered = (|| -> Result<String> {
                            if permissions.len() >= 8
                                || permissions.iter().any(|p| p.request_id == *request)
                            {
                                return Err("permission_request_limit");
                            }
                            let turn = self.turn.as_ref().ok_or("permission_outside_turn")?;
                            let ttl = deadline
                                .checked_duration_since(Instant::now())
                                .unwrap_or_default()
                                .as_secs()
                                .clamp(1, 120);
                            let mut identity = json!({"harness":task.harness,"binding":task.binding,"native":task.native,"model":task.options.model,"mode":task.options.mode});
                            if let Some(metadata) = task.checkout.metadata_stamp() {
                                identity["nativeMetadata"] = json!(metadata);
                            }
                            let fingerprint = hex::encode(Sha256::digest(
                                serde_json::to_vec(&identity)
                                    .map_err(|_| "invalid_runtime_identity")?,
                            ));
                            journal(state, task, |j, a| {
                                j.register_permission(
                                    a,
                                    &task.id,
                                    &task.lease,
                                    turn,
                                    dunlin_session_runtime::PermissionChallenge {
                                        request_id: request.clone(),
                                        tool_call: value["params"]["toolCall"].clone(),
                                        options: value["params"]["options"].clone(),
                                        ttl_seconds: ttl,
                                        starter_device: task.device.clone(),
                                        binding_revision: task.binding,
                                        harness_digest: fingerprint,
                                    },
                                )
                            })
                        })();
                        match registered {
                            Ok(nonce) => {
                                let ttl = deadline
                                    .checked_duration_since(Instant::now())
                                    .unwrap_or_default()
                                    .as_secs()
                                    .clamp(1, 120);
                                permissions.push(PendingPermission {
                                    nonce,
                                    request_id: request.clone(),
                                    deadline: std::cmp::min(
                                        deadline,
                                        Instant::now() + Duration::from_secs(ttl),
                                    ),
                                });
                                continue;
                            }
                            Err(reason) => {
                                self.send(state,task,json!({"jsonrpc":"2.0","id":request,"result":{"outcome":{"outcome":"cancelled"}}}),deadline).await?;
                                return Err(reason);
                            }
                        }
                    }
                    self.send(state,task,json!({"jsonrpc":"2.0","id":request,"error":{"code":-32601,"message":"Client tool capability is unavailable"}}),deadline).await?;
                    return Err("unapproved_client_tool_request");
                }
                if method != "session/update" {
                    return Err("unsupported_agent_notification");
                }
                let update = &value["params"]["update"];
                if self.turn.is_some() && update["sessionUpdate"] == "config_option_update" {
                    verify_options(&update["configOptions"], &task.options)?;
                }
                if self.turn.is_some()
                    && update["sessionUpdate"] == "current_mode_update"
                    && update["currentModeId"] != task.options.mode
                {
                    return Err("agent_mode_changed");
                }
                if let Some(turn) = &self.turn {
                    journal(state, task, |j, a| {
                        j.agent_update(a, &task.id, &task.lease, turn, update.clone())
                    })?;
                }
            } else {
                if value["id"] != id {
                    return Err("agent_response_id_changed");
                }
                if value.get("error").is_some() {
                    return Err("agent_rpc_error");
                }
                self.discard_permissions(
                    state,
                    task,
                    &mut permissions,
                    "prompt_finished",
                    deadline,
                )
                .await?;
                return value
                    .get("result")
                    .cloned()
                    .map(|value| RequestReply {
                        value,
                        cancellation_not_sent: false,
                    })
                    .ok_or("agent_result_missing");
            }
        }
    }
}
fn verify_options(options: &Value, profile: &Options) -> Result<()> {
    for (category, wanted) in [
        ("model", profile.model.as_str()),
        ("mode", profile.mode.as_str()),
    ] {
        let option = options
            .as_array()
            .and_then(|v| v.iter().find(|o| o["category"] == category))
            .ok_or("agent_profile_unverifiable")?;
        if option["currentValue"] != wanted {
            return Err("agent_profile_changed");
        }
    }
    Ok(())
}
async fn run(
    state: &AppState,
    task: &Task,
    settings: &ProjectSettings,
    others: &[String],
) -> Result<()> {
    let lifetime = Instant::now() + Duration::from_secs(3600);
    guard(state, task, lifetime).await?;
    let mcp = mcp::Peer::start(state, task, lifetime).await?;
    let mut parts = task.harness.split_whitespace();
    let program = parts.next().ok_or("harness_missing")?.to_string();
    let args = parts.map(str::to_owned).collect::<Vec<_>>();
    #[cfg(test)]
    let (program, args) = state.interactive.command.clone().unwrap_or((program, args));
    let mut command = crate::acp::harness_command_with_brokers(
        &state.config,
        &program,
        &args,
        &task.root,
        &[],
        others,
        settings,
        &task.user,
        &mcp.paths,
        &task.checkout.metadata_roots(),
    );
    command
        .env("PATH", crate::management::path(&state.config))
        .current_dir(&task.root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| "agent_spawn_failed")?;
    let mut group = Group(Some(child.id().ok_or("agent_pid_missing")?));
    let result = conversation(state, task, &mut child, lifetime, &mcp.configs).await;
    group.terminate();
    let _ = child.kill().await;
    let _ = child.wait().await;
    result
}
async fn conversation(
    state: &AppState,
    task: &Task,
    child: &mut Child,
    lifetime: Instant,
    mcp_servers: &[Value],
) -> Result<()> {
    let mut acp = Acp {
        input: child.stdin.take().ok_or("agent_stdin_missing")?,
        output: BufReader::new(child.stdout.take().ok_or("agent_stdout_missing")?),
        partial: Vec::new(),
        remote: String::new(),
        turn: None,
        bytes: 0,
        frames: 0,
    };
    let startup = std::cmp::min(lifetime, Instant::now() + Duration::from_secs(30));
    let initialized=acp.request(state,task,1,"initialize",json!({"protocolVersion":1,"clientInfo":{"name":"Dunlin interactive runtime","version":env!("CARGO_PKG_VERSION")},"clientCapabilities":{"fs":{"readTextFile":false,"writeTextFile":false},"terminal":false}}),startup).await?;
    if initialized["protocolVersion"] != 1 {
        return Err("unsupported_acp_version");
    }
    let session = acp
        .request(
            state,
            task,
            2,
            "session/new",
            json!({"cwd":task.root,"mcpServers":mcp_servers}),
            startup,
        )
        .await?;
    acp.remote = session["sessionId"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or("agent_session_missing")?
        .into();
    let mut options = session["configOptions"].clone();
    for (index, (category, wanted)) in
        [("model", &task.options.model), ("mode", &task.options.mode)]
            .into_iter()
            .enumerate()
    {
        let option = options
            .as_array()
            .and_then(|v| v.iter().find(|o| o["category"] == category))
            .ok_or("agent_profile_unverifiable")?;
        let config = option["id"]
            .as_str()
            .ok_or("agent_config_id_missing")?
            .to_string();
        options = acp
            .request(
                state,
                task,
                3 + index as u64,
                "session/set_config_option",
                json!({"sessionId":acp.remote,"configId":config,"value":wanted}),
                startup,
            )
            .await?["configOptions"]
            .clone();
    }
    verify_options(&options, &task.options)?;
    journal(state, task, |j, a| {
        j.harness_ready(
            a,
            &task.id,
            &task.lease,
            HarnessInfo {
                profile: task.profile.clone(),
                remote_session_id: acp.remote.clone(),
                binding_revision: task.binding,
                model: task.options.model.clone(),
                mode: task.options.mode.clone(),
                max_seconds: task.options.max_seconds,
                max_output_bytes: task.options.max_output_bytes,
            },
        )
    })?;
    let mut idle = Instant::now();
    let mut request_id = 10;
    loop {
        guard(state, task, lifetime).await?;
        let turn = journal(state, task, |j, a| j.claim_next(a, &task.id, &task.lease))?;
        let Some(turn) = turn else {
            if idle.elapsed() > Duration::from_secs(60) {
                return Err("idle_timeout");
            }
            tokio::select! {_=tokio::time::sleep(Duration::from_millis(200))=>{},value=frame(&mut acp.output,&mut acp.partial)=>{value?;return Err("unsolicited_agent_frame");}}
            continue;
        };
        let deadline = std::cmp::min(
            lifetime,
            Instant::now() + Duration::from_secs(task.options.max_seconds),
        );
        guard(state, task, deadline).await?;
        let turn_started = Instant::now();
        *task
            .handle
            .active_turn
            .lock()
            .map_err(|_| "registry_unavailable")? = Some(ActiveTurn {
            id: turn.id.clone(),
            started: turn_started,
        });
        journal(state, task, |j, a| {
            j.transition(a, &task.id, &task.lease, &turn.id, TurnPhase::Running)
        })?;
        acp.turn = Some(turn.id.clone());
        acp.bytes = 0;
        acp.frames = 0;
        request_id += 1;
        let result = acp
            .request(
                state,
                task,
                request_id,
                "session/prompt",
                json!({"sessionId":acp.remote,"prompt":[{"type":"text","text": if task.instructions.is_empty() { turn.prompt.clone() } else { format!("{}\n\n## User request\n{}",task.instructions,turn.prompt) }}]}),
                deadline,
            )
            .await?;
        let phase = match result["stopReason"].as_str() {
            Some("end_turn") => TurnPhase::Completed,
            Some("cancelled") => TurnPhase::Cancelled,
            Some("refusal" | "max_tokens") => TurnPhase::Failed,
            _ => return Err("unknown_stop_reason"),
        };
        let duration =
            u64::try_from(turn_started.elapsed().as_millis()).map_err(|_| "duration_overflow")?;
        journal(state, task, |j, a| {
            if result.cancellation_not_sent {
                j.settle_cancellation(a, &task.id, &task.lease, &turn.id, "not_sent", duration)
            } else {
                // The journal linearizes cancellation vs completion in this txn.
                j.transition_timed(a, &task.id, &task.lease, &turn.id, phase, Some(duration))
            }
        })?;
        *task
            .handle
            .active_turn
            .lock()
            .map_err(|_| "registry_unavailable")? = None;
        acp.turn = None;
        idle = Instant::now();
    }
}
#[cfg(test)]
mod tests;
