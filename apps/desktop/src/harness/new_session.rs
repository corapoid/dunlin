//! A new conversation inherits the current project's confirmed wire model.
//! The snapshot belongs to its runtime, never to another project/provider/agent.
use dunlin_agent::{ModelChoice, SessionModels};

#[derive(Clone, Debug)]
pub(crate) struct InheritedModel {
    pub harness: String,
    pub choice: ModelChoice,
}

impl InheritedModel {
    pub fn capture(
        current_harness: &str,
        target_harness: &str,
        models: Option<&SessionModels>,
        visible: &[ModelChoice],
        draft: Option<&Self>,
    ) -> Option<Self> {
        if current_harness != target_harness {
            return None;
        }
        if let Some(models) = models {
            // Never copy a display label or a model hidden by provider filtering.
            return visible
                .iter()
                .find(|choice| choice.id == models.current)
                .map(|choice| Self {
                    harness: target_harness.into(),
                    choice: choice.clone(),
                });
        }
        // Multiple new tabs can be opened before connecting the first draft.
        draft
            .filter(|model| model.harness == target_harness)
            .cloned()
    }

    pub fn id_for(&self, harness: &str) -> Option<&str> {
        (self.harness == harness).then_some(self.choice.id.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(id: &str) -> ModelChoice {
        ModelChoice {
            id: id.into(),
            name: format!("Label for {id}"),
            description: None,
        }
    }

    #[test]
    fn new_tabs_inherit_the_confirmed_wire_id_not_the_adapter_default_or_label() {
        let choices = vec![choice("provider/default"), choice("provider/selected")];
        let models = SessionModels {
            current: "provider/selected".into(),
            choices: choices.clone(),
            config_id: Some("model".into()),
        };
        let inherited =
            InheritedModel::capture("OpenCode", "OpenCode", Some(&models), &choices, None).unwrap();
        assert_eq!(inherited.id_for("OpenCode"), Some("provider/selected"));
        assert_eq!(inherited.choice.name, "Label for provider/selected");
        assert!(inherited.id_for("Codex").is_none());
        let next =
            InheritedModel::capture("OpenCode", "OpenCode", None, &[], Some(&inherited)).unwrap();
        assert_eq!(next.choice.id, inherited.choice.id);
    }

    #[test]
    fn other_harnesses_and_models_hidden_by_provider_filter_are_not_inherited() {
        let choices = vec![choice("provider/default"), choice("provider/selected")];
        let models = SessionModels {
            current: "provider/selected".into(),
            choices: choices.clone(),
            config_id: None,
        };
        assert!(
            InheritedModel::capture("OpenCode", "Codex", Some(&models), &choices, None).is_none()
        );
        assert!(
            InheritedModel::capture("OpenCode", "OpenCode", Some(&models), &choices[..1], None)
                .is_none()
        );
        assert!(InheritedModel::capture("OpenCode", "OpenCode", None, &[], None).is_none());
    }
}
