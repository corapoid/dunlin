//! Normalize ACP model/mode selectors, including their older wire formats.
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionChoice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSelector {
    /// `None` means the harness uses the dedicated model/mode setter.
    pub config_id: Option<String>,
    pub current: String,
    pub choices: Vec<SessionChoice>,
}

pub type ModelChoice = SessionChoice;
pub type SessionModels = SessionSelector;
pub type SessionModes = SessionSelector;

impl SessionSelector {
    pub fn label(&self) -> &str {
        self.choices
            .iter()
            .find(|choice| choice.id == self.current)
            .map(|choice| choice.name.as_str())
            .unwrap_or(&self.current)
    }
}

fn config_selector(response: &Value, category: &str) -> Option<SessionSelector> {
    if let Some(options) = response.get("configOptions").and_then(Value::as_array) {
        for option in options {
            if option.get("category").and_then(Value::as_str) != Some(category)
                && option.get("id").and_then(Value::as_str) != Some(category)
            {
                continue;
            }
            let Some(options) = option.get("options").and_then(Value::as_array) else {
                continue;
            };
            let mut choices = Vec::new();
            for item in options {
                if let Some(group) = item.get("options").and_then(Value::as_array) {
                    choices.extend(group.iter().filter_map(config_choice));
                } else if let Some(choice) = config_choice(item) {
                    choices.push(choice);
                }
            }
            return Some(SessionSelector {
                config_id: Some(option.get("id")?.as_str()?.to_string()),
                current: option.get("currentValue")?.as_str()?.to_string(),
                choices,
            });
        }
    }
    None
}

pub(crate) fn session_models(response: &Value) -> Option<SessionModels> {
    if let Some(selector) = config_selector(response, "model") {
        return Some(selector);
    }
    let models = response.get("models")?;
    Some(SessionModels {
        config_id: None,
        current: models.get("currentModelId")?.as_str()?.to_string(),
        choices: models
            .get("availableModels")?
            .as_array()?
            .iter()
            .filter_map(|model| {
                Some(ModelChoice {
                    id: model.get("modelId")?.as_str()?.to_string(),
                    name: model.get("name")?.as_str()?.to_string(),
                    description: model
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect(),
    })
}

pub(crate) fn session_modes(response: &Value) -> Option<SessionModes> {
    if let Some(selector) = config_selector(response, "mode") {
        return Some(selector);
    }
    let modes = response.get("modes")?;
    Some(SessionSelector {
        config_id: None,
        current: modes.get("currentModeId")?.as_str()?.to_string(),
        choices: modes
            .get("availableModes")?
            .as_array()?
            .iter()
            .filter_map(|mode| {
                Some(SessionChoice {
                    id: mode.get("id")?.as_str()?.to_string(),
                    name: mode.get("name")?.as_str()?.to_string(),
                    description: mode
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect(),
    })
}

fn config_choice(value: &Value) -> Option<ModelChoice> {
    Some(ModelChoice {
        id: value.get("value")?.as_str()?.to_string(),
        name: value.get("name")?.as_str()?.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_flat_and_grouped_models_and_prefers_config_options() {
        let state = session_models(&json!({"configOptions": [{
            "id": "provider-model", "category": "model", "currentValue": "a",
            "options": [{"group": "provider", "options": [
                {"value": "a", "name": "Model A"}, {"value": "b", "name": "Model B"}
            ]}]
        }], "models": {"currentModelId": "legacy", "availableModels": []}}))
        .unwrap();
        assert_eq!(state.config_id.as_deref(), Some("provider-model"));
        assert_eq!(state.choices.len(), 2);
        assert_eq!(state.current, "a");
        let flat = session_models(&json!({"configOptions": [{
            "id": "model", "currentValue": "b", "options": [{"value": "b", "name": "B"}]
        }]}))
        .unwrap();
        assert_eq!(flat.choices[0].name, "B");
    }

    #[test]
    fn reads_legacy_models_and_ignores_unrelated_options() {
        assert!(session_models(&json!({"configOptions": [{"id": "mode"}]})).is_none());
        let state = session_models(&json!({"models": {
            "currentModelId": "provider/a", "availableModels": [{"modelId": "provider/a", "name": "A"}]
        }})).unwrap();
        assert_eq!(state.config_id, None);
        assert_eq!(state.choices[0].id, "provider/a");
    }

    #[test]
    fn reads_modes_without_inventing_unsupported_choices() {
        assert!(session_modes(&json!({})).is_none());
        let modes = session_modes(&json!({"modes": {
            "currentModeId": "build", "availableModes": [
                {"id": "build", "name": "Build", "description": "Make changes"},
                {"id": "plan", "name": "Plan", "description": "Plan before making changes"}
            ]
        }}))
        .unwrap();
        assert_eq!(modes.label(), "Build");
        assert_eq!(
            modes.choices[1].description.as_deref(),
            Some("Plan before making changes")
        );
        assert_eq!(modes.config_id, None);
    }

    #[test]
    fn config_modes_take_precedence_and_do_not_mix_with_models() {
        let response = json!({"configOptions": [
            {"id": "provider-model", "category": "model", "currentValue": "a", "options": [{"value": "a", "name": "A"}]},
            {"id": "agent-mode", "category": "mode", "currentValue": "plan", "options": [{"value": "plan", "name": "Plan"}]}
        ], "modes": {"currentModeId": "build", "availableModes": []}});
        let modes = session_modes(&response).unwrap();
        assert_eq!(modes.config_id.as_deref(), Some("agent-mode"));
        assert_eq!(modes.current, "plan");
        assert_eq!(session_models(&response).unwrap().current, "a");
    }
}
