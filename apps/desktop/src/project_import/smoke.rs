//! Isolated native sheet/controller acceptance; fake gh, no credentials/network.
use super::*;
use crate::app::{SettingsTab, StoredSettings};
use dunlin_ui::{assets, component::TitleBar};

fn repo(path: &Path) {
    assert!(
        std::process::Command::new("git")
            .args(["init", "--template=", "-q"])
            .arg(path)
            .status()
            .unwrap()
            .success()
    );
}
pub(crate) fn run() {
    crate::workspace_ui_smoke::enable_probes();
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("fresh absolute fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-project-import-smoke-")
    );
    std::fs::create_dir(&dir).expect("fixture must not exist");
    let bin = dir.join("bin");
    std::fs::create_dir(&bin).unwrap();
    let gh = bin.join("gh");
    std::fs::write(&gh, r##"#!/usr/bin/env python3
import json, sys, subprocess, time, pathlib
a = sys.argv[1:]
if a == ['--version'] or a[:2] == ['auth', 'status']: sys.exit(0)
if a[:2] == ['repo', 'clone']:
 subprocess.run(['git', 'init', '--template=', '-q', a[3]], check=True)
 sys.exit(0)
if 'search/repositories' in a:
 if 'q=slow' in a:
  (pathlib.Path(__file__).parent / 'search-started').write_text('started')
  time.sleep(20)
 if '--include' in a: print('HTTP/2.0 200 OK\n')
 print(json.dumps({'total_count': 54072, 'incomplete_results': False, 'items': [{'full_name': 'fixture/one'}, {'full_name': 'fixture/two'}]}))
 sys.exit(0)
if 'user' in a:
 print(json.dumps({'id': 42, 'login': 'github-fixture', 'name': 'GitHub Fixture', 'avatar_url': None})); sys.exit(0)
sys.exit(1)
"##).unwrap();
    let glab = bin.join("glab");
    std::fs::write(&glab, r##"#!/usr/bin/env python3
import json, sys, subprocess
a = sys.argv[1:]
if a == ['--version'] or a[:2] == ['auth', 'status']: sys.exit(0)
if a[:2] == ['repo', 'clone']:
 subprocess.run(['git', 'init', '--template=', '-q', a[3]], check=True); sys.exit(0)
if 'projects' in a:
 print(json.dumps([{'path_with_namespace':'gitlab-fixture/sub/one'}, {'path_with_namespace':'gitlab-fixture/sub/two'}])); sys.exit(0)
if 'user' in a:
 print(json.dumps({'id': 84, 'username': 'gitlab-fixture', 'name': 'GitLab Fixture', 'avatar_url': None})); sys.exit(0)
sys.exit(1)
"##).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&glab, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let paths: Vec<_> = std::iter::once(bin)
        .chain(std::env::split_paths(&path))
        .collect();
    // SAFETY: this debug entry point runs before application/worker threads.
    unsafe {
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap());
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    let collection = dir.join("collection");
    repo(&collection.join("one"));
    repo(&collection.join("two"));
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                vault_enabled: false,
                restore_session: false,
                notifications_enabled: false,
                onboarding_step: 2,
                ..Default::default()
            },
        )
        .unwrap();
    drop(store);
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.project-import-smoke", "Dunlin Project Import Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let (parent, owner) = open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(60.), px(60.)), size(px(1100.), px(720.))))),
            ..TitleBar::window_options()
        }, cx, |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        owner.update(cx, |app, cx| { app.open_project_import(cx); app.project_import.mode = Mode::Local; });
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0;
            let mut sheet_size = None;
            loop {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let done = cx.update(|cx| {
                    parent.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                        child.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    }
                    assert!(started.elapsed() < Duration::from_secs(90), "project import smoke stalled at {stage}");
                     if stage == 17 || stage == 18 {
                        let bounds = crate::workspace_ui_smoke::measured(if stage == 17 { "title-user-menu" } else { "profile-density" });
                        let position = if stage == 18 { point(bounds.right() - px(12.), bounds.center().y) } else { bounds.center() };
                        parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, position, cx)).unwrap();
                     }
                     if stage == 20 {
                         let bounds = crate::workspace_ui_smoke::measured("add-project");
                         parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, bounds.center(), cx)).unwrap();
                         stage = 21;
                         return false;
                     }
                     if stage == 21 {
                         assert!(!owner.read(cx).project_import.open, "Add project must show source choices before opening a form");
                         parent.update(cx, |_, window, cx| {
                             crate::controls_smoke::key(window, cx, "down");
                             crate::controls_smoke::key(window, cx, "down");
                             crate::controls_smoke::key(window, cx, "enter");
                         }).unwrap();
                         stage = 22;
                         return false;
                     }
                     if stage == 70 || (stage == 76 && dir.join("bin/search-started").exists()) {
                         let bounds = crate::workspace_ui_smoke::measured(if stage == 76 { "import-cancel" } else { "import-search-action" });
                        if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                            child.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, bounds.center(), cx)).unwrap();
                        } else {
                            parent.update(cx, |_, window, cx| crate::workspace_ui_smoke::click(window, bounds.center(), cx)).unwrap();
                        }
                         stage = if stage == 76 { 77 } else { 8 };
                        return false;
                    }
                    parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                        match stage {
                              0 if app.project_import.inputs.is_some() => {
                                 sheet_size = Some(crate::workspace_ui_smoke::measured("project-import-sheet").size);
                                 assert!(app.has_modal_overlay());
                                  assert!(app.project_import.local.is_none());
                                 app.select_import_folder(collection.join("one"), Purpose::Local, window, cx);
                                stage = 1;
                            },
                             1 if !app.project_import.busy => {
                                 stage = 2;
                            },
                            2 if !app.project_import.busy => {
                                let Some(Preview::Local { root, discovery }) = &app.project_import.preview else { panic!("missing local preview") };
                                assert_eq!(discovery.projects.len(), 1); assert!(!can_watch(root, discovery));
                                 app.select_import_folder(collection.clone(), Purpose::Local, window, cx); stage = 3;
                            },
                             3 if !app.project_import.busy => {
                                 stage = 4;
                            },
                            4 if !app.project_import.busy => {
                                let Some(Preview::Local { root, discovery }) = &app.project_import.preview else { panic!("missing collection preview") };
                                assert_eq!(discovery.projects.len(), 2); assert!(can_watch(root, discovery));
                                app.project_import.watch = true;
                                app.confirm_project_import(cx); stage = 5;
                            },
                            5 if !app.local_import_running && app.projects.len() == 2 => {
                                assert!(app.projects.iter().all(|p| p.trusted));
                                assert_eq!(app.watched_project_folders.len(), 1);
                                assert_eq!(app.store.as_ref().unwrap().load_setting::<StoredSettings>("app").unwrap().unwrap().watched_project_folders.len(), 1);
                                assert!(app.assign_project_group(0, None, cx));
                                repo(&collection.join("three"));
                                app.project_import.last_scan = None;
                                app.poll_project_folders(cx); stage = 6;
                            },
                            6 if !app.local_import_running && app.projects.len() == 3 => {
                                assert!(app.projects[0].group_id.is_none(), "tracking changed existing membership");
                                assert!(!app.projects[2].trusted);
                                assert_eq!(app.selected_project, 0, "tracking switched the active project");
                                app.watched_project_folders.clear();
                                 app.open_project_import(cx); stage = 7;
                            },
                             7 if app.project_import.inputs.is_some() => {
                                 app.project_import.mode = Mode::GitHub;
                                app.project_import.request = app.project_import.request.wrapping_add(1);
                                let inputs = app.project_import.inputs.as_ref().unwrap();
                                assert_eq!(inputs.destination.read(cx).value().as_ref(), dir.join("repositories").to_str().unwrap());
                                 inputs.query.update(cx, |s, cx| s.set_value("slow", window, cx));
                                 app.preview_project_import(cx);
                                 stage = 76;
                             },
                             77 => {
                                 assert!(!app.project_import.open && !app.project_import.busy, "Cancel must dismiss an in-flight CLI search immediately");
                                 app.open_project_import(cx); stage = 78;
                             },
                             78 if app.project_import.inputs.is_some() => {
                                 assert!(app.project_import.preview.is_none(), "cancelled search must not publish results");
                                 app.project_import.mode = Mode::GitHub;
                                 app.project_import.inputs.as_ref().unwrap().query.update(cx, |s, cx| s.set_value("org:fixture archived:false", window, cx));
                                 stage = 70;
                             },
                             8 if !app.project_import.busy => {
                                 assert_eq!(Some(crate::workspace_ui_smoke::measured("project-import-sheet").size), sheet_size, "source tabs must not resize the sheet");
                                 let Some(Preview::Hosted { repositories, .. }) = &app.project_import.preview else { panic!("missing GitHub preview: {:?}", app.project_import.error) };
                                assert_eq!(repositories.len(), 2);
                                // Editing a query invalidates the displayed import list.
                                app.project_import.inputs.as_ref().unwrap().query.update(cx, |s, cx| s.replace_all("org:fixture archived:true", window, cx));
                                stage = 80;
                            },
                             80 => {
                                 assert!(app.project_import.preview.is_none());
                                 app.preview_project_import(cx);
                                 app.project_import.inputs.as_ref().unwrap().query.update(cx, |s, cx| s.replace_all("org:fixture archived:false", window, cx));
                                 stage = 81;
                             },
                             81 if !app.project_import.busy => {
                                 assert!(app.project_import.preview.is_none() && app.project_import.error.is_none(), "a stale query must not publish results or an error");
                                 app.preview_project_import(cx); stage = 9;
                            },
                             9 if !app.project_import.busy => {
                                 let destination = dir.join("custom downloads");
                                 std::fs::create_dir(&destination).unwrap();
                                  let destination = destination.join("selected child");
                                  std::fs::create_dir(&destination).unwrap();
                                  app.select_import_folder(destination, Purpose::Destination, window, cx); stage = 100;
                             },
                              100 if !app.project_import.busy => {
                                  assert_eq!(app.project_import.inputs.as_ref().unwrap().destination.read(cx).value().as_ref(), dir.join("custom downloads/selected child").to_str().unwrap());
                                  assert!(app.project_import.preview.is_some(), "destination selection discarded preview");
                                 app.project_import.group = Some(app.project_groups[0].id.clone());
                                 app.confirm_project_import(cx); stage = 11;
                            },
                            11 if !app.project_import.busy => {
                                assert_eq!(app.projects.len(), 5);
                                assert!(app.projects[3..].iter().all(|p| !p.trusted && p.path.contains("custom downloads")));
                                 assert!(dir.join("custom downloads/selected child/fixture/one/.git").is_dir());
                                 assert!(app.projects[3..].iter().all(|p| p.group_id.as_deref() == Some(&app.project_groups[0].id)));
                                 assert!(app.store.as_ref().unwrap().projects().unwrap().iter().filter(|p| p.path.contains("custom downloads")).all(|p| p.group_id == app.projects[3].group_id));
                                assert_eq!(app.store.as_ref().unwrap().projects().unwrap().len(), 5);
                                 app.settings_open = true; app.settings_tab = SettingsTab::Sidebar; stage = 12;
                            },
                             12 if !app.project_import.checking_cli => {
                                 assert!(app.project_import.cli.as_ref().is_some_and(|s| s.installed && s.authenticated));
                                 assert!(app.project_import.gitlab_cli.as_ref().is_some_and(|s| s.installed && s.authenticated));
                                  app.open_project_import(cx); stage = 13;
                             },
                              13 if app.project_import.inputs.is_some() => {
                                  app.project_import.mode = Mode::GitLab;
                                 app.project_import.inputs.as_ref().unwrap().query.update(cx, |s, cx| s.set_value("fixture", window, cx));
                                 app.preview_project_import(cx); stage = 14;
                             },
                             14 if !app.project_import.busy => {
                                 let Some(Preview::Hosted { mode, repositories, .. }) = &app.project_import.preview else { panic!("missing GitLab preview: {:?}", app.project_import.error) };
                                 assert!(*mode == Mode::GitLab && repositories.len() == 2);
                                 app.confirm_project_import(cx); stage = 15;
                             },
                             15 if !app.project_import.busy => {
                                 assert_eq!(app.projects.len(), 7);
                                 assert!(dir.join("repositories/gitlab-fixture/sub/one/.git").exists());
                                 assert!(app.projects[5..].iter().all(|p| !p.trusted));
                                 app.refresh_user_profile(cx); stage = 16;
                             },
                              16 if app.user_profile.checked.is_some() => {
                                 let avatar = crate::workspace_ui_smoke::measured("title-user-menu");
                                 let host = crate::workspace_ui_smoke::measured("host-menu-button");
                                 assert_eq!(host.size, avatar.size, "host hover target must match the avatar");
                                 assert!(crate::workspace_ui_smoke::measured("titlebar-content").right() - avatar.right() >= px(12.), "avatar needs a right inset");
                                 assert_eq!(app.user_profile.profiles.len(), 2);
                                 assert_eq!(app.user_profile.profiles[0].0.name, "GitHub Fixture");
                                 assert_eq!(app.user_profile.profiles[1].0.name, "GitLab Fixture");
                                 let language = app.project_list.show_language; let topics = app.project_list.show_topics;
                                 app.set_ui_density(crate::app::UiDensity::Compact, cx);
                                 assert_eq!((app.project_list.show_language, app.project_list.show_topics), (language, topics));
                                 app.set_ui_density(crate::app::UiDensity::Comfortable, cx);
                                 stage = 17;
                             },
                             17 => { stage = 18; },
                             18 => { stage = 19; },
                             19 => {
                                 assert!(app.density == crate::app::UiDensity::Compact, "avatar menu switch must change interface density");
                                  app.set_ui_density(crate::app::UiDensity::Comfortable, cx);
                                  app.section = dunlin_domain::SidebarSection::Projects;
                                  app.settings_open = false;
                                  stage = 20;
                             },
                             22 if app.project_import.open && app.project_import.inputs.is_some() => {
                                  assert!(app.project_import.mode == Mode::GitHub, "remote import should start with provider input, not a local folder sheet");
                                   println!("PASS: project import source menu, remote form, local folder handling, live search Cancel/retry, GitHub/GitLab safe clones and profiles");
                                 cx.quit(); return true;
                            },
                            _ => {},
                        }
                        cx.notify(); false
                    })).unwrap()
                });
                if done { break; }
            }
        }).detach();
    });
}
