//! Debounced project file watcher.
//!
//! The explorer, source control and the open editor should react when the
//! agent, a terminal or another app changes a file. FSEvents can emit bursts
//! of events for a single save, so the UI consumes a quiet signal instead of
//! refreshing on every event.

use std::path::{Component, Path};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use notify::{Config, RecommendedWatcher, RecursiveMode, Watcher};

const DEBOUNCE: Duration = Duration::from_millis(420);

pub(crate) struct ProjectWatcher {
    _watcher: RecommendedWatcher,
    events: Receiver<()>,
    dirty: bool,
    last_event: Instant,
}

impl ProjectWatcher {
    pub(crate) fn start(root: &Path) -> Option<Self> {
        if !root.is_dir() {
            return None;
        }
        let (sender, events) = std::sync::mpsc::channel();
        let watcher = RecommendedWatcher::new(
            move |result: Result<notify::Event, notify::Error>| {
                let Ok(event) = result else {
                    return;
                };
                if event.paths.iter().any(|path| !ignored(path)) {
                    let _ = sender.send(());
                }
            },
            Config::default(),
        )
        .map_err(|error| error.to_string())
        .ok()?;
        let mut watcher = watcher;
        if let Err(error) = watcher.watch(root, RecursiveMode::Recursive) {
            crate::logging::warn(&format!("file watcher: {error}"));
            return None;
        }
        Some(Self {
            _watcher: watcher,
            events,
            dirty: false,
            last_event: Instant::now(),
        })
    }

    /// Return true at most once per quiet burst.
    pub(crate) fn poll(&mut self) -> bool {
        loop {
            match self.events.try_recv() {
                Ok(()) => {
                    self.dirty = true;
                    self.last_event = Instant::now();
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        if self.dirty && self.last_event.elapsed() >= DEBOUNCE {
            self.dirty = false;
            true
        } else {
            false
        }
    }
}

fn ignored(path: &Path) -> bool {
    path.components().any(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        matches!(
            name.to_str(),
            Some(
                ".git"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".next"
                    | ".cache"
                    | ".turbo"
                    | ".DS_Store"
            )
        )
    })
}
