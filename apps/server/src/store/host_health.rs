//! Manager-scoped operational counts. No transcript, prompt, user identity or
//! execution handle leaves this observation, and no recovery is triggered.
use super::hosts::Host;
use super::*;
use crate::worker_channel::{LiveChannelObservation, Service as Channels};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct HostRuntimeObservation {
    pub one_shot_reservations: u64,
    pub session_reservations: u64,
    pub active_sessions: u64,
    pub active_turns: u64,
    pub uncertain_reservations: u64,
    pub unconfirmed_acknowledgements: u64,
}
#[derive(Serialize)]
pub(crate) struct HostHealthObservation {
    pub host: Host,
    pub connection: Option<LiveChannelObservation>,
    pub runtime: HostRuntimeObservation,
    pub controller_boot_id: String,
    pub observed_at_ms: i64,
    pub observational: bool,
}
fn count(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(row.get::<_, i64>(column)?).map_err(|_| rusqlite::Error::InvalidQuery)
}
impl Store {
    pub(crate) fn host_health_observation(
        &self,
        actor: &str,
        workspace: &str,
        host: &str,
        channels: &Channels,
    ) -> Result<HostHealthObservation> {
        let tx = self.connection.unchecked_transaction()?;
        // host() checks current ManageHosts and the exact workspace/host scope.
        let definition = self
            .host(actor, workspace, host)?
            .ok_or(StoreError::Policy(
                dunlin_domain::collaboration::PolicyError::PermissionDenied,
            ))?;
        let runtime = self.connection.query_row(
            "SELECT
              (SELECT count(*) FROM controller_remote_executions WHERE workspace=?1 AND host=?2 AND phase<>'terminal'),
              (SELECT coalesce(sum(reservation),0) FROM controller_remote_sessions WHERE workspace=?1 AND host=?2),
              (SELECT count(*) FROM controller_remote_executions WHERE workspace=?1 AND host=?2 AND phase='unknown')+
              (SELECT count(*) FROM controller_remote_sessions WHERE workspace=?1 AND host=?2 AND reservation=1 AND (phase='interrupted' OR closing=1)),
              (SELECT count(*) FROM controller_remote_execution_outcomes o JOIN controller_remote_executions e ON e.attempt_id=o.attempt_id WHERE e.workspace=?1 AND e.host=?2 AND o.ack_pending=1)+
              (SELECT count(*) FROM controller_remote_sessions WHERE workspace=?1 AND host=?2 AND ack_sequence>confirmed_ack),
              (SELECT count(*) FROM controller_remote_sessions WHERE workspace=?1 AND host=?2 AND phase='active' AND closing=0),
              (SELECT count(*) FROM controller_remote_session_turns t JOIN controller_remote_sessions s ON s.session=t.session WHERE s.workspace=?1 AND s.host=?2 AND s.phase='active' AND s.closing=0 AND t.phase IN('sent','accepted'))",
            rusqlite::params![workspace, host],
            |row| Ok(HostRuntimeObservation {
                one_shot_reservations: count(row,0)?, session_reservations: count(row,1)?,
                uncertain_reservations: count(row,2)?, unconfirmed_acknowledgements: count(row,3)?,
                active_sessions: count(row,4)?, active_turns: count(row,5)?,
            }),
        )?;
        let observed_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .ok_or_else(|| StoreError::Limit("Host observation clock unavailable".into()))?;
        let connection = channels.live_channel_observation(self, host, observed_at_ms);
        let result = HostHealthObservation {
            host: definition,
            connection,
            runtime,
            controller_boot_id: channels.boot_id.clone(),
            observed_at_ms,
            observational: true,
        };
        tx.commit()?;
        Ok(result)
    }
}
