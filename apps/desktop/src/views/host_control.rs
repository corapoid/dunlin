//! Title-bar host switcher: the current machine, paired relays and the SSH
//! terminal entry point. The control stays visible in every view and mode.

use dunlin_ui::assets::IconName;
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::{Button, ButtonVariants};
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
                ),
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

    Button::new("title-host")
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
            menu = menu.label("Host · where the harness runs").item(item(
                &app,
                "Local host".into(),
                "This Mac · local projects".into(),
                IconName::Monitor,
                active_host.is_none(),
                busy,
                HostAction::Local,
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
                ))
        })
        .into_any_element()
}
