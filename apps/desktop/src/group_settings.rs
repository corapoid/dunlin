//! Explicit inheritable group defaults; saving never restarts connected agents.
use dunlin_domain::{GroupSettings, PermissionMode, ResolvedSettings, SettingSource};
use dunlin_ui::{
    component::{
        Disableable, Sizable,
        button::ButtonVariants,
        input::{Editor, EditorState, Input, InputState},
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::app::DunlinApp;

pub(crate) struct Draft {
    pub group_id: String,
    tone: dunlin_domain::ProjectTone,
    settings: GroupSettings,
    model_harness: String,
    model: Entity<InputState>,
    environment: Entity<EditorState>,
    focus: FocusHandle,
    error: Option<String>,
}

impl Draft {
    pub(crate) fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus.clone()
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
            focus: cx.focus_handle(),
            group_id: id.into(),
            tone: group.tone,
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
        // Preserve current parent/name, even if membership changed since open.
        let mut group = self.project_groups[index].clone();
        group.tone = draft.tone;
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
        self.refresh_project_icons(cx);
        self.sync_vault_read_only();
        self.set_notice("Group defaults saved · client permissions apply to following requests; harness/model to new conversations; environment at the next local connection. No agents restarted.", cx);
    }

    fn reset_group_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &mut self.group_settings_draft else {
            return;
        };
        let icon = draft.settings.icon.clone();
        draft.settings = GroupSettings::default();
        draft.settings.icon = icon;
        draft
            .model
            .update(cx, |state, cx| state.set_value("", window, cx));
        draft
            .environment
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.select_group_harness(None, window, cx);
    }

    fn choose_group_icon(&mut self, cx: &mut Context<Self>) {
        let Some(draft) = &self.group_settings_draft else {
            return;
        };
        let group = draft.group_id.clone();
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose group icon (PNG, JPEG, SVG or ICO; up to 2 MiB)".into()),
        });
        let paths = self.overlay_backdrop.track_native(paths, cx);
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let result = cx
                .background_executor()
                .spawn(async move { crate::project_icons::read_icon(path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(draft) = this
                    .group_settings_draft
                    .as_mut()
                    .filter(|d| d.group_id == group)
                {
                    match result {
                        Ok(icon) => {
                            draft.settings.icon = Some(icon);
                            draft.error = None;
                        }
                        Err(error) => draft.error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    #[cfg(debug_assertions)]
    pub(crate) fn smoke_group_appearance(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let group = dunlin_domain::ProjectGroup {
            id: "appearance-fixture".into(),
            name: "Appearance fixture".into(),
            tone: dunlin_domain::ProjectTone::Blue,
            parent_id: None,
            settings: Default::default(),
        };
        self.store
            .as_ref()
            .unwrap()
            .upsert_project_group(&group)
            .unwrap();
        self.project_groups.push(group);
        self.open_group_settings("appearance-fixture", window, cx);
        let draft = self.group_settings_draft.as_mut().unwrap();
        draft.tone = dunlin_domain::ProjectTone::Rose;
        draft.settings.icon = Some("aws-lambda".into());
        self.save_group_settings(cx);
        let saved = self
            .store
            .as_ref()
            .unwrap()
            .project_groups()
            .unwrap()
            .into_iter()
            .find(|g| g.id == "appearance-fixture")
            .unwrap();
        assert_eq!(saved.tone, dunlin_domain::ProjectTone::Rose);
        assert_eq!(saved.settings.icon.as_deref(), Some("aws-lambda"));
        self.open_group_settings("appearance-fixture", window, cx);
        let draft = self.group_settings_draft.as_mut().unwrap();
        draft.settings.icon = Some("kubernetes".into());
        draft.tone = dunlin_domain::ProjectTone::Teal;
        self.close_group_settings(cx);
        let saved = self
            .project_groups
            .iter()
            .find(|g| g.id == "appearance-fixture")
            .unwrap();
        assert_eq!(saved.tone, dunlin_domain::ProjectTone::Rose);
        assert_eq!(saved.settings.icon.as_deref(), Some("aws-lambda"));
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
    let mut colors = div().flex().flex_wrap().gap(px(6.));
    for tone in [
        dunlin_domain::ProjectTone::Blue,
        dunlin_domain::ProjectTone::Green,
        dunlin_domain::ProjectTone::Violet,
        dunlin_domain::ProjectTone::Amber,
        dunlin_domain::ProjectTone::Rose,
        dunlin_domain::ProjectTone::Teal,
    ] {
        let button = dunlin_ui::button(format!("group-color:{tone:?}"))
            .label(format!("{tone:?}"))
            .when(draft.tone == tone, |b| b.primary())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(draft) = &mut this.group_settings_draft {
                    draft.tone = tone;
                }
                cx.notify();
            }));
        #[cfg(debug_assertions)]
        let button = button
            .relative()
            .when(tone == dunlin_domain::ProjectTone::Teal, |b| {
                b.children(crate::workspace_ui_smoke::probe("group-color-teal"))
            });
        colors = colors.child(button);
    }
    let mut icons = div().flex().flex_wrap().gap(px(6.));
    for (value, label) in [
        (None, "Color dot"),
        (Some("aws-lambda"), "AWS Lambda"),
        (Some("kubernetes"), "Kubernetes"),
    ] {
        let button = dunlin_ui::button(format!("group-icon:{label}"))
            .label(label)
            .when(draft.settings.icon.as_deref() == value, |b| b.primary())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(draft) = &mut this.group_settings_draft {
                    draft.settings.icon = value.map(str::to_string);
                }
                cx.notify();
            }));
        #[cfg(debug_assertions)]
        let button =
            button
                .relative()
                .children(crate::workspace_ui_smoke::probe(if value.is_none() {
                    "group-icon-dot"
                } else if value == Some("kubernetes") {
                    "group-icon-kubernetes"
                } else {
                    "group-icon-lambda"
                }));
        icons = icons.child(button);
    }
    icons = icons.child(
        dunlin_ui::button("group-icon-custom")
            .label(
                if draft
                    .settings
                    .icon
                    .as_ref()
                    .is_some_and(|v| v.starts_with("png:"))
                {
                    "Change custom icon…"
                } else {
                    "Custom icon…"
                },
            )
            .on_click(cx.listener(|this, _, _, cx| this.choose_group_icon(cx))),
    );
    let mut harnesses = div().flex().flex_wrap().gap(px(5.));
    let mut harness_options = vec![None];
    harness_options.extend(crate::harness_discovery::catalog_names().map(Some));
    harness_options.push(Some("Custom ACP"));
    for name in harness_options {
        if name.is_some_and(|name| !this.harness_visible(name)) {
            continue;
        }
        let selected = draft.settings.harness.as_deref() == name;
        harnesses = harnesses.child(
            dunlin_ui::button(format!("group-harness-{}", name.unwrap_or("inherit")))
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
            dunlin_ui::button(format!(
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
    let save = dunlin_ui::button("save-group-settings")
        .primary()
        .label("Save group settings")
        .on_click(cx.listener(|this, _, _, cx| this.save_group_settings(cx)));
    let cleanup_group = draft.group_id.clone();
    #[cfg(debug_assertions)]
    let save = save
        .relative()
        .children(crate::workspace_ui_smoke::probe("group-settings-save"));
    div().id("group-settings-overlay").absolute().inset_0().size_full().flex().items_center().justify_center()
        .on_click(cx.listener(|this, _, _, cx| this.close_group_settings(cx)))
        .child(div().id("group-settings-panel").w(px(620.)).max_w(relative(0.95)).max_h(relative(0.85)).overflow_hidden()
            .child(crate::effects::rounded_probe(&this.overlay_backdrop, 12.))
            .rounded(px(12.)).border_1().border_color(p.border_strong).bg(p.card).text_color(p.fg).flex().flex_col()
            .on_click(|_, _, cx| cx.stop_propagation())
            .track_focus(&draft.focus)
            .child(div().id("group-settings-content").min_h(px(0.)).overflow_y_scroll().p(px(16.)).flex().flex_col().gap(px(10.))
            .child(div().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).child(format!("Group settings · {}", group.name)))
            .child(div().text_size(px(12.)).font_weight(FontWeight::MEDIUM).child("Group appearance"))
            .child(div().text_size(px(11.)).text_color(p.muted_fg).child("Choose a logo or your own image instead of the dot, or keep a color dot."))
            .child(icons)
            .child(div().text_size(px(11.)).child("Dot color · used when no image is selected"))
            .child(colors)
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
            .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error))))
            .child(div().p(px(16.)).border_t_1().border_color(p.border).flex_shrink_0().flex().flex_wrap().gap(px(7.))
                .child(dunlin_ui::button("reset-group-defaults").label("Reset to inherited").on_click(cx.listener(|this, _, window, cx| this.reset_group_draft(window, cx))))
                .child(dunlin_ui::button("group-clear").label(if this.group_cleanup.busy { "Inspecting…" } else { "Clear…" })
                    .disabled(this.group_cleanup.busy)
                    .tooltip("Review ignored dependency/cache folders, then move them to Trash. Never removes source or tracked files.")
                    .on_click(cx.listener(move |this, _, window, cx| this.preview_group_cleanup(cleanup_group.clone(), window, cx))))
                .child(div().flex_1())
                .child(dunlin_ui::button("cancel-group-settings").label("Cancel").on_click(cx.listener(|this, _, _, cx| this.close_group_settings(cx))))
                .child(save)))
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
