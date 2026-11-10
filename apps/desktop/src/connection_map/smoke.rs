//! Disposable native-window checks, using relay-shaped snapshots. No network,
//! credential writes, agents or real user data.
use crate::{app::SettingsTab, gui_test::Step, remote::RemoteHost};
use dunlin_domain::presence::{PRESENCE_TTL_SECS, Snapshot, User};
use std::time::{Duration, Instant};

fn snapshot(peers: &[(&str, &str)]) -> Snapshot {
    Snapshot {
        workload: None,
        self_user_id: "me".into(),
        ttl_secs: PRESENCE_TTL_SECS,
        users: std::iter::once(User {
            id: "me".into(),
            name: "Me".into(),
            devices: 1,
        })
        .chain(peers.iter().map(|(id, name)| User {
            id: id.to_string(),
            name: name.to_string(),
            devices: 1,
        }))
        .collect(),
    }
}

pub(crate) fn steps() -> Vec<Step> {
    vec![
        Step::act(
            "fixture: two relays and three other users",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    app.remote_hosts = [("build", "Build server"), ("dev", "Dev server")]
                        .into_iter()
                        .map(|(id, name)| RemoteHost {
                            id: id.into(),
                            name: name.into(),
                            url: "https://fixture.invalid".into(),
                            user_name: "Me".into(),
                            user_role: "member".into(),
                            device_id: format!("fixture-{id}"),
                        })
                        .collect();
                    let now = Instant::now();
                    app.connection_map
                        .layout
                        .reconcile(vec!["build".into(), "dev".into()], now);
                    for (id, peers) in [
                        ("build", vec![("anna", "Anna"), ("jan", "Jan")]),
                        ("dev", vec![("piotr", "Piotr")]),
                    ] {
                        let host = app.connection_map.hosts.entry(id.into()).or_default();
                        host.apply(Ok(snapshot(&peers)), now);
                        // Suppress the transport for synthetic hosts for the
                        // lifetime of this fixture, not just its first frame.
                        host.ticket = Some("fixture-no-network".into());
                    }
                    app.settings_tab = SettingsTab::Remote;
                    app.settings_open = true;
                    cx.notify();
                });
            },
        ),
        Step::until(
            "one user on the left, host cards and peer avatars on the right",
            |owner, _, cx| {
                let Some(me) = crate::gui_probe::bounds("connection-map-self") else {
                    return false;
                };
                let Some(build) = crate::gui_probe::bounds("connection-host:build") else {
                    return false;
                };
                let Some(dev) = crate::gui_probe::bounds("connection-host:dev") else {
                    return false;
                };
                let Some(anna) = crate::gui_probe::bounds("connection-peer:build:anna") else {
                    return false;
                };
                assert!(me.right() < build.left());
                assert!(me.right() < dev.left());
                assert!(anna.left() >= build.left() && anna.right() <= build.right());
                assert!(anna.top() >= build.top() && anna.bottom() <= build.bottom());
                assert!(crate::gui_probe::bounds("connection-peer:build:me").is_none());
                assert_eq!(owner.read(cx).connection_map.layout.hosts.len(), 2);
                true
            },
        ),
        Step::act(
            "Anna disconnects; animate only the avatar on that server",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    let host = app.connection_map.hosts.get_mut("build").unwrap();
                    host.apply(Ok(snapshot(&[("jan", "Jan")])), Instant::now());
                    host.ticket = Some("fixture-no-network".into());
                    assert!(
                        host.avatars
                            .iter()
                            .find(|avatar| avatar.user.id == "anna")
                            .unwrap()
                            .leaving
                    );
                    cx.notify();
                });
            },
        ),
        Step::until(
            "departure animation completes without removing the host or other users",
            |owner, _, cx| {
                let app = owner.read(cx);
                let host = &app.connection_map.hosts["build"];
                if host.avatars.iter().any(|avatar| avatar.user.id == "anna") {
                    return false;
                }
                assert!(crate::gui_probe::bounds("connection-peer:build:anna").is_none());
                assert!(crate::gui_probe::bounds("connection-peer:build:jan").is_some());
                assert!(crate::gui_probe::bounds("connection-peer:dev:piotr").is_some());
                assert!(host.connected(Instant::now()));
                true
            },
        ),
        Step::act(
            "a lost relay connection hides stale avatars but leaves the other host intact",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    let host = app.connection_map.hosts.get_mut("build").unwrap();
                    host.apply(
                        Err("Connection unknown · reconnecting…".into()),
                        Instant::now(),
                    );
                    host.ticket = Some("fixture-no-network".into());
                    assert!(host.avatars.is_empty());
                    assert!(!host.connected(Instant::now()));
                    assert!(app.connection_map.hosts["dev"].connected(Instant::now()));
                    cx.notify();
                });
            },
        ),
        Step::until(
            "unknown is not presented as every user going offline",
            |_, _, _| {
                crate::gui_probe::bounds("connection-peer:build:jan").is_none()
                    && crate::gui_probe::bounds("connection-peer:dev:piotr").is_some()
            },
        ),
        Step::act(
            "reduce motion uses final geometry and stale presence expires",
            |owner, _, cx| {
                owner.update(cx, |app, cx| {
                    app.reduce_motion = true;
                    let now = Instant::now();
                    let host = app.connection_map.hosts.get_mut("dev").unwrap();
                    host.received = Some(now - Duration::from_secs(PRESENCE_TTL_SECS));
                    assert!(host.maintain(now));
                    assert_eq!(host.connection.value(now, true), 0.);
                    assert!(host.avatars.is_empty());
                    cx.notify();
                });
            },
        ),
    ]
}
