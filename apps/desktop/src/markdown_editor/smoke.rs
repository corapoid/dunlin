//! Isolated native-input/buffer smoke; no user database, harness, or user files.
use super::{Format, Mode, table::Operation};
use crate::app::{DunlinApp, StoredSettings};
use dunlin_ui::{
    component::{
        TitleBar,
        input::{Redo, Undo},
    },
    *,
};
use std::{
    fs,
    net::TcpListener,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let light_narrow = std::env::args().any(|arg| arg == "--light-narrow");
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("--markdown-smoke needs a fresh fixture directory"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-markdown-smoke-")
    );
    fs::create_dir(&dir).expect("fixture directory must not already exist");
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let original = format!(
        "Hello world\n\n# Title\n\n| A | B |\n| :- | -: |\n| left | **right** |\n\n```rust\nlet x = 1;\n```\n\nEntity &amp; żółw\n\n![blocked](http://{0}/image.png)\n\n<img src=\"http://{0}/raw.png\">\n",
        listener.local_addr().unwrap()
    );
    let formatted = original.replacen("world", "**world**", 1);
    let typed = formatted.replacen("world", "friend", 1);
    let cell_edited = typed.replacen("| left |", "| changed \\| żółw |", 1);
    let code_edited = cell_edited.replacen(
        "```rust\nlet x = 1;\n```",
        "````rust\nlet x = 2;\n```\nend\n````",
        1,
    );
    let image_edited = code_edited.replacen("![blocked]", "![description]", 1);
    let one_row = image_edited.replacen(
        "| changed \\| żółw | **right** |",
        "| changed \\| żółw | **right** |\n|  |  |",
        1,
    );
    let two_rows = one_row.replacen(
        "| changed \\| żółw | **right** |",
        "| changed \\| żółw | **right** |\n|  |  |",
        1,
    );
    let split_source = image_edited.replacen("Hello **friend**", "Hello&#32;\n\n**friend**", 1);
    let merged_source = image_edited.replacen("Hello **friend**", "Hello&#32;**friend**", 1);
    let soft_source = image_edited.replacen("Hello **friend**", "Hello&#32;\n**friend**", 1);
    let code_entered = soft_source.replacen("end\n````", "end\n\n````", 1);
    let path = root.join("notes.md");
    fs::write(&path, &original).unwrap();
    let advanced = "Alpha 👩‍💻\n\nBeta żółw\n\n- first\n- second\n  - child\n\n> quote\n\n[link](https://example.com)\n\n![local](pixel.png)\n";
    let advanced_path = root.join("advanced.md");
    fs::write(&advanced_path, advanced).unwrap();
    image::RgbaImage::from_pixel(2, 2, image::Rgba([20, 100, 200, 255]))
        .save(root.join("pixel.png"))
        .unwrap();
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
            id: "markdown-fixture".into(),
            name: "Markdown smoke".into(),
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
    // main calls this before application/worker threads exist.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::remove_var("DUNLIN_AUTOPROMPT");
        std::env::remove_var("DUNLIN_HARNESS_COMMAND");
    }
    application().with_assets(assets::AllAssets).run(move |cx| {
        cx.set_app_identity("dev.dunlin.ide.markdown-smoke", "Dunlin Markdown Smoke");
        dunlin_ui::init(cx); crate::logging::init();
        let (window, owner) = open_window(WindowOptions { window_bounds: Some(WindowBounds::Windowed(
             Bounds::new(point(px(90.), px(90.)), size(px(if light_narrow {760.} else {1200.}), px(800.))))), ..TitleBar::window_options() }, cx,
            |window, cx| cx.new(|cx| DunlinApp::new(window, cx))).unwrap();
        window.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
            app.section = dunlin_domain::SidebarSection::Files;
            app.open_workbench_file(path.to_str().unwrap(), None, window, cx);
        })).unwrap();
         let mut original_clipboard=cx.read_from_clipboard();
         cx.spawn(async move |cx| {
            let started = Instant::now(); let mut stage = 0; let mut editor_id = None;
            let mut visual_geometry: Vec<(EntityId, Bounds<Pixels>, Option<Pixels>)> = Vec::new();
            loop {
                cx.background_executor().timer(Duration::from_millis(220)).await;
                let done = cx.update(|cx| {
                    assert!(started.elapsed() < Duration::from_secs(60), "Markdown smoke stalled at stage {stage}");
                    // Native windows may not receive frame callbacks while the
                    // user's other app is frontmost. Build the real dispatch tree.
                     window.update(cx, |_, window, cx| {window.refresh();window.draw(cx).clear(cx);}).unwrap();
                    if stage == 100 || stage == 102 {
                        let bounds: Bounds<Pixels> = visual_geometry[if stage == 100 { 0 } else { 1 }].1;
                        let position = point(bounds.left() + px(12.), bounds.top() + px(10.));
                        window.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
                            window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Left, position, click_count: 1, ..Default::default() }), cx);
                        }).unwrap();
                    }
                    if matches!(stage, 20 | 22 | 24) {
                        window.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent {
                                keystroke: Keystroke::parse(if stage == 24 { "shift-tab" } else { "tab" }).unwrap(),
                                is_held: false, prefer_character_input: false,
                            }), cx);
                        }).unwrap();
                    }
                    if matches!(stage,77|78|80) {
                        window.update(cx,|_,window,cx| {
                            let state=&owner.read(cx).active_file_tab().unwrap().markdown.visual;
                            let index=if stage==78 {1} else {0};let offset=if stage==78 {2} else {1};
                            let r=state.editors[index].editor.read(cx).range_to_bounds(&(offset..offset)).unwrap();let position=point(r.left()+px(0.1),r.top()+px(3.));
                            if stage==78 {
                                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent{position,pressed_button:Some(MouseButton::Left),..Default::default()}),cx);
                            } else {
                                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent{button:MouseButton::Left,position,click_count:1,modifiers:Modifiers{shift:stage==80,..Default::default()},..Default::default()}),cx);
                            }
                            if stage!=77 {window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent{button:MouseButton::Left,position,click_count:1,..Default::default()}),cx);}
                        }).unwrap();
                    }
                    let structural_key = match stage { 40 | 46 | 52 | 54 => Some("enter"), 42 => Some("backspace"), 48 => Some("delete"), 50 => Some("shift-enter"),61=>Some("right"),63=>Some("left"),65|67|110..=113=>Some("shift-right"),85=>Some("tab"),87=>Some("shift-tab"),89=>Some("down"),91=>Some("up"), _ => None };
                    if let Some(key) = structural_key {
                        window.update(cx, |_, window, cx| {
                            window.dispatch_event(PlatformInput::KeyDown(KeyDownEvent { keystroke: Keystroke::parse(key).unwrap(), is_held: false, prefer_character_input: false }), cx);
                        }).unwrap();
                    }
                    window.update(cx, |_, window, cx| owner.update(cx, |app, cx| {
                        match stage {
                            0 if app.active_file_tab().is_some_and(|t| !t.loading && t.markdown.visual.document.is_some()) => {
                                assert_eq!(app.active_file_tab().unwrap().markdown.mode, Mode::Visual);
                                editor_id = Some(app.editor.entity_id());
                                let state = &app.active_file_tab().unwrap().markdown.visual;
                                assert_eq!(state.editors.len(), state.document.as_ref().unwrap().blocks.len());
                                assert!(state.document.as_ref().unwrap().blocks.iter().all(|b| b.editable));
                                visual_geometry = state.editors.iter().map(|e| (e.editor.entity_id(), e.editor.read(cx).input_bounds(), e.editor.read(cx).line_height())).collect();
                                assert!(visual_geometry[0].2.unwrap() <= px(20.), "body typography must stay compact");
                                assert!(visual_geometry[1].2.unwrap() <= px(28.), "headings must not dwarf the app UI");
                                let code = state.editors.iter().find(|e| matches!(e.block.kind, super::document::Kind::Code)).unwrap();
                                assert!(code.editor.read(cx).line_height().unwrap() <= px(18.));
                                stage = 100;
                            }
                            100 => { stage = 101; }
                            101 => {
                                let state = &app.active_file_tab().unwrap().markdown.visual;
                                let active = state.active.as_ref().expect("one click must focus a mounted visual editor");
                                assert_eq!(active.editor.entity_id(), visual_geometry[0].0);
                                assert!(active.editor.read(cx).selected_range().end < "Hello world".len(), "the first click must place the caret at the pointer, not at the block end");
                                stage = 102;
                            }
                            102 => { stage = 103; }
                            103 => {
                                let state = &app.active_file_tab().unwrap().markdown.visual;
                                assert_eq!(state.active.as_ref().unwrap().editor.entity_id(), visual_geometry[1].0, "the previous editor's empty minimum rows must not intercept the heading click");
                                let block = state.document.as_ref().unwrap().blocks[0].clone();
                                app.begin_visual_block(block, Some(6..11), window, cx); stage = 1;
                            }
                            1 => {
                                let state = &app.active_file_tab().unwrap().markdown.visual;
                                for (editor, (id, bounds, line_height)) in state.editors.iter().zip(&visual_geometry) {
                                    assert_eq!(editor.editor.entity_id(), *id, "focus must reuse the mounted editor");
                                    assert_eq!(editor.editor.read(cx).input_bounds(), *bounds, "focus must not resize text or its field");
                                    assert_eq!(editor.editor.read(cx).line_height(), *line_height, "focus must not change typography");
                                }
                                assert_eq!(app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.read(cx).text().to_string(), "Hello world");
                                app.format_markdown(Format::Bold, window, cx); stage = 2;
                            }
                            2 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), formatted);
                                assert!(app.editor_dirty && app.active_file_tab().unwrap().dirty);
                                assert_eq!(fs::read_to_string(&path).unwrap(), original);
                                let active = app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();
                                assert!(active.block.leaves.iter().any(|l| l.marks.bold));
                                let visual = active.editor.clone();
                                visual.update(cx, |state, cx| { state.set_selected_range(6..11, cx); state.replace("friend", window, cx); });
                                stage = 3;
                            }
                            3 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), typed);
                                assert!(window.is_action_available(&Undo, cx));
                                window.dispatch_action(Box::new(Undo), cx); stage = 4;
                            }
                            4 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), formatted);
                                window.dispatch_action(Box::new(Undo), cx); stage = 5;
                            }
                            5 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), original);
                                assert!(!app.editor_dirty);
                                window.dispatch_action(Box::new(Redo), cx); stage = 6;
                            }
                            6 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), formatted);
                                window.dispatch_action(Box::new(Redo), cx); stage = 7;
                            }
                            7 => {
                                assert_eq!(Some(app.editor.entity_id()), editor_id);
                                assert_eq!(app.editor.read(cx).text().to_string(), typed);
                                app.toggle_markdown_mode(window, cx); stage = 8;
                            }
                            8 => {
                                assert_eq!(app.active_file_tab().unwrap().markdown.mode, Mode::Source);
                                assert_eq!(app.editor.read(cx).text().to_string(), typed);
                                app.toggle_markdown_mode(window, cx); stage = 9;
                            }
                            9 if app.active_file_tab().is_some_and(|t| !t.markdown.pending) => {
                                assert_eq!(app.active_file_tab().unwrap().markdown.mode, Mode::Visual);
                                app.save_editor(cx);
                                assert_eq!(fs::read_to_string(&path).unwrap(), typed);
                                assert!(!app.editor_dirty);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b| b.text == "left").unwrap().clone();
                                app.begin_visual_block(block, Some(0..4), window, cx); stage = 10;
                            }
                            10 => {
                                let visual = app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.clone();
                                visual.update(cx, |state, cx| state.replace("changed | żółw", window, cx)); stage = 11;
                            }
                            11 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), cell_edited);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b| b.text == "let x = 1;").unwrap().clone();
                                let end = block.text.len(); app.begin_visual_block(block, Some(0..end), window, cx); stage = 12;
                            }
                            12 => {
                                let visual = app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.clone();
                                visual.update(cx, |state, cx| state.replace("let x = 2;\n```\nend", window, cx)); stage = 13;
                            }
                            13 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), code_edited);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b| b.text == "blocked").unwrap().clone();
                                app.begin_visual_block(block, Some(0..7), window, cx); stage = 14;
                            }
                            14 => {
                                let visual = app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.clone();
                                visual.update(cx, |state, cx| state.replace("description", window, cx)); stage = 15;
                            }
                            15 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), image_edited);
                                app.visual_history(false, window, cx); stage = 16;
                            }
                            16 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), code_edited);
                                app.visual_history(true, window, cx); stage = 17;
                            }
                            17 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), image_edited);
                                app.save_editor(cx);
                                assert_eq!(fs::read_to_string(&path).unwrap(), image_edited);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b| b.text == "right").unwrap().clone();
                                app.begin_visual_block(block, Some(0..5), window, cx); stage = 18;
                            }
                            18 => {
                                let target = app.visual_table_target().unwrap();
                                app.edit_visual_table(target.clone(), Operation::RowAfter, window, cx);
                                // A menu opened on an older source cannot apply to the new row.
                                app.edit_visual_table(target, Operation::DeleteRow, window, cx); stage = 19;
                            }
                            19 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), one_row);
                                let target = app.visual_table_target().unwrap(); assert_eq!((target.row, target.column), (2, 1));
                                stage = 20;
                            }
                            20 => { stage = 21; }
                            21 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), two_rows);
                                let target = app.visual_table_target().unwrap(); assert_eq!((target.row, target.column), (3, 0));
                                stage = 22;
                            }
                            22 => { stage = 23; }
                            23 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), two_rows);
                                let target = app.visual_table_target().unwrap(); assert_eq!((target.row, target.column), (3, 1));
                                stage = 24;
                            }
                            24 => { stage = 25; }
                            25 => {
                                let target = app.visual_table_target().unwrap(); assert_eq!((target.row, target.column), (3, 0));
                                app.edit_visual_table(target, Operation::ColumnAfter, window, cx); stage = 26;
                            }
                            26 => {
                                let target = app.visual_table_target().unwrap(); assert_eq!((target.row, target.column, target.columns), (3, 1, 3));
                                app.edit_visual_table(target, Operation::Align(markdown::mdast::AlignKind::Center), window, cx); stage = 27;
                            }
                            27 => {
                                let target = app.visual_table_target().unwrap(); assert_eq!(target.align, markdown::mdast::AlignKind::Center);
                                app.edit_visual_table(target, Operation::DeleteColumn, window, cx); stage = 28;
                            }
                            28 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), two_rows);
                                let target = app.visual_table_target().unwrap();
                                app.edit_visual_table(target, Operation::DeleteRow, window, cx); stage = 29;
                            }
                            29 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), one_row);
                                let target = app.visual_table_target().unwrap();
                                app.edit_visual_table(target, Operation::DeleteRow, window, cx); stage = 30;
                            }
                            30 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), image_edited);
                                app.visual_history(false, window, cx); stage = 31;
                            }
                            31 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), one_row);
                                app.visual_history(true, window, cx); stage = 32;
                            }
                            32 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), image_edited);
                                let target = app.visual_table_target().unwrap();
                                let id = app.active_file_tab().unwrap().id.clone();
                                app.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap().readonly = true;
                                app.edit_visual_table(target, Operation::RowAfter, window, cx);
                                assert_eq!(app.editor.read(cx).text().to_string(), image_edited, "read-only table edits must not mutate source");
                                app.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap().readonly = false;
                                app.save_editor(cx);
                                assert_eq!(fs::read_to_string(&path).unwrap(), image_edited);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();
                                app.begin_visual_block(block, Some(6..6), window, cx); stage = 40;
                            }
                            40 => { stage = 41; }
                            41 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), split_source);
                                let active = app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();
                                assert_eq!(active.block.text, "friend"); assert_eq!(active.editor.read(cx).selected_range(), 0..0);
                                stage = 42;
                            }
                            42 => { stage = 43; }
                            43 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), merged_source);
                                assert_eq!(app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.read(cx).selected_range(), 6..6);
                                app.visual_history(false, window, cx); stage = 44;
                            }
                            44 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), split_source);
                                app.visual_history(true, window, cx); stage = 45;
                            }
                            45 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), merged_source);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();
                                app.begin_visual_block(block, Some(6..6), window, cx); stage = 46;
                            }
                            46 => { stage = 47; }
                            47 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), split_source);
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();
                                app.begin_visual_block(block, Some(6..6), window, cx); stage = 48;
                            }
                            48 => { stage = 49; }
                            49 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), merged_source);
                                stage = 50;
                            }
                            50 => { stage = 51; }
                            51 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), soft_source);
                                assert_eq!(app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().block.text, "Hello \nfriend");
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b| matches!(b.kind, super::document::Kind::Code)).unwrap().clone();
                                let end = block.text.len(); app.begin_visual_block(block, Some(end..end), window, cx); stage = 52;
                            }
                            52 => { stage = 53; }
                            53 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), code_entered);
                                assert!(app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().block.text.ends_with('\n'));
                                let block = app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();
                                app.begin_visual_block(block, Some(6..6), window, cx);
                                let id = app.active_file_tab().unwrap().id.clone();
                                app.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap().readonly = true;
                                stage = 54;
                            }
                            54 => { stage = 55; }
                            55 => {
                                assert_eq!(app.editor.read(cx).text().to_string(), code_entered, "locked prose must not split");
                                let id = app.active_file_tab().unwrap().id.clone();
                                app.workbench.tabs.iter_mut().find(|t| t.id == id).unwrap().readonly = false;
                                app.save_editor(cx); assert_eq!(fs::read_to_string(&path).unwrap(), code_entered);
                                app.set_markdown_mode(Mode::Visual, window, cx);
                                 app.close_active_tab();
                                 app.open_workbench_file(advanced_path.to_str().unwrap(),None,window,cx);stage=60;
                             }
                             60 if app.active_file_tab().is_some_and(|t|!t.loading && t.markdown.visual.document.is_some()) => {
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();let end=b.text.len();app.begin_visual_block(b,Some(end..end),window,cx);stage=61;
                             }
                             61|63|65|67|85|87|89|91=>{stage+=1;}
                             62=>{
                                 let a=app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();assert_eq!(a.block.text,"Beta żółw");assert_eq!(a.editor.read(cx).cursor(),0);stage=63;
                             }
                             64=>{
                                 let a=app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();assert_eq!(a.block.text,"Alpha 👩‍💻");assert_eq!(a.editor.read(cx).cursor(),a.block.text.len());stage=65;
                             }
                             66=>{
                                 let s=&app.active_file_tab().unwrap().markdown.visual;let selection=s.selection.expect("shift-right must cross blocks");assert_eq!((selection.anchor.block,selection.head.block),(0,1));assert_eq!(selection.text(s.document.as_ref().unwrap()).unwrap(),"\n\n");stage=67;
                             }
                             68=>{
                                 let s=&app.active_file_tab().unwrap().markdown.visual;assert_eq!(s.selection.unwrap().text(s.document.as_ref().unwrap()).unwrap(),"\n\nB");window.dispatch_action(Box::new(dunlin_ui::component::input::Copy),cx);stage=69;
                             }
                             69=>{assert_eq!(cx.read_from_clipboard().unwrap().text().unwrap(),"\n\nB");window.dispatch_action(Box::new(dunlin_ui::component::input::Cut),cx);stage=70;}
                             70=>{assert_eq!(app.editor.read(cx).text().to_string(),advanced.replacen("👩‍💻\n\nB","👩‍💻",1));app.visual_history(false,window,cx);stage=71;}
                             71=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);
                                 app.set_visual_selection(super::selection::Selection{anchor:super::selection::Point{block:0,offset:1},head:super::selection::Point{block:1,offset:2}},window,cx);
                                 let editor=app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap().editor.clone();editor.update(cx,|s,cx|s.replace("X",window,cx));stage=72;
                             }
                             72=>{assert_eq!(app.editor.read(cx).text().to_string(),advanced.replacen("Alpha 👩‍💻\n\nBe","AX",1));app.visual_history(false,window,cx);stage=73;}
                             73=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);app.set_visual_selection(super::selection::Selection{anchor:super::selection::Point{block:0,offset:1},head:super::selection::Point{block:1,offset:2}},window,cx);cx.write_to_clipboard(ClipboardItem::new_string("paste".into()));window.dispatch_action(Box::new(dunlin_ui::component::input::Paste),cx);stage=74;
                             }
                             74=>{assert_eq!(app.editor.read(cx).text().to_string(),advanced.replacen("Alpha 👩‍💻\n\nBe","Apaste",1));app.visual_history(false,window,cx);stage=75;}
                             75=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);app.select_visual_document(window,cx);
                                 let id=app.active_file_tab().unwrap().id.clone();app.workbench.tabs.iter_mut().find(|t|t.id==id).unwrap().readonly=true;window.dispatch_action(Box::new(dunlin_ui::component::input::Cut),cx);stage=76;
                             }
                             76=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);let id=app.active_file_tab().unwrap().id.clone();app.workbench.tabs.iter_mut().find(|t|t.id==id).unwrap().readonly=false;app.clear_visual_selection(cx);
                                 let tab=app.workbench.tabs.iter_mut().find(|t|t.id==id).unwrap();tab.markdown.scroll.set_offset(point(px(0.),px(0.)));
                                 stage=77;
                             }
                             77|78|80=>{stage+=1;}
                             79=>{let s=&app.active_file_tab().unwrap().markdown.visual;let selection=s.selection.expect("pointer drag must select across fields");assert_eq!((selection.anchor.block,selection.head.block),(0,1));assert_eq!(selection.text(s.document.as_ref().unwrap()).unwrap(),"lpha 👩‍💻\n\nBe");stage=80;}
                             81=>{
                                 let state=&app.active_file_tab().unwrap().markdown.visual;assert!(state.selection.is_none());assert_eq!(state.active.as_ref().unwrap().block.text,"Alpha 👩‍💻");assert_eq!(state.active.as_ref().unwrap().editor.read(cx).cursor(),1,"shift-click must return to the anchor field");
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();app.begin_visual_block(b,Some(4..4),window,cx);stage=110;
                             }
                             110..=113=>{stage+=1;}
                             114=>{
                                 let s=&app.active_file_tab().unwrap().markdown.visual;let selection=s.selection.expect("native inline selection must extend into the next field");assert_eq!(selection.anchor.offset,4);assert_eq!(selection.head.block,1);assert_eq!(selection.text(s.document.as_ref().unwrap()).unwrap(),"a 👩‍💻\n\n");
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();app.begin_visual_block(b,Some(2..2),window,cx);stage=89;
                             }
                             90=>{let a=app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();assert_eq!(a.block.text,"Beta żółw");assert!(a.editor.read(cx).cursor()>0);stage=91;}
                             92=>{
                                 let a=app.active_file_tab().unwrap().markdown.visual.active.as_ref().unwrap();assert_eq!(a.block.text,"Alpha 👩‍💻");assert_eq!(a.editor.read(cx).cursor(),2);
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b|b.text=="second").unwrap().clone();app.begin_visual_block(b,Some(2..2),window,cx);stage=85;
                             }
                             86=>{assert!(app.editor.read(cx).text().to_string().contains("- first\n  - second\n    - child"));stage=87;}
                             88=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b|b.text=="link").unwrap().clone();app.begin_visual_block(b,Some(1..1),window,cx);app.open_visual_resource(window,cx);stage=93;
                             }
                             93=>{
                                 let d=app.active_file_tab().unwrap().markdown.resource.as_ref().unwrap();d.address.update(cx,|s,cx|s.set_value("https://example.org/new",window,cx));d.title.update(cx,|s,cx|s.set_value("new title",window,cx));app.save_visual_resource(window,cx);stage=94;
                             }
                             94=>{
                                 assert!(app.editor.read(cx).text().to_string().contains("[link](<https://example.org/new> \"new title\")"));app.visual_history(false,window,cx);stage=95;
                             }
                             95=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks.iter().find(|b|b.text=="local").unwrap().clone();app.begin_visual_block(b,Some(2..2),window,cx);app.open_visual_resource(window,cx);app.activate_visual_resource(window,cx);stage=96;
                             }
                             96 if app.active_file_tab().unwrap().markdown.resource.as_ref().unwrap().preview.is_some()=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced,"explicit preview is read-only");
                                 app.activate_visual_resource(window,cx);app.close_visual_resource(cx);app.open_visual_resource(window,cx);stage=97;
                             }
                             97=>{
                                 assert!(app.active_file_tab().unwrap().markdown.resource.as_ref().unwrap().preview.is_none(),"cancelled image completion must not restore a reopened panel");
                                 app.close_visual_resource(cx);
                                 let b=app.active_file_tab().unwrap().markdown.visual.document.as_ref().unwrap().blocks[0].clone();app.begin_visual_block(b,Some(1..3),window,cx);app.format_visual_markdown(Format::Image,window,cx);stage=98;
                             }
                             98=>{
                                 assert!(app.editor.read(cx).text().to_string().contains("A![lp](image.png)ha"));
                                 assert_eq!(app.active_file_tab().unwrap().markdown.resource.as_ref().expect("insert image must open properties").address.read(cx).value(),"image.png");
                                 app.close_visual_resource(cx);app.visual_history(false,window,cx);stage=99;
                             }
                             99=>{
                                 assert_eq!(app.editor.read(cx).text().to_string(),advanced);app.save_editor(cx);app.close_active_tab();stage=33;
                             }
                             33 => {
                                 assert!(app.workbench.tabs.is_empty());
                                 if let Some(clipboard)=original_clipboard.take() {cx.write_to_clipboard(clipboard);}
                                assert!(matches!(listener.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock), "Preview attempted an image request");
                                 println!("PASS: compact focus-stable typography, prose/table/code editing, cross-block arrows and Shift-selection, pointer drag/Shift-click, copy/cut/paste/typing, nested Tab/Shift+Tab, link properties, explicit project image preview, locks/history/Raw/save, no external image requests");
                                cx.quit(); return true;
                            }
                            _ => {}
                        }
                        false
                    })).unwrap()
                });
                if done { break; }
            }
        }).detach();
    });
}
