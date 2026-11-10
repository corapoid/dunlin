//! Shared-host presence, independent of the selected project and ACP runtime.
use crate::{
    app::DunlinApp,
    remote::{self, RemoteHost},
};
use dunlin_domain::presence::{HEARTBEAT_SECS, Snapshot};
use dunlin_ui::*;
use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
pub(crate) mod smoke;
mod state;
mod transport;
pub(crate) mod view;
pub(crate) use state::{Avatar, State, TRANSITION, initials};
const MAX_REQUESTS: usize = 4;

fn heartbeat(host: &RemoteHost) -> Result<Snapshot, String> {
    remote::validate_relay_url(&host.url)?;
    let token = remote::load_token(&host.id)?.ok_or("Pair this server again to restore access")?;
    transport::heartbeat(&remote::http_agent(), &host.url, &token)
}

impl DunlinApp {
    /// Called by the existing app timer; requests do not depend on an open map
    /// or chat, and no network/Keychain access runs on the render thread.
    pub(crate) fn poll_host_presence(&mut self, cx: &mut Context<Self>) {
        if self.factory_resetting {
            return;
        }
        let now = Instant::now();
        let mut changed = self.connection_map.layout.reconcile(
            self.remote_hosts
                .iter()
                .map(|host| host.id.clone())
                .collect(),
            now,
        );
        self.connection_map
            .hosts
            .retain(|id, _| self.remote_hosts.iter().any(|host| &host.id == id));
        let mut available = MAX_REQUESTS.saturating_sub(
            self.connection_map
                .hosts
                .values()
                .filter(|host| host.ticket.is_some())
                .count(),
        );
        let mut hosts = self.remote_hosts.clone();
        // Prefer never-checked / oldest hosts so slow or unreachable servers
        // cannot starve later entries when the concurrency limit is reached.
        hosts.sort_by_key(|host| {
            self.connection_map
                .hosts
                .get(&host.id)
                .and_then(|state| state.checked)
        });
        for host in hosts {
            let state = self
                .connection_map
                .hosts
                .entry(host.id.clone())
                .or_default();
            changed |= state.maintain(now);
            if available == 0
                || state.ticket.is_some()
                || state.checked.is_some_and(|checked| {
                    now.duration_since(checked) < Duration::from_secs(HEARTBEAT_SECS)
                })
            {
                continue;
            }
            available -= 1;
            let ticket = uuid::Uuid::now_v7().to_string();
            state.ticket = Some(ticket.clone());
            state.checked = Some(now);
            cx.spawn(async move |this, cx| {
                let request_host = host.clone();
                let result = cx
                    .background_executor()
                    .spawn(async move { heartbeat(&request_host) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    // Never attach a late response to a removed/re-paired host.
                    if !this.remote_hosts.iter().any(|current| current == &host) {
                        if let Some(state) = this.connection_map.hosts.get_mut(&host.id)
                            && state.ticket.as_deref() == Some(&ticket)
                        {
                            state.ticket = None;
                        }
                        return;
                    }
                    let Some(state) = this.connection_map.hosts.get_mut(&host.id) else {
                        return;
                    };
                    if state.ticket.as_deref() != Some(&ticket) {
                        return;
                    }
                    state.apply(result, Instant::now());
                    if this.settings_open && this.settings_tab == crate::app::SettingsTab::Remote {
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        if changed {
            if self.settings_open && self.settings_tab == crate::app::SettingsTab::Remote {
                cx.notify();
            }
        }
    }
}
