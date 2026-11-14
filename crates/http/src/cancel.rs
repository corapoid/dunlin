//! Terminal cancellation, with a watchdog retained until the I/O owner finishes.
use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
struct State {
    done: Mutex<bool>,
    wake: Condvar,
    denied: AtomicBool,
    authorize: Box<dyn Fn() -> bool + Send + Sync>,
    cancel: Box<dyn Fn() + Send + Sync>,
}
impl State {
    fn deny(&self) {
        if !self.denied.swap(true, Ordering::AcqRel) {
            let _ = catch_unwind(AssertUnwindSafe(|| (self.cancel)()));
        }
    }
    fn check(&self) -> bool {
        if self.denied.load(Ordering::Acquire) {
            return false;
        }
        if !catch_unwind(AssertUnwindSafe(|| (self.authorize)())).unwrap_or(false) {
            self.deny();
        }
        !self.denied.load(Ordering::Acquire)
    }
}
/// Calls `cancel` once when live authority is lost. The owner must retain this
/// guard through response-body consumption and inspect `finish` before success.
/// Dropping it wakes and joins the watchdog, including during unwinding.
pub struct Cancellation {
    state: Arc<State>,
    worker: Option<JoinHandle<()>>,
}
impl Cancellation {
    pub fn new(
        authorize: impl Fn() -> bool + Send + Sync + 'static,
        cancel: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let state = Arc::new(State {
            done: Mutex::new(false),
            wake: Condvar::new(),
            denied: AtomicBool::new(false),
            authorize: Box::new(authorize),
            cancel: Box::new(cancel),
        });
        if !state.check() {
            return Err("HTTP authority is no longer available".into());
        }
        let checked = state.clone();
        let worker = thread::Builder::new()
            .name("dunlin-http-guard".into())
            .spawn(move || {
                loop {
                    if !checked.check() {
                        break;
                    }
                    let Ok(done) = checked.done.lock() else {
                        checked.deny();
                        break;
                    };
                    if *done {
                        break;
                    }
                    let Ok((done, _)) = checked.wake.wait_timeout(done, Duration::from_millis(25))
                    else {
                        checked.deny();
                        break;
                    };
                    if *done {
                        break;
                    }
                }
            })
            .map_err(|_| "HTTP guard capacity is unavailable")?;
        Ok(Self {
            state,
            worker: Some(worker),
        })
    }
    fn stop(&mut self) {
        if let Ok(mut done) = self.state.done.lock() {
            *done = true;
            self.state.wake.notify_all();
        }
        if let Some(worker) = self.worker.take()
            && worker.join().is_err()
        {
            self.state.deny();
        }
    }
    pub fn finish(mut self) -> bool {
        self.stop();
        self.state.check()
    }
}
impl Drop for Cancellation {
    fn drop(&mut self) {
        self.stop();
    }
}
#[cfg(test)]
mod tests;
