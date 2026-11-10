//! Native acceptance with a disposable project, pinned skill files and mock ACP.
//! Reports below are fixture data, never results of a real security audit.
use super::*;
use crate::app::{Command, StoredSettings, UiDensity};
use crate::workspace_ui_smoke::{click, measured};
use dunlin_ui::component::TitleBar;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let dir = PathBuf::from(std::env::args_os().nth(2).unwrap());
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-workspace-tools-smoke-")
    );
    fs::create_dir(&dir).expect("fresh fixture directory");
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("README.md"), "# Disposable tools fixture\n").unwrap();
    let bin = dir.join("bin");
    fs::create_dir(&bin).unwrap();
    for cli in ["gh", "glab"] {
        let path = bin.join(cli);
        fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mock = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("mock_acp_agent");
    assert!(
        mock.is_file(),
        "Build mock_acp_agent before the tools smoke"
    );
    // This entry point runs before worker threads. No real home, account CLI or adapter is used.
    unsafe {
        std::env::set_var("HOME", &dir);
        std::env::set_var("PATH", format!("{}:/usr/bin:/bin", bin.display()));
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::set_var("DUNLIN_HARNESS_COMMAND", &mock);
        std::env::set_var("DUNLIN_MOCK_CONFIG_VARIANTS", "1");
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        for key in [
            "DUNLIN_MOCK_RICH_CHAT",
            "DUNLIN_MOCK_DELAY_MS",
            "DUNLIN_MOCK_REJECT_MODEL",
            "DUNLIN_MOCK_NO_LOAD",
            "DUNLIN_MOCK_CONFIG_MODES",
        ] {
            std::env::remove_var(key);
        }
    }
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                restore_session: false,
                notifications_enabled: false,
                vault_enabled: false,
                onboarding_step: 2,
                reduce_motion: true,
                default_harness: "Custom ACP".into(),
                permission_mode: dunlin_domain::PermissionMode::ReadOnly,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "tools-fixture".into(),
            name: "Tools fixture".into(),
            path: root.to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        })
        .unwrap();
    drop(store);
    crate::platform::set_application_name();
    #[cfg(target_os = "macos")]
    assert_eq!(
        objc2_foundation::NSProcessInfo::processInfo()
            .processName()
            .to_string(),
        "Dunlin"
    );
    crate::workspace_ui_smoke::enable_probes();
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.tools-smoke", "Dunlin Tools Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let (parent, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(60.), px(60.)), size(px(1200.), px(850.))))),
            ..TitleBar::window_options()
        }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        owner.update(cx, |app, cx| {
            app.recent_models.insert(0, crate::providers::RecentModel { scope: "local:Custom ACP".into(), id: "mock-b".into() });
            app.harness.transcript.push(crate::harness::ChatItem::User { text: "Fixture conversation".into(), timing: None, at_ms: 1 });
            let session = app.session().clone();
            app.store.as_ref().unwrap().save_transcript(&session.id, &app.harness.transcript).unwrap();
            app.section = dunlin_domain::SidebarSection::Sessions; app.sidebar_tab = crate::sidebar_layout::Tab::Sessions;
            assert!(!app.security_workspace_enabled());
            cx.notify();
        });
        cx.spawn(async move |cx| {
            let start = Instant::now(); let mut stage = 0;
            let mut menu_size = None; let mut session_height = px(0.); let mut footer_height = px(0.);
            let mut commit_card = None; let mut change_offset = px(0.);
            let mut inherited_from = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(180)).await;
                let done = cx.update(|cx| {
                    assert!(start.elapsed() < Duration::from_secs(85), "tools smoke stalled at {stage}: {:?}", owner.read(cx).security.workspace.error);
                    parent.update(cx, |_, window, cx| { window.refresh(); window.draw(cx).clear(cx); }).unwrap();
                    match stage {
                        0 => {
                            session_height = measured("compact-session-row").size.height;
                            footer_height = measured("sidebar-knowledge").size.height;
                            measured("status-git-changes"); // visible while the active document is Chat
                            parent.update(cx, |_, window, cx| click(window, measured("title-user-menu").center(), cx)).unwrap();
                            stage = 1;
                        }
                        1 => {
                            menu_size = Some(measured("profile-menu").size);
                            parent.update(cx, |_, window, cx| click(window, measured("profile-density").center(), cx)).unwrap();
                            stage = 2;
                        }
                        2 => {
                            assert_eq!(owner.read(cx).density, UiDensity::Compact);
                            assert_eq!(measured("profile-menu").size, menu_size.unwrap(), "Minimal view must not resize the avatar menu");
                            assert!(measured("minimal-session-row").size.height < session_height);
                            assert!(measured("sidebar-knowledge").size.height < footer_height);
                            parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "escape")).unwrap();
                            stage = 3;
                        }
                        3 => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                app.open_workspace_tool(Tool::Knowledge, window, cx);
                                app.open_workspace_tool(Tool::Artifacts, window, cx);
                                let keys = app.workspace_tab_keys();
                                assert!(keys.contains(&TabKey::Tool(app.project().id.clone(), Tool::Knowledge)));
                                assert!(keys.contains(&TabKey::Tool(app.project().id.clone(), Tool::Artifacts)));
                                assert_eq!(app.current_workspace_tool(), Some(Tool::Artifacts));
                            })).unwrap();
                            stage = 4;
                        }
                        4 => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                app.split_workspace(window, cx);
                                app.drop_workspace_tab(&crate::workspace_panes::TabDrag {
                                    key: TabKey::Tool(app.project().id.clone(), Tool::Knowledge), project: app.project().id.clone()
                                }, 1, window, cx);
                                assert!(app.focus_workspace_pane(0, window, cx));
                            })).unwrap();
                            stage = 41;
                        }
                        41 => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                assert_eq!(app.current_workspace_tool(), Some(Tool::Artifacts));
                                assert_eq!(app.workbench.panes.active[1], Some(TabKey::Tool(app.project().id.clone(), Tool::Knowledge)));
                                crate::workspace_ui_smoke::assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                app.merge_workspace(cx);
                                assert!(!app.overlay_window.is_open(), "knowledge and artifacts are ordinary documents, not overlay windows");
                                app.open_workspace_tool(Tool::Knowledge, window, cx);
                                let catalog = dunlin_plugin::catalog_plugin("dunlin-governance").unwrap();
                                let pack = dunlin_plugin::LoadedPack { manifest: catalog.manifest, manifest_hash: catalog.manifest_hash,
                                    source: dunlin_plugin::Source::External, directory: Some(root.join("fixture-governance").to_string_lossy().into_owned()) };
                                app.plugins.packs.retain(|entry| entry.manifest.id != "dunlin-governance");
                                app.plugins.packs.push(pack.clone());
                                if !app.plugins.governance_enabled_for(None) { app.plugins.toggle_for(None, &pack); }
                                app.run_command(Command::Spec, window, cx);
                                assert!(app.spec.is_active_for(app.project()), "Intent opens immediately after enabling its package while Knowledge was active");
                                assert_eq!(app.current_workspace_tool(), None);
                                let pack = app.plugins.packs.iter().find(|pack| pack.manifest.id == "spec-kit").unwrap().clone();
                                let project = app.project().id.clone();
                                if !app.plugins.is_enabled_for(Some(&project), &pack) { app.plugins.toggle_for(Some(&project), &pack); }
                                assert!(app.spec_kit_available());
                                crate::workspace_ui_smoke::take_measured("spec-install-pack-skill");
                            })).unwrap();
                            stage = 42;
                        }
                        42 => {
                            assert!(crate::workspace_ui_smoke::take_measured("spec-install-pack-skill").is_none(), "enabled Spec Kit must not offer installation");
                            owner.update(cx, |app, cx| {
                                let pack = app.plugins.packs.iter().find(|pack| pack.manifest.id == "spec-kit").unwrap().clone();
                                app.plugins.toggle_for(Some(&app.project().id.clone()), &pack);
                                assert!(!app.spec_kit_available()); cx.notify();
                            });
                            stage = 43;
                        }
                        43 => {
                            parent.update(cx, |_, window, cx| click(window, measured("spec-install-pack-skill").center(), cx)).unwrap();
                            stage = 44;
                        }
                        44 if !owner.read(cx).skills_ui.busy => {
                            owner.update(cx, |app, cx| {
                                assert!(app.spec_kit_available(), "installation must enable Spec Kit and register its files: {:?}", app.skills_ui.message);
                                assert!(app.notice.is_some(), "installation outcome must be visible outside Settings");
                                crate::workspace_ui_smoke::take_measured("spec-install-pack-skill");
                                cx.notify();
                            });
                            stage = 45;
                        }
                        45 => {
                            assert!(crate::workspace_ui_smoke::take_measured("spec-install-pack-skill").is_none(), "installation action disappears once the skill is enabled");
                            owner.update(cx, |app, cx| app.install_catalog_skill(crate::skills::SECURITY_AUDIT_SKILL, crate::skills::Target::Shared, crate::skills::Scope::Global, "", None, cx));
                            stage = 5;
                        }
                        5 if !owner.read(cx).skills_ui.busy && owner.read(cx).security_workspace_enabled() => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.open_workspace_tool(Tool::Security, window, cx))).unwrap();
                            stage = 6;
                        }
                        6 => {
                            assert_eq!(owner.read(cx).security.workspace.harness, "Custom ACP");
                            assert_eq!(owner.read(cx).security.workspace.model.as_ref().unwrap().read(cx).value().as_ref(), "mock-b");
                            measured("sidebar-security");
                            assert!(crate::workspace_ui_smoke::take_measured("project-audit-refresh").is_none(), "no report means no refresh-results action");
                            assert!(measured("reading-surface").size.width - measured("project-audit-form").size.width <= px(52.), "audit form must fill the workspace width, with only its page insets");
                            parent.update(cx, |_, window, cx| click(window, measured("run-project-audit").center(), cx)).unwrap();
                            stage = 7;
                        }
                        7 if owner.read(cx).harness.transcript.iter().any(|item| matches!(item, crate::harness::ChatItem::User { text, .. } if text.contains("complete source-only workflow"))) => {
                            owner.update(cx, |app, cx| {
                                assert_eq!(app.session_harness(), "Custom ACP");
                                assert_eq!(app.harness.models.as_ref().unwrap().current, "mock-b", "the last used model, not the adapter's initial model, must receive the audit prompt");
                                assert_eq!(app.current_workspace_tool(), Some(Tool::Security));
                                let runs = dunlin_audit::discover_runs(&dunlin_audit::run_root(&dir, &app.project().id));
                                let run = runs.first().expect("audit run directory");
                                // Simulate an external harness report. This is not a real audit.
                                fs::write(run.dir.join("REPORT.md"), "# Fixture report\nNo real security audit was performed.\n").unwrap();
                                app.refresh_security_runs(cx);
                            });
                            stage = 8;
                        }
                        8 if !owner.read(cx).security.loading && !owner.read(cx).harness.running && owner.read(cx).security.runs.iter().any(|run| run.files.iter().any(|file| file == "REPORT.md")) => {
                            measured("project-audit-refresh");
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                app.security.workspace.model.as_ref().unwrap().update(cx, |input, cx| input.set_value("unavailable-fixture-model", window, cx));
                            })).unwrap();
                            parent.update(cx, |_, window, cx| click(window, measured("run-project-audit").center(), cx)).unwrap();
                            stage = 9;
                        }
                        9 if owner.read(cx).security.workspace.error.as_ref().is_some_and(|error| error.contains("not advertised")) => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                assert!(!app.harness.transcript.iter().any(|item| matches!(item, crate::harness::ChatItem::User { text, .. } if text.contains("complete source-only workflow"))), "an unavailable model must not dispatch to the adapter default");
                                crate::workspace_ui_smoke::assert_painted_corners(window, "reading-surface", app.render_workbench_tabs(0, cx).1);
                                let project = app.project().id.clone();
                                app.close_workspace_tool(&project, Tool::Artifacts, cx);
                                assert!(app.workspace_tab_keys().contains(&TabKey::Tool(project.clone(), Tool::Knowledge)));
                                assert!(!app.workspace_tab_keys().contains(&TabKey::Tool(project, Tool::Artifacts)));
                                let skill = app.skills.iter_mut().find(|skill| skill.id == crate::skills::SECURITY_AUDIT_SKILL).unwrap();
                                skill.disabled = true;
                                assert!(!app.security_workspace_enabled());
                                app.section = dunlin_domain::SidebarSection::SourceControl;
                                app.git = dunlin_workspace::git::GitStatus {
                                    branch: "fixture".into(),
                                    changes: (0..2000).map(|index| dunlin_workspace::git::GitChange {
                                        path: format!("src/file-{index:04}.rs"), staged: index % 2 == 0, status: 'M', additions: 1, deletions: 0,
                                    }).collect(), ..Default::default()
                                };
                                app.git_loading = false;
                                cx.notify();
                            })).unwrap();
                            stage = 10;
                        }
                         10 => {
                            let app = owner.read(cx);
                            assert!(!app.ai_commit.manual, "AI remains the default even without verified model authentication");
                            assert_eq!(app.source_control.rows.len(), 2002);
                             assert!(app.source_control.rendered.len() < 60, "only visible rows may be constructed, not all 2000 files");
                             measured("stage-all-action");
                             let row = measured("changed-file-row");
                             let action = measured("changed-file-stage");
                             let list = measured("source-control-change-list");
                             assert!(row.right() <= list.right() + px(1.));
                             assert!(action.right() <= row.right() && row.right() - action.center().x <= px(19.), "file actions must stay at the right edge: row={row:?} action={action:?} list={list:?}");
                             assert!(action.top() >= row.top() && action.bottom() <= row.bottom(), "file controls must fit their row");
                            commit_card = Some(measured("source-control-card"));
                            parent.update(cx, |_, window, cx| {
                                window.dispatch_event(PlatformInput::Scroll(ScrollEvent {
                                    position: measured("source-control-change-list").center(),
                                    delta: ScrollDelta::Pixels(point(px(0.), px(-7.5))), touch_phase: TouchPhase::Moved,
                                    ..Default::default()
                                }), cx);
                            }).unwrap();
                            stage = 11;
                        }
                        11 => {
                            let offset = owner.read(cx).source_control.scroll.0.borrow().base_handle.offset().y;
                            assert!(offset < px(0.) && offset > px(-12.), "trackpad scrolling must retain small pixel deltas, not jump a file row: {offset:?}");
                            assert_eq!(measured("source-control-card").origin, commit_card.unwrap().origin, "the commit controls must not scroll with the files");
                            owner.read(cx).source_control.scroll.scroll_to_item_strict(1500, ScrollStrategy::Top);
                            stage = 12;
                        }
                        12 => {
                            let app = owner.read(cx);
                            assert!(app.source_control.rendered.start >= 1499 && app.source_control.rendered.len() < 60);
                            change_offset = app.source_control.scroll.0.borrow().base_handle.offset().y;
                            parent.update(cx, |_, window, cx| click(window, measured("commit-mode-manual").center(), cx)).unwrap();
                            stage = 13;
                        }
                         13 => {
                            assert!(owner.read(cx).ai_commit.manual, "manual mode must always be available");
                            assert_eq!(owner.read(cx).source_control.scroll.0.borrow().base_handle.offset().y, change_offset, "mode changes must preserve the change-list position");
                             parent.update(cx, |_, window, cx| click(window, measured("commit-mode-ai").center(), cx)).unwrap();
                             owner.update(cx, |app, cx| { for change in &mut app.git.changes { change.staged = true; } cx.notify(); });
                            stage = 14;
                        }
                         14 => {
                             measured("unstage-all-action");
                            assert!(!owner.read(cx).ai_commit.manual);
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| app.open_workspace_tool(Tool::Knowledge, window, cx))).unwrap();
                            stage = 16;
                        }
                        16 => {
                            parent.update(cx, |_, window, cx| {
                                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { position: measured("active-workspace-tab").center(), button: MouseButton::Right, click_count: 1, ..Default::default() }), cx);
                                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { position: measured("active-workspace-tab").center(), button: MouseButton::Right, click_count: 1, ..Default::default() }), cx);
                            }).unwrap();
                            stage = 17;
                        }
                        17 => {
                            assert!(matches!(&owner.read(cx).context_menu.as_ref().unwrap().target, crate::app::ContextTarget::Tab(TabKey::Tool(_, Tool::Knowledge))));
                            parent.update(cx, |_, window, cx| click(window, measured("ctx-tab-lock").center(), cx)).unwrap();
                            stage = 18;
                        }
                        18 => {
                            parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                                let key = TabKey::Tool(app.project().id.clone(), Tool::Knowledge);
                                assert!(app.workspace_tab_locked(&key));
                                app.request_close_workspace_tab(&key, window, cx);
                                assert!(app.workspace_tab_keys().contains(&key), "locked tabs must resist close commands");
                                app.toggle_workspace_tab_lock(&key, cx);
                                app.request_close_workspace_tab(&key, window, cx);
                                assert!(!app.workspace_tab_keys().contains(&key));
                                app.show_chat_tab(window, cx);
                                app.show_shortcut_hints = false;
                            })).unwrap();
                            stage = 19;
                        }
                        19 if !owner.read(cx).chat_controls_busy() => {
                            assert_eq!(owner.read(cx).harness.modes.as_ref().unwrap().current, "build");
                            parent.update(cx, |_, window, cx| crate::controls_smoke::key(window, cx, "shift-tab")).unwrap();
                            stage = 20;
                        }
                        20 if owner.read(cx).harness.modes.as_ref().is_some_and(|modes| modes.current == "plan") => {
                            parent.update(cx, |_, window, cx| {
                                owner.update(cx, |app, cx| { app.show_shortcut_hints = true; app.focus_handle.focus(window, cx); });
                                crate::controls_smoke::key(window, cx, "shift-tab");
                            }).unwrap();
                            stage = 21;
                        }
                        21 if owner.read(cx).harness.modes.as_ref().is_some_and(|modes| modes.current == "build") => {
                            owner.update(cx, |app, cx| app.select_model("mock-b", cx));
                            stage = 22;
                        }
                        22 if !owner.read(cx).chat_controls_busy() && owner.read(cx).harness.models.as_ref().is_some_and(|models| models.current == "mock-b") => {
                            owner.update(cx, |app, cx| {
                                inherited_from = Some(app.session().id.clone());
                                app.auto_connect = false;
                                app.new_session(cx);
                                assert_ne!(Some(&app.session().id), inherited_from.as_ref());
                                assert_eq!(app.session().model, "Mock B", "new chat displays the inherited choice immediately");
                                assert!(app.harness.models.is_none(), "no model catalog should be fabricated before connecting");
                                assert_eq!(app.harness.inherited_model.as_ref().unwrap().choice.id, "mock-b");
                                app.connect_harness(cx);
                            });
                            stage = 23;
                        }
                        23 if !owner.read(cx).chat_controls_busy() && owner.read(cx).harness.models.is_some() => {
                            owner.update(cx, |app, cx| {
                                assert_eq!(app.harness.models.as_ref().unwrap().current, "mock-b", "new ACP session must restore the selected model, not mock-a");
                                assert!(app.harness.inherited_model.is_none(), "the real ACP acknowledgement clears the pending snapshot");
                                let original = inherited_from.as_ref().unwrap();
                                assert_eq!(app.all_sessions.iter().find(|session| &session.id == original).unwrap().model, "Mock B", "creating a new chat must not rewrite the previous conversation");
                                app.new_session(cx);
                                assert_eq!(app.harness.inherited_model.as_ref().unwrap().choice.id, "mock-b");
                                app.new_session(cx);
                                assert_eq!(app.session().model, "Mock B", "successive unconnected tabs keep the choice too");
                                assert_eq!(app.harness.inherited_model.as_ref().unwrap().choice.id, "mock-b");
                                app.connect_harness(cx);
                            });
                            stage = 24;
                        }
                        24 if !owner.read(cx).chat_controls_busy() && owner.read(cx).harness.models.is_some() => {
                            assert_eq!(owner.read(cx).harness.models.as_ref().unwrap().current, "mock-b");
                            assert_eq!(owner.read(cx).harness.config_options.iter().find(|option| option.id == "reasoning-effort").unwrap().selector.current, "medium");
                            parent.update(cx, |_, window, cx| click(window, measured("composer-effort").center(), cx)).unwrap();
                            stage = 25;
                        }
                        25 => {
                            parent.update(cx, |_, window, cx| click(window, measured("chat-option-effort-high").center(), cx)).unwrap();
                            stage = 26;
                        }
                        26 if !owner.read(cx).chat_controls_busy() => {
                            let app = owner.read(cx);
                            let effort = app.harness.config_options.iter().find(|option| option.id == "reasoning-effort").unwrap();
                            assert_eq!(effort.selector.current, "high", "text-only Effort menu must apply the real ACP selection");
                            println!("PASS: workspace tabs and right-click locks, Spec Kit installation/gating, full-width audit with report-only refresh, exact-model/no-fallback dispatch, new chat confirmed-model inheritance, text-only Effort selection with ACP acknowledgement, pixel-scrolled 2000-file list, Shift+Tab harness mode cycling with hints hidden and visible, capitalized native process name");
                            cx.quit(); return true;
                        }
                        _ => {}
                    }
                    false
                });
                if done { break; }
            }
        }).detach();
    });
}
