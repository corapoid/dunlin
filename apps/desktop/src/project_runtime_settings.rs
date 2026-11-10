//! Explicit, project-scoped runtime defaults. Editing never restarts an agent.
use dunlin_domain::ProjectSettings;
use dunlin_ui::{
    component::{Disableable, button::ButtonVariants, input::InputState},
    prelude::FluentBuilder as _,
    *,
};

use crate::app::DunlinApp;
use crate::remote;

/// Refresh only relay-owned runtime defaults. A delayed read must not replace
/// local appearance/approval choices, a retargeted project or a newer save.
pub(crate) fn refresh_remote_defaults(
    current: &mut dunlin_domain::Project,
    expected: &dunlin_domain::Project,
    incoming: ProjectSettings,
) -> bool {
    let same_remote = current
        .remote
        .as_ref()
        .zip(expected.remote.as_ref())
        .is_some_and(|(current, expected)| {
            current.host_id == expected.host_id
                && current.project_id == expected.project_id
                && current.path == expected.path
        });
    if current.id != expected.id
        || !same_remote
        || current.settings.default_models != expected.settings.default_models
        || current.settings.environment != expected.settings.environment
    {
        return false;
    }
    current.settings.default_models = incoming.default_models;
    current.settings.environment = incoming.environment;
    true
}

pub(crate) struct Draft {
    project_id: String,
    harness: String,
    model: Entity<InputState>,
    error: Option<String>,
    saving: bool,
}

impl Draft {
    pub(crate) fn belongs_to(&self, project_id: &str) -> bool {
        self.project_id == project_id
    }

    pub(crate) fn is_saving(&self) -> bool {
        self.saving
    }
    pub(crate) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.model.read(cx).focus_handle(cx)
    }
}

fn updated_settings(
    current: &ProjectSettings,
    harness: &str,
    model: &str,
) -> Result<ProjectSettings, String> {
    let mut updated = current.clone();
    updated.set_default_model(harness, model)?;
    Ok(updated)
}

fn live_model_target(
    preferred: Option<&str>,
    models: Option<&dunlin_agent::SessionModels>,
    visible: &[dunlin_agent::ModelChoice],
    busy: bool,
) -> Result<Option<String>, String> {
    if busy {
        return Err("Wait for the current agent operation before applying a saved model".into());
    }
    let preferred = preferred.ok_or("No saved model default for this conversation's harness")?;
    let models = models.ok_or("Connect a harness that advertises ACP models first")?;
    if models.current == preferred {
        return Ok(None);
    }
    if !models.choices.iter().any(|model| model.id == preferred)
        || !visible.iter().any(|model| model.id == preferred)
    {
        return Err("The saved model is unavailable or hidden by the current provider/model filter. Select an allowed provider/model in the composer first.".into());
    }
    Ok(Some(preferred.into()))
}

impl DunlinApp {
    pub(crate) fn request_close_project_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &self.project_runtime_draft else {
            self.close_project_settings(cx);
            return;
        };
        if draft.saving {
            self.set_notice("Wait for project settings to finish saving", cx);
            return;
        }
        if self.settings_ui.confirming_leave {
            return;
        }
        self.settings_ui.confirming_leave = true;
        let id = draft.project_id.clone();
        let answer = window.prompt(PromptLevel::Warning, "Discard model edits?",
            Some("These model edits have not been saved. Appearance and agent choices already saved outside this form are not reverted."),
            &[PromptButton::cancel("Keep editing"), PromptButton::new("Discard edits")], cx);
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn(async move |this, cx| {
            let discard = answer.await == Ok(1);
            let _ = this.update(cx, |this, cx| {
                this.settings_ui.confirming_leave = false;
                if discard
                    && this
                        .project_runtime_draft
                        .as_ref()
                        .is_some_and(|draft| draft.project_id == id && !draft.saving)
                {
                    this.close_project_settings(cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn apply_saved_project_model(&mut self, cx: &mut Context<Self>) {
        if self.project().is_remote() {
            self.set_notice("Saved project model defaults are local-only", cx);
            return;
        }
        let result = self
            .resolved_project_settings(self.project())
            .and_then(|settings| {
                live_model_target(
                    settings.default_model(&self.session_harness()),
                    self.harness.models.as_ref(),
                    &self.visible_models(),
                    self.chat_controls_busy() || !self.harness.status.is_ready(),
                )
            });
        match result {
            Ok(Some(id)) => self.select_model(&id, cx),
            Ok(None) => self.set_notice("This conversation already uses the saved model", cx),
            Err(error) => self.set_notice(error, cx),
        }
    }

    pub(crate) fn edit_project_runtime_settings(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.projects.get(index) else {
            return;
        };
        if let Some(remote) = &project.remote {
            let admin = self
                .remote_host(&remote.host_id)
                .is_some_and(|host| host.user_role == "admin");
            if !admin {
                self.set_notice(
                    "Remote project runtime settings are admin-only; ask the relay administrator",
                    cx,
                );
                return;
            }
        }
        let harness = match self.resolved_project_settings(project) {
            Ok(settings) => settings.harness.value,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let model = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Blank = inherit model (otherwise ACP wire ID)")
        });
        model.update(cx, |state, cx| {
            state.set_value(
                project.settings.default_model(&harness).unwrap_or_default(),
                window,
                cx,
            )
        });
        self.project_runtime_draft = Some(Draft {
            project_id: project.id.clone(),
            harness,
            model,
            error: None,
            saving: false,
        });
        cx.notify();
    }

    fn save_project_runtime_settings(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.project_runtime_draft else {
            return;
        };
        // Resolve by identity, not a potentially stale modal index.
        if draft.saving {
            return;
        }
        let Some(index) = self.projects.iter().position(|p| p.id == draft.project_id) else {
            return;
        };
        let result = updated_settings(
            &self.projects[index].settings,
            &draft.harness,
            &draft.model.read(cx).value(),
        );
        let settings = match result {
            Ok(settings) => settings,
            Err(error) => {
                self.project_runtime_draft.as_mut().unwrap().error = Some(error);
                cx.notify();
                return;
            }
        };
        let mut project = self.projects[index].clone();
        project.settings = settings.clone();
        if let Err(error) = self
            .resolved_project_settings(&project)
            .and_then(|settings| settings.process_environment())
        {
            self.project_runtime_draft.as_mut().unwrap().error = Some(error);
            cx.notify();
            return;
        }
        if let Some(remote) = project.remote.clone() {
            let Some(host) = self.remote_host(&remote.host_id) else {
                self.project_runtime_draft.as_mut().unwrap().error =
                    Some("Remote server is unavailable".into());
                cx.notify();
                return;
            };
            let token = match self.remote_token(&host.id) {
                Ok(token) => token,
                Err(error) => {
                    self.project_runtime_draft.as_mut().unwrap().error = Some(error);
                    cx.notify();
                    return;
                }
            };
            let url = host.url;
            let remote_id = remote.project_id;
            let settings_for_save = settings.clone();
            self.project_runtime_draft.as_mut().unwrap().saving = true;
            cx.spawn(async move |this, cx| {
                let result = cx
                    .background_executor()
                    .spawn(async move {
                        remote::save_project_settings(&url, &token, &remote_id, &settings_for_save)
                    })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match result {
                        Ok(()) => {
                            if let Some(project) =
                                this.projects.iter_mut().find(|p| p.id == project.id)
                            {
                                // Appearance/approval may have changed during the
                                // HTTP save. The form owns only model/env fields.
                                project.settings.default_models = settings.default_models;
                                project.settings.environment = settings.environment;
                                if let Some(store) = &this.store {
                                    let _ = store.upsert_project(project);
                                }
                            }
                            this.project_runtime_draft = None;
                            this.set_notice("Model saved on the relay · applies to new conversations; running agents were not restarted.", cx);
                        }
                        Err(error) => {
                            if let Some(draft) = &mut this.project_runtime_draft {
                                draft.saving = false;
                                draft.error = Some(error);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            cx.notify();
            return;
        }
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project(&project)
        {
            // Do not update the in-memory project or lose the draft on failure.
            self.project_runtime_draft.as_mut().unwrap().error =
                Some(format!("Could not save project settings: {error}"));
            cx.notify();
            return;
        }
        self.projects[index] = project;
        self.project_runtime_draft = None;
        self.set_notice(
            "Model saved · applies to new conversations. Running agents were not restarted.",
            cx,
        );
    }
}

pub(crate) fn render(this: &DunlinApp, index: usize, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(project) = this.projects.get(index) else {
        return div().into_any_element();
    };
    let Some(draft) = this
        .project_runtime_draft
        .as_ref()
        .filter(|d| d.project_id == project.id)
    else {
        let resolved = match this.resolved_project_settings(project) {
            Ok(settings) => settings,
            Err(error) => {
                return div()
                    .text_size(px(11.))
                    .text_color(p.red)
                    .child(error)
                    .into_any_element();
            }
        };
        let harness = &resolved.harness.value;
        return div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "{} · model: {}",
                        harness,
                        resolved.default_model(harness).unwrap_or("Harness default")
                    )),
            )
            .child(
                dunlin_ui::action_button("edit-project-runtime")
                    .label("Default model…")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_project_runtime_settings(index, window, cx)
                    })),
            )
            .when(!project.is_remote() && index == this.selected_project && resolved.default_model(&this.session_harness()).is_some(), |view| view
                .child(dunlin_ui::action_button("apply-project-model").label("Apply saved model to current session")
                    .disabled(this.chat_controls_busy() || !this.harness.status.is_ready())
                    .on_click(cx.listener(|this, _, _, cx| this.apply_saved_project_model(cx))))
                .child(div().text_size(px(10.5)).text_color(p.muted_fg).child(format!("For {}: {}. ACP change only; no prompt or restart. Other conversations stay unchanged.", this.session_harness(), resolved.default_model(&this.session_harness()).unwrap_or_default()))))
            .into_any_element();
    };
    let current = (index == this.selected_project && this.session_harness() == draft.harness)
        .then(|| {
            this.harness
                .models
                .as_ref()
                .map(|models| models.current.clone())
        })
        .flatten();
    div().flex().flex_col().gap(px(8.))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child(format!("Default model · {} · new conversations only; blank = inherit", draft.harness)))
        .child(dunlin_ui::input(&draft.model).aria_label("Project default model").disabled(draft.saving))
        .when_some(current, |view, model| view.child(dunlin_ui::action_button("project-use-current-model").label("Use current session model").disabled(draft.saving)
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(draft) = &this.project_runtime_draft {
                    draft.model.update(cx, |s, cx| s.set_value(model.clone(), window, cx));
                }
            }))))
        .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error)))
        .child(div().flex().gap(px(6.))
            .child(dunlin_ui::action_button("save-project-runtime").primary().label(if draft.saving { "Saving…" } else { "Save model" }).disabled(draft.saving).on_click(cx.listener(|this, _, _, cx| this.save_project_runtime_settings(cx))))
            .child(dunlin_ui::action_button("cancel-project-runtime").label("Cancel edit").disabled(draft.saving).on_click(cx.listener(|this, _, _, cx| { this.project_runtime_draft = None; cx.notify(); }))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{live_model_target, refresh_remote_defaults};
    use dunlin_domain::ProjectSettings;

    fn updated_settings(
        current: &ProjectSettings,
        harness: &str,
        model: &str,
        environment: &str,
    ) -> Result<ProjectSettings, String> {
        let mut updated = super::updated_settings(current, harness, model)?;
        updated.environment = dunlin_domain::parse_project_environment(environment)?;
        Ok(updated)
    }

    #[test]
    fn model_form_preserves_hidden_environment_and_permission_overrides() {
        let mut current = ProjectSettings::default();
        current.permission_mode = Some(dunlin_domain::PermissionMode::ReadOnly);
        current
            .environment
            .insert("MULTILINE".into(), "one\ntwo".into());
        let updated = super::updated_settings(&current, "OpenCode", "provider/model").unwrap();
        assert_eq!(updated.environment, current.environment);
        assert_eq!(updated.permission_mode, current.permission_mode);
        assert_eq!(updated.default_model("OpenCode"), Some("provider/model"));
    }

    fn remote_project() -> dunlin_domain::Project {
        dunlin_domain::Project {
            id: "local-id".into(),
            name: "Remote project".into(),
            path: "/remote/project".into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: Some(dunlin_domain::RemoteProject {
                host_id: "host".into(),
                project_id: "remote-id".into(),
                path: "/remote/project".into(),
            }),
            group_id: Some("local-group".into()),
            settings: updated_settings(&ProjectSettings::default(), "OpenCode", "old", "LANG=old")
                .unwrap(),
        }
    }

    #[test]
    fn remote_refresh_updates_runtime_defaults_without_replacing_local_choices() {
        let baseline = remote_project();
        let mut current = baseline.clone();
        current.settings.custom_icon = Some("local-icon".into());
        current.settings.custom_color = Some(0x123456ff);
        current.settings.permission_mode = Some(dunlin_domain::PermissionMode::ReadOnly);
        current.settings.harness = Some("Local choice".into());
        let mut incoming = updated_settings(
            &ProjectSettings::default(),
            "OpenCode",
            "new",
            "LANG=pl_PL.UTF-8",
        )
        .unwrap();
        incoming.custom_icon = Some("server-icon".into());
        incoming.permission_mode = Some(dunlin_domain::PermissionMode::AutoApprove);
        assert!(refresh_remote_defaults(&mut current, &baseline, incoming));
        assert_eq!(current.settings.default_model("OpenCode"), Some("new"));
        assert_eq!(current.settings.environment["LANG"], "pl_PL.UTF-8");
        assert_eq!(current.settings.custom_icon.as_deref(), Some("local-icon"));
        assert_eq!(current.settings.custom_color, Some(0x123456ff));
        assert_eq!(
            current.settings.permission_mode,
            Some(dunlin_domain::PermissionMode::ReadOnly)
        );
        assert_eq!(current.settings.harness.as_deref(), Some("Local choice"));
        assert_eq!(current.group_id, baseline.group_id);
    }

    #[test]
    fn delayed_remote_refresh_does_not_overwrite_newer_runtime_edits_or_retargeted_projects() {
        let baseline = remote_project();
        let incoming = updated_settings(
            &ProjectSettings::default(),
            "OpenCode",
            "server",
            "LANG=server",
        )
        .unwrap();
        for mutation in 0..6 {
            let mut current = baseline.clone();
            match mutation {
                0 => current
                    .settings
                    .set_default_model("OpenCode", "newer-save")
                    .unwrap(),
                1 => {
                    current
                        .settings
                        .environment
                        .insert("LANG".into(), "newer-save".into());
                }
                2 => current.remote.as_mut().unwrap().host_id = "other-host".into(),
                3 => current.remote.as_mut().unwrap().project_id = "other-project".into(),
                4 => current.remote = None,
                _ => current.id = "replacement".into(),
            }
            let before = serde_json::to_value(&current).unwrap();
            assert!(!refresh_remote_defaults(
                &mut current,
                &baseline,
                incoming.clone()
            ));
            assert_eq!(serde_json::to_value(&current).unwrap(), before);
        }
    }

    #[test]
    fn remote_refresh_can_clear_runtime_overrides_and_is_not_a_local_fallback() {
        let baseline = remote_project();
        let mut current = baseline.clone();
        assert!(refresh_remote_defaults(
            &mut current,
            &baseline,
            ProjectSettings::default()
        ));
        assert!(current.settings.default_models.is_empty());
        assert!(current.settings.environment.is_empty());
        let mut local = baseline;
        local.remote = None;
        let expected = local.clone();
        assert!(!refresh_remote_defaults(
            &mut local,
            &expected,
            ProjectSettings::default()
        ));
    }

    #[test]
    fn editing_runtime_preserves_other_settings_and_other_harness_models() {
        let mut original = ProjectSettings {
            harness: Some("OpenCode".into()),
            custom_icon: Some("icon".into()),
            ..Default::default()
        };
        original.set_default_model("Claude Code", "other").unwrap();
        let updated =
            updated_settings(&original, "OpenCode", "provider/model", "LANG=pl_PL.UTF-8").unwrap();
        assert_eq!(updated.harness, original.harness);
        assert_eq!(updated.custom_icon, original.custom_icon);
        assert_eq!(updated.default_model("Claude Code"), Some("other"));
        assert_eq!(updated.default_model("OpenCode"), Some("provider/model"));
        assert_eq!(updated.environment["LANG"], "pl_PL.UTF-8");
        assert!(original.environment.is_empty());
        assert!(updated_settings(&updated, "OpenCode", "new", "INVALID").is_err());
        assert_eq!(updated.default_model("OpenCode"), Some("provider/model"));
        let cleared = updated_settings(&updated, "OpenCode", "", "").unwrap();
        assert!(cleared.environment.is_empty());
        assert_eq!(cleared.default_model("OpenCode"), None);
    }

    #[test]
    fn live_apply_requires_an_idle_advertised_allowed_model_and_never_invents_a_fallback() {
        use dunlin_agent::{ModelChoice, SessionModels};
        let choices = vec![
            ModelChoice {
                id: "a".into(),
                name: "A".into(),
                description: None,
            },
            ModelChoice {
                id: "b".into(),
                name: "B".into(),
                description: None,
            },
        ];
        let models = SessionModels {
            config_id: Some("model".into()),
            current: "a".into(),
            choices: choices.clone(),
        };
        assert_eq!(
            live_model_target(Some("b"), Some(&models), &choices, false).unwrap(),
            Some("b".into())
        );
        assert_eq!(
            live_model_target(Some("a"), Some(&models), &choices, false).unwrap(),
            None
        );
        assert!(live_model_target(Some("b"), Some(&models), &choices, true).is_err());
        assert!(live_model_target(Some("b"), Some(&models), &choices[..1], false).is_err());
        assert!(live_model_target(Some("unknown"), Some(&models), &choices, false).is_err());
        assert!(live_model_target(None, Some(&models), &choices, false).is_err());
        assert!(live_model_target(Some("b"), None, &choices, false).is_err());
    }
}
