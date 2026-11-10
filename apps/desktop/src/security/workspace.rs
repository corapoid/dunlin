//! Project audit execution and evidence, deliberately separate from tool setup.
use super::*;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::component::input::InputState;
use dunlin_ui::component::menu::{DropdownMenu, PopupMenuItem};
use std::time::Instant;

#[derive(Default)]
pub(crate) struct State {
    scope: Option<(String, String)>,
    pub harness: String,
    pub model: Option<Entity<InputState>>,
    pending: Option<Pending>,
    pub error: Option<String>,
    models: std::collections::HashMap<String, Vec<(String, String)>>,
    model_request: Option<uuid::Uuid>,
}

struct Pending {
    project: String,
    root: String,
    session: String,
    harness: String,
    model: String,
    provider: Option<String>,
    prompt: String,
    started: Instant,
    model_requested: bool,
}

fn model_scope(this: &DunlinApp, harness: &str) -> String {
    if let Some(remote) = &this.project().remote {
        format!("remote:{}:{}:{harness}", remote.host_id, this.project().id)
    } else {
        format!("local:{harness}")
    }
}

fn recent_in_scope<'a>(
    recents: &'a [crate::providers::RecentModel],
    scope: &str,
) -> Option<&'a str> {
    recents
        .iter()
        .find(|entry| entry.scope == scope)
        .map(|entry| entry.id.as_str())
}

fn recent_model(this: &DunlinApp, harness: &str) -> Option<String> {
    recent_in_scope(&this.recent_models, &model_scope(this, harness)).map(str::to_owned)
}

impl DunlinApp {
    fn ensure_audit_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let scope = (self.project().id.clone(), self.project().path.clone());
        if self.security.workspace.scope.as_ref() == Some(&scope) {
            return;
        }
        let harness = if self.project().is_remote() {
            self.session_harness()
        } else {
            self.default_harness.clone()
        };
        let model = recent_model(self, &harness).unwrap_or_default();
        let input = cx
            .new(|cx| InputState::new(window, cx).placeholder("Harness default (no recent model)"));
        input.update(cx, |state, cx| state.set_value(model, window, cx));
        cx.subscribe(
            &input,
            |_, _, _: &dunlin_ui::component::input::InputEvent, cx| cx.notify(),
        )
        .detach();
        self.security.workspace.scope = Some(scope);
        self.security.workspace.harness = harness;
        self.security.workspace.model = Some(input);
        self.security.workspace.error = None;
        self.security.workspace.models.clear();
        self.security.workspace.model_request = None;
    }

    fn load_audit_models(&mut self, cx: &mut Context<Self>) {
        if self.security.workspace.model_request.is_some() || self.project().is_remote() {
            return;
        }
        let harness = self.security.workspace.harness.clone();
        let mut config = match self.local_harness_config(&harness) {
            Ok(config) => config,
            Err(error) => {
                self.security.workspace.error = Some(error);
                cx.notify();
                return;
            }
        };
        // Catalog discovery is an empty ACP session scoped only to this project.
        // It never inherits a previously selected group chat's extra roots.
        config.additional_directories.clear();
        let project = self.project().id.clone();
        let root = self.project().path.clone();
        let request = uuid::Uuid::now_v7();
        self.security.workspace.model_request = Some(request);
        cx.spawn(async move |this, cx| {
            let adapter = dunlin_agent::AcpHarness::spawn(config);
            let read = async {
                while let Ok(event) = adapter.events().recv().await {
                    match event {
                        dunlin_agent::HarnessEvent::Models {
                            state: Some(models),
                            ..
                        } => {
                            return Ok(models
                                .choices
                                .into_iter()
                                .map(|model| (model.id, model.name))
                                .collect::<Vec<_>>());
                        }
                        dunlin_agent::HarnessEvent::Models { state: None, .. } => {
                            return Ok(Vec::new());
                        }
                        dunlin_agent::HarnessEvent::Error { message }
                        | dunlin_agent::HarnessEvent::Exited { reason: message } => {
                            return Err(message);
                        }
                        dunlin_agent::HarnessEvent::PermissionRequested { request_id, .. } => {
                            adapter.respond_permission(request_id, None);
                        }
                        _ => {}
                    }
                }
                Err("Harness closed before advertising its models".into())
            };
            let read = std::pin::pin!(read);
            let timeout = std::pin::pin!(cx.background_executor().timer(Duration::from_secs(25)));
            let result = match futures_util::future::select(read, timeout).await {
                futures_util::future::Either::Left((result, _)) => result,
                futures_util::future::Either::Right(_) => {
                    Err("Model discovery timed out; no audit prompt was sent".into())
                }
            };
            adapter.shutdown();
            let _ = this.update(cx, |this, cx| {
                if this.security.workspace.model_request != Some(request) {
                    return;
                }
                this.security.workspace.model_request = None;
                if this.project().id != project || this.project().path != root {
                    return;
                }
                match result {
                    Ok(models) => {
                        this.security.workspace.models.insert(harness, models);
                        this.security.workspace.error = None;
                    }
                    Err(error) => this.security.workspace.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn start_project_audit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.security_workspace_enabled() || self.security.workspace.pending.is_some() {
            return;
        }
        let harness = self.security.workspace.harness.clone();
        let model = self
            .security
            .workspace
            .model
            .as_ref()
            .map(|input| input.read(cx).value().trim().to_string())
            .unwrap_or_default();
        if model.len() > 1024 || model.chars().any(char::is_control) {
            self.security.workspace.error = Some("Choose a valid model identifier".into());
            cx.notify();
            return;
        }
        if !self.project().is_remote()
            && let Err(reason) = self.local_harness_config(&harness)
        {
            self.security.workspace.error = Some(reason);
            cx.notify();
            return;
        }
        let prompt = if self.project().is_remote() {
            let before = self.session().id.clone();
            super::start_remote_audit(self, self.project().clone(), true, window, cx);
            if self.session().id == before {
                return;
            }
            self.connect_harness(cx);
            self.composer.read(cx).value().to_string()
        } else {
            let Some(prompt) = super::prepare_local_audit(self, &harness, true, window, cx) else {
                return;
            };
            prompt
        };
        let provider = if harness == "OpenCode" {
            self.last_ai_target
                .as_ref()
                .filter(|target| target.harness == harness && target.model == model)
                .and_then(|target| target.provider.clone())
                .or_else(|| crate::providers::provider_id(&model).map(str::to_owned))
        } else {
            None
        };
        self.security.workspace.error = None;
        self.security.workspace.pending = Some(Pending {
            project: self.project().id.clone(),
            root: self.project().path.clone(),
            session: self.session().id.clone(),
            harness: self.session_harness(),
            model,
            provider,
            prompt,
            started: Instant::now(),
            model_requested: false,
        });
        self.open_workspace_tool(crate::workspace_tools::Tool::Security, window, cx);
        cx.spawn(async |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                let keep = this
                    .update(cx, |this, cx| {
                        this.advance_project_audit(cx);
                        this.security.workspace.pending.is_some()
                    })
                    .unwrap_or(false);
                if !keep {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn advance_project_audit(&mut self, cx: &mut Context<Self>) {
        let Some(mut pending) = self.security.workspace.pending.take() else {
            return;
        };
        let fail = |this: &mut Self, message: &str, cx: &mut Context<Self>| {
            this.security.workspace.error = Some(message.into());
            cx.notify();
        };
        if self.project().id != pending.project
            || self.project().path != pending.root
            || self.session().id != pending.session
            || self.session_harness() != pending.harness
            || !self.security_workspace_enabled()
        {
            fail(
                self,
                "Audit launch cancelled because its project, conversation, harness or skill changed. No request was sent.",
                cx,
            );
            return;
        }
        if pending.started.elapsed() > Duration::from_secs(45)
            || matches!(
                self.harness.status,
                crate::harness::HarnessStatus::Failed(_) | crate::harness::HarnessStatus::Exited(_)
            )
        {
            fail(
                self,
                "The selected harness/model was not ready. No audit request or fallback was sent; retry explicitly.",
                cx,
            );
            return;
        }
        if !self.harness.status.is_ready()
            || self.harness.local_session_loading
            || self.harness.session_id.is_none()
            || self.harness.model_changing
            || self.harness.mode_changing
            || self.harness.config_changing
            || self.mcp_busy()
            || (self.local_opencode() && self.provider_catalog.loading)
        {
            self.security.workspace.pending = Some(pending);
            return;
        }
        if self.local_opencode()
            && let Some(provider) = &pending.provider
            && self.selected_provider() != Some(provider.as_str())
        {
            self.select_provider(provider, cx);
            if self.selected_provider() != Some(provider.as_str()) {
                fail(
                    self,
                    "The selected model's provider is unavailable. Connect it in Chat first; no fallback was sent.",
                    cx,
                );
                return;
            }
        }
        if !pending.model.is_empty() {
            let Some(models) = &self.harness.models else {
                self.security.workspace.pending = Some(pending);
                return;
            };
            if !self
                .visible_models()
                .iter()
                .any(|choice| choice.id == pending.model)
            {
                fail(
                    self,
                    "The chosen model is not advertised by this harness/provider. No replacement was selected.",
                    cx,
                );
                return;
            }
            if models.current != pending.model {
                if !pending.model_requested {
                    self.select_model(&pending.model, cx);
                    pending.model_requested = true;
                }
                self.security.workspace.pending = Some(pending);
                return;
            }
        }
        if !self.check_provider_model(cx) {
            fail(
                self,
                "Select an available authenticated model before running the audit.",
                cx,
            );
            return;
        }
        if !self.submit_automation_prompt(pending.prompt, cx) {
            fail(
                self,
                "Audit request was not sent. Retry explicitly when the selected harness/model is ready.",
                cx,
            );
            return;
        }
        self.security.checked_project = None;
        self.ensure_security_runs(cx);
        cx.notify();
    }
}

pub(crate) fn render(
    this: &mut DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    if !this.security_workspace_enabled() {
        return dunlin_ui::settings::description("Enable a security audit skill in Settings → Skills to run project audits. Existing artifacts are preserved.", p).p(px(24.)).into_any_element();
    }
    this.ensure_audit_selection(window, cx);
    this.ensure_security_runs(cx);
    let harness = this.security.workspace.harness.clone();
    let input = this.security.workspace.model.as_ref().unwrap().clone();
    let busy = this.security.workspace.model_request.is_some()
        || this.security.workspace.pending.is_some()
        || this.harness.running
        || this.harness.status.is_busy();
    let mut choices: Vec<(String, String)> = this
        .recent_models
        .iter()
        .filter(|entry| entry.scope == model_scope(this, &harness))
        .map(|entry| (entry.id.clone(), entry.id.clone()))
        .collect();
    if this.session_harness() == harness {
        for choice in this.visible_models() {
            if !choices.iter().any(|(id, _)| id == &choice.id) {
                choices.push((choice.id.clone(), choice.name.clone()));
            }
        }
    }
    if let Some(models) = this.security.workspace.models.get(&harness) {
        for model in models {
            if !choices.iter().any(|(id, _)| id == &model.0) {
                choices.push(model.clone());
            }
        }
    }
    let app = cx.entity().downgrade();
    let model_app = app.clone();
    let remote = this.project().is_remote();
    let fields = dunlin_ui::settings::card(p).bg(p.card).w_full().relative().children({
        #[cfg(debug_assertions)]
        { crate::workspace_ui_smoke::probe("project-audit-form") }
        #[cfg(not(debug_assertions))]
        { None::<AnyElement> }
    }).p(px(16.)).flex().flex_col().gap(px(12.))
        .child(div().text_size(px(13.)).font_weight(FontWeight::MEDIUM).child("Run a project audit"))
        .child(dunlin_ui::settings::description("Source-only Cloudflare workflow. Harness permission prompts still apply; no source edits, target execution or live probes are authorized.", p))
        .child(div().flex().flex_wrap().items_end().gap(px(12.))
            .child(dunlin_ui::settings::field("Harness", dunlin_ui::settings::button("audit-harness").w_full().justify_start().label(harness.clone()).dropdown_caret(true)
                .disabled(busy || this.project().is_remote())
                .dropdown_menu(move |mut menu, _, cx| {
                    let Some(entity) = app.upgrade() else { return menu; };
                    for name in entity.read(cx).visible_harness_names() {
                        let app = app.clone(); let selected = entity.read(cx).security.workspace.harness == name;
                        menu = menu.item(PopupMenuItem::new(name.clone()).checked(selected).on_click(move |_, window, cx| {
                            let _ = app.update(cx, |this, cx| {
                                let model = recent_model(this, &name).unwrap_or_default();
                                this.security.workspace.harness = name.clone();
                                if let Some(input) = &this.security.workspace.model { input.update(cx, |state, cx| state.set_value(model, window, cx)); }
                                cx.notify();
                            });
                        }));
                    }
                    menu
                }), p).flex_1().flex_basis(px(180.)).min_w(px(0.)))
            .child(dunlin_ui::settings::field("Model · last used by this harness", div().flex().gap(px(6.))
                .child(dunlin_ui::settings::input(&input, p).aria_label("Audit model").disabled(busy).flex_1().min_w(px(0.)))
                .child(dunlin_ui::button("audit-models").icon(IconName::ChevronDown).accessibility_label("Choose audit model").disabled(busy)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (id, label) in &choices {
                            let app = model_app.clone(); let id = id.clone();
                            menu = menu.item(PopupMenuItem::new(label.clone()).on_click(move |_, window, cx| { let _ = app.update(cx, |this, cx| {
                                if let Some(input) = &this.security.workspace.model { input.update(cx, |state, cx| state.set_value(id.clone(), window, cx)); }
                                cx.notify();
                            }); }));
                        }
                        if !remote {
                            let app = model_app.clone();
                            menu = menu.separator().item(PopupMenuItem::new("Load / refresh harness models").on_click(move |_, _, cx| {
                                let _ = app.update(cx, |this, cx| this.load_audit_models(cx));
                            }));
                        }
                        menu
                    })), p).flex_1().flex_basis(px(260.)).min_w(px(0.))))
        .when(this.project().is_remote(), |card| card.child(dunlin_ui::settings::description("The relay controls the remote harness; only its advertised models can be selected.", p)))
        .when(!this.project().trusted, |card| card.child(dunlin_ui::settings::description("Trust the active project before running an audit.", p)))
        .child(dunlin_ui::settings::actions()
            .child(action("run-project-audit", if this.security.workspace.pending.is_some() { "Preparing audit…" } else { "Run audit" }, p)
                 .primary().icon(IconName::ShieldCheck).disabled(busy || !this.project().trusted)
                 .relative().children({
                     #[cfg(debug_assertions)]
                     { crate::workspace_ui_smoke::probe("run-project-audit") }
                     #[cfg(not(debug_assertions))]
                     { None::<AnyElement> }
                 })
                .on_click(cx.listener(|this, _, window, cx| this.start_project_audit(window, cx))))
            .when(this.security.workspace.pending.is_some(), |row| row.child(action("cancel-audit-launch", "Cancel launch", p)
                .on_click(cx.listener(|this, _, _, cx| { this.security.workspace.pending = None; cx.notify(); })))))
        .when_some(this.security.workspace.error.clone(), |card, error| card.child(dunlin_ui::settings::description(error, p).text_color(p.red)));
    div()
        .id("security-project-workspace")
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .child(
            div()
                .px(px(24.))
                .pb(px(12.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(16.))
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("Security Audit · {}", this.project().name)),
                )
                .when(
                    this.security.runs.iter().any(|run| run.report.is_some()),
                    |header| {
                        header.child(
                            dunlin_ui::toolbar_button("project-audit-refresh")
                                .label("Refresh results")
                                .relative()
                                .children({
                                    #[cfg(debug_assertions)]
                                    {
                                        crate::workspace_ui_smoke::probe("project-audit-refresh")
                                    }
                                    #[cfg(not(debug_assertions))]
                                    {
                                        None::<AnyElement>
                                    }
                                })
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.refresh_security_runs(cx)),
                                ),
                        )
                    },
                ),
        )
        .child(
            div()
                .id("project-audit-results-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px(px(24.))
                .pb(px(24.))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(px(16.))
                        .child(fields)
                        .child(super::runs_section(this, cx)),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn audit_model_recents_are_harness_scoped_not_the_last_unrelated_model() {
        let recents = [
            crate::providers::RecentModel {
                scope: "local:Other".into(),
                id: "wrong".into(),
            },
            crate::providers::RecentModel {
                scope: "local:OpenCode".into(),
                id: "provider/model".into(),
            },
        ];
        assert_eq!(
            super::recent_in_scope(&recents, "local:OpenCode"),
            Some("provider/model")
        );
        assert_eq!(super::recent_in_scope(&recents, "local:Unknown"), None);
        assert_eq!(
            super::recent_in_scope(&recents, "remote:host:project:OpenCode"),
            None
        );
    }
}
