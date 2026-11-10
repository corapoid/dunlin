//! Sidebar connection status and host switcher, including paired relays and SSH.

use dunlin_ui::assets::IconName;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};

#[derive(Clone)]
enum HostAction {
    Local,
    Remote(String),
    Connect,
    ManageRemote,
}

fn item(
    app: &WeakEntity<DunlinApp>,
    (label, detail): (String, String),
    icon: IconName,
    selected: bool,
    disabled: bool,
    action: HostAction,
    status: Option<(String, String, IconName, Hsla)>,
) -> PopupMenuItem {
    let app = app.clone();
    PopupMenuItem::element(move |window, cx| {
        let p = palette(cx);
        crate::effects::menu_surface(
            div()
                .w(px(260.))
                .py(px(6.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .opacity(if disabled { 0.5 } else { 1.0 })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(13.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(p.fg)
                                .child(label.clone()),
                        )
                        .when_some(status.clone(), |item, (label, _, _, color)| {
                            let badge = div()
                                .relative()
                                .flex()
                                .items_center()
                                .gap(px(5.))
                                .text_size(px(10.5))
                                .text_color(color)
                                .child(div().size(px(5.)).rounded_full().bg(color))
                                .child(label);
                            #[cfg(debug_assertions)]
                            let badge = badge
                                .children(crate::workspace_ui_smoke::probe("selected-host-status"));
                            item.child(badge)
                        }),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(relative(1.4))
                        .text_color(p.muted_fg)
                        .child(detail.clone()),
                )
                .when_some(status.clone(), |item, (_, detail, _, _)| {
                    item.when(!detail.is_empty(), |item| {
                        item.child(
                            div()
                                .text_size(px(11.))
                                .line_height(relative(1.4))
                                .text_color(p.muted_fg)
                                .child(detail),
                        )
                    })
                }),
            window,
            cx,
        )
    })
    .icon(icon)
    .checked(selected)
    .disabled(disabled)
    .on_click(move |_, window, cx| {
        let _ = app.update(cx, |this, cx| match &action {
            HostAction::Local => {
                this.settings_open = false;
                if this.project().is_remote() {
                    this.open_folder(cx);
                }
            }
            HostAction::Remote(id) => {
                this.manage_server(id, window, cx);
            }
            HostAction::Connect => this.open_remote_connection(window, cx),
            HostAction::ManageRemote => {
                this.settings_tab = SettingsTab::Remote;
                this.settings_open = true;
                cx.notify();
            }
        });
    })
}

fn status_color(this: &DunlinApp, p: &Palette) -> Hsla {
    if this.projects.is_empty() {
        return p.muted_fg;
    }
    match &this.harness.status {
        crate::harness::HarnessStatus::Ready { .. } => p.green,
        crate::harness::HarnessStatus::Starting => p.amber,
        crate::harness::HarnessStatus::Failed(_) | crate::harness::HarnessStatus::Exited(_) => {
            p.red
        }
        crate::harness::HarnessStatus::Offline => p.muted_fg,
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let selected_host = crate::server_panel::selected_host(this);
    let managing =
        this.settings_open && this.server_panel.host == selected_host && selected_host.is_some();
    let host_name = selected_host
        .as_deref()
        .and_then(|id| this.remote_host(id))
        .map(|host| host.name)
        .unwrap_or_else(|| "Local host".into());
    let detail = if managing {
        if this.server_panel.loading {
            "Connecting to relay…".into()
        } else if let Some(info) = &this.server_panel.info {
            format!("Connected to {} · server settings", info.name)
        } else {
            "Relay disconnected · open server settings".into()
        }
    } else {
        match &this.harness.status {
            crate::harness::HarnessStatus::Ready { agent, .. } => format!("Connected · {agent}"),
            crate::harness::HarnessStatus::Starting => "Connecting…".into(),
            crate::harness::HarnessStatus::Failed(_) => "Connection failed".into(),
            crate::harness::HarnessStatus::Exited(_) => "Connection ended".into(),
            crate::harness::HarnessStatus::Offline => "Not connected".into(),
        }
    };
    let dot = if managing {
        if this.server_panel.info.is_some() {
            p.green
        } else {
            p.muted_fg
        }
    } else {
        status_color(this, &p)
    };
    let app = cx.entity().downgrade();
    let error = matches!(
        this.harness.status,
        crate::harness::HarnessStatus::Failed(_) | crate::harness::HarnessStatus::Exited(_)
    );

    let control = dunlin_ui::button("sidebar-host")
        .ghost()
        .w_full()
        .min_w(px(0.))
        .h(px(32.))
        .px(px(10.))
        .justify_start()
        .rounded(px(dunlin_ui::radii::CONTROL))
        .child(
            div()
                .relative()
                .size(px(16.))
                .flex_shrink_0()
                .child(
                    component::Icon::new(if selected_host.is_some() {
                        IconName::Cloud
                    } else {
                        IconName::Monitor
                    })
                    .size(px(16.)),
                )
                .child(
                    div()
                        .absolute()
                        .right(-px(1.))
                        .bottom_0()
                        .size(px(5.5))
                        .rounded_full()
                        .bg(dot)
                        .when(error, |dot| dot.border_1().border_color(p.red))
                        .children({
                            #[cfg(debug_assertions)]
                            {
                                crate::workspace_ui_smoke::probe("host-status-dot")
                            }
                            #[cfg(not(debug_assertions))]
                            {
                                None::<AnyElement>
                            }
                        }),
                )
                .children({
                    #[cfg(debug_assertions)]
                    {
                        crate::workspace_ui_smoke::probe("host-icon")
                    }
                    #[cfg(not(debug_assertions))]
                    {
                        None::<AnyElement>
                    }
                }),
        )
        .child(
            div().flex_1().min_w(px(0.)).child(
                div()
                    .text_size(px(11.5))
                    .truncate()
                    .child(host_name.clone()),
            ),
        )
        .child(
            component::Icon::new(IconName::ChevronsUpDown)
                .size(px(12.))
                .text_color(p.muted_fg),
        )
        .accessibility_label(format!("Host: {host_name}. {detail}"))
        .tooltip(format!("Host · {host_name} · {detail}"))
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let busy = this.chat_controls_busy();
            let selected_host = crate::server_panel::selected_host(this);
            let active_host = selected_host.as_deref();
            let (label, icon) = crate::harness::status_label(
                &this.harness,
                this.session().status == dunlin_domain::SessionStatus::Waiting,
            );
            let color = match label {
                "Working" | "Ready" => palette(cx).green,
                "Waiting for permission" => palette(cx).amber,
                "Connection failed" => palette(cx).red,
                _ => palette(cx).muted_fg,
            };
            let detail = match this.harness.status {
                crate::harness::HarnessStatus::Failed(_) => {
                    "Open the conversation for connection details".into()
                }
                crate::harness::HarnessStatus::Exited(_) => {
                    "Reconnect from the conversation".into()
                }
                _ => this.harness.status.detail().unwrap_or_default(),
            };
            let status = (label.to_string(), detail, icon, color);
            menu = menu.label("Run agent on").item(item(
                &app,
                ("Local host".into(), "This Mac · local projects".into()),
                IconName::Monitor,
                active_host.is_none(),
                busy,
                HostAction::Local,
                active_host.is_none().then(|| status.clone()),
            ));
            if this.remote_hosts.is_empty() {
                menu = menu.item(item(
                    &app,
                    ("Remote server…".into(), "Pair a self-hosted relay".into()),
                    IconName::Cloud,
                    false,
                    false,
                    HostAction::Connect,
                    None,
                ));
            } else {
                for host in &this.remote_hosts {
                    menu = menu.item(item(
                        &app,
                        (format!("Remote server · {}", host.name), host.url.clone()),
                        IconName::Cloud,
                        active_host == Some(host.id.as_str()),
                        busy,
                        HostAction::Remote(host.id.clone()),
                        (active_host == Some(host.id.as_str())).then(|| status.clone()),
                    ));
                }
            }
            menu.separator()
                .item(item(
                    &app,
                    (
                        "Connect to server…".into(),
                        "Endpoint and credentials".into(),
                    ),
                    IconName::Cloud,
                    false,
                    false,
                    HostAction::Connect,
                    None,
                ))
                .separator()
                .item(item(
                    &app,
                    (
                        "Manage remote environments…".into(),
                        "Pair or remove a self-hosted relay".into(),
                    ),
                    IconName::Settings2,
                    false,
                    false,
                    HostAction::ManageRemote,
                    None,
                ))
        });
    #[cfg(debug_assertions)]
    let control = div()
        .relative()
        .w_full()
        .child(control)
        .children(crate::workspace_ui_smoke::probe("host-menu-button"));
    control.into_any_element()
}
