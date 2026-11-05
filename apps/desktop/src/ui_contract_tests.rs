//! Prevent screens from bypassing Dunlin's shared button defaults.
use std::path::Path;

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
fn application_actions_use_shared_button_constructor() {
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
