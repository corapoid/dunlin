//! SSH host profiles, relay bootstrap and loopback tunnels.

use crate::app::DunlinApp;
use dunlin_domain::SshHost;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use std::path::PathBuf;

pub(crate) fn settings_section(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            dunlin_ui::settings::section("SSH hosts · this Mac", p),
        )
        .child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child("OpenSSH aliases from ~/.ssh/config. Host keys are verified by OpenSSH; Dunlin never disables StrictHostKeyChecking."),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    dunlin_ui::button("ssh-import-config")
                        .small()
                        .label("Import ~/.ssh/config")
                        .on_click(cx.listener(|this, _, _, cx| this.import_ssh_hosts(cx))),
                )
                .child(
                    dunlin_ui::button("ssh-refresh")
                        .ghost()
                        .small()
                        .label("Refresh")
                        .on_click(cx.listener(|this, _, _, cx| this.reload_ssh_hosts(cx))),
                ),
        );
    if this.ssh_hosts.is_empty() {
        body = body.child(
            dunlin_ui::settings::notice("No SSH hosts imported yet. Import aliases from your OpenSSH configuration to get started.", p),
        );
    }
    for host in this.ssh_hosts.clone() {
        let id = host.id.clone();
        let test_id = id.clone();
        let bootstrap_id = id.clone();
        let tunnel_id = id.clone();
        let remove_id = id.clone();
        let status = this.ssh_status.get(&id).cloned().unwrap_or_default();
        body = body.child(
            dunlin_ui::settings::card(p)
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .text_size(px(11.5))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(host.name.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(9.5))
                                        .font_family(dunlin_ui::theme::code_font())
                                        .text_color(p.muted_fg)
                                        .truncate()
                                        .child(format!(
                                            "{}@{}:{} · relay {}",
                                            host.user, host.target, host.port, host.relay_port
                                        )),
                                ),
                        )
                        .child(
                            dunlin_ui::button(format!("ssh-test:{test_id}"))
                                .ghost()
                                .small()
                                .label("Test")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.test_ssh_host(&test_id, cx);
                                })),
                        )
                        .child(
                            dunlin_ui::button(format!("ssh-bootstrap:{bootstrap_id}"))
                                .small()
                                .label("Bootstrap relay")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.bootstrap_ssh_host(&bootstrap_id, cx);
                                })),
                        )
                        .child(
                            dunlin_ui::button(format!("ssh-tunnel:{tunnel_id}"))
                                .ghost()
                                .small()
                                .label("Tunnel")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_ssh_tunnel(&tunnel_id, cx);
                                })),
                        )
                        .child(
                            dunlin_ui::button(format!("ssh-remove:{remove_id}"))
                                .ghost()
                                .small()
                                .label("Remove")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.remove_ssh_host(&remove_id, cx);
                                })),
                        ),
                )
                .when(!status.is_empty(), |row| {
                    row.child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .child(status),
                    )
                }),
        );
    }
    body.into_any_element()
}

impl DunlinApp {
    pub(crate) fn reload_ssh_hosts(&mut self, cx: &mut Context<Self>) {
        self.ssh_hosts = self
            .store
            .as_ref()
            .and_then(|store| store.ssh_hosts().ok())
            .unwrap_or_default();
        cx.notify();
    }

    pub(crate) fn import_ssh_hosts(&mut self, cx: &mut Context<Self>) {
        let Some(store) = self.store.as_ref() else {
            self.set_notice("Storage is unavailable", cx);
            return;
        };
        let aliases = match dunlin_ssh::import_aliases() {
            Ok(aliases) => aliases,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let mut imported = 0;
        let mut warnings = Vec::new();
        for alias in aliases {
            let id = format!("ssh:{alias}");
            if self.ssh_hosts.iter().any(|host| host.id == id) {
                continue;
            }
            match dunlin_ssh::resolve(&alias) {
                Ok(resolved) => {
                    let host = SshHost {
                        id,
                        name: alias.clone(),
                        target: alias,
                        user: if resolved.user.is_empty() {
                            std::env::var("USER").unwrap_or_else(|_| "root".into())
                        } else {
                            resolved.user
                        },
                        port: resolved.port,
                        identity: resolved.identity,
                        relay_port: 8787,
                        local_port: 0,
                        host_key: String::new(),
                        created_at: chrono::Local::now().timestamp(),
                    };
                    if let Err(error) = store.upsert_ssh_host(&host) {
                        warnings.push(error.to_string());
                    } else {
                        imported += 1;
                    }
                }
                Err(error) => warnings.push(format!("{alias}: {error}")),
            }
        }
        self.reload_ssh_hosts(cx);
        self.set_notice(
            if warnings.is_empty() {
                format!("Imported {imported} SSH host(s)")
            } else {
                format!(
                    "Imported {imported} SSH host(s); {} warning(s): {}",
                    warnings.len(),
                    warnings.join(" · ")
                )
            },
            cx,
        );
    }

    pub(crate) fn test_ssh_host(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(host) = self.ssh_hosts.iter().find(|host| host.id == id).cloned() else {
            return;
        };
        let askpass = askpass_path();
        let id = id.to_string();
        self.ssh_status.insert(id.clone(), "Testing…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { dunlin_ssh::ensure_master(&host, askpass.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.ssh_status.insert(
                    id,
                    match result {
                        Ok(()) => "Master connection is up".into(),
                        Err(error) => error,
                    },
                );
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn bootstrap_ssh_host(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(host) = self.ssh_hosts.iter().find(|host| host.id == id).cloned() else {
            return;
        };
        let candidates = relay_binary_candidates();
        if !candidates.iter().any(|path| path.is_file()) {
            self.set_notice(
                "Place a matching dunlin-relay binary in dist/ or target/release before bootstrapping",
                cx,
            );
            return;
        }
        let askpass = askpass_path();
        let id = id.to_string();
        self.ssh_status
            .insert(id.clone(), "Bootstrapping relay…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let binary = dunlin_ssh::service::select_relay_binary(
                        &host,
                        &candidates,
                        askpass.as_deref(),
                    )?;
                    let bootstrap = dunlin_ssh::bootstrap(&host, &binary, askpass.as_deref())?;
                    let local_port = bootstrap.tunnel.local_port;
                    let url = format!("http://127.0.0.1:{local_port}");
                    let pair =
                        crate::remote::pair(&url, &bootstrap.pairing_code, "Dunlin via SSH")?;
                    Ok::<_, String>((host, url, pair, bootstrap.tunnel))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok((host, url, pair, tunnel)) => {
                        let host_id = host.id.clone();
                        if let Err(error) = crate::remote::save_token(&host_id, &pair.token) {
                            this.ssh_status.insert(id, error);
                            cx.notify();
                            return;
                        }
                        this.remote_tokens.insert(host_id.clone(), pair.token);
                        this.ssh_tunnels.insert(id.clone(), tunnel);
                        if !this.remote_hosts.iter().any(|remote| remote.id == host_id) {
                            this.remote_hosts.push(crate::remote::RemoteHost {
                                id: host_id,
                                name: format!("{} (SSH)", host.name),
                                url,
                                user_name: pair.user.name,
                                user_role: pair.user.role,
                                device_id: pair.device_id,
                            });
                        }
                        this.persist_settings(cx);
                        this.ssh_status
                            .insert(id, "Relay installed, paired and tunnel-ready".into());
                    }
                    Err(error) => {
                        this.ssh_status.insert(id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn open_ssh_tunnel(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(host) = self.ssh_hosts.iter().find(|host| host.id == id).cloned() else {
            return;
        };
        let askpass = askpass_path();
        let id = id.to_string();
        self.ssh_status.insert(id.clone(), "Opening tunnel…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { dunlin_ssh::start_tunnel(&host, askpass.as_deref()) })
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(tunnel) => {
                        let port = tunnel.local_port;
                        this.ssh_tunnels.insert(id.clone(), tunnel);
                        if let Some(remote) =
                            this.remote_hosts.iter_mut().find(|remote| remote.id == id)
                        {
                            remote.url = format!("http://127.0.0.1:{port}");
                        }
                        this.ssh_status
                            .insert(id, format!("Tunnel up on http://127.0.0.1:{port}"));
                    }
                    Err(error) => {
                        this.ssh_status.insert(id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn remove_ssh_host(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(host) = self.ssh_hosts.iter().find(|host| host.id == id).cloned() {
            dunlin_ssh::exit_master(&host);
        }
        self.ssh_tunnels.remove(id);
        if let Some(store) = self.store.as_ref()
            && let Err(error) = store.delete_ssh_host(id)
        {
            self.set_notice(format!("Could not remove SSH host: {error}"), cx);
            return;
        }
        self.ssh_status.remove(id);
        self.reload_ssh_hosts(cx);
        self.set_notice("SSH host removed", cx);
    }
}

fn askpass_path() -> Option<PathBuf> {
    let path = std::env::current_exe()
        .ok()?
        .parent()?
        .join("dunlin-ssh-askpass");
    path.is_file().then_some(path)
}

fn relay_binary_candidates() -> Vec<PathBuf> {
    let mut candidates = vec![
        PathBuf::from("dist/relay/linux-arm64/dunlin-relay"),
        PathBuf::from("dist/relay/linux-x86_64/dunlin-relay"),
        PathBuf::from("dist/relay/macos-arm64/dunlin-relay"),
        PathBuf::from("dist/dunlin-relay"),
        PathBuf::from("target/release/dunlin-relay"),
        PathBuf::from("target/debug/dunlin-relay"),
    ];
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        candidates.push(parent.join("dunlin-relay"));
    }
    candidates
}
