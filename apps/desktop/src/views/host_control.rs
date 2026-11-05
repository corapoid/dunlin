//! Title-bar host switcher: the current machine, paired relays and the SSH
//! terminal entry point. The control stays visible in every view and mode.

use dunlin_ui::assets::IconName;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;

use crate::app::{DunlinApp, SettingsTab};

#[derive(Clone)]
enum HostAction {
    Local,
    Remote(String),
    Ssh,
    ManageRemote,
}

fn item(
    app: &WeakEntity<DunlinApp>,
    label: String,
    detail: String,
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
                .min_w(px(240.))
                .max_w(px(360.))
                .py(px(4.))
                .flex()
                .flex_col()
                .gap(px(3.))
                .opacity(if disabled { 0.5 } else { 1.0 })
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(p.fg)
                        .child(label.clone()),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(relative(1.4))
                        .text_color(p.muted_fg)
                        .child(detail.clone()),
                )
                .when_some(status.clone(), |item, (label, detail, icon, color)| {
                    let row = div().relative().flex().items_center().gap(px(5.)).text_size(px(11.)).text_color(color)
                        .child(component::Icon::new(icon).size(px(12.))).child(label);
                    #[cfg(debug_assertions)]
                    let row = row.children(crate::workspace_ui_smoke::probe("selected-host-status"));
                    item.child(row).child(div().text_size(px(10.5)).text_color(p.muted_fg).child(detail))
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
                if this.project().is_remote() {
                    this.open_folder(cx);
                }
            }
            HostAction::Remote(id) => {
                if this
                    .project()
                    .remote
                    .as_ref()
                    .map(|project| &project.host_id)
                    != Some(id)
                {
                    this.open_remote_picker_for(id, cx);
                }
            }
            HostAction::Ssh => this.open_ssh_terminal_setup(window, cx),
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
    let remote = this.project().remote.as_ref();
    let host_name = remote
        .and_then(|project| this.remote_host(&project.host_id))
        .map(|host| host.name)
        .unwrap_or_else(|| "Local host".into());
    let detail = match &this.harness.status {
        crate::harness::HarnessStatus::Ready { agent, .. } => format!("Connected · {agent}"),
        crate::harness::HarnessStatus::Starting => "Connecting…".into(),
        crate::harness::HarnessStatus::Failed(_) => "Connection failed".into(),
        crate::harness::HarnessStatus::Exited(_) => "Connection ended".into(),
        crate::harness::HarnessStatus::Offline => "Not connected".into(),
    };
    let dot = status_color(this, &p);
    let app = cx.entity().downgrade();
    let error = matches!(
        this.harness.status,
        crate::harness::HarnessStatus::Failed(_) | crate::harness::HarnessStatus::Exited(_)
    );

    let control = dunlin_ui::button("title-host")
        .ghost()
        .small()
        .icon(if remote.is_some() {
            IconName::Cloud
        } else {
            IconName::Monitor
        })
        .child(
            div()
                .size(px(5.5))
                .flex_shrink_0()
                .rounded_full()
                .bg(dot)
                .when(error, |dot| dot.border_1().border_color(p.red)),
        )
        .accessibility_label(format!("Host: {host_name}. {detail}"))
        .tooltip(format!("Host · {host_name} · {detail}"))
        .dropdown_menu_with_anchor(Anchor::BottomRight, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let this = entity.read(cx);
            let busy = this.chat_controls_busy();
            let active_host = this
                .project()
                .remote
                .as_ref()
                .map(|project| project.host_id.as_str());
            let (label, icon) = crate::harness::status_label(&this.harness, this.session().status == dunlin_domain::SessionStatus::Waiting);
            let color = match label { "Working" | "Ready" => palette(cx).green, "Waiting for permission" => palette(cx).amber, "Connection failed" => palette(cx).red, _ => palette(cx).muted_fg };
            let status = (label.to_string(), this.notice.clone().or_else(|| this.harness.status.detail()).unwrap_or_default(), icon, color);
            menu = menu.label("Host · where the harness runs").item(item(
                &app,
                "Local host".into(),
                "This Mac · local projects".into(),
                IconName::Monitor,
                active_host.is_none(),
                busy,
                HostAction::Local,
                active_host.is_none().then(|| status.clone()),
            ));
            if this.remote_hosts.is_empty() {
                menu = menu.item(item(
                    &app,
                    "Remote server…".into(),
                    "Pair a self-hosted relay".into(),
                    IconName::Cloud,
                    false,
                    false,
                    HostAction::ManageRemote,
                    None,
                ));
            } else {
                for host in &this.remote_hosts {
                    menu = menu.item(item(
                        &app,
                        format!("Remote server · {}", host.name),
                        host.url.clone(),
                        IconName::Cloud,
                        active_host == Some(host.id.as_str()),
                        busy,
                        HostAction::Remote(host.id.clone()),
                        (active_host == Some(host.id.as_str())).then(|| status.clone()),
                    ));
                }
            }
            menu.separator()
                .label("SSH")
                .item(item(
                    &app,
                    "SSH…".into(),
                    "Open an OpenSSH terminal · harness stays local in this build".into(),
                    IconName::Terminal,
                    false,
                    false,
                    HostAction::Ssh,
                    None,
                ))
                .separator()
                .label("Changing host opens a project there")
                .item(item(
                    &app,
                    "Manage remote environments…".into(),
                    "Pair or remove a self-hosted relay".into(),
                    IconName::Settings2,
                    false,
                    false,
                    HostAction::ManageRemote,
                    None,
                ))
        });
    #[cfg(debug_assertions)]
    let control = div().relative().child(control).children(crate::workspace_ui_smoke::probe("host-menu-button"));
    control.into_any_element()
}
