//! Architectural guardrails complement the native control behavior smoke.
use std::path::Path;

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
        ("SSH settings", include_str!("ssh.rs")),
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
fn settings_surfaces_share_the_recipe_and_agent_visibility_has_a_label() {
    for (name, source) in [
        ("settings", include_str!("views/settings.rs")),
        ("credentials", include_str!("vault/settings.rs")),
        ("SSH", include_str!("ssh.rs")),
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
    assert!(source.contains(".label(\"Show in agent picker\")"));
    assert!(source.contains("agent_import_visible(name, installed, cli_only)"));
}
