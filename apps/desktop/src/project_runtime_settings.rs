//! Explicit, project-scoped runtime defaults. Editing never restarts an agent.
use dunlin_domain::{ProjectSettings, parse_project_environment};
use dunlin_ui::{
    component::{
        Disableable, Sizable,
        button::{Button, ButtonVariants},
        input::{Editor, EditorState, Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::app::DunlinApp;

pub(crate) struct Draft {
    project_id: String,
    harness: String,
    model: Entity<InputState>,
    environment: Entity<EditorState>,
    error: Option<String>,
}

impl Draft {
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
        let Some(project) = self.projects.get(index).filter(|p| !p.is_remote()) else {
            return;
        };
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
        });
        cx.notify();
    }

    fn save_project_runtime_settings(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.project_runtime_draft else {
            return;
        };
        // Resolve by identity, not a potentially stale modal index.
        let Some(index) = self
            .projects
            .iter()
            .position(|p| p.id == draft.project_id && !p.is_remote())
        else {
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
        project.settings = settings;
        if let Err(error) = self
            .resolved_project_settings(&project)
            .and_then(|settings| settings.process_environment())
        {
            self.project_runtime_draft.as_mut().unwrap().error = Some(error);
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
    if project.is_remote() {
        return div().text_size(px(11.)).text_color(p.muted_fg).child("Model/environment defaults are local-only. Relay configuration stays on the server.").into_any_element();
    }
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
                Button::new("edit-project-runtime")
                    .small()
                    .label("Model & environment…")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.edit_project_runtime_settings(index, window, cx)
                    })),
            )
            .when(index == this.selected_project && resolved.default_model(&this.session_harness()).is_some(), |view| view
                .child(Button::new("apply-project-model").small().label("Apply saved model to current session")
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
        .child(Input::new(&draft.model).small())
        .when_some(current, |view, model| view.child(Button::new("project-use-current-model").small().label("Use current session model")
            .on_click(cx.listener(move |this, _, window, cx| {
                if let Some(draft) = &this.project_runtime_draft {
                    draft.model.update(cx, |s, cx| s.set_value(model.clone(), window, cx));
                }
            }))))
        .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Environment overrides · NAME=value; omitted names inherit; no shell expansion"))
        .child(div().h(px(100.)).overflow_hidden().child(Editor::new(&draft.environment).h_full()))
        .child(div().text_size(px(10.5)).text_color(p.muted_fg).child("Stored unencrypted in the local database, not Keychain. Do not paste secrets. Only the local agent and its child tools receive these values, at the next connection; never sent to relay. Values may change tool behavior."))
        .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error)))
        .child(div().flex().gap(px(6.))
            .child(Button::new("save-project-runtime").small().primary().label("Save model & environment").on_click(cx.listener(|this, _, _, cx| this.save_project_runtime_settings(cx))))
            .child(Button::new("cancel-project-runtime").small().label("Cancel edit").on_click(cx.listener(|this, _, _, cx| { this.project_runtime_draft = None; cx.notify(); }))))
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
