//! Relay authority stays on the relay. Devices share a user/project ledger;
//! approval is bound to the approving device and the target's active ACP lease.
use crate::api::{ApiError, AppState, AuthUser};
use axum::{Json, extract::State};
use dunlin_coordination::{ContinuationStatus as CS, Execution, Ledger, Scope, Snapshot, now_ms};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    project: String,
    operation: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    session: String,
    #[serde(default)]
    expected: Option<dunlin_coordination::Continuation>,
}
pub(crate) async fn request(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<Input>,
) -> Result<Json<Value>, ApiError> {
    state.project_for_user(&auth.user.id, &input.project)?;
    let service = state
        .coordination
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("Relay coordination is unavailable"))?
        .clone();
    let scope = Scope {
        authority: auth.user.id.clone(),
        project: input.project.clone(),
    };
    // Do SQLite and serialization away from Tokio's request executor.
    let value = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        let mut db = service
            .store
            .lock()
            .map_err(|_| "Coordination store unavailable")?;
        if input.operation == "archive" {
            db.archive_scope(Some(&scope))?;
            return Ok(json!({"archived":true}));
        }
        if input.operation == "snapshot" {
            let stale: Vec<_> = {
                let leases = state.leases.lock().map_err(|_| "Lease state unavailable")?;
                db.ledger().continuations.iter().filter(|c| {
                    c.scope == scope && c.status == CS::Waiting
                        && leases.get(&c.session).is_none_or(|lease| {
                            c.executor_device.as_deref() != Some(lease.device_id.as_str())
                                || lease.expires_at <= std::time::Instant::now()
                                || c.executor_connection.as_deref() != Some(lease.connection_id.as_str())
                        })
                }).map(|c| c.id.clone()).collect()
            };
            if !stale.is_empty() {
                db.mutate(|l| {
                    for c in l.continuations.iter_mut().filter(|c| stale.contains(&c.id)) {
                        c.status = CS::NeedsAttention;
                        c.reason = Some("ACP executor connection changed or expired; reconnect and explicitly reapprove".into());
                    }
                    Ok(())
                })?;
            }
            let l = db.ledger();
            let ledger = Ledger {
                runs: l
                    .runs
                    .iter()
                    .filter(|v| v.scope == scope)
                    .cloned()
                    .collect(),
                tasks: l
                    .tasks
                    .iter()
                    .filter(|v| v.scope == scope)
                    .cloned()
                    .collect(),
                continuations: l
                    .continuations
                    .iter()
                    .filter(|v| v.scope == scope)
                    .cloned()
                    .collect(),
                messages: l
                    .messages
                    .iter()
                    .filter(|v| v.scope == scope)
                    .cloned()
                    .collect(),
                audit: l
                    .audit
                    .iter()
                    .filter(|v| v.scope == scope)
                    .rev()
                    .take(128)
                    .cloned()
                    .collect(),
                worktree_links: Vec::new(),
            };
            let value = serde_json::to_value(Snapshot {
                scope,
                device: auth.device.id,
                ledger,
            })
            .map_err(|e| e.to_string())?;
            if serde_json::to_vec(&value).map_err(|e| e.to_string())?.len() > 8 * 1024 * 1024 {
                return Err("Coordination snapshot exceeds 8 MiB; archive finished history".into());
            }
            return Ok(value);
        }
        if input.operation == "ack" {
            let owner = state
                .store
                .lock()
                .map_err(|_| "Relay store unavailable")?
                .session_owner(&input.session)
                .map_err(|e| e.to_string())?
                .ok_or("Session unavailable")?;
            if owner.user_id != auth.user.id || owner.project_id != input.project {
                return Err("Session unavailable".into());
            }
            db.mutate(|l| {
                let m = l
                    .messages
                    .iter_mut()
                    .find(|m| m.id == input.id && m.scope == scope && m.to == input.session)
                    .ok_or("Message unavailable")?;
                m.read_ms = Some(now_ms());
                Ok(())
            })?;
            return Ok(json!({"acknowledged":true}));
        }
        let c = db
            .ledger()
            .continuations
            .iter()
            .find(|c| c.id == input.id && c.scope == scope && c.session == input.session)
            .cloned()
            .ok_or("Continuation unavailable")?;
        let owner = state
            .store
            .lock()
            .map_err(|_| "Relay store unavailable")?
            .session_owner(&c.session)
            .map_err(|e| e.to_string())?
            .ok_or("Session unavailable")?;
        if owner.user_id != auth.user.id || owner.project_id != input.project {
            return Err("Session unavailable".into());
        }
        match input.operation.as_str() {
            "approve" => {
                let expected = input.expected.ok_or("Exact proposal is required")?;
                if expected.prompt != c.prompt
                    || expected.dependencies != c.dependencies
                    || expected.execution != c.execution
                {
                    return Err("Proposal changed during approval".into());
                }
                let execution: Execution = db
                    .ledger()
                    .current_run(&scope, &c.session)
                    .ok_or("Run unavailable")?
                    .execution
                    .clone();
                let connection={let leases=state.leases.lock().map_err(|_|"Lease state unavailable")?;leases.get(&c.session).filter(|l|l.device_id==auth.device.id && l.expires_at>std::time::Instant::now()).map(|l|l.connection_id.clone()).ok_or("Connect or take over this session on the approving device first")?};
                db.approve(&c.id, &execution, now_ms() + 3_600_000)?;
                db.mutate(|l| {
                    let approved=l.continuations
                        .iter_mut()
                        .find(|v| v.id == c.id)
                        .unwrap();
                    approved.executor_device = Some(auth.device.id.clone());approved.executor_connection=Some(connection);
                    Ok(())
                })?;
            }
            "cancel" => db.set_continuation(
                &c.id,
                CS::Cancelled,
                Some("Cancelled by authorized device".into()),
            )?,
            "pause" => db.set_continuation(
                &c.id,
                CS::NeedsAttention,
                Some("Executor paused; reconnect/review before approval".into()),
            )?,
            _ => return Err("Unknown coordination operation".into()),
        }
        Ok(json!({"ok":true}))
    })
    .await
    .map_err(|_| ApiError::internal("Coordination worker failed"))?
    .map_err(ApiError::bad_request)?;
    Ok(Json(value))
}
