//! A bounded, coalescing upload queue. A single worker preserves snapshot order;
//! relay revisions also reject late writes from other clients or legacy uploaders.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex, OnceLock},
};

type Key = (String, String);
const MAX_BYTES: usize = 16 * 1024 * 1024;
const MAX_JOBS: usize = 64;

struct Snapshot {
    key: Key,
    token: String,
    payload: serde_json::Value,
    bytes: usize,
}

#[derive(Default)]
struct Queue {
    pending: VecDeque<Snapshot>,
    bytes: usize,
}

impl Queue {
    fn push(&mut self, snapshot: Snapshot) -> Result<(), String> {
        let old = self.pending.iter().position(|job| job.key == snapshot.key);
        let old_bytes = old.map_or(0, |index| self.pending[index].bytes);
        if self.bytes - old_bytes + snapshot.bytes > MAX_BYTES
            || (old.is_none() && self.pending.len() >= MAX_JOBS)
        {
            return Err(
                "Transcript upload queue is full; the local conversation is preserved".into(),
            );
        }
        if let Some(index) = old {
            self.pending.remove(index);
        }
        self.bytes = self.bytes - old_bytes + snapshot.bytes;
        self.pending.push_back(snapshot);
        Ok(())
    }

    fn pop(&mut self) -> Option<Snapshot> {
        let snapshot = self.pending.pop_front()?;
        self.bytes -= snapshot.bytes;
        Some(snapshot)
    }
}

type Shared = Arc<(Mutex<Queue>, Condvar)>;

pub(crate) fn enqueue(
    url: String,
    token: String,
    session_id: String,
    payload: serde_json::Value,
) -> Result<(), String> {
    static UPLOADER: OnceLock<Result<Shared, String>> = OnceLock::new();
    let shared = UPLOADER
        .get_or_init(|| {
            let shared: Shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
            let worker = shared.clone();
            std::thread::Builder::new()
                .name("dunlin-transcripts".into())
                .spawn(move || run(worker))
                .map_err(|error| format!("Could not start transcript uploader: {error}"))?;
            Ok(shared)
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let bytes = serde_json::to_vec(&payload)
        .map_err(|error| error.to_string())?
        .len();
    if bytes > 4 * 1024 * 1024 {
        return Err(
            "Transcript exceeds the relay's 4 MiB limit; the local conversation is preserved"
                .into(),
        );
    }
    let (queue, ready) = &**shared;
    queue
        .lock()
        .map_err(|_| "Transcript upload queue is unavailable")?
        .push(Snapshot {
            key: (url, session_id),
            token,
            payload,
            bytes,
        })?;
    ready.notify_one();
    Ok(())
}

fn run(shared: Shared) {
    let mut revisions = HashMap::<Key, i64>::new();
    let (queue, ready) = &*shared;
    loop {
        let snapshot = {
            let Ok(mut queue) = queue.lock() else {
                return;
            };
            while queue.pending.is_empty() {
                let Ok(next) = ready.wait(queue) else {
                    return;
                };
                queue = next;
            }
            queue.pop().unwrap()
        };
        let result = (|| {
            let expected = match revisions.get(&snapshot.key) {
                Some(revision) => *revision,
                None => {
                    let revision = super::transcript_revision(
                        &snapshot.key.0,
                        &snapshot.token,
                        &snapshot.key.1,
                    )?;
                    revisions.insert(snapshot.key.clone(), revision);
                    revision
                }
            };
            super::save_transcript(
                &snapshot.key.0,
                &snapshot.token,
                &snapshot.key.1,
                &snapshot.payload,
                expected,
            )?;
            revisions.insert(snapshot.key, expected + 1);
            Ok::<(), String>(())
        })();
        if let Err(error) = result {
            // Never silently rebase a rejected snapshot onto someone else's revision.
            crate::logging::warn(&format!(
                "remote transcript: {error}; local history is preserved"
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(session: &str, value: usize) -> Snapshot {
        Snapshot {
            key: ("relay".into(), session.into()),
            token: "test".into(),
            payload: serde_json::json!(value),
            bytes: 1,
        }
    }

    #[test]
    fn pending_snapshots_coalesce_and_cannot_overtake_an_in_flight_upload() {
        let mut queue = Queue::default();
        queue.push(snapshot("a", 1)).unwrap();
        let in_flight = queue.pop().unwrap();
        queue.push(snapshot("a", 2)).unwrap();
        queue.push(snapshot("b", 1)).unwrap();
        queue.push(snapshot("a", 3)).unwrap();
        assert_eq!(in_flight.payload, 1);
        assert_eq!(queue.pop().unwrap().key.1, "b");
        assert_eq!(queue.pop().unwrap().payload, 3);
        assert_eq!(queue.bytes, 0);
    }

    #[test]
    fn full_queue_keeps_existing_snapshots() {
        let mut queue = Queue::default();
        for index in 0..MAX_JOBS {
            queue.push(snapshot(&index.to_string(), 1)).unwrap();
        }
        assert!(queue.push(snapshot("overflow", 1)).is_err());
        assert_eq!(queue.pending.len(), MAX_JOBS);
        queue.push(snapshot("0", 2)).unwrap();
        assert_eq!(queue.pending.back().unwrap().payload, 2);
    }
}
