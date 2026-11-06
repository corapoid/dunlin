//! Debug-only fresh-install/TCC smoke. No harness, provider request, screenshot
//! or simulated system prompt: exercise onboarding state and the permission
//! center in an isolated data directory, then quit that process only.
use crate::app::{DunlinApp, SettingsTab};
use dunlin_domain::SidebarSection;
use dunlin_ui::{assets, *};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--onboarding-smoke requires a fresh absolute fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-onboarding-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture directory must not already exist");
    // SAFETY: main invokes this before starting any application/worker threads.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application()
        .with_assets(assets::AllAssets)
        .run(move |cx| {
            cx.set_app_identity(
                "dev.dunlin.ide.onboarding-smoke",
                "Dunlin Onboarding Smoke",
            );
            dunlin_ui::init(cx);
            crate::logging::init();
            let bounds = Bounds::new(point(px(80.), px(80.)), size(px(1200.), px(800.)));
            let (parent, owner) = open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..dunlin_ui::window_options()
                },
                cx,
                |window, cx| cx.new(|cx| DunlinApp::new(window, cx)),
            )
            .unwrap();
            cx.spawn(async move |cx| {
                let started = Instant::now();
                let mut stage = 0;
                loop {
                    cx.background_executor()
                        .timer(Duration::from_millis(120))
                        .await;
                    let done = cx.update(|cx| {
                        parent
                            .update(cx, |_, window, cx| {
                                window.refresh();
                                window.draw(cx).clear(cx);
                            })
                            .unwrap();
                        assert!(
                            started.elapsed() < Duration::from_secs(30),
                            "onboarding smoke stalled at stage {stage}"
                        );
                        match stage {
                            0 => {
                                owner.update(cx, |app, cx| {
                                    assert!(app.projects.is_empty());
                                    assert_eq!(app.onboarding_step, 0);
                                    assert!(app.store.is_some(), "fresh store was not created");
                                    app.section = SidebarSection::Projects;
                                    app.onboarding_show_all_harnesses = true;
                                    cx.notify();
                                });
                                stage = 1;
                            }
                            1 => {
                                owner.update(cx, |app, cx| {
                                    assert!(app.onboarding_show_all_harnesses);
                                    assert_eq!(app.harness_discovery.len(), crate::harness_discovery::SPECS.len());
                                    app.harness_discovery.clear();
                                    cx.notify();
                                });
                                stage = 2;
                            }
                            2 => {
                                owner.update(cx, |app, cx| {
                                    assert!(app.harness_discovery.is_empty());
                                    app.onboarding_step = 1;
                                    app.set_app_active(false, cx);
                                    app.set_app_active(true, cx);
                                    assert!(app.permissions_refreshing, "returning to the app must recheck cached permissions");
                                    app.refresh_permissions(cx);
                                    assert!(app.permissions_refreshing);
                                    cx.notify();
                                });
                                stage = 3;
                            }
                            3 => {
                                if owner.read(cx).permissions_refreshing {
                                    return false;
                                }
                                let report = owner.read_with(cx, |app, _| {
                                    assert_eq!(app.onboarding_step, 1);
                                    assert_eq!(app.harness_discovery.len(), crate::harness_discovery::SPECS.len());
                                    let statuses = crate::platform::permission_statuses();
                                    assert_eq!(statuses.len(), crate::platform::PermissionKind::ALL.len());
                                    assert!(statuses.iter().all(|status| !status.detail.is_empty()));
                                    crate::qa::capture(app)
                                });
                                assert!(report.contains("# Dunlin QA report"));
                                assert!(report.contains("Onboarding: visible"));
                                assert!(report.contains("Manual R11 status: NOT evaluated"));
                                owner.update(cx, |app, cx| {
                                    app.settings_open = true;
                                    app.settings_tab = SettingsTab::Permissions;
                                    cx.notify();
                                });
                                stage = 4;
                            }
                            4 => {
                                assert!(owner.read(cx).settings_open);
                                assert_eq!(owner.read(cx).settings_tab, SettingsTab::Permissions);
                                parent
                                    .update(cx, |_, window, cx| {
                                        window.refresh();
                                        window.draw(cx).clear(cx);
                                    })
                                    .unwrap();
                                let repo = dir.join("collection/repo");
                                std::fs::create_dir_all(&repo).unwrap();
                                assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&repo).status().unwrap().success());
                                std::fs::write(repo.join("README.md"), "nested repository\n").unwrap();
                                owner.update(cx, |app, cx| {
                                    app.settings_open = false;
                                    app.auto_connect = false;
                                    app.import_local_folders(vec![dir.join("collection")], cx);
                                });
                                stage = 5;
                            }
                            5 => {
                                let app = owner.read(cx);
                                if app.local_import_running || app.explorer.tree_loading {
                                    return false;
                                }
                                let expected = std::fs::canonicalize(dir.join("collection/repo")).unwrap().to_string_lossy().into_owned();
                                assert_eq!(app.projects.len(), 1);
                                assert_eq!(app.selected_project, 0);
                                assert_eq!(app.project().path, expected);
                                assert_eq!(app.file_tree.root, expected, "first import skipped workspace activation");
                                assert!(app.file_tree.error.is_none());
                                assert!(app.file_tree.entries.iter().any(|entry| entry.name == "README.md"));
                                assert!(!app.sessions.is_empty());
                                assert!(app.sessions.iter().all(|session| session.project_id == app.project().id));
                                assert_eq!(app.project_groups.len(), 1);
                                assert_eq!(app.project_groups[0].name, "collection");
                                assert_eq!(app.project().group_id.as_ref(), Some(&app.project_groups[0].id));
                                let second = dir.join("collection/repo-two");
                                std::fs::create_dir(&second).unwrap();
                                assert!(std::process::Command::new("git").args(["init", "-q"]).arg(&second).status().unwrap().success());
                                owner.update(cx, |app, cx| {
                                    let navigation = app.navigation_epoch();
                                    app.select_project(0, cx);
                                    assert_eq!(app.navigation_epoch(), navigation, "reselecting a loaded project must remain a no-op");
                                    assert!(app.assign_project_group(0, None, cx));
                                    app.import_local_folders(vec![dir.join("collection")], cx);
                                });
                                stage = 6;
                            }
                            6 => {
                                let app = owner.read(cx);
                                if app.local_import_running || app.explorer.tree_loading {
                                    return false;
                                }
                                assert_eq!(app.projects.len(), 2);
                                assert_eq!(app.project_groups.len(), 1, "reimport must reuse the folder group");
                                let group = app.project_groups[0].id.clone();
                                assert!(app.projects.iter().all(|project| project.group_id.as_ref() == Some(&group)));
                                assert_eq!(app.store.as_ref().unwrap().project_groups().unwrap().len(), 1);
                                assert!(app.store.as_ref().unwrap().projects().unwrap().iter().all(|project| project.group_id.as_ref() == Some(&group)));
                                owner.update(cx, |app, cx| {
                                    app.start_group_session(&group, cx);
                                    assert_eq!(app.group_contexts[&app.session().id].roots.len(), 2);
                                    assert_eq!(app.session_additional_directories().len(), 1);
                                    let prompt = app.client_prompt_text("Review the whole project");
                                    assert!(prompt.contains("one logical multi-repository project"));
                                    assert!(prompt.contains("collection"));
                                    assert!(prompt.contains("repo-two"));
                                    for project in &app.projects { assert!(prompt.contains(&std::fs::canonicalize(&project.path).unwrap().to_string_lossy().to_string())); }
                                    assert!(prompt.ends_with("Review the whole project"));
                                    assert!(app.harness.queued_prompts().is_empty(), "starting the agent must not send an unsolicited request");
                                    let mut manual = app.project_groups[0].clone();
                                    manual.id = "manual-group".into();
                                    manual.name = "Keep my assignment".into();
                                    app.store.as_ref().unwrap().upsert_project_group(&manual).unwrap();
                                    app.project_groups.push(manual);
                                    assert!(app.assign_project_group(1, Some("manual-group".into()), cx));
                                    app.import_local_folders(vec![dir.join("collection")], cx);
                                });
                                stage = 7;
                            }
                            7 => {
                                let app = owner.read(cx);
                                if app.local_import_running || app.explorer.tree_loading { return false; }
                                assert_eq!(app.projects.len(), 2);
                                assert_eq!(app.project_groups.len(), 2);
                                assert_eq!(app.projects[1].group_id.as_deref(), Some("manual-group"));
                                assert_eq!(app.group_contexts[&app.session().id].roots.len(), 2, "existing group sessions retain their frozen scope");
                                let empty = dir.join("empty");
                                std::fs::create_dir(&empty).unwrap();
                                owner.update(cx, |app, cx| app.import_local_folders(vec![empty], cx));
                                stage = 8;
                            }
                            8 => {
                                let app = owner.read(cx);
                                if app.local_import_running || app.explorer.tree_loading { return false; }
                                assert_eq!(app.project().name, "empty");
                                assert!(app.project().group_id.is_none(), "ordinary folders must not create collection groups");
                                assert_eq!(app.file_tree.root, app.project().path);
                                assert!(app.file_tree.error.is_none());
                                assert!(app.file_tree.entries.is_empty());
                                println!("PASS: onboarding/permissions, parent-folder groups, all nested repos, idempotent persisted reimport, preserved manual memberships, frozen group-agent prompt and scope, empty folder; Quit");
                                cx.dispatch_action(&crate::app::Quit);
                                return true;
                            }
                            _ => {}
                        }
                        false
                    });
                    if done {
                        break;
                    }
                }
            })
            .detach();
        });
}
