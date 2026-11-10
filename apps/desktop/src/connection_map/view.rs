//! A two-column graph: exactly one local identity, and host-local peer avatars.
use super::{
    Avatar, TRANSITION, initials,
    state::{CARD_HEIGHT, VIEWPORT_HEIGHT},
};
use crate::app::DunlinApp;
use dunlin_ui::{
    assets::IconName,
    component::{Icon, button::ButtonVariants, tooltip::Tooltip},
    prelude::FluentBuilder as _,
    *,
};
use std::time::Instant;

const LEFT: f32 = 24.;
const USER_WIDTH: f32 = 176.;
const HOST_LEFT: f32 = 344.;
const HOST_WIDTH: f32 = 272.;
const WIDTH: f32 = HOST_LEFT + HOST_WIDTH + 24.;

pub(crate) fn render(
    this: &DunlinApp,
    window: &mut Window,
    cx: &mut Context<DunlinApp>,
) -> AnyElement {
    let p = palette(cx);
    let now = Instant::now();
    let reduce_motion = this.reduce_motion || cx.reduce_motion();
    let height = this.connection_map.layout.height.max(240.);
    let self_y = this.connection_map.layout.source.value(now, reduce_motion);
    if !reduce_motion && this.connection_map.animating(now) {
        let app = cx.entity().downgrade();
        window.on_next_frame(move |_, cx| {
            let _ = app.update(cx, |_, cx| cx.notify());
        });
        window.request_animation_frame();
    }
    let connected = this
        .remote_hosts
        .iter()
        .filter(|host| {
            this.connection_map
                .hosts
                .get(&host.id)
                .is_some_and(|host| host.connected(now))
        })
        .count();

    let edges: Vec<_> = this
        .remote_hosts
        .iter()
        .filter_map(|host| {
            let state = this.connection_map.hosts.get(&host.id)?;
            let alpha = state.connection.value(now, reduce_motion);
            if alpha <= 0. {
                return None;
            }
            let position = this.connection_map.layout.hosts.get(&host.id)?;
            Some((position.value(now, reduce_motion) + CARD_HEIGHT / 2., alpha))
        })
        .collect();
    let edge_color = p.blue.opacity(0.65);
    let edge_layer = canvas(
        |bounds, _, _| bounds,
        move |bounds, _, window, _| {
            let start = bounds.origin + point(px(LEFT + USER_WIDTH), px(self_y + CARD_HEIGHT / 2.));
            for (y, alpha) in &edges {
                let end = bounds.origin + point(px(HOST_LEFT), px(*y));
                let mid = (start.x + end.x) / 2.;
                let mut path = PathBuilder::stroke(px(1.5));
                path.move_to(start);
                path.cubic_bezier_to(end, point(mid, start.y), point(mid, end.y));
                if let Ok(path) = path.build() {
                    window.paint_path(path, edge_color.opacity(*alpha));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .w(px(WIDTH))
    .h(px(height));

    let profile = this.user_profile.profiles.first();
    let name = profile
        .map(|(profile, _)| profile.name.clone())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| "You".into());
    let face = div()
        .size(px(36.))
        .rounded_full()
        .bg(p.blue_soft)
        .text_color(p.blue)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(13.))
        .font_weight(FontWeight::SEMIBOLD)
        .map(|face| match profile.and_then(|(_, image)| image.clone()) {
            Some(image) => face.child(img(image).size(px(36.)).rounded_full()),
            None => face.child(initials(&name)),
        });
    let me = SurfaceGeometry::GRAPH_NODE
        .paint(p.panel, p.blue.opacity(0.5))
        .id("connection-map-self")
        .children(crate::gui_probe::marker("connection-map-self"))
        .absolute()
        .left(px(LEFT))
        .top(px(self_y))
        .w(px(USER_WIDTH))
        .h(px(CARD_HEIGHT))
        .p(px(14.))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(10.))
        .child(
            div().flex().items_center().gap(px(10.)).child(face).child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .truncate()
                            .child(name),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child("You · this app"),
                    ),
            ),
        )
        .child(
            div()
                .text_size(px(10.5))
                .text_color(p.muted_fg)
                .child(format!("{connected} connected servers")),
        )
        .role(Role::Group)
        .aria_label("Your identity. Only your server connections are drawn.");

    let mut scene = div()
        .relative()
        .w(px(WIDTH))
        .h(px(height))
        .flex_shrink_0()
        .child(edge_layer)
        .child(me);
    for host in &this.remote_hosts {
        let state = this.connection_map.hosts.get(&host.id);
        let online = state.is_some_and(|state| state.connected(now));
        let (status, color) = if online {
            ("Connected", p.green)
        } else if state.is_some_and(|state| state.error.is_some() || state.received.is_some()) {
            ("Unknown", p.amber)
        } else {
            ("Connecting…", p.muted_fg)
        };
        let workload=state.and_then(|state|state.snapshot.as_ref()).and_then(|snapshot|snapshot.workload.as_ref());
        let workload_text=workload.filter(|_|online).map(|workload|format!("{} running · {} queued{}{}",workload.active,workload.queued,if workload.paused{" · paused"}else{""},if workload.interrupted>0{" · interrupted"}else{""}));
        let detail = state
            .and_then(|state| state.error.as_deref())
            .unwrap_or(if online {
                "Click to manage server"
            } else {
                "Checking relay presence…"
            });
        let mut peers = div().flex().items_center().gap(px(4.)).min_h(px(26.));
        if online {
            let avatars = state
                .map(|state| state.avatars.as_slice())
                .unwrap_or_default();
            let active = avatars.iter().filter(|avatar| !avatar.leaving).count();
            for avatar in avatars
                .iter()
                .filter(|avatar| !avatar.leaving)
                .chain(avatars.iter().filter(|avatar| avatar.leaving))
                .take(5)
            {
                peers = peers.child(peer_avatar(&host.id, avatar, now, reduce_motion, &p));
            }
            if active > 5 {
                let extra = avatars
                    .iter()
                    .filter(|avatar| !avatar.leaving)
                    .skip(5)
                    .map(|avatar| {
                        format!("{} · {} device(s)", avatar.user.name, avatar.user.devices)
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                peers = peers.child(
                    div()
                        .id(SharedString::from(format!(
                            "connection-peer-overflow:{}",
                            host.id
                        )))
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .tooltip(move |window, cx| Tooltip::new(extra.clone()).build(window, cx))
                        .child(format!("+{}", active - 5)),
                );
            }
            if avatars.is_empty() {
                peers = peers.child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child("No other users online"),
                );
            }
        } else {
            peers = peers.child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child("Other users: unknown"),
            );
        }
        let id = host.id.clone();
        let keyboard_id = id.clone();
        let label = format!("Server {}. {}. {}", host.name, status, detail);
        let card = SurfaceGeometry::GRAPH_NODE
            .paint(
                p.panel,
                if online {
                    p.blue.opacity(0.4)
                } else {
                    p.border
                },
            )
            .id(SharedString::from(format!("connection-host:{}", host.id)))
            .children(crate::gui_probe::marker_named(format!(
                "connection-host:{}",
                host.id
            )))
            .absolute()
            .left(px(HOST_LEFT))
            .top(px(this
                .connection_map
                .layout
                .hosts
                .get(&host.id)
                .map(|position| position.value(now, reduce_motion))
                .unwrap_or(24.)))
            .w(px(HOST_WIDTH))
            .h(px(CARD_HEIGHT))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(7.))
            .cursor_pointer()
            .hover(move |card| card.border_color(p.blue))
            .role(Role::Button)
            .tab_index(0)
            .aria_label(label)
            .on_click(cx.listener(move |this, _, window, cx| this.manage_server(&id, window, cx)))
            .on_key_down(cx.listener(move |this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    this.manage_server(&keyboard_id, window, cx);
                    cx.stop_propagation();
                }
            }))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .child(Icon::new(IconName::Cloud).size(px(15.)).text_color(p.blue))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(px(12.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(p.fg)
                            .child(host.name.clone()),
                    )
                    .child(div().size(px(5.)).rounded_full().bg(color))
                    .child(div().text_size(px(10.)).text_color(color).child(status)),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .truncate()
                    .child(workload_text.unwrap_or_else(||detail.to_string())),
            )
            .child(peers);
        scene = scene.child(card);
    }
    if this.remote_hosts.is_empty() {
        scene = scene.child(
            div()
                .absolute()
                .left(px(HOST_LEFT))
                .top(px(self_y))
                .w(px(HOST_WIDTH))
                .h(px(CARD_HEIGHT))
                .flex()
                .flex_col()
                .justify_center()
                .gap(px(6.))
                .text_color(p.muted_fg)
                .child(div().text_size(px(12.)).child("No servers paired"))
                .child(
                    div()
                        .text_size(px(11.))
                        .child("Connect a server to see it here."),
                ),
        );
    }

    div().flex().flex_col().gap(px(10.)).min_w(px(0.))
        .child(div().flex().items_center().gap(px(8.))
            .child(Icon::new(IconName::Network).size(px(15.)).text_color(p.blue))
            .child(div().flex_1().text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(p.fg).child("Connection map"))
            .child(dunlin_ui::button("connection-map-refresh").ghost().label("Refresh")
                .on_click(cx.listener(|this, _, _, cx| {
                    for state in this.connection_map.hosts.values_mut() { state.checked = None; }
                    this.poll_host_presence(cx);
                    cx.notify();
                }))))
        .child(div().relative().h(px(height.min(VIEWPORT_HEIGHT))).w_full().min_w(px(0.)).overflow_hidden()
            .rounded(px(dunlin_ui::radii::GRAPH_NODE)).border_1().border_color(p.border).bg(p.bg)
            .child(dot_grid(p.graph_grid))
            .child(div().id("connection-map-scroll").relative().size_full().overflow_scroll()
                .role(Role::Group).aria_label("Connection map. You on the left, servers on the right. Other users appear only as avatars on servers.")
                .child(scene)))
        .child(div().text_size(px(10.5)).text_color(p.muted_fg)
            .child("Only your connections are shown. Avatars indicate other users currently connected to that server."))
        .into_any_element()
}

fn peer_avatar(
    host: &str,
    avatar: &Avatar,
    now: Instant,
    reduce_motion: bool,
    p: &Palette,
) -> AnyElement {
    let tooltip = format!(
        "{} · {} connected device{}",
        avatar.user.name,
        avatar.user.devices,
        if avatar.user.devices == 1 { "" } else { "s" }
    );
    let progress = if reduce_motion {
        1.
    } else {
        (now.saturating_duration_since(avatar.changed).as_secs_f32() / TRANSITION.as_secs_f32())
            .clamp(0., 1.)
    };
    let eased = 1. - (1. - progress).powi(5);
    let face = div()
        .id(SharedString::from(format!(
            "connection-peer:{host}:{}",
            avatar.user.id
        )))
        .relative()
        .children(crate::gui_probe::marker_named(format!(
            "connection-peer:{host}:{}",
            avatar.user.id
        )))
        .size(px(26.))
        .rounded_full()
        .bg(p.violet_soft)
        .border_1()
        .border_color(p.panel)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(p.violet)
        .opacity(if avatar.leaving { 1. - eased } else { eased })
        .role(Role::Image)
        .aria_label(tooltip.clone())
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(initials(&avatar.user.name));
    face.into_any_element()
}
