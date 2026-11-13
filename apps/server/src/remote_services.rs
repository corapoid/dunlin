//! Shared startup/shutdown for production and process-relaunch acceptance.
use crate::api::AppState;
use std::sync::Arc;
pub(crate) struct Services {
    execution: Option<Arc<crate::remote_execution::Service>>,
    sessions: Option<Arc<crate::remote_sessions::Service>>,
}
impl Services {
    pub(crate) async fn start(state: &AppState) -> Result<Self, String> {
        let execution = if state
            .config
            .worker_controller
            .as_ref()
            .is_some_and(|controller| controller.execution_enabled)
        {
            Some(
                crate::remote_execution::Service::start(state)
                    .await
                    .map_err(|_| "Remote execution controller unavailable".to_owned())?,
            )
        } else {
            None
        };
        let sessions = if state
            .config
            .worker_controller
            .as_ref()
            .is_some_and(|controller| {
                controller.execution_enabled && controller.persistent_sessions
            }) {
            let service = crate::remote_sessions::Service::start(state)
                .await
                .map_err(|_| "Remote session controller unavailable".to_owned())?;
            state
                .remote_sessions
                .set(service.clone())
                .map_err(|_| "Remote session controller already started".to_owned())?;
            Some(service)
        } else {
            None
        };
        Ok(Self {
            execution,
            sessions,
        })
    }
    pub(crate) async fn shutdown(&self) {
        if let Some(service) = &self.sessions {
            service.shutdown().await;
        }
        if let Some(service) = &self.execution {
            service.shutdown().await;
        }
    }
}
