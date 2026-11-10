//! First-run intent, explicit adapter-install consent, and scoped setup progress.
use crate::{app::DunlinApp, harness::HarnessStatus, harness_install};
use dunlin_ui::{
    assets::IconName,
    component::{Disableable, Icon, button::ButtonVariants, menu::DropdownMenu},
    prelude::FluentBuilder as _,
    *,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub name: String,
    pub package: String,
}
impl Plan {
    fn recipe(&self) -> Option<&'static harness_install::Recipe> {
        harness_install::recipe(&self.name).filter(|r| r.package == self.package)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Phase {
    Installing,
    Checking,
    Ready,
    Failed(String),
}
pub(crate) struct Setup {
    pub ticket: uuid::Uuid,
    pub name: String,
    pub phase: Phase,
    cancelled: Arc<harness_install::Control>,
}
#[derive(Default)]
pub(crate) struct State {
    pub selection: Option<String>,
    pub plan: Option<Plan>,
    pub waiting: bool,
    confirmed: bool,
    pub setup: Option<Setup>,
    pub reveal: Option<Instant>,
}
impl State {
    pub fn restore(plan: Option<Plan>) -> Self {
        Self {
            waiting: plan.is_some(),
            plan,
            ..Default::default()
        }
    }
    pub fn blocks_connection(&self) -> bool {
        self.waiting || self.setup.is_some()
    }
}
impl Drop for Setup {
    fn drop(&mut self) {
        self.cancelled.cancel();
    }
}

impl DunlinApp {
    pub(crate) fn onboarding_visible(&self) -> bool {
        self.projects.is_empty() || self.onboarding.waiting || self.onboarding.setup.is_some()
    }

    pub(crate) fn choose_onboarding_harness(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.onboarding.setup.is_some() {
            return;
        }
        self.onboarding.selection = Some(name.into());
        cx.notify();
    }

    pub(crate) fn continue_onboarding(&mut self, cx: &mut Context<Self>) {
        let Some(name) = self.onboarding.selection.clone() else {
            return;
        };
        let Some(detected) = self.harness_discovery.iter().find(|d| d.name == name) else {
            return;
        };
        let plan = if detected.adapter.is_some() {
            None
        } else if detected.cli.is_some() {
            let Some(recipe) = harness_install::recipe(&name) else {
                return;
            };
            Some(Plan {
                name: name.clone(),
                package: recipe.package.into(),
            })
        } else {
            return;
        };
        self.default_harness = name;
        self.onboarding.plan = plan;
        self.onboarding.confirmed = true;
        self.onboarding.waiting = true;
        self.onboarding_step = 1;
        self.persist_settings(cx);
        cx.notify();
    }

    pub(crate) fn back_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.onboarding.setup.is_some() {
            return;
        }
        self.onboarding.plan = None;
        self.onboarding.confirmed = false;
        self.onboarding.waiting = !self.projects.is_empty();
        self.onboarding_step = 0;
        self.persist_settings(cx);
        cx.notify();
    }

    fn setup_failed(&mut self, ticket: uuid::Uuid, message: String, cx: &mut Context<Self>) {
        if let Some(setup) = self
            .onboarding
            .setup
            .as_mut()
            .filter(|s| s.ticket == ticket)
        {
            setup.phase = Phase::Failed(message);
            cx.notify();
        }
    }

    pub(crate) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        let connect = self
            .onboarding
            .setup
            .as_ref()
            .is_none_or(|s| matches!(s.phase, Phase::Ready));
        self.onboarding.plan = None;
        self.onboarding.waiting = false;
        self.onboarding.setup = None;
        self.onboarding.reveal = Some(Instant::now());
        self.onboarding_step = 2;
        if !self.projects.is_empty()
            && !self.project().is_remote()
            && self.harness.transcript.is_empty()
            && self.session().acp_session_id.is_none()
        {
            let name = self.effective_harness();
            if self.session_harness() != name {
                self.update_active_session(|session| session.harness = name);
            }
        }
        self.persist_settings(cx);
        if connect && self.auto_connect && !self.projects.is_empty() {
            self.connect_harness(cx);
        }
        cx.notify();
    }

    pub(crate) fn retry_onboarding_setup(&mut self, cx: &mut Context<Self>) {
        if !self
            .onboarding
            .setup
            .as_ref()
            .is_some_and(|s| matches!(s.phase, Phase::Failed(_)))
        {
            return;
        }
        self.onboarding.setup = None;
        self.onboarding.confirmed = true;
        self.onboarding.waiting = true;
        cx.notify();
    }

    pub(crate) fn change_onboarding_harness(&mut self, cx: &mut Context<Self>) {
        if !self
            .onboarding
            .setup
            .as_ref()
            .is_some_and(|s| matches!(s.phase, Phase::Failed(_)))
        {
            return;
        }
        self.onboarding.setup = None;
        self.back_onboarding(cx);
    }

    /// Called before rendering. Only Continue grants install intent; merely
    /// opening welcome, selecting a row, scanning, or importing never does.
    pub(crate) fn advance_onboarding(&mut self, cx: &mut Context<Self>) {
        if self.projects.is_empty()
            || self.local_import_running
            || self.project_import.open
            || self.onboarding.setup.is_some()
        {
            return;
        }
        if !self.onboarding.waiting {
            return;
        }
        // Changing the selection after a failure keeps the chooser open even
        // though the first project already exists.
        if self.onboarding_step == 0 {
            return;
        }
        let Some(plan) = self.onboarding.plan.clone() else {
            self.finish_onboarding(cx);
            return;
        };
        if self.project().is_remote() {
            self.set_notice(
                "This project uses its relay harness; no local adapter was installed",
                cx,
            );
            self.finish_onboarding(cx);
            return;
        }
        let ticket = uuid::Uuid::now_v7();
        let cancelled = Arc::new(harness_install::Control::default());
        self.onboarding.setup = Some(Setup {
            ticket,
            name: plan.name.clone(),
            phase: Phase::Installing,
            cancelled: cancelled.clone(),
        });
        if !self.onboarding.confirmed {
            self.setup_failed(
                ticket,
                "Setup was interrupted. Retry to resume installation, or choose another harness."
                    .into(),
                cx,
            );
            return;
        }
        let Some(recipe) = plan.recipe() else {
            self.setup_failed(ticket, "The adapter installation recipe changed. Choose the harness again to confirm the new package.".into(), cx);
            return;
        };
        if !self.project().trusted {
            self.setup_failed(ticket, "The project is not trusted. Skip setup, trust the project, and configure its agent in Settings.".into(), cx);
            return;
        }
        let prefix = match harness_install::prefix(recipe) {
            Ok(path) => path,
            Err(e) => {
                self.setup_failed(ticket, e, cx);
                return;
            }
        };
        let project = self.project().id.clone();
        let root = self.project().path.clone();
        cx.spawn(async move |this, cx| {
            let installed = cx
                .background_executor()
                .spawn(async move { harness_install::install(recipe, &prefix, cancelled) })
                .await;
            if let Err(error) = installed {
                let _ = this.update(cx, |this, cx| this.setup_failed(ticket, error, cx));
                return;
            }
            let config = this
                .update(cx, |this, cx| {
                    if this
                        .onboarding
                        .setup
                        .as_ref()
                        .is_none_or(|s| s.ticket != ticket)
                    {
                        return None;
                    }
                    if this.project().id != project || this.project().path != root {
                        this.setup_failed(
                            ticket,
                            "The selected project changed. Choose the harness again.".into(),
                            cx,
                        );
                        return None;
                    }
                    this.harness_discovery = crate::harness_discovery::discover();
                    this.onboarding.setup.as_mut().unwrap().phase = Phase::Checking;
                    cx.notify();
                    match this.local_harness_config(&plan.name) {
                        Ok(mut config) => {
                            config.additional_directories.clear();
                            Some(config)
                        }
                        Err(error) => {
                            this.setup_failed(ticket, error, cx);
                            None
                        }
                    }
                })
                .ok()
                .flatten();
            let Some(config) = config else {
                return;
            };
            let key = crate::harness::readiness::config_key(&plan.name, &config);
            let adapter = dunlin_agent::AcpHarness::spawn(config);
            let check = async {
                let mut ready = HarnessStatus::Offline;
                while let Ok(event) = adapter.events().recv().await {
                    match event {
                        dunlin_agent::HarnessEvent::Ready { agent, version } => {
                            ready = HarnessStatus::Ready { agent, version }
                        }
                        dunlin_agent::HarnessEvent::SessionCreated { .. } => return ready,
                        dunlin_agent::HarnessEvent::Error { message }
                        | dunlin_agent::HarnessEvent::Exited { reason: message } => {
                            return HarnessStatus::Failed(message);
                        }
                        dunlin_agent::HarnessEvent::PermissionRequested { request_id, .. } => {
                            adapter.respond_permission(request_id, None);
                        }
                        _ => {}
                    }
                }
                HarnessStatus::Failed("The adapter closed before creating an ACP session".into())
            };
            let check = std::pin::pin!(check);
            let timeout = std::pin::pin!(cx.background_executor().timer(Duration::from_secs(35)));
            let status = match futures_util::future::select(check, timeout).await {
                futures_util::future::Either::Left((status, _)) => status,
                futures_util::future::Either::Right(_) => HarnessStatus::Failed(
                    "ACP connection timed out. Check the harness setup and retry.".into(),
                ),
            };
            adapter.shutdown();
            let ready = this
                .update(cx, |this, cx| {
                    if this
                        .onboarding
                        .setup
                        .as_ref()
                        .is_none_or(|s| s.ticket != ticket)
                    {
                        return false;
                    }
                    if this.project().id != project || this.project().path != root {
                        this.setup_failed(
                            ticket,
                            "The selected project changed. Choose the harness again.".into(),
                            cx,
                        );
                        return false;
                    }
                    let unchanged = this.local_harness_config(&plan.name).is_ok_and(|config| {
                        crate::harness::readiness::config_key(&plan.name, &config) == key
                    });
                    if !status.is_ready() || !unchanged {
                        this.setup_failed(
                            ticket,
                            if unchanged {
                                status
                                    .detail()
                                    .unwrap_or_else(|| "ACP connection failed".into())
                                    .to_string()
                            } else {
                                "Project configuration changed. Choose the harness again.".into()
                            },
                            cx,
                        );
                        return false;
                    }
                    this.harness_checks.insert(key, status);
                    this.onboarding.setup.as_mut().unwrap().phase = Phase::Ready;
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !ready {
                return;
            }
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let _ = this.update(cx, |this, cx| {
                if this
                    .onboarding
                    .setup
                    .as_ref()
                    .is_some_and(|s| s.ticket == ticket && matches!(s.phase, Phase::Ready))
                {
                    if this.project().id == project && this.project().path == root {
                        this.finish_onboarding(cx);
                    } else {
                        this.setup_failed(
                            ticket,
                            "The selected project changed. Choose the harness again.".into(),
                            cx,
                        );
                    }
                }
            });
        })
        .detach();
        cx.notify();
    }
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let step = this.onboarding_step.min(1);
    let busy = this.onboarding.setup.is_some();
    if this.onboarding.selection.is_none() {
        let eligible = |d: &&crate::harness_discovery::Detected| {
            d.adapter.is_some() || (d.cli.is_some() && harness_install::recipe(d.name).is_some())
        };
        this.onboarding.selection = this
            .harness_discovery
            .iter()
            .filter(eligible)
            .find(|d| d.name == this.default_harness)
            .or_else(|| this.harness_discovery.iter().filter(eligible).next())
            .map(|d| d.name.into());
    }
    let selection = this.onboarding.selection.clone();
    let selected = this
        .harness_discovery
        .iter()
        .find(|d| Some(d.name) == selection.as_deref());
    let install = selected
        .filter(|d| d.adapter.is_none() && d.cli.is_some())
        .and_then(|d| harness_install::recipe(d.name));
    let eligible = selected.is_some_and(|d| d.adapter.is_some()) || install.is_some();
    let mut dots = div().flex().gap(px(5.));
    for index in 0..2 {
        dots = dots.child(div().size(px(5.)).rounded_full().bg(if index <= step {
            p.fg
        } else {
            p.border_strong
        }));
    }
    let mut content = div().w_full().max_w(px(600.)).flex().flex_col().items_center().gap(px(18.))
        .child(div().size(px(38.)).rounded(px(radii::DRAG_PREVIEW)).bg(p.fg).flex().items_center().justify_center().child(Icon::new(IconName::Bird).size(px(19.)).text_color(p.card)))
        .child(dots)
        .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).text_center().child(if step == 0 { "Choose your default harness" } else if busy { "Preparing your workspace" } else { "Create your first project" }))
        .child(div().max_w(px(480.)).text_center().text_size(px(12.)).line_height(relative(1.5)).text_color(p.muted_fg).child(if step == 0 {
            "Select the default harness for new conversations. You can change it later in Settings → Agents."
        } else { "Pick a folder and start working. Nothing is uploaded; your project stays on this Mac." }));
    if step == 0 {
        let mut rows = div()
            .id("onboarding-harness-list")
            .w(px(460.))
            .max_w_full()
            .max_h(px(260.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(4.));
        for detected in this
            .harness_discovery
            .iter()
            .filter(|d| d.visible_in_onboarding(this.onboarding_show_all_harnesses))
        {
            let name = detected.name;
            let active = selection.as_deref() == Some(name);
            rows = rows.child(
                action_row(
                    format!("onboarding-harness:{name}"),
                    format!("Choose {name} as default"),
                )
                .toggled(active)
                .disabled(busy)
                .relative()
                .children(probe(match name {
                    "Claude Code" => "onboarding-claude",
                    "OpenCode" => "onboarding-opencode",
                    _ => "onboarding-other-harness",
                }))
                .on_click(
                    cx.listener(move |this, _, _, cx| this.choose_onboarding_harness(name, cx)),
                )
                .child(choice_indicator(active, p))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .child(
                            div()
                                .text_size(px(12.5))
                                .font_weight(FontWeight::MEDIUM)
                                .child(name),
                        )
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(p.muted_fg)
                                .child(detected.status()),
                        ),
                ),
            );
        }
        content = content
            .child(rows)
            .child(
                icon_button(
                    "onboarding-harness-show-all",
                    if this.onboarding_show_all_harnesses {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    },
                    "Show remaining harnesses",
                )
                .relative()
                .children(probe("onboarding-show-all"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.onboarding_show_all_harnesses = !this.onboarding_show_all_harnesses;
                    cx.notify();
                })),
            )
            .child(
                div().w(px(460.)).max_w_full().flex().justify_end().child(
                    dunlin_ui::settings::button("onboarding-continue")
                        .label("Continue")
                        .primary()
                        .disabled(!eligible || busy)
                        .relative()
                        .children(probe("onboarding-continue"))
                        .on_click(cx.listener(|this, _, _, cx| this.continue_onboarding(cx))),
                ),
            );
    } else {
        content = content
            .child(
                dunlin_ui::settings::button("onboarding-create-project")
                    .primary()
                    .label("Create first project…")
                    .disabled(busy)
                    .relative()
                    .children(probe("onboarding-create-project"))
                    .dropdown_menu_with_anchor(Anchor::TopLeft, {
                        let app = cx.entity().downgrade();
                        move |menu, _, _| crate::project_import::source_menu(menu, app.clone())
                    }),
            )
            .child(
                icon_button(
                    "onboarding-back",
                    IconName::ArrowLeft,
                    "Back to harness selection",
                )
                .disabled(busy)
                .relative()
                .children(probe("onboarding-back"))
                .on_click(cx.listener(|this, _, _, cx| this.back_onboarding(cx))),
            );
    }
    if step == 0 && !eligible {
        content = content.child(dunlin_ui::settings::description("Install the selected harness CLI first, then return here. Use the arrow to browse other harnesses.", p).text_center());
        if let Some(name) = selection
            .as_ref()
            .filter(|name| crate::harness_discovery::catalog_docs(name).is_some())
        {
            let name = name.clone();
            content = content.child(
                dunlin_ui::toolbar_button("onboarding-setup-guide")
                    .icon(IconName::ExternalLink)
                    .label("Setup guide")
                    .on_click(cx.listener(move |this, _, _, cx| this.open_harness_docs(&name, cx))),
            );
        }
    }
    let notice = if step == 0 {
        install.map(|recipe| format!("The {} ACP adapter will be installed automatically after Continue and choosing your first project. Downloads {} into Dunlin’s data directory; no sudo or system-global changes.", recipe.name, recipe.package))
    } else {
        this.onboarding.plan.as_ref().map(|p| {
            format!(
                "The {} ACP adapter will be installed after you choose a local project.",
                p.name
            )
        })
    };
    div()
        .id("onboarding-screen")
        .relative()
        .flex_1()
        .min_h(px(0.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .children(probe("onboarding-screen"))
        .child(
            div()
                .id("onboarding-scroll")
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .px(px(24.))
                .py(px(24.))
                .child(content.relative().children(probe("onboarding-content"))),
        )
        // Reserve the same footer space with or without a notice, so changing
        // the selected harness never moves the centered welcome controls.
        .child(
            div()
                .id("onboarding-notice-slot")
                .w_full()
                .h(px(96.))
                .flex_shrink_0()
                .overflow_y_scroll()
                .px(px(32.))
                .pb(px(18.))
                .flex()
                .flex_col()
                .when_some(notice, |footer, notice| {
                    footer.child(
                        div()
                            .mt_auto()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .line_height(relative(1.45))
                            .text_center()
                            .text_color(p.muted_fg)
                            .relative()
                            .children(probe("onboarding-adapter-notice"))
                            .child(notice),
                    )
                }),
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

pub(crate) fn render_setup(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(setup) = &this.onboarding.setup else {
        return div().into_any_element();
    };
    let (title, detail) = match &setup.phase {
        Phase::Installing => ("Installing ACP adapter", "Downloading the selected adapter into Dunlin’s data directory. This may take a few minutes.".to_string()),
        Phase::Checking => ("Checking connection", "Creating an empty ACP session. No prompt is sent and permission requests are denied.".to_string()),
        Phase::Ready => ("Your workspace is ready", "Adapter installed and ACP connection verified. Opening Dunlin in a moment…".to_string()),
        Phase::Failed(error) => ("Setup needs attention", error.clone()),
    };
    let failed = matches!(setup.phase, Phase::Failed(_));
    let ready = matches!(setup.phase, Phase::Ready);
    let content = dialog_surface(p)
        .id("onboarding-setup-dialog")
        .relative()
        .w(px(480.))
        .max_w_full()
        .max_h(relative(0.9))
        .overflow_y_scroll()
        .children(probe("onboarding-setup-dialog"))
        .child(crate::effects::panel_probe(
            &this.overlay_backdrop,
            SurfaceGeometry::DIALOG,
            true,
        ))
        .flex()
        .flex_col()
        .gap(px(16.))
        .child(
            Icon::new(if ready {
                IconName::CircleCheck
            } else if failed {
                IconName::CircleAlert
            } else {
                IconName::Download
            })
            .size(px(28.))
            .text_color(if ready {
                p.green
            } else if failed {
                p.red
            } else {
                p.fg
            }),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child(setup.name.clone()),
        )
        .child(dunlin_ui::settings::description(detail, p))
        .when(!ready && !failed, |card| {
            card.child(component::spinner::Spinner::new())
        })
        .when(failed, |card| {
            card.child(
                dunlin_ui::settings::actions()
                    .child(
                        dunlin_ui::settings::button("onboarding-setup-retry")
                            .primary()
                            .label("Retry")
                            .relative()
                            .children(probe("onboarding-setup-retry"))
                            .on_click(
                                cx.listener(|this, _, _, cx| this.retry_onboarding_setup(cx)),
                            ),
                    )
                    .child(
                        dunlin_ui::toolbar_button("onboarding-setup-change")
                            .label("Choose another harness")
                            .on_click(
                                cx.listener(|this, _, _, cx| this.change_onboarding_harness(cx)),
                            ),
                    )
                    .child(
                        dunlin_ui::toolbar_button("onboarding-setup-skip")
                            .label("Skip setup")
                            .on_click(cx.listener(|this, _, _, cx| this.finish_onboarding(cx))),
                    ),
            )
        });
    div()
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(24.))
        .child(content)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persisted_install_intent_is_pinned_and_never_resumes_silently() {
        let recipe = harness_install::recipe("Claude Code").unwrap();
        let plan = Plan {
            name: recipe.name.into(),
            package: recipe.package.into(),
        };
        let mut state = State::restore(Some(plan.clone()));
        assert!(state.blocks_connection());
        assert!(!state.confirmed);
        assert!(plan.recipe().is_some());
        let changed = Plan {
            package: "@unknown/package@latest".into(),
            ..plan
        };
        assert!(changed.recipe().is_none());
        state.waiting = false;
        assert!(!state.blocks_connection());
    }
}
