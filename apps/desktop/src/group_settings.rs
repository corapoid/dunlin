//! Explicit inheritable group defaults; saving never restarts connected agents.
use dunlin_domain::{GroupSettings, PermissionMode, ResolvedSettings, SettingSource};
use dunlin_ui::{
    component::{
        Sizable,
        button::{Button, ButtonVariants},
        input::{Editor, EditorState, Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::app::DunlinApp;

pub(crate) struct Draft {
    pub group_id: String,
    settings: GroupSettings,
    model_harness: String,
    model: Entity<InputState>,
    environment: Entity<EditorState>,
    error: Option<String>,
}

impl Draft {
    pub(crate) fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.model.read(cx).focus_handle(cx)
    }
}

pub(crate) fn source_label(this: &DunlinApp, source: &SettingSource) -> String {
    match source {
        SettingSource::Global => "Global settings".into(),
        SettingSource::Group(id) => format!(
            "Group: {}",
            crate::project_tree::group_path(&this.project_groups, id)
        ),
        SettingSource::Project(id) => format!(
            "Project: {}",
            this.projects
                .iter()
                .find(|p| &p.id == id)
                .map(|p| p.name.as_str())
                .unwrap_or("missing")
        ),
    }
}

/// Expose each effective value's source, but never paint inherited env values
/// into a summary or copy them into a descendant's editable override fields.
pub(crate) fn render_sources(
    this: &DunlinApp,
    resolved: &ResolvedSettings,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let model = resolved.default_models.get(&resolved.harness.value);
    let mut rows = div()
        .id("settings-value-sources")
        .max_h(px(140.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(4.))
        .text_size(px(10.5))
        .text_color(p.muted_fg)
        .child(format!(
            "Harness: {} · {}",
            resolved.harness.value,
            source_label(this, &resolved.harness.source)
        ))
        .child(format!(
            "Permissions: {} · {}",
            resolved.permission_mode.value.label(),
            source_label(this, &resolved.permission_mode.source)
        ))
        .child(
            model
                .map(|model| {
                    format!(
                        "Model: {} · {}",
                        model.value,
                        source_label(this, &model.source)
                    )
                })
                .unwrap_or_else(|| "Model: harness default".into()),
        );
    for (name, value) in &resolved.environment {
        rows = rows.child(format!(
            "Env {name} · {}",
            source_label(this, &value.source)
        ));
    }
    rows.into_any_element()
}

fn updated_settings(
    settings: &GroupSettings,
    harness: &str,
    model: &str,
    environment: &str,
) -> Result<GroupSettings, String> {
    let mut settings = settings.clone();
    settings.set_default_model(harness, model)?;
    settings.environment = dunlin_domain::parse_project_environment(environment)?;
    Ok(settings)
}

/// Validate the complete proposed inheritance before writing anything, including
/// combined environment limits of affected descendants. Never changes projects.
pub(crate) fn validate_update(
    id: &str,
    settings: &GroupSettings,
    groups: &[dunlin_domain::ProjectGroup],
    projects: &[dunlin_domain::Project],
    harness: &str,
    mode: PermissionMode,
) -> Result<(), String> {
    let mut proposed = groups.to_vec();
    let group = proposed
        .iter_mut()
        .find(|g| g.id == id)
        .ok_or("The group no longer exists")?;
    group.settings = settings.clone();
    dunlin_domain::resolve_group_settings(Some(id), &proposed, harness, mode)?
        .process_environment()?;
    let descendants = crate::project_tree::descendants(&proposed, id);
    for group in proposed
        .iter()
        .filter(|group| descendants.contains(&group.id))
    {
        dunlin_domain::resolve_group_settings(Some(&group.id), &proposed, harness, mode)?
            .process_environment()?;
    }
    for project in projects.iter().filter(|p| {
        !p.is_remote()
            && p.group_id
                .as_ref()
                .is_some_and(|id| descendants.contains(id))
    }) {
        dunlin_domain::resolve_project_settings(project, &proposed, harness, mode)?
            .process_environment()?;
    }
    Ok(())
}

impl DunlinApp {
    pub(crate) fn open_group_settings(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.project_groups.iter().find(|g| g.id == id) else {
            return;
        };
        let resolved = match dunlin_domain::resolve_group_settings(
            Some(id),
            &self.project_groups,
            &self.default_harness,
            self.permission_mode,
        ) {
            Ok(settings) => settings,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let model_harness = resolved.harness.value;
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
                group
                    .settings
                    .default_model(&model_harness)
                    .unwrap_or_default(),
                window,
                cx,
            )
        });
        let text = group
            .settings
            .environment
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("\n");
        environment.update(cx, |state, cx| state.set_value(text, window, cx));
        self.group_settings_draft = Some(Draft {
            group_id: id.into(),
            settings: group.settings.clone(),
            model_harness,
            model,
            environment,
            error: None,
        });
        self.group_prompt = None;
        self.project_group_editor = None;
        self.project_settings_target = None;
        self.project_runtime_draft = None;
        self.context_menu = None;
        cx.notify();
    }

    pub(crate) fn close_group_settings(&mut self, cx: &mut Context<Self>) {
        self.group_settings_draft = None;
        cx.notify();
    }

    fn select_group_harness(
        &mut self,
        harness: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(draft) = &self.group_settings_draft else {
            return;
        };
        let inherited = self
            .project_groups
            .iter()
            .find(|g| g.id == draft.group_id)
            .ok_or_else(|| "The group no longer exists".to_string())
            .and_then(|g| {
                dunlin_domain::resolve_group_settings(
                    g.parent_id.as_deref(),
                    &self.project_groups,
                    &self.default_harness,
                    self.permission_mode,
                )
            });
        let inherited = match inherited {
            Ok(settings) => settings.harness.value,
            Err(error) => {
                self.set_notice(error, cx);
                return;
            }
        };
        let draft = self.group_settings_draft.as_mut().unwrap();
        if let Err(error) = draft
            .settings
            .set_default_model(&draft.model_harness, &draft.model.read(cx).value())
        {
            draft.error = Some(error);
            cx.notify();
            return;
        }
        draft.model_harness = harness.clone().unwrap_or(inherited);
        draft.settings.harness = harness;
        let model = draft
            .settings
            .default_model(&draft.model_harness)
            .unwrap_or_default()
            .to_string();
        draft
            .model
            .update(cx, |state, cx| state.set_value(model, window, cx));
        draft.error = None;
        cx.notify();
    }

    fn save_group_settings(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.group_settings_draft else {
            return;
        };
        let Some(index) = self
            .project_groups
            .iter()
            .position(|g| g.id == draft.group_id)
        else {
            self.close_group_settings(cx);
            return;
        };
        let result = updated_settings(
            &draft.settings,
            &draft.model_harness,
            &draft.model.read(cx).value(),
            &draft.environment.read(cx).value(),
        )
        .and_then(|settings| {
            validate_update(
                &draft.group_id,
                &settings,
                &self.project_groups,
                &self.projects,
                &self.default_harness,
                self.permission_mode,
            )?;
            Ok(settings)
        });
        let settings = match result {
            Ok(settings) => settings,
            Err(error) => {
                self.group_settings_draft.as_mut().unwrap().error = Some(error);
                cx.notify();
                return;
            }
        };
        // Preserve current parent/name/tone, even if membership changed since open.
        let mut group = self.project_groups[index].clone();
        group.settings = settings;
        if let Some(store) = &self.store
            && let Err(error) = store.upsert_project_group(&group)
        {
            self.group_settings_draft.as_mut().unwrap().error =
                Some(format!("Could not save group settings: {error}"));
            cx.notify();
            return;
        }
        self.project_groups[index] = group;
        self.group_settings_draft = None;
        self.sync_vault_read_only();
        self.set_notice("Group defaults saved · client permissions apply to following requests; harness/model to new conversations; environment at the next local connection. No agents restarted.", cx);
    }

    fn reset_group_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.group_settings_draft else {
            return;
        };
        draft.settings = GroupSettings::default();
        draft
            .model
            .update(cx, |state, cx| state.set_value("", window, cx));
        draft
            .environment
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.select_group_harness(None, window, cx);
    }
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(draft) = &this.group_settings_draft else {
        return div().into_any_element();
    };
    let Some(group) = this.project_groups.iter().find(|g| g.id == draft.group_id) else {
        return div().into_any_element();
    };
    let mut harnesses = div().flex().flex_wrap().gap(px(5.));
    let mut harness_options = vec![None];
    harness_options.extend(crate::harness_discovery::catalog_names().map(Some));
    harness_options.push(Some("Custom ACP"));
    for name in harness_options {
        let selected = draft.settings.harness.as_deref() == name;
        harnesses = harnesses.child(
            Button::new(format!("group-harness-{}", name.unwrap_or("inherit")))
                .small()
                .label(name.unwrap_or("Inherit"))
                .when(selected, |b| b.primary())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_group_harness(name.map(str::to_owned), window, cx)
                })),
        );
    }
    let mut permissions = div().flex().flex_wrap().gap(px(5.));
    for mode in [
        None,
        Some(PermissionMode::Ask),
        Some(PermissionMode::ReadOnly),
        Some(PermissionMode::AutoApprove),
    ] {
        let selected = draft.settings.permission_mode == mode;
        permissions = permissions.child(
            Button::new(format!(
                "group-permissions-{}",
                mode.map(|m| m.label()).unwrap_or("inherit")
            ))
            .small()
            .label(mode.map(|m| m.label()).unwrap_or("Inherit"))
            .when(selected, |b| b.primary())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(draft) = &mut this.group_settings_draft {
                    draft.settings.permission_mode = mode;
                }
                cx.notify();
            })),
        );
    }
    let preview = updated_settings(
        &draft.settings,
        &draft.model_harness,
        &draft.model.read(cx).value(),
        &draft.environment.read(cx).value(),
    )
    .and_then(|settings| {
        let mut groups = this.project_groups.clone();
        groups
            .iter_mut()
            .find(|g| g.id == draft.group_id)
            .unwrap()
            .settings = settings;
        dunlin_domain::resolve_group_settings(
            Some(&draft.group_id),
            &groups,
            &this.default_harness,
            this.permission_mode,
        )
    });
    div().id("group-settings-overlay").absolute().inset_0().size_full().flex().items_center().justify_center()
        .on_click(cx.listener(|this, _, _, cx| this.close_group_settings(cx)))
        .child(div().id("group-settings-panel").w(px(620.)).max_h(px(620.)).overflow_y_scroll()
            .child(crate::effects::rounded_probe(&this.overlay_backdrop, 12.))
            .rounded(px(12.)).border_1().border_color(p.border_strong).bg(p.card).text_color(p.fg).p(px(16.)).flex().flex_col().gap(px(10.))
            .on_click(|_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(format!("Group settings · {}", group.name)))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Inherited by projects in this group and its subgroups unless overridden. No files are moved and no agents are restarted. Approval changes affect following requests, including existing group conversations (strictest member policy wins)."))
            .child(div().text_size(px(11.)).child("Default harness"))
            .child(harnesses)
            .child(div().text_size(px(11.)).child("Client permissions"))
            .child(permissions)
            .child(div().text_size(px(11.)).child(format!("Default model · {} · blank = inherit", draft.model_harness)))
            .child(Input::new(&draft.model).small())
            .child(div().text_size(px(11.)).child("Local environment overrides · NAME=value; omitted keys inherit"))
            .child(div().h(px(100.)).overflow_hidden().child(Editor::new(&draft.environment).h_full()))
            .child(div().text_size(px(10.5)).text_color(p.muted_fg).child("Values are literal and stored unencrypted locally, not in Keychain. Do not paste secrets. Environment/model defaults are not forwarded to relay; env applies at the next local connection."))
            .child(match preview { Ok(resolved) => render_sources(this, &resolved, cx), Err(error) => div().text_size(px(11.)).text_color(p.red).child(error).into_any_element() })
            .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error)))
            .child(div().flex().flex_wrap().gap(px(7.))
                .child(Button::new("reset-group-defaults").small().label("Reset to inherited").on_click(cx.listener(|this, _, window, cx| this.reset_group_draft(window, cx))))
                .child(div().flex_1())
                .child(Button::new("cancel-group-settings").small().label("Cancel").on_click(cx.listener(|this, _, _, cx| this.close_group_settings(cx))))
                .child(Button::new("save-group-settings").small().primary().label("Save group defaults").on_click(cx.listener(|this, _, _, cx| this.save_group_settings(cx))))))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{updated_settings, validate_update};
    use dunlin_domain::{GroupSettings, PermissionMode};

    #[test]
    fn edits_only_explicit_overrides_and_clearing_restores_inheritance() {
        let mut original = GroupSettings {
            harness: Some("OpenCode".into()),
            permission_mode: Some(PermissionMode::ReadOnly),
            ..Default::default()
        };
        original.set_default_model("Claude Code", "keep").unwrap();
        let edited = updated_settings(&original, "OpenCode", "new", "LANG=pl_PL.UTF-8").unwrap();
        assert_eq!(edited.default_model("Claude Code"), Some("keep"));
        assert_eq!(edited.permission_mode, Some(PermissionMode::ReadOnly));
        assert!(original.environment.is_empty());
        let cleared = updated_settings(&edited, "OpenCode", "", "").unwrap();
        assert!(cleared.environment.is_empty());
        assert_eq!(cleared.default_model("OpenCode"), None);
        assert!(updated_settings(&edited, "OpenCode", "new", "bad").is_err());
        assert!(
            validate_update(
                "missing",
                &original,
                &[],
                &[],
                "OpenCode",
                PermissionMode::Ask
            )
            .is_err()
        );
    }

    #[test]
    fn ancestor_updates_validate_combined_limits_of_empty_descendant_groups() {
        let root = dunlin_domain::ProjectGroup {
            id: "root".into(),
            name: "Root".into(),
            tone: dunlin_domain::ProjectTone::Blue,
            parent_id: None,
            settings: GroupSettings::default(),
        };
        let mut child = dunlin_domain::ProjectGroup {
            id: "child".into(),
            parent_id: Some(root.id.clone()),
            ..root.clone()
        };
        child.settings.environment =
            dunlin_domain::parse_project_environment("EXTRA=child").unwrap();
        let settings = GroupSettings {
            environment: dunlin_domain::parse_project_environment(
                &(0..128).map(|i| format!("K{i}=v\n")).collect::<String>(),
            )
            .unwrap(),
            ..Default::default()
        };
        assert!(
            validate_update(
                "root",
                &settings,
                &[root.clone(), child.clone()],
                &[],
                "OpenCode",
                PermissionMode::Ask
            )
            .is_err()
        );
        // Overriding an existing key does not increase the combined count.
        child.settings.environment = dunlin_domain::parse_project_environment("K0=child").unwrap();
        assert!(
            validate_update(
                "root",
                &settings,
                &[root, child],
                &[],
                "OpenCode",
                PermissionMode::Ask
            )
            .is_ok()
        );
    }
}
