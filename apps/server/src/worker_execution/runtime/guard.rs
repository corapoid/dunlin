//! Deadline observation never acquires SQLite, filesystem or the lease mutex.
use super::*;
impl Runtime {
    pub(super) async fn validate_active(
        self: &Arc<Self>,
        active: &Arc<Active>,
    ) -> Result<(), Code> {
        self.current(active)?;
        let runtime = self.clone();
        let active = active.clone();
        let slots = self.fs.clone();
        tokio::time::timeout(Duration::from_secs(3), async move {
            let permit = slots.acquire_owned().await.map_err(|_| Code::Unavailable)?;
            tokio::task::spawn_blocking(move || {
                let _permit = permit;
                runtime.current(&active)?;
                active.filesystem.validate().map_err(code)?;
                runtime.current(&active)?;
                let store = runtime.store()?;
                let guard = || active.connection.authorize();
                let context = active
                    .connection
                    .context(&runtime.config.scope, &guard)
                    .map_err(code)?;
                let lease = active.lease.lock().map_err(|_| Code::Unavailable)?;
                store.validate_execution(&context, &lease).map_err(code)?;
                runtime.current(&active)
            })
            .await
            .map_err(|_| Code::Unavailable)?
        })
        .await
        .map_err(|_| Code::Unavailable)?
    }
    pub(super) fn start_guards(
        self: &Arc<Self>,
        active: &Arc<Active>,
    ) -> (tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>) {
        let runtime = self.clone();
        let clock = active.clone();
        let watchdog = tokio::spawn(async move {
            while !clock.finished.load(Ordering::Acquire) {
                if clock.group.guard_exited()
                    || runtime.closed.load(Ordering::Acquire)
                    || clock.stop.load(Ordering::Acquire)
                    || runtime.milliseconds(Instant::now())
                        >= clock.deadline_ms.load(Ordering::Acquire)
                {
                    clock.stop();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        });
        let runtime = self.clone();
        let checked = active.clone();
        let authority = tokio::spawn(async move {
            while !checked.finished.load(Ordering::Acquire) {
                if runtime.validate_active(&checked).await.is_err() {
                    checked.stop();
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        });
        (watchdog, authority)
    }
}
