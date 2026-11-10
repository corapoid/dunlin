//! Architectural guardrails complement the native control behavior smoke.
use std::path::Path;

#[test]
fn group_and_project_forms_use_dropdowns_without_hidden_runtime_controls() {
    let group = include_str!("group_settings.rs");
    let form = group.split("pub(crate) fn render(").nth(1).unwrap();
    assert!(form.contains("group-color-picker"));
    assert!(form.contains("dropdown_menu_with_anchor"));
    assert!(form.contains("group_choice_icon("));
    assert!(form.contains("harness_picker("));
    for removed in [
        "Client permissions",
        "Local environment",
        "Reset to inherited",
    ] {
        assert!(!form.contains(removed));
    }
    let overlays = include_str!("app/overlays.rs");
    let project = overlays
        .split("fn project_settings_modal(")
        .nth(1)
        .unwrap()
        .split("fn context_menu_overlay(")
        .next()
        .unwrap();
    assert!(project.contains("harness_picker("));
    assert!(project.contains("preview_project_cleanup("));
    for removed in [
        "project-permission",
        "Reset overrides",
        "Model & environment",
    ] {
        assert!(!project.contains(removed));
    }
    let runtime = include_str!("project_runtime_settings.rs")
        .split("pub(crate) fn render(")
        .nth(1)
        .unwrap();
    assert!(
        !runtime
            .split("#[cfg(test)]")
            .next()
            .unwrap()
            .contains("Environment overrides")
    );
}

#[test]
fn project_history_uses_the_same_recency_order_as_the_session_list() {
    let sidebar = include_str!("views/sidebar.rs");
    let project = sidebar
        .split("fn project_sessions(")
        .nth(1)
        .unwrap()
        .split("fn ")
        .next()
        .unwrap();
    assert!(project.contains("session_list::recency_key(session)"));
    assert!(!project.contains("running(&b.id)"));
}

#[test]
fn model_option_controls_and_menu_rows_do_not_add_decorative_icons() {
    let source = include_str!("views/chat_controls.rs");
    let options = source
        .split("pub(crate) fn model_options(")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn options(")
        .next()
        .unwrap();
    assert!(!options.contains(".icon("));
    assert!(!options.contains("IconName::SlidersHorizontal"));
    assert!(options.contains("None,"));
    assert!(source.contains("icon.filter(|_| !branded)"));
    assert!(options.contains("Action::ConfigOption("));
}

#[test]
fn titlebar_fields_and_standard_sidebar_icons_use_shared_controls() {
    let sidebar = include_str!("views/sidebar.rs");
    let switcher = sidebar
        .split("fn project_switcher(")
        .nth(1)
        .unwrap()
        .split("fn section_header(")
        .next()
        .unwrap();
    assert!(switcher.contains("titlebar_field("));
    assert!(!switcher.contains(".border_1()"));
    let titlebar = include_str!("views/titlebar.rs");
    assert!(titlebar.contains("titlebar_field(\"global-search-trigger\""));
    let navigation = include_str!("views/sidebar/navigation.rs");
    for icon in [
        "IconName::LayoutGrid",
        "IconName::MessageSquare",
        "IconName::Files",
    ] {
        assert!(navigation.contains(icon));
    }
    assert!(!navigation.contains("RetroIcon"));
}

#[test]
fn search_entrance_animation_restarts_only_on_open_and_honors_reduced_motion() {
    let palette = include_str!("views/palette.rs");
    assert!(palette.contains("this.reduce_motion || cx.reduce_motion()"));
    assert!(palette.contains("Duration::ZERO"));
    assert!(palette.contains("this.command_epoch"));
    assert!(palette.contains("search-panel-open"));
    assert!(palette.contains("search-backdrop-open"));
    let app = include_str!("app.rs");
    assert!(app.contains("if !self.command_open {\n            self.command_epoch = self.command_epoch.wrapping_add(1);"));
}

#[test]
fn skin_selection_is_a_text_only_dropdown_without_preview_cards() {
    let source = include_str!("views/settings.rs");
    let picker = source
        .split("fn skin_picker(")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn appearance_picker(")
        .next()
        .unwrap();
    assert!(picker.contains("dropdown_menu_with_anchor"));
    assert!(picker.contains("Skin::ALL"));
    assert!(picker.contains(".checked(skin == selected)"));
    for preview in [
        "grain_layer(",
        "choice_card(",
        "for_skin(",
        "skin.description(",
    ] {
        assert!(
            !picker.contains(preview),
            "skin selection must not render {preview}"
        );
    }
}

#[test]
fn free_endpoint_permission_is_gated_at_selection_and_wire_boundary() {
    let providers = include_str!("providers.rs");
    let selection = providers.split("fn select_provider(").nth(1).unwrap();
    assert!(
        selection.find("!self.opencode_free_consent").unwrap()
            < selection.find("self.opencode_provider =").unwrap()
    );
    let gate = providers
        .split("fn check_provider_model(")
        .nth(1)
        .unwrap()
        .split("fn reload_provider_models(")
        .next()
        .unwrap();
    assert!(gate.contains("self.selected_provider() == Some(FREE)"));
    assert!(gate.contains("self.provider_catalog.free_models.contains(id)"));
    assert!(gate.contains("self.open_free_endpoint_consent(false, cx)"));
    let harness = include_str!("harness.rs");
    let dispatch = harness.split("fn dispatch_pending_prompt(").nth(1).unwrap();
    assert!(
        dispatch.find("!self.check_provider_model(cx)").unwrap()
            < dispatch.find("harness.prompt(wire_text)").unwrap()
    );
    let dialog = include_str!("free_endpoint_consent.rs");
    assert!(dialog.contains("accepted: false"));
    assert!(dialog.contains("if !dialog.accepted"));
    assert!(dialog.contains(".disabled(!dialog.accepted)"));
    assert!(dialog.contains("It does not change or verify your OpenCode Console"));
    assert!(!dialog.contains("submit_automation_prompt("));
    assert!(!dialog.contains("send_prompt("));
}

#[test]
fn group_settings_keep_one_stable_scroll_and_confirm_deletion() {
    let source = include_str!("group_settings.rs");
    assert_eq!(source.matches(".overflow_y_scroll()").count(), 1);
    assert!(source.contains("scroll: ScrollHandle::new()"));
    assert!(source.contains(".track_scroll(&draft.scroll)"));
    assert!(source.contains("Scrollbar::vertical(&draft.scroll)"));
    assert!(source.contains("Delete group…"));
    assert!(source.contains("if answer.await != Ok(1)"));
    assert!(source.contains("this.delete_project_group(&id, cx)"));
}

fn has_literal_radius(source: &str) -> bool {
    // Whitespace-insensitive; covers per-corner methods, not only `.rounded`.
    let source: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    source.split(".rounded").skip(1).any(|tail| {
        let Some((_, argument)) = tail.split_once('(') else {
            return false;
        };
        argument.strip_prefix("px(").is_some_and(|value| {
            value
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == '-')
        })
    })
}

#[test]
fn radius_guard_covers_multiline_and_per_corner_literals() {
    assert!(has_literal_radius(".rounded ( px ( 12. ) )"));
    assert!(has_literal_radius(".rounded_tl(\n px(8.)\n)"));
    assert!(!has_literal_radius(".rounded(px(dunlin_ui::radii::CARD))"));
    assert!(!has_literal_radius(".rounded_full()"));
}

#[test]
fn application_surfaces_use_central_geometry_and_radius_tokens() {
    fn check(path: &Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path);
            } else if path.extension().is_some_and(|ext| ext == "rs")
                && path.file_name().unwrap() != "ui_contract_tests.rs"
            {
                let source = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !has_literal_radius(&source),
                    "{}: use dunlin-ui surface geometry/radius tokens",
                    path.display()
                );
                for legacy in [
                    "rounded_probe(",
                    "rounded_scroll_probe(",
                    "borderless_probe(",
                ] {
                    assert!(
                        !source.contains(legacy),
                        "{}: use panel_probe with the surface's geometry",
                        path.display()
                    );
                }
            }
        }
    }
    let desktop = Path::new(env!("CARGO_MANIFEST_DIR"));
    check(&desktop.join("src"));
    check(&desktop.join("../../crates/ui/src"));
}

fn has_constructor(source: &str, constructor: &str) -> bool {
    source.match_indices(constructor).any(|(index, _)| {
        source[..index]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
    })
}

#[test]
fn constructor_check_distinguishes_controls_from_prompt_answers() {
    assert!(has_constructor(
        concat!("view.child(Button", "::new(\"save\"))"),
        concat!("Button", "::new(")
    ));
    assert!(!has_constructor(
        concat!("PromptButton", "::new(\"Save\")"),
        concat!("Button", "::new(")
    ));
}

#[test]
fn application_controls_use_shared_constructors() {
    fn check(path: &Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                for constructor in [
                    concat!("Button", "::new("),
                    concat!("Checkbox", "::new("),
                    concat!("Radio", "::new("),
                    concat!("Switch", "::new("),
                    concat!("TabBar", "::new("),
                    concat!("Input", "::new("),
                    concat!("Textarea", "::new("),
                ] {
                    assert!(
                        !has_constructor(&source, constructor),
                        "{} bypasses shared control defaults with {constructor}",
                        path.display()
                    );
                }
            }
        }
    }
    check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
}

#[test]
fn migrated_action_surfaces_do_not_reintroduce_hand_drawn_buttons() {
    for (name, source) in [
        ("chat", include_str!("views/chat.rs")),
        ("dialogs and docks", include_str!("app/overlays.rs")),
        ("document tabs", include_str!("workbench.rs")),
    ] {
        assert!(
            !source.contains(".cursor_pointer()"),
            "{name}: use a shared action control, not a clickable div"
        );
    }
}

#[test]
fn ordinary_forms_keep_the_shared_focus_frame() {
    for (name, source) in [
        ("remote connection", include_str!("remote_connection.rs")),
        ("skills", include_str!("skills.rs")),
        ("sidebar forms", include_str!("views/sidebar.rs")),
        ("command search", include_str!("views/palette.rs")),
    ] {
        assert!(
            !source.contains(".appearance(false)") && !source.contains(".bordered(false)"),
            "{name}: use shared fields, not a static frame that loses focus/disabled styling"
        );
    }
}

#[test]
fn audited_settings_and_forms_use_standard_actions_not_quiet_text_buttons() {
    for (name, source) in [
        ("credential settings", include_str!("vault/settings.rs")),
        ("credential panel", include_str!("vault/panel.rs")),
        ("host task queue", include_str!("jobs_panel.rs")),
        ("provider authorization", include_str!("provider_auth.rs")),
        (
            "project runtime form",
            include_str!("project_runtime_settings.rs"),
        ),
        ("Git confirmations", include_str!("git_actions.rs")),
        (
            "Git mutation actions",
            include_str!("git_history/mutations.rs"),
        ),
        (
            "Repository inventory",
            include_str!("git_history/overview.rs"),
        ),
        ("Run actions", include_str!("run.rs")),
        (
            "Markdown resource form",
            include_str!("markdown_editor/resources.rs"),
        ),
    ] {
        assert!(
            !has_constructor(source, "dunlin_ui::button("),
            "{name}: use action_button for ordinary labeled actions"
        );
    }
    let settings = include_str!("views/settings.rs");
    for suffix in settings.split("dunlin_ui::button(").skip(1) {
        assert!(
            suffix.starts_with("format!(\"settings-tab:"),
            "only settings navigation may use the quiet base"
        );
    }
    for (name, source) in [
        ("server administration", include_str!("server_panel.rs")),
        ("remote connection", include_str!("remote_connection.rs")),
    ] {
        assert!(
            !has_constructor(source, "button("),
            "{name}: use standard actions, not the quiet base"
        );
    }
}

#[test]
fn standard_actions_do_not_override_their_size_or_become_ghost_buttons() {
    fn check(path: &Path) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                check(&path);
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs")
                || path.file_name().unwrap() == "ui_contract_tests.rs"
            {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for suffix in ["dunlin_ui::action_button(", "dunlin_ui::settings::button("]
                .into_iter()
                .flat_map(|constructor| source.split(constructor).skip(1))
            {
                let prefix = suffix
                    .split(".on_click(")
                    .next()
                    .unwrap()
                    .split("\n}")
                    .next()
                    .unwrap()
                    .split(';')
                    .next()
                    .unwrap();
                for legacy in [".small()", ".xsmall()", ".ghost()", ".text()"] {
                    assert!(
                        !prefix.contains(legacy),
                        "{}: standard action overrides its presentation with {legacy}",
                        path.display()
                    );
                }
            }
        }
    }
    check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"));
}

#[test]
fn settings_surfaces_share_the_recipe_and_agents_have_one_stateful_action() {
    for (name, source) in [
        ("settings", include_str!("views/settings.rs")),
        ("credentials", include_str!("vault/settings.rs")),
        ("host task queue", include_str!("jobs_panel.rs")),
        ("skills", include_str!("skills.rs")),
        ("extensions", include_str!("plugins.rs")),
        ("backup", include_str!("backup.rs")),
    ] {
        assert!(
            !has_constructor(source, "dunlin_ui::action_button("),
            "{name}: use the shared settings recipe"
        );
    }
    let source = include_str!("views/settings.rs");
    assert!(!source.contains("dunlin_ui::switch(format!(\"harness-visible:"));
    assert!(!source.contains("Show in agent picker"));
    assert!(!source.contains("harness-default:"));
    assert!(source.contains("let stage = agent_action("));
    assert!(source.contains("agent_import_visible(name, installed, cli_only)"));
}

#[test]
fn settings_do_not_restore_removed_agent_and_extension_panels() {
    let settings = include_str!("views/settings.rs");
    for removed in [
        "OpenCode provider accounts",
        "Custom ACP command",
        "harness-connect",
        "add-custom-harness",
    ] {
        assert!(!settings.contains(removed), "removed setting: {removed}");
    }
    let extensions = include_str!("plugins.rs");
    for removed in [
        "Technical details",
        "Methodology packs",
        "spec-tools\"",
        "extensions-open-proposals",
        "extensions-scope-global",
    ] {
        assert!(
            !extensions.contains(removed),
            "removed extension panel: {removed}"
        );
    }
    let skills = include_str!("skills.rs");
    assert!(!skills.contains("supporting files are installed together and are not editable here"));
    assert!(skills.contains("if skill.is_builtin()"));
    assert!(skills.contains("toggle_builtin_skill"));
}

#[test]
fn navigation_is_owned_by_the_sidebar_and_search_replaces_titlebar_history() {
    let chrome = include_str!("views/titlebar.rs");
    assert!(chrome.contains("global-search-trigger"));
    assert!(!chrome.contains("host_control::render"));
    assert!(!chrome.contains("title-git-history"));
    let app = include_str!("app.rs");
    assert!(!app.contains("self.rail(cx)"));
    let sidebar = include_str!("views/sidebar/navigation.rs");
    for id in [
        "sidebar-source-control",
        "sidebar-knowledge",
        "sidebar-intent",
        "sidebar-artifacts",
        "sidebar-security",
        "sidebar-settings",
    ] {
        assert!(sidebar.contains(id));
    }
    assert!(sidebar.contains(".when(this.spec_tools_enabled()"));
    assert!(!sidebar.contains("SidebarSection::Dashboard"));
    let profile = include_str!("user_profile.rs");
    assert!(
        !profile.contains("PopupMenuItem"),
        "compound theme controls must not sit in a hoverable menu row"
    );
    assert!(profile.contains(".anchor(Anchor::TopRight)"));
    assert!(!profile.contains("Application settings"));
    assert!(!profile.contains("profile-settings"));
    assert!(profile.contains("github_accounts.len() > 1"));
    assert!(!include_str!("views/sidebar.rs").contains("search-projects"));
}

#[test]
fn git_forms_and_harness_choices_use_shared_ui_controls() {
    let mutations = include_str!("git_history/mutations.rs");
    assert!(mutations.contains("dunlin_ui::settings::input"));
    assert!(mutations.contains("dunlin_ui::settings::button"));
    assert!(!mutations.contains("compact_input("));
    let controls = include_str!("views/chat_controls.rs");
    let trigger = controls
        .split("pub(crate) fn harness(")
        .nth(1)
        .unwrap()
        .split(".dropdown_menu_with_anchor")
        .next()
        .unwrap();
    assert!(
        !trigger.contains("harness_brand::logo"),
        "composer shows only the selected harness name"
    );
    assert!(controls.contains("let branded = matches!(action, Action::Harness(_))"));
}

#[test]
fn project_tools_are_tabs_and_audit_execution_is_not_settings_configuration() {
    let sidebar = include_str!("views/sidebar/navigation.rs");
    assert!(sidebar.contains(".when(this.security_workspace_enabled()"));
    assert!(sidebar.contains(".font_family(dunlin_ui::theme::code_font())"));
    let profile = include_str!("user_profile.rs");
    assert!(profile.contains("profile_appearance_picker"));
    let tools = include_str!("workspace_tools.rs");
    assert!(!tools.contains("open_window("));
    assert!(tools.contains("TabKey::Tool"));
    let runner = include_str!("security/workspace.rs");
    assert!(runner.contains("submit_automation_prompt"));
    assert!(runner.contains("visible_models()"));
    assert!(!runner.contains("install_catalog_skill"));
    assert!(!runner.contains("request_settings_location"));
    let chrome = include_str!("app/overlays.rs");
    assert!(
        !chrome.contains(".when(!chat,"),
        "Git sync and change counts also belong in Chat"
    );
}

#[test]
fn workspace_surfaces_share_reading_colors_and_audit_refresh_is_report_scoped() {
    let spec = include_str!("spec.rs");
    let render = spec
        .split("pub(crate) fn render(")
        .nth(1)
        .unwrap()
        .split("#[cfg(test)]")
        .next()
        .unwrap();
    assert!(
        !render.contains(".bg(p.bg)"),
        "the Spec root must not repaint the reading surface"
    );
    assert!(render.contains(".when(!this.spec_kit_available()"));
    let audit = include_str!("security/workspace.rs");
    assert!(!audit.contains(".max_w(px(1100.))"));
    assert!(audit.contains("run.report.is_some()"));
    assert!(audit.contains(".bg(p.card)"));
    let controls = include_str!("views/chat_controls.rs");
    assert!(controls.contains(".label(option.selector.label())"));
    assert!(controls.contains("menu.label(current.name.clone())"));
    let app = include_str!("app.rs");
    assert!(app.contains("capture_action"));
    assert!(app.contains("OutdentInline"));
}

#[test]
fn welcome_is_a_focused_default_choice_and_portable_export_uses_switches() {
    let welcome = include_str!("onboarding.rs");
    assert!(welcome.contains("Choose your default harness"));
    assert!(!welcome.contains("Scan again"));
    assert!(!welcome.contains("Set default"));
    assert!(welcome.contains("choice_indicator(active, p)"));
    assert!(welcome.contains("IconName::ChevronDown"));
    assert!(welcome.contains("Duration::from_secs(2)"));
    let backup = include_str!("backup.rs");
    assert!(backup.contains("dunlin_ui::switch(\"backup-settings\""));
    assert!(backup.contains("dunlin_ui::switch(\"backup-projects\""));
    assert!(!backup.contains("dunlin_ui::checkbox(\"backup-settings\""));
    let portable = include_str!("backup/data.rs");
    assert!(
        !portable.contains("pending_adapter_install"),
        "install consent must never be portable preference data"
    );
}

#[test]
fn repository_inventory_keeps_rounded_sections_and_the_workspaces_background() {
    let inventory = include_str!("git_history/overview.rs");
    assert!(inventory.contains("SurfaceGeometry::SETTINGS_CARD"));
    assert!(
        !inventory.contains(".bg(p.bg)"),
        "a full-size square fill covers the reading surface corners"
    );
    assert!(
        !inventory.contains("dunlin_ui::action_button("),
        "inventory actions use the shared rounded recipe"
    );
    let files = include_str!("git_history/details/files.rs");
    assert!(!files.contains(".bg(p.panel)"));
    assert!(!files.contains("else { p.panel }"));
    let fields = include_str!("../../../crates/ui/src/controls.rs");
    assert!(fields.contains("scale::INPUT_HEIGHT"));
    let compact = fields
        .split("pub fn compact_input(")
        .nth(1)
        .unwrap()
        .split("pub fn search_input(")
        .next()
        .unwrap();
    assert!(
        !compact.contains(".small()"),
        "search and form fields must not diverge in height"
    );
    assert!(!include_str!("../../../crates/ui/src/settings.rs").contains("Styled::h("));
}
