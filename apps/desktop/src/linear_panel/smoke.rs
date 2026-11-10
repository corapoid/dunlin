//! Isolated native form/cache checks. No real credentials or MCP calls.
use super::*;
use crate::{app::SettingsTab, gui_test::Step, remote::RemoteHost};

pub(crate) fn reveal(id: &'static str) -> Step {
    Step::until(format!("scroll to {id}"), move |_, window, cx| {
        let Some(bounds) = crate::gui_probe::bounds(id) else {
            return false;
        };
        let height = f32::from(window.viewport_size().height);
        let top = f32::from(bounds.top());
        let bottom = f32::from(bounds.bottom());
        if top >= 80. && bottom < height - 50. {
            return true;
        }
        let x = (f32::from(bounds.left()) + 30.)
            .clamp(300., f32::from(window.viewport_size().width) - 50.);
        window.dispatch_event(
            PlatformInput::Scroll(ScrollEvent {
                position: point(px(x), px(height - 130.)),
                delta: ScrollDelta::Pixels(point(px(0.), px(if top < 80. { 200. } else { -200. }))),
                touch_phase: TouchPhase::Moved,
                ..Default::default()
            }),
            cx,
        );
        false
    })
}

pub(crate) fn steps() -> Vec<Step> {
    vec![
        Step::act(
            "fixture: server-owned Linear settings, no network",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    app.remote_hosts = vec![RemoteHost {
                        id: "linear-fixture".into(),
                        name: "Fixture server".into(),
                        url: "https://fixture.invalid".into(),
                        user_name: "Me".into(),
                        user_role: "member".into(),
                        device_id: "fixture-device".into(),
                    }];
                    app.connection_map
                        .hosts
                        .entry("linear-fixture".into())
                        .or_default()
                        .ticket = Some("no-fixture-network".into());
                    app.server_panel.host = Some("linear-fixture".into());
                    app.server_panel.loading = true;
                    app.server_panel.linear.fetched =
                        Some(Instant::now() + Duration::from_secs(86400));
                    app.server_panel.jobs = crate::jobs_panel::smoke::state();
                    app.server_panel.info = Some(dunlin_workspace::server::Server {
                        role: "member".into(),
                        isolated_agents: true,
                        name: "Fixture server".into(),
                        version: "fixture".into(),
                        os: "linux".into(),
                        arch: "aarch64".into(),
                        harnesses: vec![],
                        default_harness: String::new(),
                        account_scope: "Fixture account".into(),
                        sandbox_available: true,
                    });
                    app.settings_tab = SettingsTab::Remote;
                    app.settings_open = true;
                    cx.notify();
                });
            },
        ),
        reveal("linear-configure"),
        Step::click("linear-configure"),
        Step::act(
            "invalid label is checked before any token leaves the UI",
            |owner, window, cx| {
                owner.update(cx, |app, cx| {
                    let state = &mut app.server_panel.linear;
                    assert!(state.form_open);
                    state.token.as_ref().unwrap().update(cx, |input, cx| {
                        input.set_value("fixture-only-not-a-real-key", window, cx)
                    });
                    state
                        .label
                        .as_ref()
                        .unwrap()
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    cx.notify();
                });
            },
        ),
        reveal("linear-save"),
        Step::click("linear-save"),
        Step::until(
            "invalid form shows an error and never starts a request",
            |owner, _, cx| {
                let state = &owner.read(cx).server_panel.linear;
                assert!(state.request.is_none());
                state.error.is_some()
            },
        ),
        reveal("linear-configure"),
        Step::click("linear-configure"),
        Step::act(
            "cancel clears the token draft and cached tasks remain displayable",
            |owner, window, cx| {
                owner.update(cx, |app, cx| {
                    let state = &mut app.server_panel.linear;
                    assert!(!state.form_open);
                    assert!(state.token.as_ref().unwrap().read(cx).value().is_empty());
                    state.error = None;
                    state.inbox = Some(Inbox {
                        filter: Some(Filter {
                            label: "dunlin".into(),
                            team: None,
                        }),
                        issues: vec![dunlin_mcp_client::linear::Issue {
                            id: "fixture-issue".into(),
                            identifier: "APP-1".into(),
                            title: "Tagged task from the relay".into(),
                            description: "Requirements".into(),
                            url: "https://linear.app/team/issue/APP-1/task".into(),
                            status: "Todo".into(),
                            labels: vec!["dunlin".into()],
                        }],
                        synced_at: Some(chrono::Utc::now().timestamp()),
                        error: None,
                    });
                    // Ensure no secret survives reopening the form either.
                    app.edit_linear(window, cx);
                    assert!(
                        app.server_panel
                            .linear
                            .token
                            .as_ref()
                            .unwrap()
                            .read(cx)
                            .value()
                            .is_empty()
                    );
                    app.edit_linear(window, cx);
                    cx.notify();
                });
            },
        ),
        reveal("linear-issue:fixture-issue"),
        Step::act(
            "switching hosts drops cached issues and private form state",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    app.server_panel.linear = State {
                        fetched: Some(Instant::now() + Duration::from_secs(86400)),
                        ..State::default()
                    };
                    assert!(app.server_panel.linear.inbox.is_none());
                    assert!(app.server_panel.linear.token.is_none());
                    cx.notify();
                });
            },
        ),
    ]
    .into_iter()
    .chain(crate::jobs_panel::smoke::steps())
    .collect()
}
