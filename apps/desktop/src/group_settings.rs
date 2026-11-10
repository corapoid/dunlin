//! Explicit inheritable group defaults; saving never restarts connected agents.
use dunlin_domain::{GroupSettings, PermissionMode, ResolvedSettings, SettingSource};
use dunlin_ui::{
    component::{
        Disableable,
        button::ButtonVariants,
        input::InputState,
        menu::{DropdownMenu, PopupMenuItem},
    },
    prelude::FluentBuilder as _,
    *,
};

use crate::app::{DunlinApp, tone_color};

pub(crate) struct Draft {
    pub group_id: String,
    tone: dunlin_domain::ProjectTone,
    settings: GroupSettings,
    model_harness: String,
    model: Entity<InputState>,
    focus: FocusHandle,
    pub(crate) scroll: ScrollHandle,
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

/// Show the source of harness/model defaults without exposing hidden settings.
pub(crate) fn render_sources(
    this: &DunlinApp,
    resolved: &ResolvedSettings,
    cx: &App,
) -> AnyElement {
    let p = palette(cx);
    let model = resolved.default_models.get(&resolved.harness.value);
    let rows = div()
        .id("settings-value-sources")
        .flex_shrink_0()
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
    rows.into_any_element()
}

fn updated_settings(
    settings: &GroupSettings,
    harness: &str,
    model: &str,
) -> Result<GroupSettings, String> {
    let mut settings = settings.clone();
    settings.set_default_model(harness, model)?;
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
        self.group_settings_draft = Some(Draft {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            group_id: id.into(),
            tone: group.tone,
            settings: group.settings.clone(),
            model_harness,
            model,
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

    fn confirm_delete_group(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = &self.group_settings_draft else {
            return;
        };
        let id = draft.group_id.clone();
        let Some(group) = self.project_groups.iter().find(|group| group.id == id) else {
            return;
        };
        let answer = window.prompt(PromptLevel::Warning, &format!("Delete group ‘{}’?", group.name),
            Some("Projects and subgroups will move up one level. No files or conversations are deleted. Unsaved group settings will be discarded. You can undo the group deletion."),
            &["Cancel", "Delete group"], cx);
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if this
                    .group_settings_draft
                    .as_ref()
                    .is_none_or(|draft| draft.group_id != id)
                {
                    return;
                }
                this.delete_project_group(&id, cx);
                if !this.project_groups.iter().any(|group| group.id == id) {
                    this.close_group_settings(cx);
                }
            });
        })
        .detach();
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
        self.set_notice(
            "Group defaults saved · harness/model apply to new conversations. No agents restarted.",
            cx,
        );
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

/// One compact harness dropdown shared by project and group forms.
pub(crate) fn harness_picker(
    this: &DunlinApp,
    id: &'static str,
    selected: Option<&str>,
    disabled: bool,
    cx: &mut Context<DunlinApp>,
    on_select: impl Fn(&mut DunlinApp, Option<String>, &mut Window, &mut Context<DunlinApp>) + 'static,
) -> AnyElement {
    let selected = selected.map(str::to_owned);
    let mut options = vec![None];
    options.extend(
        crate::harness_discovery::catalog_names()
            .chain(std::iter::once("Custom ACP"))
            .filter(|name| this.harness_visible(name))
            .map(|name| Some(name.to_owned())),
    );
    if selected.is_some() && !options.contains(&selected) {
        options.push(selected.clone());
    }
    let app = cx.entity().downgrade();
    let on_select = std::rc::Rc::new(on_select);
    dunlin_ui::settings::button(id)
        .label(selected.clone().unwrap_or_else(|| "Inherit".into()))
        .accessibility_label("Default harness")
        .w(px(220.))
        .max_w_full()
        .dropdown_caret(true)
        .disabled(disabled)
        .relative()
        .children(probe(id))
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, _| {
            for name in &options {
                let app = app.clone();
                let on_select = on_select.clone();
                let name = name.clone();
                menu = menu.item(
                    PopupMenuItem::new(name.clone().unwrap_or_else(|| "Inherit".into()))
                        .checked(name == selected)
                        .on_click(move |_, window, cx| {
                            let _ = app
                                .update(cx, |this, cx| on_select(this, name.clone(), window, cx));
                        }),
                );
            }
            menu
        })
        .into_any_element()
}

pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let Some(draft) = &this.group_settings_draft else {
        return div().into_any_element();
    };
    let Some(group) = this.project_groups.iter().find(|g| g.id == draft.group_id) else {
        return div().into_any_element();
    };
    let app = cx.entity().downgrade();
    let colors = dunlin_ui::settings::button("group-color-picker")
        .child(dot(tone_color(&p, draft.tone), 12.))
        .label(format!("{:?}", draft.tone))
        .accessibility_label("Group dot color")
        .w(px(180.))
        .max_w_full()
        .dropdown_caret(true)
        .relative()
        .children(probe("group-color-picker"))
        .dropdown_menu_with_anchor(Anchor::BottomLeft, move |mut menu, _, cx| {
            let Some(entity) = app.upgrade() else {
                return menu;
            };
            let Some(draft) = &entity.read(cx).group_settings_draft else {
                return menu;
            };
            let selected = draft.tone;
            for tone in [
                dunlin_domain::ProjectTone::Blue,
                dunlin_domain::ProjectTone::Green,
                dunlin_domain::ProjectTone::Violet,
                dunlin_domain::ProjectTone::Amber,
                dunlin_domain::ProjectTone::Rose,
                dunlin_domain::ProjectTone::Teal,
            ] {
                let app = app.clone();
                menu = menu.item(
                    crate::effects::menu_item(move |_, cx| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .min_w(px(130.))
                            .child(dot(tone_color(&palette(cx), tone), 12.))
                            .child(format!("{tone:?}"))
                            .relative()
                            .when(tone == dunlin_domain::ProjectTone::Teal, |row| {
                                row.children(probe("group-color-teal"))
                            })
                    })
                    .checked(tone == selected)
                    .on_click(move |_, _, cx| {
                        let _ = app.update(cx, |this, cx| {
                            if let Some(draft) = &mut this.group_settings_draft {
                                draft.tone = tone;
                            }
                            cx.notify();
                        });
                    }),
                );
            }
            menu
        });
    let mut icons = div().flex().flex_wrap().gap(px(6.));
    for (value, label) in [
        (None, "Color dot"),
        (Some("aws-lambda"), "AWS Lambda"),
        (Some("kubernetes"), "Kubernetes"),
    ] {
        let button = dunlin_ui::settings::segment(
            format!("group-icon:{label}"),
            label,
            draft.settings.icon.as_deref() == value,
        )
        .child(crate::project_icons::group_choice_icon(
            value, draft.tone, 18., &p,
        ))
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
        dunlin_ui::settings::button("group-icon-custom")
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
    let harnesses = harness_picker(
        this,
        "group-harness-picker",
        draft.settings.harness.as_deref(),
        false,
        cx,
        |this, harness, window, cx| this.select_group_harness(harness, window, cx),
    );
    let preview = updated_settings(
        &draft.settings,
        &draft.model_harness,
        &draft.model.read(cx).value(),
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
    let save = dunlin_ui::settings::button("save-group-settings")
        .primary()
        .label("Save group settings")
        .on_click(cx.listener(|this, _, _, cx| this.save_group_settings(cx)));
    let cleanup_group = draft.group_id.clone();
    #[cfg(debug_assertions)]
    let save = save
        .relative()
        .children(crate::workspace_ui_smoke::probe("group-settings-save"));
    let content = div().w_full().min_w(px(0.)).p(px(18.)).flex().flex_col().gap(px(16.))
        .child(dunlin_ui::settings::notice("Group scope · this Mac. Changes remain drafts until Save. Cancel discards them. Defaults inherit from parent groups, then global settings.", p))
        .child(dunlin_ui::settings::field("Group appearance", icons, p))
        .child(dunlin_ui::settings::field("Dot color · used when no image is selected", colors, p))
        .child(dunlin_ui::settings::description("Inherited by projects and subgroups unless overridden. Harness/model defaults apply to new conversations. No agents are restarted.", p))
        .child(dunlin_ui::settings::field("Default harness", harnesses, p))
        .child(dunlin_ui::settings::field(&format!("Default model · {} · blank = inherit", draft.model_harness), dunlin_ui::settings::input(&draft.model, p).aria_label("Group default model"), p))
        .child(match preview { Ok(resolved) => render_sources(this, &resolved, cx), Err(error) => div().text_size(px(11.)).text_color(p.red).child(error).into_any_element() })
        .when_some(draft.error.clone(), |view, error| view.child(div().text_size(px(11.)).text_color(p.red).child(error)))
        .child(dunlin_ui::settings::actions()
            .child(dunlin_ui::settings::button("group-clear").label(if this.group_cleanup.busy { "Inspecting…" } else { "Clean group…" })
                .relative().children(crate::gui_probe::marker("group-clear"))
                .disabled(this.group_cleanup.busy)
                .tooltip("Review generated dependencies, caches and builds with sizes, then move selected folders to system Trash.")
                .on_click(cx.listener(move |this, _, window, cx| this.preview_group_cleanup(cleanup_group.clone(), window, cx)))));
    div()
        .id("group-settings-overlay")
        .absolute()
        .inset_0()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p(px(16.))
        .on_click(cx.listener(|this, _, _, cx| this.close_group_settings(cx)))
        .child(
            dialog_surface(p)
                .id("group-settings-panel")
                .relative()
                .role(Role::Dialog)
                .aria_label("Group settings")
                .tab_group()
                .w(px(700.))
                .max_w_full()
                .h(relative(0.85))
                .min_h(px(0.))
                .p(px(0.))
                .gap(px(0.))
                .overflow_hidden()
                .children(probe("group-settings-panel"))
                .child(crate::effects::panel_probe(
                    &this.overlay_backdrop,
                    SurfaceGeometry::DIALOG,
                    false,
                ))
                .on_click(|_, _, cx| cx.stop_propagation())
                .track_focus(&draft.focus)
                .child(
                    div()
                        .p(px(18.))
                        .flex_shrink_0()
                        .border_b_1()
                        .border_color(p.border)
                        .child(
                            div()
                                .text_size(px(dunlin_ui::settings::Tokens::TITLE_SIZE))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(format!("Group settings · {}", group.name)),
                        ),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(0.))
                        .child(
                            div()
                                .id("group-settings-content")
                                .h_full()
                                .overflow_y_scroll()
                                .track_scroll(&draft.scroll)
                                .relative()
                                .children(probe("group-settings-scroll"))
                                .child(content),
                        )
                        .child(
                            gpui_kit::base::Scrollbar::vertical(&draft.scroll)
                                .mode(gpui_kit::base::ScrollbarMode::Always)
                                .viewport_from_layout(),
                        ),
                )
                .child(
                    dunlin_ui::settings::actions()
                        .p(px(16.))
                        .border_t_1()
                        .border_color(p.border)
                        .flex_shrink_0()
                        .relative()
                        .children(probe("group-settings-actions"))
                        .child(
                            dunlin_ui::settings::button("delete-settings-group")
                                .danger()
                                .outline()
                                .label("Delete group…")
                                .relative()
                                .children(probe("group-settings-delete"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_delete_group(window, cx)
                                })),
                        )
                        .child(div().flex_1())
                        .child(
                            dunlin_ui::settings::button("cancel-group-settings")
                                .label("Cancel")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.close_group_settings(cx)),
                                ),
                        )
                        .child(save),
                ),
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
        original.environment =
            dunlin_domain::parse_project_environment("LANG=pl_PL.UTF-8").unwrap();
        let edited = updated_settings(&original, "OpenCode", "new").unwrap();
        assert_eq!(edited.default_model("Claude Code"), Some("keep"));
        assert_eq!(edited.permission_mode, Some(PermissionMode::ReadOnly));
        assert_eq!(edited.environment, original.environment);
        let cleared = updated_settings(&edited, "OpenCode", "").unwrap();
        assert_eq!(cleared.environment, original.environment);
        assert_eq!(cleared.default_model("OpenCode"), None);
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
