//! Isolated native ACP/broker/background-dispatch regression. No model/network
//! requests, Keychain credentials, real repositories or real user history.
use super::*;
use crate::app::StoredSettings;
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};

pub(crate) fn run() {
    let dir = PathBuf::from(
        std::env::args_os()
            .nth(2)
            .expect("Fresh fixture directory required"),
    );
    assert!(
        dir.is_absolute()
            && dir
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("dunlin-coordination-smoke-")
    );
    fs::create_dir(&dir).expect("Fixture directory must be new");
    let root = dir.join("project");
    fs::create_dir(&root).unwrap();
    let mock = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .join("mock_acp_agent");
    assert!(
        mock.is_file(),
        "Build mock_acp_agent next to the desktop binary first"
    );
    let store = dunlin_storage::Store::open(dir.join("dunlin.db")).unwrap();
    store
        .save_setting(
            "app",
            &StoredSettings {
                default_harness: "Custom".into(),
                custom_harness_command: mock.to_string_lossy().into(),
                auto_connect: false,
                restore_session: false,
                notifications_enabled: false,
                vault_enabled: false,
                security_enabled: false,
                ..Default::default()
            },
        )
        .unwrap();
    store
        .upsert_project(&dunlin_domain::Project {
            id: "coordination-fixture".into(),
            name: "Coordination fixture".into(),
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
    // Before GPUI or any worker starts; fixture-scoped only.
    unsafe {
        std::env::set_var("DUNLIN_DATA_DIR", &dir);
        std::env::set_var("DUNLIN_LOG_FILE", dir.join("dunlin.log"));
        std::env::set_var("DUNLIN_HARNESS_COMMAND", &mock);
        std::env::set_var("DUNLIN_MOCK_STREAM_CHUNKS", "1");
        std::env::set_var("DUNLIN_MOCK_DELAY_MS", "500");
        std::env::remove_var("DUNLIN_AUTOPROMPT");
    }
    application().with_assets(assets::AllAssets).run(move|cx|{
        cx.set_app_identity("dev.dunlin.ide.coordination-smoke","Dunlin Coordination Smoke");dunlin_ui::init(cx);crate::native_menu::install(cx);crate::logging::init();
        let(window,owner)=open_window(WindowOptions::default(),cx,|window,cx|cx.new(|cx|DunlinApp::new(window,cx))).unwrap();
        window.update(cx,|_,_,cx|owner.update(cx,|app,cx|app.connect_harness(cx))).unwrap();
        cx.spawn(async move|cx|{
            let start=Instant::now();let mut stage=0;let mut requester=String::new();let mut visible=String::new();let mut proposal=String::new();let mut peers=Vec::new();let mut finished_at=Instant::now();
            loop{
                cx.background_executor().timer(Duration::from_millis(25)).await;
                let done=cx.update(|cx|window.update(cx,|_,window,cx|{
                    window.refresh();window.draw(cx).clear(cx);
                    owner.update(cx,|app,cx|{
                        assert!(start.elapsed()<Duration::from_secs(35),"Coordination smoke stalled at {stage}: {:?}, {:?}",app.notice,app.coordination.error);
                        match stage{
                            0 if app.harness.status.is_ready() && !app.harness.session_metadata_pending && app.harness.modes.is_some() && app.harness.models.is_some()=>{
                                assert!(app.harness.harness.as_ref().unwrap().set_mode(app.harness.session_id.clone().unwrap(),None,"plan".into()));stage=1;
                            }
                            1 if app.harness.modes.as_ref().is_some_and(|m|m.current=="plan")=>{
                                let execution=app.coordination_execution().unwrap();let scope=app.coordination_scope();let store=app.coordination.store().unwrap();
                                for session in ["peer-a","peer-b"]{let r=Run{id:new_id(),session:session.into(),scope:scope.clone(),execution:execution.clone(),title:session.into(),status:RunStatus::Running,queued_prompts:Some(0),created_ms:now_ms(),updated_ms:now_ms(),stop_reason:None,report:None,continuation:None};store.lock().unwrap().start_run(r.clone()).unwrap();peers.push(r.id);}
                                requester=app.session().id.clone();assert!(app.submit_prompt("Prepare a deferred task".into(),cx));
                                let service=app.coordination.service.as_ref().unwrap();let handle=app.harness.coordination_session.as_ref().unwrap();
                                let c:Continuation=serde_json::from_value(dunlin_coordination::service::request(service.socket(),&handle.token,"continuations_create",serde_json::json!({"prompt":"Implement only after both peers finish","idempotency_key":"smoke-once"})).unwrap()).unwrap();
                                assert_eq!(c.dependencies.len(),2);proposal=c.id.clone();store.lock().unwrap().approve(&c.id,&execution,now_ms()+60_000).unwrap();
                                app.new_session(cx);visible=app.session().id.clone();assert_ne!(visible,requester);app.bottom_panel=BottomPanel::Agents;stage=2;
                            }
                            2 if app.parked_runtimes.get(&requester).is_some_and(|r|!r.state.running)=>{
                                let store=app.coordination.store().unwrap();let mut db=store.lock().unwrap();assert_eq!(db.ledger().continuations[0].status,CS::Waiting);db.finish_run(&peers[0],RunStatus::TurnFinished,"end_turn",Some(0),None).unwrap();drop(db);app.poll_coordination(cx);assert!(!app.parked_runtimes[&requester].state.running);stage=3;
                            }
                            3=>{
                                let store=app.coordination.store().unwrap();store.lock().unwrap().finish_run(&peers[1],RunStatus::TurnFinished,"end_turn",Some(0),None).unwrap();app.poll_coordination(cx);
                                assert!(app.parked_runtimes[&requester].state.running,"Background continuation was not dispatched: {:?}",app.coordination.snapshot.continuations);
                                assert_eq!(app.session().id,visible);stage=4;
                            }
                            4 if app.parked_runtimes.get(&requester).is_some_and(|r|!r.state.running)=>{
                                let store=app.coordination.store().unwrap();let db=store.lock().unwrap();assert_eq!(db.ledger().continuations.iter().find(|c|c.id==proposal).unwrap().status,CS::Dispatched);
                                let runs:Vec<_>=db.ledger().runs.iter().filter(|r|r.session==requester).collect();assert_eq!(runs.len(),2);assert!(runs.iter().all(|r|r.execution.mode=="plan" && r.status==RunStatus::TurnFinished));
                                let settings:StoredSettings=app.store.as_ref().unwrap().load_setting("app").unwrap().unwrap();assert_eq!(settings.last_session_id.as_deref(),Some(visible.as_str()));finished_at=Instant::now();stage=5;
                            }
                            5 if finished_at.elapsed()>Duration::from_secs(2)=>{
                                assert_eq!(app.coordination.store().unwrap().lock().unwrap().ledger().runs.iter().filter(|r|r.session==requester).count(),2,"Continuation replayed");
                                let store=app.coordination.store().unwrap();let mut db=store.lock().unwrap();let mut blocked=db.ledger().continuations.iter().find(|c|c.id==proposal).unwrap().clone();
                                blocked.id=new_id();blocked.key="budget-blocked".into();blocked.requester_run=db.ledger().current_run(&blocked.scope,&requester).unwrap().id.clone();blocked.status=CS::PendingApproval;blocked.dispatched_run=None;blocked.approved_ms=None;
                                db.mutate(|l|{l.continuations.push(blocked.clone());Ok(())}).unwrap();db.approve(&blocked.id,&blocked.execution,now_ms()+60_000).unwrap();drop(db);
                                app.approve_task_budget(dunlin_domain::task_budget::Budget{id:new_id(),label:"Stopped fixture budget".into(),started_ms:now_ms(),deadline_ms:now_ms()+60_000,max_attempts:1,reported_usd_limit:None,stopped:Some("Explicitly stopped by fixture".into()),members:vec![dunlin_domain::task_budget::Member{session:requester.clone(),project:app.project().id.clone(),root:app.project().path.clone(),attempts:0,baseline:None,latest:None,opaque_children:false}]}).unwrap();
                                app.poll_coordination(cx);let db=store.lock().unwrap();assert_eq!(db.ledger().continuations.iter().find(|c|c.id==blocked.id).unwrap().status,CS::NeedsAttention);assert_eq!(db.ledger().runs.iter().filter(|r|r.session==requester).count(),2);
                                println!("PASS: native ACP + MCP snapshot + two-run barrier + one approved background turn + preserved plan mode and foreground selection; no replay; stopped budget blocks further dispatch");cx.quit();return true;
                            }
                            _=>{}
                        }
                        false
                    })
                }).unwrap());
                if done{break;}
            }
        }).detach();
    });
}
