//! Bounded, memory-only cache. Each instance belongs to exactly one immutable
//! endpoint/bearer/input-capability context, including results marked public.
use super::Error;
use serde_json::Value;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 64;
const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_KEY: usize = 16 * 1024;
const MAX_TTL_MS: u64 = 300_000;

#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) struct Key {
    method: String,
    params: String,
}
impl Key {
    pub(super) fn new(method: &str, params: &Value) -> Result<Option<Self>, Error> {
        if !matches!(
            method,
            "server/discover"
                | "tools/list"
                | "prompts/list"
                | "resources/list"
                | "resources/templates/list"
                | "resources/read"
        ) || params.get("requestState").is_some()
            || params.get("inputResponses").is_some()
        {
            return Ok(None);
        }
        let params = serde_json::to_string(params).map_err(|_| Error::Protocol)?;
        if params.len() > MAX_KEY {
            return Ok(None);
        }
        Ok(Some(Self {
            method: method.into(),
            params,
        }))
    }
    fn bytes(&self) -> usize {
        self.method.len() + self.params.len()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Scope {
    Public,
    Private,
}
pub(super) fn scope(result: &Value) -> Result<Option<Scope>, Error> {
    match result.get("cacheScope") {
        None => Ok(None),
        Some(Value::String(value)) if value == "public" => Ok(Some(Scope::Public)),
        Some(Value::String(value)) if value == "private" => Ok(Some(Scope::Private)),
        _ => Err(Error::Protocol),
    }
}

struct Entry {
    value: Value,
    received: Instant,
    ttl: Duration,
    epoch: [u64; 3],
    bytes: usize,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: HashMap<Key, Entry>,
    bytes: usize,
}
impl Cache {
    pub(super) fn get(&mut self, key: &Key, epoch: [u64; 3]) -> Option<Value> {
        let fresh = self
            .entries
            .get(key)
            .is_some_and(|entry| entry.epoch == epoch && entry.received.elapsed() < entry.ttl);
        if !fresh {
            self.remove(key);
            return None;
        }
        self.entries.get(key).map(|entry| entry.value.clone())
    }
    pub(super) fn put(
        &mut self,
        key: Key,
        value: &Value,
        epoch: [u64; 3],
        received: Instant,
    ) -> Result<(), Error> {
        self.remove(&key);
        if value["resultType"] != "complete" || value["isError"] == true {
            return Ok(());
        }
        let ttl = match value.get("ttlMs") {
            None => 0,
            Some(value) if value.as_i64().is_some_and(|n| n < 0) => 0,
            Some(value) => value.as_u64().ok_or(Error::Protocol)?.min(MAX_TTL_MS),
        };
        let scope = scope(value)?;
        if ttl == 0 || scope.is_none() {
            return Ok(());
        }
        let bytes = key.bytes()
            + serde_json::to_vec(value)
                .map_err(|_| Error::Protocol)?
                .len();
        if bytes > MAX_BYTES {
            return Ok(());
        }
        while self.entries.len() >= MAX_ENTRIES || self.bytes + bytes > MAX_BYTES {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.received)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(key) => self.remove(&key),
                None => break,
            }
        }
        self.bytes += bytes;
        self.entries.insert(
            key,
            Entry {
                value: value.clone(),
                received,
                ttl: Duration::from_millis(ttl),
                epoch,
                bytes,
            },
        );
        Ok(())
    }
    fn remove(&mut self, key: &Key) {
        if let Some(entry) = self.entries.remove(key) {
            self.bytes -= entry.bytes;
        }
    }
    pub(super) fn invalidate(&mut self, method: &str) {
        self.entries.retain(|key, entry| {
            if key.method == method {
                self.bytes -= entry.bytes;
                false
            } else {
                true
            }
        });
    }
    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.bytes = 0;
    }
}

#[cfg(test)]
mod tests;
