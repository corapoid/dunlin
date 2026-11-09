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
