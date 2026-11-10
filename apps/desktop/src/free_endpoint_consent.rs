//! Local authorization to use OpenCode Free endpoints. Console's organization
//! privacy setting is a separate provider-owned control, not a local CLI option.
use crate::app::DunlinApp;
use dunlin_ui::{
    component::{Disableable, button::ButtonVariants},
    *,
};

pub(crate) const ACCOUNT_URL: &str = "https://opencode.ai/console";
pub(crate) const PRIVACY_URL: &str = "https://opencode.ai/docs/zen#privacy";
pub(crate) const DISCLOSURE: &str = "Some free endpoints may retain prompts, code and responses or use them to improve or train models. Policies differ by model. Do not send confidential or personal data unless its provider policy permits it.";
pub(crate) const SCOPE: &str = "This is Dunlin’s local permission on this Mac. It does not change or verify your OpenCode Console account or organization setting. If the provider requires ‘Allow free endpoints that train on request data’, enable it separately in Console. Dunlin has no documented API for that account setting.";

pub(crate) struct Dialog {
    pub accepted: bool,
    /// Only a provider-picker action can select Free after confirmation. A
    /// settings-only dialog must not change the current project or provider.
    target: Option<(String, String)>,
    pub focus: FocusHandle,
}

impl DunlinApp {
    pub(crate) fn open_free_endpoint_consent(
        &mut self,
        select_provider: bool,
        cx: &mut Context<Self>,
    ) {
        if self.free_endpoint_dialog.is_some() {
            return;
        }
        self.free_endpoint_dialog = Some(Dialog {
            accepted: false,
            target: select_provider.then(|| (self.project().id.clone(), self.session().id.clone())),
            focus: cx.focus_handle(),
        });
        self.chat_menu_open = false;
        cx.notify();
    }

    pub(crate) fn close_free_endpoint_consent(&mut self, cx: &mut Context<Self>) {
        self.free_endpoint_dialog = None;
        cx.notify();
    }

    pub(crate) fn confirm_free_endpoint_consent(&mut self, cx: &mut Context<Self>) {
        let Some(dialog) = &self.free_endpoint_dialog else {
            return;
        };
        if !dialog.accepted {
            return;
        }
        let target = dialog.target.clone();
        if target.as_ref().is_some_and(|(project, session)| {
            self.project().id != *project || self.session().id != *session
        }) {
            self.close_free_endpoint_consent(cx);
            self.set_notice(
                "The conversation changed. Choose the provider again to confirm consent.",
                cx,
            );
            return;
        }
        self.opencode_free_consent = true;
        self.free_endpoint_dialog = None;
        self.persist_settings(cx);
        if target.is_some() {
            self.select_provider(crate::providers::FREE, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_free_endpoint_consent(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if enabled {
            self.open_free_endpoint_consent(false, cx);
        } else {
            self.opencode_free_consent = false;
            self.persist_settings(cx);
            cx.notify();
        }
    }
}

pub(crate) fn settings(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    dunlin_ui::settings::card(p).p(px(16.)).flex().flex_col().gap(px(12.))
        .relative().children(probe("free-endpoint-settings-card"))
        .child(dunlin_ui::settings::section("OpenCode Free privacy", p))
        .child(dunlin_ui::settings::row("Allow OpenCode Free endpoints", DISCLOSURE,
            dunlin_ui::switch("free-endpoint-settings", "Allow OpenCode Free endpoints", this.opencode_free_consent)
                .on_click(cx.listener(|this, checked: &bool, _, cx| this.set_free_endpoint_consent(*checked, cx)))
                .into_any_element(), p))
        .child(dunlin_ui::settings::description(SCOPE, p))
        .child(dunlin_ui::settings::description("Turning this off blocks new Free requests in Dunlin; it cannot recall already shared data or revoke consent in the provider account.", p))
        .child(dunlin_ui::settings::actions()
            .child(dunlin_ui::settings::button("free-account-settings").label("Open provider account settings…").on_click(|_, _, cx| cx.open_url(ACCOUNT_URL)))
            .child(dunlin_ui::toolbar_button("free-privacy-policy").label("Read provider privacy policies").on_click(|_, _, cx| cx.open_url(PRIVACY_URL))))
        .into_any_element()
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(dialog) = &this.free_endpoint_dialog else {
        return div().into_any_element();
    };
    let content = div().w_full().min_w(px(0.)).flex_shrink_0().p(px(16.)).flex().flex_col().gap(px(12.))
            .child(div().text_size(px(dunlin_ui::settings::Tokens::TITLE_SIZE)).font_weight(FontWeight::SEMIBOLD).child("Before using OpenCode Free"))
            .child(dunlin_ui::settings::description(DISCLOSURE, p))
            .child(dunlin_ui::settings::notice(SCOPE, p))
            .child(div().relative().children(probe("free-endpoint-agree")).child(dunlin_ui::switch("free-endpoint-agree", "Allow Free endpoints that may retain data or use it for training", dialog.accepted)
                .label("I agree to use these Free endpoints in Dunlin")
                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                    if let Some(dialog) = &mut this.free_endpoint_dialog { dialog.accepted = *checked; }
                    cx.notify();
                }))))
            .child(dunlin_ui::settings::description("No prompt is sent by this action. You can revoke Dunlin’s permission in Settings → Agents → OpenCode Free privacy.", p))
            .child(dunlin_ui::settings::actions()
                .child(dunlin_ui::settings::button("free-endpoint-account").label("Open provider settings…").on_click(|_, _, cx| cx.open_url(ACCOUNT_URL)))
                .child(dunlin_ui::toolbar_button("free-endpoint-policy").label("Privacy policies").on_click(|_, _, cx| cx.open_url(PRIVACY_URL))))
            .child(dunlin_ui::settings::actions()
                .child(div().flex_1())
                .child(dunlin_ui::settings::button("free-endpoint-cancel").label("Cancel").relative().children(probe("free-endpoint-cancel"))
                    .on_click(cx.listener(|this, _, _, cx| this.close_free_endpoint_consent(cx))))
                .child(dunlin_ui::settings::button("free-endpoint-confirm").primary().label("Save consent").disabled(!dialog.accepted)
                    .relative().children(probe("free-endpoint-confirm"))
                    .on_click(cx.listener(|this, _, _, cx| this.confirm_free_endpoint_consent(cx)))));
    div()
        .absolute()
        .inset_0()
        .size_full()
        .p(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .child(
            dialog_surface(p)
                .id("free-endpoint-consent")
                .relative()
                .w(px(560.))
                .max_w_full()
                .max_h(relative(0.9))
                .overflow_y_scroll()
                .p(px(0.))
                .gap(px(0.))
                .role(Role::Dialog)
                .aria_label("OpenCode Free data privacy")
                .track_focus(&dialog.focus)
                .children(probe("free-endpoint-consent"))
                .child(crate::effects::panel_probe(
                    &this.overlay_backdrop,
                    SurfaceGeometry::DIALOG,
                    true,
                ))
                .child(content),
        )
        .into_any_element()
}

fn probe(name: &'static str) -> Option<AnyElement> {
    #[cfg(debug_assertions)]
    {
        crate::workspace_ui_smoke::probe(name)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = name;
        None
    }
}
