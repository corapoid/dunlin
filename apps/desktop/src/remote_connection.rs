//! Pair a relay through private Tailscale HTTPS. No SSH/bootstrap/tunnel path.
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{Disableable, button::ButtonVariants as _, input::InputState},
    prelude::FluentBuilder as _,
    *,
};

#[derive(Default)]
pub(crate) struct State {
    pub open: bool,
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
        cx.notify();
    }
}
fn field(
    label: &'static str,
    state: &Entity<InputState>,
    p: Palette,
    busy: bool,
    marker: &'static str,
) -> Div {
    dunlin_ui::settings::field(
        label,
        div()
            .relative()
            .child(dunlin_ui::input(state).aria_label(label).disabled(busy))
            .children(crate::gui_probe::marker(marker))
            .children({
                #[cfg(debug_assertions)]
                {
                    crate::workspace_ui_smoke::probe(marker)
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
    let busy = this.remote_loading;
    let body=div().flex().flex_col().gap(px(14.))
        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child("Connect through Tailscale"))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Join the same tailnet as your server. Use its private Tailscale Serve HTTPS address; SSH connections and tunnels are no longer supported."))
        .child(field("Tailscale HTTPS endpoint",&this.remote_url_input,p,busy,"remote-e2e-url"))
        .child(field("Login key · one-time relay pairing code",&this.remote_code_input,p,busy,"remote-e2e-code"))
        .child(field("Local server alias (optional)",&this.remote_name_input,p,busy,"remote-e2e-alias"))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Run dunlin-relay pair <user> on the server. The device token stays in this Mac's Keychain. Tailscale controls network access; relay pairing controls application access."))
        .when_some(this.remote_connection.error.clone(),|body,error|body.child(div().text_color(p.red).child(error)))
        .child(div().flex().justify_end().gap(px(8.))
            .child(action_button("connection-cancel").label("Cancel").disabled(busy).on_click(cx.listener(|this,_,window,cx|this.close_remote_connection(window,cx))))
            .child(action_button("connection-connect").relative().children(crate::gui_probe::marker("remote-e2e-connect")).primary()
                .label(if busy{"Connecting…"}else{"Connect"}).disabled(busy).on_click(cx.listener(|this,_,window,cx|this.pair_remote(window,cx)))));
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
