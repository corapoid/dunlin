//! Explicit connection sheet. Relay grants and SSH terminal credentials stay distinct.
use crate::app::DunlinApp;
use dunlin_ui::component::button::ButtonVariants as _;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::{
    component::{Disableable, input::InputState},
    *,
};

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
    pub ssh: bool,
    pub identity: Option<Entity<InputState>>,
    pub port: Option<Entity<InputState>>,
    pub error: Option<String>,
}
impl DunlinApp {
    pub(crate) fn open_remote_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation_blocked() || self.remote_loading {
            return;
        }
        self.settings_open = false;
        self.command_open = false;
        self.project_menu_open = false;
        self.remote_picker = None;
        self.ensure_ssh_inputs(window, cx);
        if self.remote_connection.identity.is_none() {
            self.remote_connection.identity = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("SSH agent / ~/.ssh/config (default)")
            }));
            self.remote_connection.port = Some(cx.new(|cx| {
                InputState::new(window, cx).placeholder("Port from SSH config (default)")
            }));
        }
        self.remote_code_input
            .update(cx, |input, cx| input.set_masked(true, window, cx));
        self.remote_connection.open = true;
        self.remote_connection.error = None;
        self.remote_url_input
            .update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    pub(crate) fn close_remote_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_loading {
            return;
        }
        self.remote_connection.open = false;
        self.remote_code_input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(identity) = &self.remote_connection.identity {
            identity.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }
    fn choose_ssh_identity(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose SSH private key".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = paths.await
                && let Some(path) = paths.first()
            {
                let value = path.to_string_lossy().into_owned();
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Some(input) = &this.remote_connection.identity {
                        input.update(cx, |state, cx| state.set_value(value, window, cx));
                    }
                });
            }
        })
        .detach();
    }
}
fn field(label: &'static str, state: &Entity<InputState>, p: Palette, busy: bool) -> Div {
    dunlin_ui::settings::field(
        label,
        div()
            .relative()
            .child(dunlin_ui::input(state).aria_label(label).disabled(busy))
            .children({
                #[cfg(debug_assertions)]
                {
                    if label == "Private key file (optional)" {
                        crate::workspace_ui_smoke::probe("ssh-key-input")
                    } else {
                        None
                    }
                }
                #[cfg(not(debug_assertions))]
                {
                    None::<AnyElement>
                }
            }),
        p,
    )
}
pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let ssh = this.remote_connection.ssh;
    let busy = this.remote_loading;
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(14.))
        .child(
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Connect to remote server"),
        )
        .child(
            segment_group(p)
                .child(
                    segment_button("connection-relay", "Dunlin relay", !ssh)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.remote_connection.ssh = false;
                            this.remote_connection.error = None;
                            this.remote_url_input
                                .update(cx, |input, cx| input.focus(window, cx));
                            cx.notify();
                        })),
                )
                .child(
                    segment_button("connection-ssh", "SSH", ssh)
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.remote_connection.ssh = true;
                            this.remote_connection.error = None;
                            if let Some(input) = &this.terminal.ssh_target {
                                input.update(cx, |input, cx| input.focus(window, cx));
                            }
                            cx.notify();
                        })),
                ),
        );
    if ssh {
        body = body.child(field("Endpoint · user@host or SSH config alias", this.terminal.ssh_target.as_ref().unwrap(), p, busy))
            .child(field("Port (optional)", this.remote_connection.port.as_ref().unwrap(), p, busy))
            .child(field("Remote working directory (optional)", this.terminal.ssh_path.as_ref().unwrap(), p, busy))
            .child(div().flex().items_end().gap(px(8.))
                .child(field("Private key file (optional)", this.remote_connection.identity.as_ref().unwrap(), p, busy).flex_1())
                .child(button("connection-key").self_end().label("Choose key…").disabled(busy)
                    .relative().children({
                        #[cfg(debug_assertions)] { crate::workspace_ui_smoke::probe("ssh-key-button") }
                        #[cfg(not(debug_assertions))] { None::<AnyElement> }
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.choose_ssh_identity(window, cx)))))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Uses OpenSSH, its agent and host verification. Passphrases/passwords are entered in the terminal and are never stored. SSH opens a terminal; remote agents and projects require a Dunlin relay."));
    } else {
        body = body.child(field("Endpoint · HTTPS relay URL", &this.remote_url_input, p, busy))
            .child(field("Login key · one-time relay pairing code", &this.remote_code_input, p, busy))
            .child(field("Local server alias (optional)", &this.remote_name_input, p, busy))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Run dunlin-relay pair <user> on the server. The issued device token is saved in the system Keychain, not in application settings."));
    }
    body = body
        .when_some(this.remote_connection.error.clone(), |body, error| {
            body.child(div().text_color(p.red).child(error))
        })
        .child(
            div()
                .flex()
                .justify_end()
                .gap(px(8.))
                .child(
                    button("connection-cancel")
                        .label("Cancel")
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.close_remote_connection(window, cx)
                        })),
                )
                .child(
                    button("connection-connect")
                        .primary()
                        .label(if busy { "Connecting…" } else { "Connect" })
                        .disabled(busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if this.remote_connection.ssh {
                                match this.open_ssh_terminal(window, cx) {
                                    Ok(()) => this.close_remote_connection(window, cx),
                                    Err(error) => {
                                        this.remote_connection.error = Some(error);
                                        cx.notify();
                                    }
                                }
                            } else {
                                this.pair_remote(window, cx);
                            }
                        })),
                ),
        );
    div()
        .id("remote-connection-overlay")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .on_click(cx.listener(|this, _, window, cx| this.close_remote_connection(window, cx)))
        .child(
            dialog_surface(p)
                .id("remote-connection-dialog")
                .w(px(560.))
                .max_w(relative(0.95))
                .max_h(relative(0.9))
                .child(crate::effects::panel_probe(
                    &this.overlay_backdrop,
                    SurfaceGeometry::DIALOG,
                    false,
                ))
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .id("remote-connection-content")
                        .overflow_y_scroll()
                        .child(body),
                ),
        )
        .into_any_element()
}
