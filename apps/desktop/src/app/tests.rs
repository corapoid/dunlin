#[test]
fn stale_terminal_and_run_main_views_migrate_to_chat_instead_of_failing_settings() {
    use crate::app::StoredSettings;
    use dunlin_domain::MainView;
    let settings: StoredSettings =
        serde_json::from_str(r#"{"view":"Terminal","default_view":"Run"}"#).unwrap();
    assert_eq!(settings.view, MainView::Chat);
    assert_eq!(settings.default_view, MainView::Chat);
    let settings: StoredSettings =
        serde_json::from_str(r#"{"view":"Graph","default_view":"Trace"}"#).unwrap();
    assert_eq!(settings.view, MainView::Graph);
    assert_eq!(settings.default_view, MainView::Trace);
}

#[test]
fn footer_context_has_one_canonical_token_display() {
    use super::context_status_label;
    use crate::harness::HarnessUsage;
    let text = context_status_label(
        Some(HarnessUsage {
            used: 6500,
            size: 1_000_000,
        }),
        6500,
    );
    assert_eq!(text, "6.5k / 1.0M context");
    assert_eq!(text.matches("6.5k").count(), 1);
    assert_eq!(context_status_label(None, 0), "Context idle");
    assert_eq!(context_status_label(None, 123), "123 tokens");
    assert_eq!(
        context_status_label(Some(HarnessUsage { used: 42, size: 0 }), 100),
        "42 tokens"
    );
}

#[test]
fn startup_recovers_only_unused_unavailable_harness_stubs() {
    use super::{blank_session, startup_session_recovery};
    use crate::harness::ChatItem;
    let opencode = blank_session("project", "OpenCode", None);
    let claude = blank_session("project", "Claude Code", None);
    let mut sessions = vec![opencode, claude];
    let errors = vec![ChatItem::Error {
        message: "missing adapter".into(),
        at_ms: 1,
    }];
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "OpenCode", false, true, false),
        Some(0)
    );
    assert_eq!(sessions[1].harness, "Claude Code");
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "OpenCode", true, true, false),
        None
    );
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "OpenCode", false, false, false),
        None
    );
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "OpenCode", false, true, true),
        None
    );
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "Custom ACP", false, true, false),
        None
    );
    let history = vec![ChatItem::User {
        text: "preserve my work".into(),
        timing: None,
        at_ms: 1,
    }];
    assert_eq!(
        startup_session_recovery(&sessions, 1, &history, "OpenCode", false, true, false),
        None
    );
    sessions[1].acp_session_id = Some("existing-claude-context".into());
    assert_eq!(
        startup_session_recovery(&sessions, 1, &errors, "OpenCode", false, true, false),
        None
    );
}

#[test]
fn composer_sends_on_enter_but_not_shift_enter() {
    use dunlin_ui::component::input::InputEvent;
    for secondary in [false, true] {
        assert!(super::is_composer_submit(&InputEvent::PressEnter {
            secondary,
            shift: false
        }));
        assert!(!super::is_composer_submit(&InputEvent::PressEnter {
            secondary,
            shift: true
        }));
    }
    assert!(!super::is_composer_submit(&InputEvent::Change));
}

use super::{
    Command, CommandItem, StoredSettings, command_matches, display_path, editor_language,
    format_bytes, migrate_data_dir, path_inside, session_index_after_delete,
};
use dunlin_domain::{MainView, PermissionMode};
use dunlin_ui::assets::IconName;
use std::path::Path;

fn item(label: &str, keywords: &str, hint: Option<&str>) -> CommandItem {
    CommandItem {
        command: Command::View(MainView::Chat),
        label: label.into(),
        hint: hint.map(str::to_string),
        icon: IconName::Search,
        keywords: keywords.into(),
    }
}

#[test]
fn command_filter_is_case_insensitive() {
    let item = item("Toggle theme", "appearance dark light", Some("⌘⇧L"));
    assert!(command_matches(&item, &["theme"]));
    assert!(command_matches(&item, &["TOGGLE"]));
    assert!(!command_matches(&item, &["folder"]));
}

#[test]
fn command_filter_matches_keywords_and_hints() {
    let item = item("Open folder", "workspace project directory", Some("⌘O"));
    assert!(command_matches(&item, &["workspace"]));
    assert!(command_matches(&item, &["⌘o"]));
    assert!(command_matches(&item, &["open", "folder"]));
}

#[test]
fn command_filter_requires_every_term() {
    let item = item("New session", "chat conversation", Some("⌘N"));
    assert!(command_matches(&item, &["new", "session"]));
    assert!(!command_matches(&item, &["new", "theme"]));
}

#[test]
fn stored_settings_fill_defaults_for_missing_fields() {
    let settings: StoredSettings =
        serde_json::from_str(r#"{"default_harness":"Claude Code"}"#).unwrap();
    assert_eq!(settings.default_harness, "Claude Code");
    assert_eq!(settings.permission_mode, PermissionMode::Ask);
    assert!(settings.auto_connect);
    assert!(settings.restore_session);
    assert_eq!(settings.code_font_size, 13.0);
    assert_eq!(settings.ui_font_family, "Geist");
    assert_eq!(settings.code_font_family, "Geist Mono");
    assert!(!settings.autosave);
    assert_eq!(settings.view, MainView::Chat);
    assert_eq!(settings.default_view, MainView::Chat);
    assert_eq!(settings.last_project_id, None);
    assert_eq!(
        settings.opencode_provider.as_deref(),
        Some(crate::providers::GO)
    );
    assert!(settings.recent_models.is_empty());
    assert!(!settings.final_answer_only);
    assert!(settings.stream_responses);
    assert!(!settings.chat_timestamps);
    assert!(!settings.chat_duration);
    assert!(settings.chat_logo);
    let mut changed = settings;
    changed.final_answer_only = true;
    changed.stream_responses = false;
    changed.chat_timestamps = true;
    changed.chat_duration = true;
    changed.chat_logo = false;
    changed.ui_font_family = "Helvetica Neue".into();
    changed.code_font_family = "Menlo".into();
    changed.code_font_size = 17.0;
    let restored: StoredSettings =
        serde_json::from_str(&serde_json::to_string(&changed).unwrap()).unwrap();
    assert!(restored.final_answer_only);
    assert!(!restored.stream_responses);
    assert!(restored.chat_timestamps);
    assert!(restored.chat_duration);
    assert!(!restored.chat_logo);
    assert_eq!(restored.ui_font_family, "Helvetica Neue");
    assert_eq!(restored.code_font_family, "Menlo");
    assert_eq!(restored.code_font_size, 17.0);
}

#[test]
fn unavailable_saved_fonts_fall_back_to_bundled_faces() {
    let installed = vec!["Geist".into(), "Geist Mono".into(), "Menlo".into()];
    assert_eq!(
        super::available_font("Menlo", "Geist Mono", &installed),
        "Menlo"
    );
    assert_eq!(
        super::available_font("Removed font", "Geist Mono", &installed),
        "Geist Mono"
    );
    assert_eq!(super::available_font("", "Geist", &installed), "Geist");
}

#[test]
fn system_theme_and_global_blame_roundtrip_without_resetting_legacy_theme() {
    use super::Appearance;
    use dunlin_ui::{WindowAppearance, component::ThemeMode};
    let legacy: StoredSettings = serde_json::from_str(r#"{"dark":false,"minimal_mode":true,"project_view":"List"}"#).unwrap();
    assert_eq!(legacy.appearance, None);
    assert!(!legacy.dark && !legacy.inline_blame);
    let mut saved = StoredSettings::default();
    assert_eq!(saved.appearance, Some(Appearance::System));
    saved.inline_blame = true;
    let restored: StoredSettings = serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
    assert!(restored.inline_blame);
    assert_eq!(restored.appearance, Some(Appearance::System));
    assert_eq!(Appearance::System.mode(WindowAppearance::Dark), ThemeMode::Dark);
    assert_eq!(Appearance::System.mode(WindowAppearance::Light), ThemeMode::Light);
    assert_eq!(Appearance::Light.mode(WindowAppearance::Dark), ThemeMode::Light);
    assert_eq!(Appearance::Dark.mode(WindowAppearance::Light), ThemeMode::Dark);
}

#[test]
fn new_sessions_are_ready_and_legacy_empty_drafts_are_recovered_safely() {
    use dunlin_domain::SessionStatus;
    let mut session = super::blank_session("project", "OpenCode", None);
    assert_eq!(session.status, SessionStatus::Idle);
    assert_eq!(session.status.label(), "Ready");
    assert_eq!(session.cost_usd, 0.0);
    assert_eq!(session.tokens, 0);
    assert!(session.acp_session_id.is_none());
    let saved = serde_json::to_string(&session).unwrap();
    let restored: dunlin_domain::SessionSummary = serde_json::from_str(&saved).unwrap();
    assert_eq!(restored.status, SessionStatus::Idle);
    for status in [SessionStatus::Waiting, SessionStatus::Interrupted] {
        session.status = status;
        assert_eq!(
            super::recovered_session_status(&session, true),
            SessionStatus::Idle
        );
    }
    session.status = SessionStatus::Waiting;
    assert_eq!(
        super::recovered_session_status(&session, false),
        SessionStatus::Interrupted
    );
    session.tokens = 100;
    assert_eq!(
        super::recovered_session_status(&session, true),
        SessionStatus::Interrupted
    );
    session.status = SessionStatus::Failed;
    assert_eq!(
        super::recovered_session_status(&session, true),
        SessionStatus::Failed
    );
}

#[test]
fn editor_language_uses_known_extensions() {
    assert_eq!(editor_language("src/main.rs"), "rust");
    assert_eq!(editor_language("README.md"), "markdown");
    assert_eq!(editor_language("ui/theme.tsx"), "tsx");
    assert_eq!(editor_language("Cargo.toml"), "toml");
    assert_eq!(editor_language("LICENSE"), "plaintext");
}

#[test]
fn byte_sizes_are_human_readable() {
    assert_eq!(format_bytes(512), "512 B");
    assert_eq!(format_bytes(2048), "2.0 KB");
    assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
}

#[test]
fn deleting_a_session_keeps_a_sensible_selection() {
    // Removing the active session selects the next one, or the last one.
    assert_eq!(session_index_after_delete(1, 1, 3), 1);
    assert_eq!(session_index_after_delete(2, 2, 2), 1);
    // Removing an earlier session shifts the selection down.
    assert_eq!(session_index_after_delete(0, 2, 2), 1);
    // Removing a later session leaves the selection alone.
    assert_eq!(session_index_after_delete(3, 1, 3), 1);
    // A fresh session is created when the list becomes empty.
    assert_eq!(session_index_after_delete(0, 0, 1), 0);
}

#[test]
fn prompts_use_relative_paths_inside_the_project() {
    assert_eq!(
        display_path("/work/dunlin", Path::new("/work/dunlin/src/main.rs")),
        "src/main.rs"
    );
    assert_eq!(
        display_path("/work/dunlin", Path::new("/tmp/notes.md")),
        "/tmp/notes.md"
    );
}

#[test]
fn new_files_must_stay_inside_the_project() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("dunlin-paths-{unique}"));
    std::fs::create_dir_all(root.join("src")).unwrap();
    let outside = std::env::temp_dir().join(format!("dunlin-outside-{unique}.txt"));

    assert!(path_inside(&root, &root.join("src/main.rs")));
    assert!(path_inside(&root, &root.join("README.md")));
    assert!(!path_inside(&root, &outside));

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn migrates_the_legacy_data_directory() {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!("dunlin-migrate-{unique}"));
    let legacy = root.join("Kestrel");
    let current = root.join("Dunlin");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("kestrel.db"), b"database").unwrap();
    std::fs::write(legacy.join("kestrel.db-wal"), b"wal").unwrap();

    let migrated = migrate_data_dir(&legacy, &current);

    assert_eq!(migrated, current);
    assert!(!legacy.exists());
    assert_eq!(
        std::fs::read(current.join("dunlin.db")).unwrap(),
        b"database"
    );
    assert_eq!(
        std::fs::read(current.join("dunlin.db-wal")).unwrap(),
        b"wal"
    );

    let _ = std::fs::remove_dir_all(&root);
}
