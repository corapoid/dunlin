//! Native queue/assignment acceptance with a disconnected fixture: no paid turn.
use super::*;
use crate::{gui_test::Step, linear_panel::smoke::reveal};

pub(crate) fn state() -> State {
    State {
        queue: Some(Queue::default()),
        fetched: Some(Instant::now() + Duration::from_secs(86400)),
        ..Default::default()
    }
}

pub(crate) fn steps() -> Vec<Step> {
    vec![
        Step::act("prepare tagged issue assignment", |owner, window, cx| {
            owner.update(cx, |app, cx| {
                app.assign_linear_job(
                    Issue {
                        id: "fixture-job".into(),
                        identifier: "APP-2".into(),
                        title: "Unattended execution fixture".into(),
                        description: "No network or paid turn may start in this test".into(),
                        url: "https://linear.app/team/issue/APP-2/task".into(),
                        status: "Todo".into(),
                        labels: vec!["dunlin".into()],
                    },
                    window,
                    cx,
                );
                let draft = app.server_panel.jobs.draft.as_mut().unwrap();
                draft.project = Some("prj-fixture".into());
                draft.loading = None;
                assert!(!draft.approval);
                assert!(app.server_panel.jobs.submission.is_none());
                cx.notify();
            });
        }),
        reveal("job-approve"),
        Step::click("job-approve"),
        Step::until(
            "empty exact model prevents approval and paid dispatch",
            |owner, _, cx| {
                let state = &owner.read(cx).server_panel.jobs;
                assert!(state.submission.is_none());
                assert!(!state.draft.as_ref().unwrap().approval);
                state.error.is_some()
            },
        ),
        reveal("job-close"),
        Step::click("job-close"),
        Step::until(
            "assignment cancellation leaves no runnable job",
            |owner, _, cx| {
                let state = &owner.read(cx).server_panel.jobs;
                assert!(state.submission.is_none());
                assert!(state.queue.as_ref().unwrap().jobs.is_empty());
                state.draft.is_none()
            },
        ),
    ]
}
