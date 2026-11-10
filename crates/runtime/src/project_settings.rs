//! Runtime defaults independent of settings forms. UI choices and local approval
//! policy are never replaced by a delayed relay settings response.
use dunlin_agent::{ModelChoice, SessionModels};
use dunlin_domain::{Project, ProjectSettings};

pub fn refresh_remote_defaults(
    current: &mut Project,
    expected: &Project,
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

pub fn updated_settings(
    current: &ProjectSettings,
    harness: &str,
    model: &str,
) -> Result<ProjectSettings, String> {
    let mut updated = current.clone();
    updated.set_default_model(harness, model)?;
    Ok(updated)
}

pub fn live_model_target(
    preferred: Option<&str>,
    models: Option<&SessionModels>,
    visible: &[ModelChoice],
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changing_a_model_does_not_replace_environment_or_permissions() {
        let mut original = ProjectSettings {
            permission_mode: Some(dunlin_domain::PermissionMode::ReadOnly),
            ..Default::default()
        };
        original
            .environment
            .insert("LANG".into(), "pl_PL.UTF-8".into());
        original
            .set_default_model("Claude Code", "other-model")
            .unwrap();
        let updated = updated_settings(&original, "OpenCode", "provider/model").unwrap();
        assert_eq!(updated.environment, original.environment);
        assert_eq!(updated.permission_mode, original.permission_mode);
        assert_eq!(updated.default_model("Claude Code"), Some("other-model"));
        assert_eq!(original.default_model("OpenCode"), None);
    }

    #[test]
    fn applying_a_default_requires_an_idle_advertised_visible_model() {
        let choices = vec![
            ModelChoice {
                id: "a".into(),
                name: "Label A".into(),
                description: None,
            },
            ModelChoice {
                id: "b".into(),
                name: "Label B".into(),
                description: None,
            },
        ];
        let models = SessionModels {
            current: "a".into(),
            choices: choices.clone(),
            config_id: None,
        };
        assert_eq!(
            live_model_target(Some("b"), Some(&models), &choices, false).unwrap(),
            Some("b".into())
        );
        assert_eq!(
            live_model_target(Some("a"), Some(&models), &choices, false).unwrap(),
            None
        );
        assert!(live_model_target(Some("b"), Some(&models), &choices[..1], false).is_err());
        assert!(live_model_target(Some("b"), Some(&models), &choices, true).is_err());
        assert!(live_model_target(Some("Label B"), Some(&models), &choices, false).is_err());
        assert!(live_model_target(Some("b"), None, &choices, false).is_err());
    }
}
