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
import json, sys, subprocess
a = sys.argv[1:]
if a == ['--version'] or a[:2] == ['auth', 'status']: sys.exit(0)
if a[:2] == ['repo', 'clone']:
 subprocess.run(['git', 'init', '--template=', '-q', a[3]], check=True)
 sys.exit(0)
if 'search/repositories' in a:
 print(json.dumps({'total_count': 2, 'incomplete_results': False, 'items': [{'full_name': 'fixture/one'}, {'full_name': 'fixture/two'}]}))
 sys.exit(0)
sys.exit(1)
"##).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
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
        owner.update(cx, |app, cx| app.open_folder(cx));
        cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0;
            loop {
                cx.background_executor().timer(Duration::from_millis(120)).await;
                let done = cx.update(|cx| {
                    parent.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    if let Some(child) = owner.read(cx).overlay_window.debug_window() {
                        child.update(cx, |_, w, cx| { w.refresh(); w.draw(cx).clear(cx); }).unwrap();
                    }
                    assert!(started.elapsed() < Duration::from_secs(90), "project import smoke stalled at {stage}");
                    parent.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                        match stage {
                             0 if app.project_import.inputs.is_some() => {
                                 assert!(app.has_modal_overlay());
                                 assert_eq!(app.project_import.browser.as_ref().unwrap().path, home());
                                app.browse_import_folder(collection.join("one"), Purpose::Local, window, cx);
                                stage = 1;
                            },
                            1 if app.project_import.browser.as_ref().is_some_and(|b| !b.loading) => {
                                app.preview_project_import(cx); stage = 2;
                            },
                            2 if !app.project_import.busy => {
                                let Some(Preview::Local { root, discovery }) = &app.project_import.preview else { panic!("missing local preview") };
                                assert_eq!(discovery.projects.len(), 1); assert!(!can_watch(root, discovery));
                                app.browse_import_folder(collection.clone(), Purpose::Local, window, cx); stage = 3;
                            },
                            3 if app.project_import.browser.as_ref().is_some_and(|b| !b.loading) => {
                                app.preview_project_import(cx); stage = 4;
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
                                app.open_folder(cx); stage = 7;
                            },
                            7 if app.project_import.inputs.is_some() => {
                                app.project_import.mode = Mode::GitHub;
                                app.project_import.browser = None;
                                app.project_import.request = app.project_import.request.wrapping_add(1);
                                let inputs = app.project_import.inputs.as_ref().unwrap();
                                assert_eq!(inputs.destination.read(cx).value().as_ref(), dir.join("repositories").to_str().unwrap());
                                inputs.query.update(cx, |s, cx| s.set_value("org:fixture archived:false", window, cx));
                                app.preview_project_import(cx); stage = 8;
                            },
                            8 if !app.project_import.busy => {
                                let Some(Preview::GitHub { repositories, .. }) = &app.project_import.preview else { panic!("missing GitHub preview: {:?}", app.project_import.error) };
                                assert_eq!(repositories.len(), 2);
                                // Editing a query invalidates the displayed import list.
                                app.project_import.inputs.as_ref().unwrap().query.update(cx, |s, cx| s.replace_all("org:fixture archived:true", window, cx));
                                stage = 80;
                            },
                            80 => {
                                assert!(app.project_import.preview.is_none());
                                app.preview_project_import(cx); stage = 9;
                            },
                            9 if !app.project_import.busy => {
                                let destination = dir.join("custom downloads");
                                std::fs::create_dir(&destination).unwrap();
                                app.browse_import_folder(destination, Purpose::Destination, window, cx); stage = 10;
                            },
                            10 if app.project_import.browser.as_ref().is_some_and(|b| !b.loading) => {
                                assert!(app.project_import.preview.is_some(), "destination browser discarded preview");
                                let path = app.project_import.browser.take().unwrap().path;
                                app.project_import.inputs.as_ref().unwrap().destination.update(cx, |s, cx| s.set_value(path.to_string_lossy().into_owned(), window, cx));
                                app.confirm_project_import(cx); stage = 11;
                            },
                            11 if !app.project_import.busy => {
                                assert_eq!(app.projects.len(), 5);
                                assert!(app.projects[3..].iter().all(|p| !p.trusted && p.path.contains("custom downloads")));
                                assert!(dir.join("custom downloads/fixture/one/.git").is_dir());
                                assert_eq!(app.store.as_ref().unwrap().projects().unwrap().len(), 5);
                                app.settings_open = true; app.settings_tab = SettingsTab::General; stage = 12;
                            },
                            12 if !app.project_import.checking_cli => {
                                assert!(app.project_import.cli.as_ref().is_some_and(|s| s.installed && s.authenticated));
                                println!("project import smoke passed: local/single/collection preview, saved tracking, untrusted new roots, GitHub preview invalidation, destination explorer, fake gh clones and settings");
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
