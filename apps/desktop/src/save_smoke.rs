//! Isolated native regression fixture with a deliberately stalled loopback relay.
use crate::{
    app::{DunlinApp, StoredSettings},
    remote,
};
use dunlin_ui::*;
use std::{
    cell::Cell,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Gate {
    armed: AtomicBool,
    entered: AtomicBool,
    released: AtomicBool,
}
impl Gate {
    fn arm(&self) {
        self.entered.store(false, Ordering::SeqCst);
        self.released.store(false, Ordering::SeqCst);
        self.armed.store(true, Ordering::SeqCst);
    }
    fn wait(&self) {
        if !self.armed.swap(false, Ordering::SeqCst) {
            return;
        }
        self.entered.store(true, Ordering::SeqCst);
        let start = Instant::now();
        while !self.released.load(Ordering::SeqCst) {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "foreground did not release the stalled relay request"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    fn entered(&self) -> bool {
        self.entered.load(Ordering::SeqCst)
    }
    fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }
}

fn serve(mut stream: TcpStream, root: PathBuf, put: Arc<Gate>, stat: Arc<Gate>) {
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut bytes = Vec::new();
    let (header_end, length) = loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0 && bytes.len() < 4 * 1024 * 1024);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..end]);
            let length = header
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|length| length.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            break (end + 4, length);
        }
    };
    while bytes.len() < header_end + length {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8_lossy(&bytes[..header_end]);
    let root = root.to_str().unwrap();
    let result = if request.starts_with("PUT /v1/fs/file ") {
        put.wait();
        let body: serde_json::Value =
            serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
        dunlin_workspace::fs::write_file_checked(
            root,
            "file.txt",
            body["contents"].as_str().unwrap(),
            body["expected_mtime_ms"].as_u64(),
        )
        .map(|stat| serde_json::to_value(stat).unwrap())
    } else if request.starts_with("GET /v1/fs/stat?") {
        stat.wait();
        dunlin_workspace::fs::stat_file(root, "file.txt")
            .map(|stat| serde_json::to_value(stat).unwrap())
    } else if request.starts_with("GET /v1/fs/file?") {
        dunlin_workspace::fs::read_file(root, "file.txt")
            .map(|file| serde_json::to_value(file).unwrap())
    } else {
        Err("Fixture does not implement this endpoint".into())
    };
    let (status, body) = match result {
        Ok(body) => (200, body),
        Err(error) => (
            if error.starts_with("conflict:") {
                409
            } else {
                400
            },
            serde_json::json!({"error":error}),
        ),
    };
    let body = body.to_string();
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--save-smoke needs a fresh fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-save-smoke-")
    );
    fs::create_dir(&dir).expect("fixture directory must not already exist");
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("file.txt"), "original").unwrap();
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                auto_connect: false,
                restore_session: false,
                notifications_enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "save-fixture".into(),
            name: "Save fixture".into(),
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
    // No application or worker thread exists yet; never touch the user's data.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let put = Arc::new(Gate::default());
    let stat = Arc::new(Gate::default());
    let (server_root, server_put, server_stat) = (root.clone(), put.clone(), stat.clone());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let (root, put, stat) = (server_root.clone(), server_put.clone(), server_stat.clone());
            std::thread::spawn(move || serve(stream.unwrap(), root, put, stat));
        }
    });
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.save-smoke", "Dunlin Save Smoke");
         dunlin_ui::init(cx);
         crate::native_menu::install(cx);
        crate::logging::init();
        let (window, owner) = open_window(WindowOptions::default(), cx, |window,cx| cx.new(|cx| DunlinApp::new(window,cx))).unwrap();
        window.update(cx, |_,window,cx| owner.update(cx, |app,cx| app.open_file(root.join("file.txt").to_str().unwrap(),window,cx))).unwrap();
        cx.spawn(async move |cx| {
            let start = Instant::now();
            let mut stage = 0;
            let mut baseline = None;
            let done_saving = Rc::new(Cell::new(false));
            let mut released_stat = Instant::now();
            loop {
                cx.background_executor().timer(Duration::from_millis(20)).await;
                let done = cx.update(|cx| {
                    window.update(cx, |_, window, cx| {
                        window.refresh(); window.draw(cx).clear(cx);
                        if stage == 10 {
                            // Dispatch outside owner.update: the real handler
                            // updates that entity and must not reborrow it.
                            crate::controls_smoke::key(window, cx, "cmd-shift-s");
                            stage = 11;
                        }
                    }).unwrap();
                    window.update(cx, |_,window,cx| owner.update(cx, |app,cx| {
                    assert!(start.elapsed()<Duration::from_secs(30), "save smoke stalled at {stage}: {:?}", app.notice);
                    match stage {
                        0 if app.active_file_tab().is_some_and(|tab| !tab.loading) => {
                            app.autosave=false;
                            app.remote_hosts.push(remote::RemoteHost {id:"fixture".into(),name:"Fixture".into(),url:url.clone(),user_name:"Test".into(),user_role:"admin".into(),device_id:"test".into()});
                            app.remote_tokens.insert("fixture".into(),"fixture-token".into());
                            let selected=app.selected_project;
                            app.projects[selected].remote=Some(dunlin_domain::RemoteProject {host_id:"fixture".into(),project_id:"project".into(),path:root.to_string_lossy().into()});
                            let backend=app.backend().unwrap();
                            let id=app.workbench.active.clone().unwrap();
                            app.workbench.tabs.iter_mut().find(|tab|tab.id==id).unwrap().backend=backend;
                            app.editor.update(cx,|input,cx| { input.set_selected_range(0..input.text().len(),cx);input.replace("snapshot",window,cx); });
                            stage=1;
                        }
                        1 if app.editor_dirty => {
                             put.arm();
                             app.editor.read(cx).focus_handle(cx).focus(window, cx);
                             stage=10;
                         }
                         11 => {
                             assert!(app.active_file_tab().unwrap().saving);
                             stage=2;
                        }
                        2 if put.entered() => {
                            // Reaching this stage while HTTP is stalled proves foreground progress.
                            app.editor.update(cx,|input,cx| { input.set_selected_range(0..input.text().len(),cx);input.replace("typed during HTTP",window,cx); });
                            put.release();stage=3;
                        }
                        3 if !app.active_file_tab().unwrap().saving => {
                            assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(),"snapshot");
                            assert_eq!(app.editor.read(cx).value().as_ref(),"typed during HTTP");
                            assert!(app.editor_dirty);
                            put.arm();
                            let done=done_saving.clone();app.save_tabs_then(None,cx,move|_,_|done.set(true));
                            assert!(!done_saving.get());stage=4;
                        }
                        4 if put.entered() => {assert!(!done_saving.get());put.release();stage=5;}
                        5 if done_saving.get() => {
                            assert!(!app.editor_dirty);
                            assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(),"typed during HTTP");
                            baseline=app.editor_mtime;
                            dunlin_workspace::fs::write_file(root.to_str().unwrap(),"file.txt","external edit").unwrap();
                            stat.arm();app.check_open_file(cx);stage=6;
                        }
                        6 if stat.entered() => {
                            app.editor.update(cx,|input,cx| {input.set_selected_range(0..input.text().len(),cx);input.replace("unsaved while stat was pending",window,cx);});
                            stat.release();released_stat=Instant::now();stage=7;
                        }
                        7 if released_stat.elapsed()>Duration::from_millis(150) => {
                            assert_eq!(app.editor_mtime,baseline,"stat must never advance the buffer's baseline");
                            assert!(app.editor_dirty);
                            app.save_editor(cx);stage=8;
                        }
                        8 if !app.active_file_tab().unwrap().saving => {
                            assert!(app.editor_dirty && app.editor_conflict.is_some(),"late stat must not authorize overwrite");
                            assert_eq!(fs::read_to_string(root.join("file.txt")).unwrap(),"external edit");
                            println!("PASS: stalled relay does not block UI; snapshot saves preserve later edits; batch continuation waits; delayed stat preserves baseline and external edits");
                            cx.quit();return true;
                        }
                        _=>{}
                    }
                    false
                })).unwrap() });
                if done {break;}
            }
        }).detach();
    });
}
