//! Native controller acceptance uses ONLY the bundled mock ACP, never a provider.
use super::*;
use crate::comparisons::{Choice, Spec};
use crate::harness::ChatItem;
fn waiting(app: &DunlinApp, id: &str) -> bool {
    app.comparison_runtime(id).is_some_and(|r| {
        r.transcript
            .iter()
            .any(|t| matches!(t, ChatItem::Permission { resolved: None, .. }))
    })
}
fn chat_waiting(app: &DunlinApp) -> bool {
    app.harness.transcript.iter().any(|item| matches!(item, ChatItem::Permission { resolved: None, .. }))
}
fn tab_session(app: &DunlinApp, title: &str) -> String {
    app.all_sessions.iter().find(|session| session.title == title).unwrap().id.clone()
}
fn approve_chat(app: &mut DunlinApp, cx: &mut Context<DunlinApp>) {
    let (id, choice) = app.harness.transcript.iter().find_map(|item| match item {
        ChatItem::Permission { request_id, options, resolved: None, .. } => Some((request_id.clone(), options.iter().find(|o| o.kind == dunlin_agent::PermissionKind::AllowOnce).unwrap().id.clone())),
        _ => None,
    }).unwrap();
    app.resolve_permission(&id, Some(choice), "Allow once".into(), cx);
}
pub(super) fn step(
    app: &mut DunlinApp,
    stage: &mut i32,
    w: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> bool {
    match *stage {
        0 if app.active_file_tab().is_some_and(|t| !t.loading) => {
            let owner = app.project().clone();
            let session = app.session().id.clone();
            app.open_comparisons(w, cx);
            let trees = dunlin_workspace::comparison::create(
                dunlin_workspace::comparison::prepare(&owner.path).unwrap(),
            );
            assert!(trees.error.is_none(), "{:?}", trees.error);
            let id = trees.id.clone();
            app.register_comparison(
                &owner,
                Spec {
                    label: "Native mock A/B".into(),
                    prompt: "Identical approved fixture prompt".into(),
                    choices: [
                        Choice {
                            harness: "Custom ACP".into(),
                            model: "mock-a".into(),
                        },
                        Choice {
                            harness: "Custom ACP".into(),
                            model: "mock-b".into(),
                        },
                    ],
                    seconds: 120,
                    requests: 2,
                },
                trees,
                Default::default(),
            )
            .unwrap();
            app.launch_comparison(&id, cx).unwrap();
            assert_eq!(app.session().id, session);
            assert_eq!(app.project().id, owner.id);
            *stage = 1;
        }
        1 => {
            let run = app.comparisons.runs[0].clone();
            if run.sides.iter().all(|s| waiting(app, &s.session)) {
                assert!(
                    run.sides.iter().all(|s| s.claimed && s.error.is_none()),
                    "{run:?}"
                );
                assert_eq!(run.sides[0].actual_model.as_deref(), Some("mock-a"));
                assert_eq!(run.sides[1].actual_model.as_deref(), Some("mock-b"));
                let budget = app
                    .task_budgets
                    .registry
                    .task(&run.sides[0].session)
                    .unwrap();
                assert_eq!(budget.attempts(), 2);
                assert_eq!(budget.spent(), None);
                for side in &run.sides {
                    let runtime = app.comparison_runtime(&side.session).unwrap();
                    assert_eq!(
                        runtime
                            .transcript
                            .iter()
                            .filter(|t| matches!(t, ChatItem::User { .. }))
                            .count(),
                        1
                    );
                }
                assert!(!app.smoke_comparison_send(
                    &run.sides[0].session,
                    "must not send".into(),
                    cx
                ));
                app.smoke_comparison_allow(&run.sides[0].session, cx);
                *stage = 2;
            }
        }
        2 => {
            let run = app.comparisons.runs[0].clone();
            let a = app.comparison_runtime(&run.sides[0].session).unwrap();
            if !a.running {
                assert!(a.transcript.iter().any(|t|matches!(t,ChatItem::Permission{resolved:Some(label),..} if label=="Smoke allow once")));
                assert!(waiting(app, &run.sides[1].session));
                let mut next = app.task_budgets.registry.clone();
                next.task_mut(&run.sides[1].session).unwrap().stopped =
                    Some("Stopped by smoke".into());
                app.persist_task_budgets(next).unwrap();
                app.poll_task_budgets(cx);
                *stage = 3;
            }
        }
        3 => {
            let run = app.comparisons.runs[0].clone();
            let b = app.comparison_runtime(&run.sides[1].session).unwrap();
            assert!(!b.running);
            assert!(!waiting(app, &run.sides[1].session));
            assert!(b.transcript.iter().any(|t|matches!(t,ChatItem::Permission{resolved:Some(label),..} if label.contains("Denied by task budget"))));
            let restored = crate::comparisons::State::load(app.store.as_ref());
            assert!(restored.active.is_empty());
            assert!(restored.runs[0].started_ms.is_some());
            assert!(restored.runs[0].sides.iter().all(|s| s.claimed));
            assert!(app.launch_comparison(&run.id, cx).is_err());
            *stage = 4;
        }
        4..=9 => {
            app.advance_comparisons(cx);
            assert_eq!(app.task_budgets.registry.tasks[0].attempts(), 2);
            *stage += 1;
        }
        10 => {
            app.open_impact_map(w, cx);
            *stage = 11;
        }
        11 if app.smoke_impact_ready() => {
            *stage = 12;
        }
        12 => {
            app.open_task_budgets(w, cx);
            *stage = 13;
        }
        13 => {
            super::context::smoke_begin(app, w, cx); *stage = 14;
        }
        14 => {
            super::context::smoke_finish(app, w, cx);
            std::fs::write(std::path::Path::new(&app.project().path).join("example.rs"), "fn changed_on_disk() {}\n// unchanged\n").unwrap();
            super::review::smoke_begin(app, w, cx); *stage = 15;
        }
        15 if super::review::smoke_edit(app, cx) => { *stage = 16; }
        16 if super::review::smoke_diff(app, cx) => { *stage = 17; }
        17 if super::review::smoke_finish(app, cx) => {
            super::checks::smoke_begin(app, w, cx); *stage = 18;
        }
        18 => {
            app.new_session(cx);
            app.composer.update(cx, |s, cx| s.set_value("Concurrent alpha", w, cx));
            app.send_prompt(w, cx);
            *stage = 19;
        }
        19 if chat_waiting(app) => {
            app.composer.update(cx, |s, cx| s.set_value("Draft alpha", w, cx));
            app.new_session(cx);
            app.composer.update(cx, |s, cx| s.set_value("Concurrent beta", w, cx));
            app.send_prompt(w, cx);
            *stage = 20;
        }
        20 if chat_waiting(app) => {
            let a = tab_session(app, "Concurrent alpha");
            let parked = &app.parked_runtimes[&a].state;
            assert!(parked.running);
            assert!(parked.transcript.iter().any(|item| matches!(item, ChatItem::User { text, .. } if text == "Concurrent alpha")));
            assert!(!parked.transcript.iter().any(|item| matches!(item, ChatItem::User { text, .. } if text == "Concurrent beta")));
            app.composer.update(cx, |s, cx| s.set_value("Draft beta", w, cx));
            app.activate_chat_tab(&a, w, cx);
            *stage = 21;
        }
        21 => {
            assert_eq!(app.composer.read(cx).value(), "Draft alpha");
            assert!(app.harness.running && chat_waiting(app));
            let b = tab_session(app, "Concurrent beta");
            app.close_session_tab(&b, w, cx);
            assert!(app.parked_runtimes[&b].state.running);
            assert!(!app.workbench.chat_tabs.contains(&b));
            app.activate_chat_tab(&b, w, cx);
            *stage = 22;
        }
        22 => {
            assert_eq!(app.composer.read(cx).value(), "Draft beta");
            app.close_chat_tab(w, cx);
            assert_eq!(app.session().title, "Concurrent alpha");
            *stage = 23;
        }
        23 => {
            assert_eq!(app.composer.read(cx).value(), "Draft alpha");
            let b = tab_session(app, "Concurrent beta");
            assert!(app.parked_runtimes[&b].state.running);
            app.activate_chat_tab(&b, w, cx);
            *stage = 24;
        }
        24 if chat_waiting(app) => { approve_chat(app, cx); *stage = 25; }
        25 if !app.harness.running => {
            let a = tab_session(app, "Concurrent alpha");
            app.activate_chat_tab(&a, w, cx);
            assert!(app.harness.running && chat_waiting(app));
            approve_chat(app, cx);
            *stage = 26;
        }
        26 if !app.harness.running => {
            for title in ["Concurrent alpha", "Concurrent beta"] {
                let id = tab_session(app, title);
                let items: Vec<ChatItem> = app.store.as_ref().unwrap().load_transcript(&id).unwrap().unwrap();
                assert_eq!(items.iter().filter(|item| matches!(item, ChatItem::User { .. })).count(), 1);
                assert!(items.iter().any(|item| matches!(item, ChatItem::User { timing: Some(t), .. } if t.speaker.as_ref().is_some_and(|s| s.harness == "Custom ACP" && s.model.is_some()) && t.elapsed_ms.is_some())));
            }
            app.smoke_remote_tab_routing(cx);
            println!(
                "PASS: native mock A/B, budgets/permissions, context/review/checks, concurrent chat tabs, dynamic session titles, isolated transcripts and composer drafts, background/active close and reopen without cancelling agents, dispatch-time harness/model metadata and recorded duration. Mock ACP only; no real provider."
            );
            cx.quit();
            return true;
        }
        _ => {}
    }
    false
}
