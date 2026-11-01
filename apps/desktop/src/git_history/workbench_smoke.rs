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
            println!(
                "PASS: native mock A/B ACP connections, distinct observed models, one identical prompt per side, shared durable cap, final-request tool approval, parked pending-permission cancellation, restart non-resumption, no duplicate dispatch, Rust impact/test rendering and budget rendering. Dialog acceptance simulated; no real provider."
            );
            cx.quit();
            return true;
        }
        _ => {}
    }
    false
}
