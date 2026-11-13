//! Source-owned Unix proxy. Its capability comes from original native leases,
//! never from a wire DTO, restored row, MCP token or controller readiness flag.
use super::sessions::SessionActive;
use super::*;
use crate::worker_client::broker::{self, Client};
use dunlin_domain::agent::AgentDefinition;
use serde_json::{Value, json};
use std::{collections::BTreeSet, sync::Weak};
use wire::session::broker::{BrokerError, BrokerOperation, BrokerScope};

pub(crate) struct Peer {
    configs: Vec<Value>,
    paths: Vec<PathBuf>,
    profile: String,
    authority: Arc<Authority>,
    #[cfg(unix)]
    _directory: tempfile::TempDir,
}
impl Drop for Peer {
    fn drop(&mut self) {
        self.authority.stop.store(true, Ordering::Release);
    }
}
/// Only a live source proxy can supply this to the ACP engine. No Deserialize,
/// public fields or constructor from MCP server JSON.
pub(crate) struct HarnessTools {
    peer: Arc<Peer>,
}
impl HarnessTools {
    pub(in crate::worker_execution) fn validate(
        &self,
        definition: &AgentDefinition,
    ) -> Result<(), &'static str> {
        self.peer
            .authority
            .live()
            .map_err(|_| "native_mcp_scope_closed")?;
        if definition.mcp.is_empty()
            || crate::worker_execution::profile_digest(definition)
                .map_err(|_| "invalid_mcp_profile")?
                != self.peer.profile
        {
            return Err("native_mcp_profile_changed");
        }
        Ok(())
    }
    pub(in crate::worker_execution) fn configs(&self) -> &[Value] {
        &self.peer.configs
    }
}
struct Authority {
    runtime: Weak<Runtime>,
    active: Weak<SessionActive>,
    client: Client,
    stop: Arc<AtomicBool>,
    handle: tokio::runtime::Handle,
}
struct Authorized {
    value: Value,
    scope: BrokerScope,
    deadline: Instant,
}
impl Authority {
    fn live(&self) -> Result<(Arc<Runtime>, Arc<SessionActive>), Code> {
        if self.stop.load(Ordering::Acquire) {
            return Err(Code::AuthorityRevoked);
        }
        let runtime = self.runtime.upgrade().ok_or(Code::Unavailable)?;
        let active = self.active.upgrade().ok_or(Code::Unavailable)?;
        runtime.mcp_live(&active)?;
        Ok((runtime, active))
    }
    fn interrupt(&self) {
        self.stop.store(true, Ordering::Release);
        if let Some(active) = self.active.upgrade() {
            active.stop();
        }
    }
    async fn forward(&self, operation: BrokerOperation) -> Result<Authorized, Code> {
        let (runtime, active) = self.live()?;
        let call = matches!(operation, BrokerOperation::ToolsCall { .. });
        let cut = runtime
            .mcp_cut(&active, call, None)
            .await
            .inspect_err(|_| self.interrupt())?;
        wire::session::broker::BrokerRequest {
            scope: cut.scope.clone(),
            request_id: "source-preflight".into(),
            operation: operation.clone(),
        }
        .validate()
        .map_err(|_| Code::UnsupportedProfile)?;
        let deadline = cut.deadline;
        let request = self.client.request_bounded(
            cut.scope.clone(),
            operation,
            deadline.into(),
            cut.limits.max_reply_bytes,
        );
        tokio::pin!(request);
        let answer = loop {
            tokio::select! {
                result=&mut request=>break result,
                _=tokio::time::sleep(Duration::from_millis(50))=>{self.live()?;}
            }
        };
        let answer = match answer {
            Ok(answer) => answer,
            Err(broker::Error::Rejected(
                BrokerError::InvalidRequest | BrokerError::CapacityReached,
            ))
            | Err(broker::Error::Capacity) => return Err(Code::UnsupportedProfile),
            Err(_) => {
                self.interrupt();
                return Err(Code::Unavailable);
            }
        };
        runtime
            .mcp_cut(&active, call, Some(cut.scope.clone()))
            .await
            .inspect_err(|_| self.interrupt())?;
        if Instant::now() >= deadline || self.stop.load(Ordering::Acquire) {
            return Err(Code::LeaseExpired);
        }
        let value = serde_json::from_str(&answer).map_err(|_| Code::Unavailable)?;
        Ok(Authorized {
            value,
            scope: cut.scope,
            deadline,
        })
    }
    fn finish(&self, answer: &Authorized) -> Result<(), Code> {
        let (runtime, active) = self.live()?;
        let call = matches!(answer.scope, BrokerScope::Turn { .. });
        self.handle
            .block_on(runtime.mcp_cut(&active, call, Some(answer.scope.clone())))?;
        if Instant::now() >= answer.deadline {
            return Err(Code::LeaseExpired);
        }
        self.live().map(|_| ())
    }
}
fn operation(method: &str, params: Value) -> Result<BrokerOperation, Code> {
    let object = params.as_object().ok_or(Code::UnsupportedProfile)?;
    match method {
        "tools/list" if object.keys().all(|key| key == "cursor") => {
            let cursor = object
                .get("cursor")
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_owned)
                        .ok_or(Code::UnsupportedProfile)
                })
                .transpose()?;
            Ok(BrokerOperation::ToolsList { cursor })
        }
        "tools/call"
            if object.len() == 2
                && object.contains_key("name")
                && object.contains_key("arguments") =>
        {
            if !params["arguments"].is_object() {
                return Err(Code::UnsupportedProfile);
            }
            Ok(BrokerOperation::ToolsCall {
                name: params["name"]
                    .as_str()
                    .ok_or(Code::UnsupportedProfile)?
                    .into(),
                arguments_json: serde_json::to_string(&params["arguments"])
                    .map_err(|_| Code::UnsupportedProfile)?,
            })
        }
        _ => Err(Code::UnsupportedProfile),
    }
}
impl Peer {
    pub(super) fn tools(self: &Arc<Self>) -> HarnessTools {
        HarnessTools { peer: self.clone() }
    }
    pub(super) fn paths(&self) -> &[PathBuf] {
        &self.paths
    }
    pub(super) async fn start(
        runtime: &Arc<Runtime>,
        active: &Arc<SessionActive>,
    ) -> Result<Option<Arc<Self>>, Code> {
        if !active.wants_mcp() {
            return Ok(None);
        }
        #[cfg(not(unix))]
        {
            let _ = runtime;
            return Err(Code::UnsupportedProfile);
        }
        #[cfg(unix)]
        {
            use crate::interactive::mcp::{proxy, runtime::proxy_executable};
            use std::io::Write;
            use std::os::unix::{fs::PermissionsExt, net::UnixListener};
            let cut = runtime.mcp_cut(active, false, None).await?;
            if cut.limits.max_reply_bytes < 128 {
                return Err(Code::UnsupportedProfile);
            }
            // The single listener serializes a session's operations, staying
            // below its selected max_pending. Its real thread owns this global
            // slot through HTTP/RPC waits, cancellation and actual termination.
            let slot = runtime
                .proxy_slots
                .clone()
                .try_acquire_owned()
                .map_err(|_| Code::CapacityReached)?;
            let directory = tempfile::Builder::new()
                .prefix("dunlin-source-mcp-")
                .tempdir_in("/tmp")
                .map_err(|_| Code::Unavailable)?;
            std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
                .map_err(|_| Code::Unavailable)?;
            let socket = directory.path().join("rpc.sock");
            let listener = UnixListener::bind(&socket).map_err(|_| Code::Unavailable)?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
                .map_err(|_| Code::Unavailable)?;
            listener
                .set_nonblocking(true)
                .map_err(|_| Code::Unavailable)?;
            let executable = proxy_executable().map_err(|_| Code::Unavailable)?;
            let token = crate::auth::random_token();
            let hash = crate::auth::hash_token(&token);
            let authority = Arc::new(Authority {
                runtime: Arc::downgrade(runtime),
                active: Arc::downgrade(active),
                client: active.client()?,
                stop: Arc::new(AtomicBool::new(false)),
                handle: tokio::runtime::Handle::current(),
            });
            let peer = Arc::new(Self {
                configs: vec![json!({"name":"dunlin-session-tools","command":executable,
                "args":["--mcp-tools"],"env":[{"name":"DUNLIN_MCP_SOCKET","value":socket},{"name":"DUNLIN_MCP_TOKEN","value":token}]})],
                paths: vec![socket, executable],
                profile: cut.profile,
                authority: authority.clone(),
                _directory: directory,
            });
            std::thread::Builder::new().name("dunlin-source-mcp".into()).spawn(move|| {
                let _slot=slot;
                let mut used=BTreeSet::new();
                while authority.live().is_ok() {
                    let (mut stream,_)=match listener.accept() {
                        Ok(value)=>value,
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{std::thread::sleep(Duration::from_millis(20));continue;}
                        Err(_)=>break,
                    };
                    let _=stream.set_read_timeout(Some(Duration::from_secs(2)));
                    let _=stream.set_write_timeout(Some(Duration::from_secs(2)));
                    let request=(|| -> Result<_,Code> {
                        let request:proxy::Request=serde_json::from_value(proxy::read_frame(&mut stream).map_err(|_|Code::UnsupportedProfile)?)
                            .map_err(|_|Code::UnsupportedProfile)?;
                        if request.token.len()>128 || crate::auth::hash_token(&request.token)!=hash
                            || request.request_id.is_empty() || request.request_id.len()>128 || request.request_id.chars().any(char::is_control)
                            || used.len()>=256 || !used.insert(request.request_id) {return Err(Code::UnsupportedProfile);}
                        operation(&request.method,request.params)
                    })();
                    let answer=request.and_then(|operation|authority.handle.block_on(authority.forward(operation)));
                    let response=match &answer {
                        Ok(answer)=>json!({"result":answer.value}),
                        Err(_)=>json!({"error":"MCP request rejected or outcome unavailable. It was not retried."}),
                    };
                    let mut packet=Vec::new();
                    if proxy::write_frame(&mut packet,&response).is_err() {authority.interrupt();break;}
                    if let Ok(answer)=&answer && authority.finish(answer).is_err() {authority.interrupt();break;}
                    if answer.as_ref().is_err_and(|code|*code!=Code::UnsupportedProfile) {authority.interrupt();}
                    if stream.write_all(&packet).and_then(|_|stream.flush()).is_err() && answer.is_ok() {
                        authority.interrupt();
                        break;
                    }
                }
            }).map_err(|_|Code::CapacityReached)?;
            Ok(Some(peer))
        }
    }
}
