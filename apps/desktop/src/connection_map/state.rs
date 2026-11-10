//! UI-independent presence projection and animation state. No network or GPUI.
use dunlin_domain::presence::{PRESENCE_TTL_SECS, Snapshot, User};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

pub(crate) const TRANSITION: Duration = Duration::from_millis(300);
pub(crate) const CARD_HEIGHT: f32 = 112.;
pub(crate) const ROW: f32 = 136.;
pub(crate) const VIEWPORT_HEIGHT: f32 = 420.;

#[derive(Default)]
pub(crate) struct State {
    pub hosts: HashMap<String, Host>,
    pub layout: Layout,
}

#[derive(Default)]
pub(crate) struct Host {
    pub snapshot: Option<Snapshot>,
    pub error: Option<String>,
    pub received: Option<Instant>,
    pub avatars: Vec<Avatar>,
    pub checked: Option<Instant>,
    pub ticket: Option<String>,
    pub connection: Tween,
}

pub(crate) struct Avatar {
    pub user: User,
    pub changed: Instant,
    pub leaving: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct Tween {
    from: f32,
    pub target: f32,
    changed: Instant,
}

impl Default for Tween {
    fn default() -> Self {
        Self::fixed(0., Instant::now())
    }
}

impl Tween {
    pub fn animating(self, now: Instant) -> bool {
        self.from != self.target && now.saturating_duration_since(self.changed) < TRANSITION
    }
    fn fixed(value: f32, now: Instant) -> Self {
        Self {
            from: value,
            target: value,
            changed: now,
        }
    }

    pub fn value(self, now: Instant, reduce_motion: bool) -> f32 {
        if reduce_motion {
            return self.target;
        }
        let progress = (now.saturating_duration_since(self.changed).as_secs_f32()
            / TRANSITION.as_secs_f32())
        .clamp(0., 1.);
        let eased = 1. - (1. - progress).powi(5);
        self.from + (self.target - self.from) * eased
    }

    fn set(&mut self, value: f32, now: Instant) {
        if self.target != value {
            self.from = self.value(now, false);
            self.target = value;
            self.changed = now;
        }
    }
}

#[derive(Default)]
pub(crate) struct Layout {
    ids: Vec<String>,
    pub hosts: HashMap<String, Tween>,
    pub source: Tween,
    pub height: f32,
}

impl Layout {
    pub fn reconcile(&mut self, ids: Vec<String>, now: Instant) -> bool {
        if self.ids == ids && self.height > 0. {
            return false;
        }
        let initial = self.height == 0.;
        self.height = (ids.len().max(1) as f32 * ROW + 48.).max(240.);
        // Keep the only user node visible on first open even with many hosts.
        let self_y = (self.height.min(VIEWPORT_HEIGHT) - CARD_HEIGHT) / 2.;
        if initial {
            self.source = Tween::fixed(self_y, now);
        } else {
            self.source.set(self_y, now);
        }
        self.hosts.retain(|id, _| ids.contains(id));
        for (index, id) in ids.iter().enumerate() {
            let target = 24. + index as f32 * ROW;
            self.hosts
                .entry(id.clone())
                .or_insert_with(|| Tween::fixed(target + if initial { 0. } else { 12. }, now))
                .set(target, now);
        }
        self.ids = ids;
        true
    }
}

impl State {
    pub fn animating(&self, now: Instant) -> bool {
        self.layout.source.animating(now)
            || self
                .layout
                .hosts
                .values()
                .any(|position| position.animating(now))
            || self.hosts.values().any(|host| {
                host.connection.animating(now)
                    || host
                        .avatars
                        .iter()
                        .any(|avatar| now.saturating_duration_since(avatar.changed) < TRANSITION)
            })
    }
}

impl Host {
    pub fn connected(&self, now: Instant) -> bool {
        self.error.is_none()
            && self.snapshot.is_some()
            && self.received.is_some_and(|received| {
                now.saturating_duration_since(received) < Duration::from_secs(PRESENCE_TTL_SECS)
            })
    }

    pub fn apply(&mut self, result: Result<Snapshot, String>, now: Instant) -> bool {
        self.ticket = None;
        let changed;
        match result {
            Ok(snapshot) => {
                changed = self.snapshot.as_ref() != Some(&snapshot) || self.error.is_some();
                self.error = None;
                self.received = Some(now);
                let peers: Vec<_> = snapshot
                    .users
                    .iter()
                    .filter(|user| user.id != snapshot.self_user_id)
                    .cloned()
                    .collect();
                for avatar in &mut self.avatars {
                    if let Some(peer) = peers.iter().find(|peer| peer.id == avatar.user.id) {
                        avatar.user = peer.clone();
                        if avatar.leaving {
                            avatar.changed = now;
                            avatar.leaving = false;
                        }
                    } else if !avatar.leaving {
                        avatar.changed = now;
                        avatar.leaving = true;
                    }
                }
                for peer in peers {
                    if !self.avatars.iter().any(|avatar| avatar.user.id == peer.id) {
                        self.avatars.push(Avatar {
                            user: peer,
                            changed: now,
                            leaving: false,
                        });
                    }
                }
                self.snapshot = Some(snapshot);
                self.connection.set(1., now);
            }
            Err(error) => {
                changed = self.error.as_ref() != Some(&error);
                self.error = Some(error);
                // Lost observation is not evidence that all peers disconnected.
                self.avatars.clear();
                self.connection.set(0., now);
            }
        }
        changed
    }

    pub fn maintain(&mut self, now: Instant) -> bool {
        let before = self.avatars.len();
        self.avatars.retain(|avatar| {
            !avatar.leaving || now.saturating_duration_since(avatar.changed) < TRANSITION
        });
        let mut changed = before != self.avatars.len();
        if self.error.is_none() && self.received.is_some() && !self.connected(now) {
            self.error = Some("Connection unknown · reconnecting…".into());
            self.avatars.clear();
            self.connection.set(0., now);
            changed = true;
        }
        changed
    }
}

pub(crate) fn initials(name: &str) -> String {
    let value: String = name
        .split_whitespace()
        .take(2)
        .filter_map(|part| part.chars().next())
        .flat_map(char::to_uppercase)
        .take(2)
        .collect();
    if value.is_empty() { "?".into() } else { value }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn only_peers_become_avatars_and_unchanged_heartbeats_do_not_restart_animations() {
        let mut host = Host::default();
        let now = Instant::now();
        assert!(host.apply(Ok(snapshot(&[("anna", "Anna")])), now));
        assert_eq!(host.avatars.len(), 1);
        assert_eq!(host.avatars[0].user.id, "anna");
        assert!(!host.apply(
            Ok(snapshot(&[("anna", "Anna")])),
            now + Duration::from_secs(5)
        ));
        assert_eq!(host.avatars[0].changed, now);
        assert!(host.connected(now + Duration::from_secs(6)));
        assert!(!host.connected(now + Duration::from_secs(35)));
    }

    #[test]
    fn departures_animate_but_lost_observation_is_not_a_peer_disconnect() {
        let mut host = Host::default();
        let now = Instant::now();
        host.apply(Ok(snapshot(&[("anna", "Anna")])), now);
        host.apply(Ok(snapshot(&[])), now + Duration::from_secs(5));
        assert!(host.avatars[0].leaving);
        host.apply(
            Ok(snapshot(&[("anna", "Anna")])),
            now + Duration::from_secs(6),
        );
        assert!(!host.avatars[0].leaving);
        host.apply(Err("unknown".into()), now + Duration::from_secs(7));
        assert!(!host.connected(now + Duration::from_secs(7)));
        assert!(host.avatars.is_empty());
    }

    #[test]
    fn maintenance_removes_departed_avatars_and_invalidates_stale_observation() {
        let now = Instant::now();
        let mut host = Host::default();
        host.apply(Ok(snapshot(&[("anna", "Anna")])), now);
        host.apply(Ok(snapshot(&[])), now + Duration::from_secs(1));
        assert!(!host.maintain(now + Duration::from_millis(1100)));
        assert!(host.maintain(now + Duration::from_millis(1300)));
        assert!(host.avatars.is_empty());
        assert!(host.maintain(now + Duration::from_secs(31)));
        assert!(host.error.is_some());
        assert_eq!(host.connection.target, 0.);
    }

    #[test]
    fn initials_handle_unicode_and_missing_names() {
        assert_eq!(initials("Łukasz Żółć"), "ŁŻ");
        assert_eq!(initials("Anna"), "A");
        assert_eq!(initials("  "), "?");
    }

    #[test]
    fn layout_preserves_host_order_and_animates_reflow_without_extra_user_nodes() {
        let now = Instant::now();
        let mut layout = Layout::default();
        assert!(layout.reconcile(vec!["a".into(), "b".into()], now));
        assert_eq!(layout.hosts.len(), 2);
        assert!(!layout.reconcile(vec!["a".into(), "b".into()], now));
        assert_eq!(layout.hosts["b"].value(now, false), 160.);
        assert!(layout.reconcile(vec!["b".into()], now));
        assert_eq!(layout.hosts["b"].value(now, false), 160.);
        assert_eq!(layout.hosts["b"].value(now, true), 24.);
        assert_eq!(layout.hosts["b"].value(now + TRANSITION, false), 24.);
        assert!(!layout.hosts.contains_key("a"));
    }

    #[test]
    fn a_mid_animation_reversal_starts_at_the_current_position() {
        let now = Instant::now();
        let mut tween = Tween::fixed(0., now);
        tween.set(1., now);
        let halfway = now + TRANSITION / 2;
        let current = tween.value(halfway, false);
        assert!(current > 0. && current < 1.);
        tween.set(0., halfway);
        assert_eq!(tween.value(halfway, false), current);
        assert_eq!(tween.value(halfway + TRANSITION, false), 0.);
    }

    #[test]
    fn many_hosts_do_not_place_the_only_user_below_the_initial_viewport() {
        let now = Instant::now();
        let mut layout = Layout::default();
        layout.reconcile((0..20).map(|id| id.to_string()).collect(), now);
        assert!(layout.source.target >= 0.);
        assert!(layout.source.target + CARD_HEIGHT <= VIEWPORT_HEIGHT);
        assert_eq!(layout.hosts.len(), 20);
    }

    #[test]
    fn users_with_the_same_name_are_not_merged_and_device_updates_are_not_arrivals() {
        let now = Instant::now();
        let mut host = Host::default();
        host.apply(Ok(snapshot(&[("anna-1", "Anna"), ("anna-2", "Anna")])), now);
        let mut next = snapshot(&[("anna-1", "Anna"), ("anna-2", "Anna")]);
        next.users[1].devices = 2;
        host.apply(Ok(next), now + Duration::from_secs(5));
        assert_eq!(host.avatars.len(), 2);
        assert_eq!(host.avatars[0].user.devices, 2);
        assert_eq!(host.avatars[0].changed, now);
    }
}
