//! Isolated native-window controller smoke. Confirmation acceptance is simulated
//! only here; no user repository, harness, or installed application is touched.
use crate::app::{DunlinApp, StoredSettings};
use dunlin_ui::{
    component::{TitleBar, input::Undo},
    *,
};
use dunlin_workspace::git::{operations::Action, overview};
use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
pub(crate) fn run() {
    let light_narrow = std::env::args().any(|arg| arg == "--light-narrow");
    let github_only = std::env::args().nth(1).as_deref() == Some("--github-smoke");
    let workbench_only = std::env::args().nth(1).as_deref() == Some("--workbench-smoke");
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--git-smoke needs an isolated directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-git-smoke-")
    );
    fs::create_dir(&dir).unwrap();
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "--template=", "-qb", "main"]);
    git(&["remote", "add", "origin", "git@github.com:fixture/repo.git"]);
    fs::create_dir(root.join(".git/hooks")).unwrap();
    git(&[
        "config",
        "core.hooksPath",
        root.join(".git/hooks").to_str().unwrap(),
    ]);
    git(&["config", "user.name", "Smoke Test"]);
    git(&["config", "core.fsmonitor", "false"]);
    git(&["config", "core.attributesFile", "/dev/null"]);
    git(&["config", "user.email", "smoke@example.invalid"]);
    git(&["config", "commit.gpgsign", "false"]);
    let original = "fn original() {}\n// unchanged\n";
    let disk = "fn changed_on_disk() {}\n// unchanged\n";
    let path = root.join("example.rs");
    fs::write(&path, original).unwrap();
    git(&["add", "example.rs"]);
    git(&["commit", "-qm", "fixture"]);
    if workbench_only {
        fs::create_dir(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='impact_fixture'\nversion='0.1.0'\n",
        )
        .unwrap();
        fs::write(root.join("src/lib.rs"),"fn target()->u32{1} fn wrapper()->u32{target()} #[test] fn verifies(){assert_eq!(wrapper(),1);}\n").unwrap();
        git(&["add", "Cargo.toml", "src/lib.rs"]);
        git(&["commit", "-qm", "Rust impact fixture"]);
    } else {
        fs::write(&path, disk).unwrap();
    }
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                restore_session: false,
                default_harness: "Custom ACP".into(),
                notifications_enabled: false,
                dark: !light_narrow,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "git-fixture".into(),
            name: "Git smoke".into(),
            path: root.to_string_lossy().into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            remote: None,
            group_id: None,
            settings: Default::default(),
        })
        .unwrap();
    drop(store);
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
        if workbench_only {
            let mock = std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .join("mock_acp_agent");
            assert!(
                mock.is_file(),
                "Build mock_acp_agent before the workbench smoke"
            );
            std::env::set_var("DUNLIN_HARNESS_COMMAND", mock);
            for key in [
                "DUNLIN_MOCK_RICH_CHAT",
                "DUNLIN_MOCK_STREAM_CHUNKS",
                "DUNLIN_MOCK_DELAY_MS",
                "DUNLIN_MOCK_REJECT_MODEL",
                "DUNLIN_MOCK_NO_LOAD",
                "DUNLIN_MOCK_CONFIG_MODES",
            ] {
                std::env::remove_var(key);
            }
        }
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.git-smoke","Dunlin Git Smoke");dunlin_ui::init(cx);crate::logging::init();
        let (handle,owner)=open_window(WindowOptions{window_bounds:Some(WindowBounds::Windowed(Bounds::new(point(px(90.),px(90.)),size(px(if light_narrow {760.} else {1200.}),px(800.))))),..TitleBar::window_options()},cx,|window,cx|cx.new(|cx|DunlinApp::new(window,cx))).unwrap();
        handle.update(cx,|_,w,cx|owner.update(cx,|app,cx|app.open_workbench_file(path.to_str().unwrap(),None,w,cx))).unwrap();
        cx.spawn(async move |cx| {
            let start=Instant::now();let mut stage=0;
            loop {cx.background_executor().timer(Duration::from_millis(250)).await;
                let done=cx.update(|cx| {
                    assert!(start.elapsed()<Duration::from_secs(75),"Git smoke stalled at stage {stage}");
                    handle.update(cx,|_,w,cx|{w.refresh();w.draw(cx).clear(cx);}).unwrap();
                    handle.update(cx,|_,w,cx|owner.update(cx,|app,cx| {
                        if github_only {return super::github::smoke_step(app,&mut stage,w,cx);}
                        if workbench_only{return super::workbench_smoke::step(app,&mut stage,w,cx);}
                        match stage {
                            0 if app.active_file_tab().is_some_and(|t|!t.loading)=>{app.toggle_inline_blame(cx);app.editor.update(cx,|s,cx|s.set_cursor_position(dunlin_ui::component::input::Position::new(1,0),w,cx));stage=1;},
                            1 if crate::inline_blame::smoke_attribution(app,cx).is_some()=>{
                                assert_eq!(crate::inline_blame::smoke_attribution(app,cx).unwrap(),"Smoke Test");
                                let offset=disk.find("//").unwrap();app.editor.update(cx,|s,cx|{s.set_selected_range(offset..disk.len()-1,cx);s.replace("// unsaved",w,cx);});stage=2;
                            },
                            2=>{assert!(app.editor_dirty);assert!(crate::inline_blame::smoke_attribution(app,cx).is_none(),"edited line must not borrow an author");app.open_git_history(w,cx);app.prepare_repository_action(Action::StashPush{message:"blocked".into(),include_untracked:true},cx);assert!(!app.git_history.running);assert!(overview::load(root.to_str().unwrap()).unwrap().stashes.is_empty());
                                 let id=app.workbench.tabs[0].id.clone();app.activate_file_tab(&id,w,cx);stage=30;
                            },
                             30=>{assert!(app.editor.read(cx).focus_handle(cx).is_focused(w));assert!(w.is_action_available(&Undo,cx));w.dispatch_action(Box::new(Undo),cx);stage=3;},
                             3 if !app.editor_dirty && !app.git_loading=>{assert_eq!(app.editor.read(cx).text().to_string(),disk);app.open_git_history(w,cx);app.open_git_overview(cx);stage=4;},
                            4 if !app.git_loading=>{app.prepare_repository_action(Action::StashPush{message:"native fixture".into(),include_untracked:true},cx);assert!(app.git_history.locks("another-project"),"aliases and linked worktrees must remain protected");stage=5;},
                             5 if !app.git_history.running && !app.git_loading=>{let view=app.git_history.view.as_mut().unwrap();assert!(view.mutations.pending.is_some(),"{:?}",view.mutations.error);assert_eq!(fs::read_to_string(&path).unwrap(),disk);view.mutations.pending=None;app.prepare_repository_action(Action::StashPush{message:"native fixture".into(),include_untracked:true},cx);assert!(app.git_history.running,"{:?}",app.notice);stage=6;},
                             6 if !app.git_history.running && !app.git_loading=>{let view=app.git_history.view.as_ref().unwrap();let prepared=view.mutations.pending.clone().expect("preflight must produce a plan");app.execute_repository_action(view.id,view.project.clone(),prepared,cx);assert!(app.git_history.running,"{:?}",app.notice);app.close_git_history(cx);assert!(app.git_history.locks("another-project"));stage=7;},
                             7 if !app.git_history.running=>{assert_eq!(fs::read_to_string(&path).unwrap(),original);assert!(!app.git_history.locks("another-project"));let tab=&app.workbench.tabs[0];assert_eq!(tab.editor.read(cx).text().to_string(),disk,"Git must not replace the canonical buffer");assert!(tab.conflict.is_some());assert!(tab.git_changed);let id=tab.id.clone();app.activate_file_tab(&id,w,cx);app.cancel_editor_conflict(cx);app.editor_stale=true;app.check_open_file(cx);stage=70;},
                             70=>{assert_eq!(app.editor.read(cx).text().to_string(),disk,"watchers must not reload a preserved buffer after conflict dismissal");assert!(app.active_file_tab().unwrap().git_changed);assert!(app.retained_buffer_banner(cx).is_some());let offset=disk.find("//").unwrap();app.editor.update(cx,|s,cx|{s.set_selected_range(offset..disk.len()-1,cx);s.replace("// unsaved after Git",w,cx);});stage=72;},
                             72=>{assert!(app.editor_dirty);let missing=root.join("temporarily-unavailable.rs");fs::rename(&path,&missing).unwrap();app.reload_editor_conflict(w,cx);assert!(app.editor_dirty,"failed reload must not clear unsaved state");assert!(app.active_file_tab().unwrap().git_changed,"failed reload must retain review protection");assert!(app.editor.read(cx).text().to_string().contains("unsaved after Git"));fs::rename(&missing,&path).unwrap();app.reload_editor_conflict(w,cx);stage=71;},
                             71=>{assert_eq!(app.editor.read(cx).text().to_string(),original,"only an explicit reload replaces the preserved buffer");assert!(!app.active_file_tab().unwrap().git_changed);app.close_active_tab();assert!(app.workbench.tabs.is_empty());app.open_git_history(w,cx);app.open_git_overview(cx);stage=8;},
                            8 if !app.git_loading=>{let oid=overview::load(root.to_str().unwrap()).unwrap().stashes[0].oid.clone();app.prepare_repository_action(Action::StashPop{oid},cx);stage=9;},
                             9 if !app.git_history.running && !app.git_loading=>{let view=app.git_history.view.as_ref().unwrap();let prepared=view.mutations.pending.clone().expect("pop preflight");app.execute_repository_action(view.id,view.project.clone(),prepared,cx);assert!(app.git_history.running,"{:?}",app.notice);stage=10;},
                             10 if !app.git_history.running=>{assert_eq!(fs::read_to_string(&path).unwrap(),disk,"pop result {:?}; fixture {}",app.notice,root.display());assert!(overview::load(root.to_str().unwrap()).unwrap().stashes.is_empty());println!("PASS: native Git inventory/forms, unchanged-line blame, unsaved-line suppression, dirty-buffer guard, preflight cancellation, stash push/pop, global mutation locks surviving tab close, canonical-buffer preservation/conflict notice. Confirmation acceptance simulated; no manual dialog QA.");cx.quit();return true;},
                            _=>{},
                        }false
                    })).unwrap()
                });if done {break;}
            }
        }).detach();
    });
}
