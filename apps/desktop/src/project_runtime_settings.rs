//! Explicit, project-scoped runtime defaults. Editing never restarts an agent.
use dunlin_domain::{ProjectSettings, parse_project_environment};
use dunlin_ui::{
    component::{
        Disableable, Sizable,
        button::ButtonVariants,
        input::{Editor, EditorState, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::app::DunlinApp;
use crate::remote;

pub(crate) struct Draft {
    project_id: String,
    harness: String,
    model: Entity<InputState>,
    environment: Entity<EditorState>,
    error: Option<String>,
    saving: bool,
}

impl Draft {
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
    environment: &str,
) -> Result<ProjectSettings, String> {
    let mut updated = current.clone();
    updated.set_default_model(harness, model)?;
    updated.environment = parse_project_environment(environment)?;
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
        let answer = window.prompt(PromptLevel::Warning, "Discard model and environment edits?",
            Some("These form edits have not been saved. Appearance, agent and approval choices already saved outside this form are not reverted."),
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

    pub(crate) fn request_reset_project_settings(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .project_runtime_draft
            .as_ref()
            .is_some_and(|draft| draft.saving)
        {
            self.set_notice("Wait for project settings to finish saving", cx);
            return;
        }
        let Some(project) = self.projects.get(index) else {
            return;
        };
        let id = project.id.clone();
        let answer = window.prompt(PromptLevel::Warning, "Reset all project overrides?",
            Some("Clear this project’s appearance, agent, approval, model and environment overrides, including any unsaved runtime form. Inherited defaults will apply. Repository files and running agents stay unchanged."),
            &["Cancel", "Reset overrides"], cx);
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(1) {
                let _ = this.update(cx, |this, cx| {
                    if let Some(index) = this.projects.iter().position(|project| project.id == id) {
                        this.reset_project_settings(index, cx);
                    }
                });
            }
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
        let environment = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("plain")
                .soft_wrap(true)
        });
        model.update(cx, |state, cx| {
            state.set_value(
                project.settings.default_model(&harness).unwrap_or_default(),
                window,
                cx,
            )
        });
        let text = project
            .settings
            .environment
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("\n");
        environment.update(cx, |state, cx| state.set_value(text, window, cx));
        self.project_runtime_draft = Some(Draft {
            project_id: project.id.clone(),
            harness,
            model,
            environment,
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
            &draft.environment.read(cx).value(),
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
                                project.settings = settings;
                                if let Some(store) = &this.store {
                                    let _ = store.upsert_project(project);
                                }
                            }
                            this.project_runtime_draft = None;
                            this.set_notice("Saved on the relay · environment applies to the next remote connection; running agents were not restarted.", cx);
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
        self.set_notice("Saved · model applies to new conversations; environment applies at the next local connection. Running agents were not restarted.", cx);
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
                        "{} · model: {} · {} effective environment entries",
                        harness,
                        resolved.default_model(harness).unwrap_or("Harness default"),
                        resolved.environment.len()
                    )),
            )
            .child(
                dunlin_ui::button("edit-project-runtime")
                    .small()
                    .label("Model & environment…")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_project_runtime_settings(index, window, cx)
                    })),
            )
            .when(!project.is_remote() && index == this.selected_project && resolved.default_model(&this.session_harness()).is_some(), |view| view
                .child(dunlin_ui::button("apply-project-model").label("Apply saved model to current session")
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
        .when_some(current, |view, model| view.child(dunlin_ui::button("project-use-current-model").label("Use current session model").disabled(draft.saving)
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(draft) = &this.project_runtime_draft {
                    draft.model.update(cx, |s, cx| s.set_value(model.clone(), window, cx));
                }
            }))))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Environment overrides · NAME=value; omitted names inherit; no shell expansion"))
        .child(div().h(px(120.)).overflow_hidden().child(Editor::new(&draft.environment).aria_label("Project environment overrides").readonly(draft.saving).h_full()))
        .child(div().text_size(px(10.5)).text_color(p.muted_fg).child(if project.is_remote() {
            "Stored on the relay for this project, unencrypted. Only relay administrators can save. Values reach the remote harness at its next connection; do not paste secrets."
        } else {
            "Stored unencrypted in the local database, not Keychain. Do not paste secrets. Only the local agent and its child tools receive these values, at the next connection; never sent to relay. Values may change tool behavior."
        }))
        .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error)))
        .child(div().flex().gap(px(6.))
            .child(dunlin_ui::button("save-project-runtime").primary().label(if draft.saving { "Saving…" } else { "Save model & environment" }).disabled(draft.saving).on_click(cx.listener(|this, _, _, cx| this.save_project_runtime_settings(cx))))
            .child(dunlin_ui::button("cancel-project-runtime").label("Cancel edit").disabled(draft.saving).on_click(cx.listener(|this, _, _, cx| { this.project_runtime_draft = None; cx.notify(); }))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{live_model_target, updated_settings};
    use dunlin_domain::ProjectSettings;

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
