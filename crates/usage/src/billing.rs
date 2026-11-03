//! Bounded, read-only local billing evidence from provider logs.
//!
//! Missing prices or malformed records are never priced as zero. The scanners
//! read Claude Code JSONL, Codex rollout JSONL and OpenCode's local SQLite
//! database. Only documented fields are interpreted; unknown formats and
//! unknown models stay explicitly `unpriced`.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufRead, BufReader, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_FILES: usize = 2_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LINE: usize = 1024 * 1024;
const MAX_EVENTS: usize = 200_000;
const MAX_SEEN: usize = 200_000;
const MAX_OPENCODE_ROWS: u64 = 100_000;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CostSource {
    /// The provider recorded an explicit cost.
    Recorded,
    /// Dunlin priced a known model from its token counts.
    Priced,
    /// No trustworthy price is available.
    Unpriced,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct BillingEvent {
    pub key: String,
    pub at_ms: u64,
    pub provider: String,
    pub harness: String,
    pub model: String,
    #[serde(default)]
    pub project: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(default)]
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost_usd: Option<f64>,
    pub cost_source: CostSource,
}

#[derive(Clone, Copy, Debug)]
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// USD per million tokens. Unknown models deliberately return `None`.
fn price(model: &str, huge_context: bool) -> Option<Price> {
    let model = model.to_ascii_lowercase();
    if model.contains("opus-4-5") || model.contains("opus-4.5") {
        Some(Price {
            input: 5.,
            output: 25.,
            cache_read: 0.5,
            cache_write: 6.25,
        })
    } else if model.contains("opus-4") {
        Some(Price {
            input: 15.,
            output: 75.,
            cache_read: 1.5,
            cache_write: 18.75,
        })
    } else if model.contains("sonnet-4") {
        if huge_context {
            Some(Price {
                input: 6.,
                output: 22.5,
                cache_read: 0.6,
                cache_write: 7.5,
            })
        } else {
            Some(Price {
                input: 3.,
                output: 15.,
                cache_read: 0.3,
                cache_write: 3.75,
            })
        }
    } else if model.contains("haiku-4") {
        Some(Price {
            input: 1.,
            output: 5.,
            cache_read: 0.1,
            cache_write: 1.25,
        })
    } else if model.contains("gpt-5-mini") {
        Some(Price {
            input: 0.25,
            output: 2.,
            cache_read: 0.025,
            cache_write: 0.25,
        })
    } else if model.contains("gpt-5-nano") {
        Some(Price {
            input: 0.05,
            output: 0.4,
            cache_read: 0.005,
            cache_write: 0.05,
        })
    } else if model.contains("gpt-5") {
        Some(Price {
            input: 1.25,
            output: 10.,
            cache_read: 0.125,
            cache_write: 1.25,
        })
    } else if model.contains("o4-mini") {
        Some(Price {
            input: 1.1,
            output: 4.4,
            cache_read: 0.275,
            cache_write: 1.1,
        })
    } else if model == "o3" || model.starts_with("o3-") {
        Some(Price {
            input: 2.,
            output: 8.,
            cache_read: 0.5,
            cache_write: 2.,
        })
    } else {
        None
    }
}

fn priced_cost(
    model: &str,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
) -> Option<f64> {
    let huge = input.saturating_add(cache_read) > 200_000;
    let price = price(model, huge)?;
    Some(
        (input as f64 * price.input
            + output as f64 * price.output
            + cache_read as f64 * price.cache_read
            + cache_write as f64 * price.cache_write)
            / 1_000_000.,
    )
}

fn timestamp_ms(value: &str) -> Option<u64> {
    chrono::DateTime::parse_from_rfc3339(value)
        .ok()
        .and_then(|at| u64::try_from(at.timestamp_millis()).ok())
}

fn timestamp_value(value: Option<&serde_json::Value>) -> Option<u64> {
    let value = value?;
    if let Some(value) = value.as_u64() {
        return Some(if value < 10_000_000_000 {
            value.saturating_mul(1000)
        } else {
            value
        });
    }
    value.as_str().and_then(timestamp_ms)
}

fn number(value: Option<&serde_json::Value>) -> u64 {
    value.and_then(serde_json::Value::as_u64).unwrap_or(0)
}

fn float(value: Option<&serde_json::Value>) -> Option<f64> {
    value
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.)
}

fn string(value: Option<&serde_json::Value>) -> Option<String> {
    value
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn project_of(value: &serde_json::Value) -> Option<String> {
    string(value.get("cwd"))
        .or_else(|| string(value.pointer("/payload/cwd")))
        .or_else(|| string(value.pointer("/payload/project")))
        .or_else(|| string(value.get("project")))
}

fn claude_event(value: &serde_json::Value) -> Option<BillingEvent> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("assistant") {
        return None;
    }
    let message = value.get("message")?;
    let usage = message.get("usage")?;
    let model = message
        .get("model")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let key = message
        .get("id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            value
                .get("uuid")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        })?;
    let at_ms = value
        .get("timestamp")
        .and_then(serde_json::Value::as_str)
        .and_then(timestamp_ms)
        .or_else(|| timestamp_value(value.get("timestamp")))?;
    let input_tokens = number(usage.get("input_tokens"));
    let output_tokens = number(usage.get("output_tokens"));
    let cache_read_tokens = number(usage.get("cache_read_input_tokens"));
    let cache_write_tokens = number(usage.get("cache_creation_input_tokens"));
    let cost = priced_cost(
        &model,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    );
    Some(BillingEvent {
        key: format!("claude:{key}"),
        at_ms,
        provider: "Claude Code".into(),
        harness: "Claude Code".into(),
        model,
        project: project_of(value),
        input_tokens,
        output_tokens,
        reasoning_tokens: 0,
        cache_read_tokens,
        cache_write_tokens,
        cost_usd: cost,
        cost_source: if cost.is_some() {
            CostSource::Priced
        } else {
            CostSource::Unpriced
        },
    })
}

#[derive(Default)]
struct CodexState {
    model: String,
    project: Option<String>,
    last_total: Option<(u64, u64, u64, u64)>,
}

fn codex_usage(info: &serde_json::Value, state: &mut CodexState) -> Option<(u64, u64, u64, u64)> {
    let value = info
        .get("last_token_usage")
        .filter(|value| value.is_object())
        .or_else(|| {
            info.get("total_token_usage")
                .filter(|value| value.is_object())
        })?;
    let input = number(value.get("input_tokens"));
    let cached = number(value.get("cached_input_tokens"));
    let output = number(value.get("output_tokens"));
    let reasoning = number(value.get("reasoning_output_tokens"));
    let current = (input.saturating_sub(cached), output, cached, reasoning);
    if info.get("last_token_usage").is_none() {
        let delta = state.last_total.map_or(current, |previous| {
            (
                current.0.saturating_sub(previous.0),
                current.1.saturating_sub(previous.1),
                current.2.saturating_sub(previous.2),
                current.3.saturating_sub(previous.3),
            )
        });
        state.last_total = Some(current);
        Some(delta)
    } else {
        Some(current)
    }
}

fn codex_event(
    value: &serde_json::Value,
    path: &Path,
    line: u64,
    state: &mut CodexState,
) -> Option<BillingEvent> {
    let kind = value.get("type").and_then(serde_json::Value::as_str);
    if kind == Some("session_meta") || kind == Some("turn_context") {
        let payload = value.get("payload")?;
        if let Some(model) = string(payload.get("model")) {
            state.model = model;
        }
        if let Some(project) = project_of(value) {
            state.project = Some(project);
        }
        return None;
    }
    let payload = value.get("payload").unwrap_or(value);
    let event_kind = payload.get("type").and_then(serde_json::Value::as_str);
    if event_kind != Some("token_count") && payload.get("info").is_none() {
        return None;
    }
    let usage = payload
        .get("info")
        .filter(|info| info.is_object())
        .or_else(|| payload.get("usage").filter(|usage| usage.is_object()))?;
    let (input, output, cache_read, reasoning) = codex_usage(usage, state)?;
    if input == 0 && output == 0 && cache_read == 0 {
        return None;
    }
    let at_ms = timestamp_value(value.get("timestamp"))
        .or_else(|| timestamp_value(payload.get("timestamp")))
        .or_else(|| timestamp_value(value.get("time")))
        .unwrap_or(0);
    let model = if state.model.is_empty() {
        string(payload.get("model")).unwrap_or_else(|| "unknown".into())
    } else {
        state.model.clone()
    };
    let cost = priced_cost(&model, input, output, cache_read, 0);
    Some(BillingEvent {
        key: format!("codex:{}:{line}", path.display()),
        at_ms,
        provider: "Codex".into(),
        harness: "OpenAI Codex".into(),
        model,
        project: state.project.clone().or_else(|| project_of(value)),
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: 0,
        cost_usd: cost,
        cost_source: if cost.is_some() {
            CostSource::Priced
        } else {
            CostSource::Unpriced
        },
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileCheckpoint {
    pub offset: u64,
    pub size: u64,
    pub mtime_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub files: BTreeMap<String, FileCheckpoint>,
    #[serde(default)]
    pub seen: BTreeSet<String>,
    #[serde(default)]
    pub last_scan_ms: u64,
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub events: Vec<BillingEvent>,
    pub checkpoint: Checkpoint,
    pub warnings: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    pub truncated: bool,
}

impl ScanReport {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.warnings.len() < 32 && !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
}

fn mtime_ms(metadata: &fs::Metadata) -> u64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn files(root: &Path, extension: &str, out: &mut Vec<PathBuf>, depth: usize) {
    if out.len() >= MAX_FILES || depth > 6 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            files(&path, extension, out, depth + 1);
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == extension) {
            out.push(path);
        }
    }
}

fn scan_jsonl(
    paths: Vec<PathBuf>,
    checkpoint: &mut Checkpoint,
    report: &mut ScanReport,
    mut parse: impl FnMut(&serde_json::Value, &Path, u64, &mut ScanReport) -> Option<BillingEvent>,
) {
    for path in paths {
        if report.events.len() >= MAX_EVENTS {
            report.truncated = true;
            report.warn("Event limit reached; older or additional log lines were not read.");
            break;
        }
        if report.files >= MAX_FILES {
            report.truncated = true;
            report.warn("File limit reached; additional provider logs were not read.");
            break;
        }
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if report.bytes.saturating_add(metadata.len()) > MAX_BYTES {
            report.truncated = true;
            report.warn("Byte limit reached; remaining provider logs were not read.");
            break;
        }
        report.files += 1;
        report.bytes = report.bytes.saturating_add(metadata.len());
        let key = path.to_string_lossy().to_string();
        let previous = checkpoint.files.get(&key).cloned().unwrap_or_default();
        let size = metadata.len();
        let modified = mtime_ms(&metadata);
        if previous.size == size && previous.mtime_ms == modified {
            continue;
        }
        let mut offset = if previous.offset <= size {
            previous.offset
        } else {
            0
        };
        let Ok(mut file) = fs::File::open(&path) else {
            continue;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            offset = 0;
            let _ = file.seek(SeekFrom::Start(0));
        }
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut line_number = 0u64;
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            if read == 0 {
                break;
            }
            line_number += 1;
            if !line.ends_with('\n') {
                // Keep a partial last line for the next incremental scan.
                break;
            }
            offset = offset.saturating_add(read as u64);
            if line.len() > MAX_LINE {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                continue;
            };
            let Some(event) = parse(&value, &path, line_number, report) else {
                continue;
            };
            if checkpoint.seen.len() >= MAX_SEEN {
                report.truncated = true;
                report.warn("Dedupe checkpoint limit reached; remaining events were not counted.");
                break;
            }
            if checkpoint.seen.insert(event.key.clone()) && report.events.len() < MAX_EVENTS {
                report.events.push(event);
            }
        }
        checkpoint.files.insert(
            key,
            FileCheckpoint {
                offset,
                size,
                mtime_ms: modified,
            },
        );
    }
}

fn claude_paths(root: &Path) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    files(root, "jsonl", &mut paths, 0);
    paths
}

/// Scan `~/.claude/projects`-style JSONL logs.
pub fn scan_claude(root: &Path) -> Vec<BillingEvent> {
    let mut checkpoint = Checkpoint::default();
    scan_claude_incremental(root, &mut checkpoint)
}

/// Bounded incremental Claude scan. Reuses complete-line offsets, never reads a
/// partial final line and dedupes provider message ids across files.
pub fn scan_claude_incremental(root: &Path, checkpoint: &mut Checkpoint) -> Vec<BillingEvent> {
    let mut report = ScanReport::default();
    let paths = claude_paths(root);
    scan_jsonl(paths, checkpoint, &mut report, |value, _, _, _| {
        claude_event(value)
    });
    checkpoint.last_scan_ms = now_ms();
    let mut events = report.events;
    events.sort_by_key(|event| event.at_ms);
    events
}

/// Scan Codex rollout JSONL logs. `token_count` records are read as per-turn
/// deltas when available; cumulative counters are differenced per file.
pub fn scan_codex_incremental(root: &Path, checkpoint: &mut Checkpoint) -> Vec<BillingEvent> {
    let mut report = ScanReport::default();
    let mut paths = Vec::new();
    files(root, "jsonl", &mut paths, 0);
    let mut state = CodexState::default();
    let mut current_file = PathBuf::new();
    scan_jsonl(paths, checkpoint, &mut report, |value, path, line, _| {
        if path != current_file {
            state = CodexState::default();
            current_file = path.to_path_buf();
        }
        codex_event(value, path, line, &mut state)
    });
    checkpoint.last_scan_ms = now_ms();
    let mut events = report.events;
    events.sort_by_key(|event| event.at_ms);
    events
}

pub fn scan_codex(root: &Path) -> Vec<BillingEvent> {
    scan_codex_incremental(root, &mut Checkpoint::default())
}

fn session_projects(connection: &Connection) -> BTreeMap<String, String> {
    let mut result = BTreeMap::new();
    let Ok(mut statement) = connection.prepare("SELECT id, data FROM session") else {
        return result;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return result;
    };
    for row in rows.flatten() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.1)
            && let Some(path) = string(value.get("directory"))
                .or_else(|| string(value.get("path")))
                .or_else(|| string(value.pointer("/project/path")))
        {
            result.insert(row.0, path);
        }
    }
    result
}

fn opencode_event(
    id: &str,
    data: &serde_json::Value,
    at_fallback: u64,
    projects: &BTreeMap<String, String>,
) -> Option<BillingEvent> {
    if data.get("role").and_then(serde_json::Value::as_str) != Some("assistant") {
        return None;
    }
    let tokens = data.get("tokens")?;
    let cache = tokens.get("cache");
    let input = number(tokens.get("input"));
    let output = number(tokens.get("output"));
    let reasoning = number(tokens.get("reasoning"));
    let cache_read =
        number(cache.and_then(|cache| cache.get("read"))).max(number(tokens.get("cache_read")));
    let cache_write =
        number(cache.and_then(|cache| cache.get("write"))).max(number(tokens.get("cache_write")));
    if input == 0 && output == 0 && cache_read == 0 && cache_write == 0 {
        return None;
    }
    let model = string(data.get("modelID"))
        .or_else(|| string(data.get("model")))
        .or_else(|| string(data.pointer("/model/id")))
        .unwrap_or_else(|| "unknown".into());
    let provider = string(data.get("providerID")).unwrap_or_else(|| "OpenCode".into());
    let at_ms = timestamp_value(data.pointer("/time/created"))
        .or_else(|| timestamp_value(data.get("created")))
        .unwrap_or(at_fallback);
    let project = string(data.get("directory"))
        .or_else(|| string(data.get("path")))
        .or_else(|| {
            data.get("sessionID")
                .and_then(serde_json::Value::as_str)
                .and_then(|session| projects.get(session).cloned())
        });
    let recorded = float(data.get("cost"));
    let cost = recorded.or_else(|| priced_cost(&model, input, output, cache_read, cache_write));
    Some(BillingEvent {
        key: format!("opencode:{id}"),
        at_ms,
        provider: format!("OpenCode · {provider}"),
        harness: "OpenCode".into(),
        model,
        project,
        input_tokens: input,
        output_tokens: output,
        reasoning_tokens: reasoning,
        cache_read_tokens: cache_read,
        cache_write_tokens: cache_write,
        cost_usd: cost,
        cost_source: if recorded.is_some() {
            CostSource::Recorded
        } else if cost.is_some() {
            CostSource::Priced
        } else {
            CostSource::Unpriced
        },
    })
}

pub fn scan_opencode_db(path: &Path) -> Result<Vec<BillingEvent>, String> {
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|_| "OpenCode database is locked or unavailable".to_string())?;
    let _ = connection.busy_timeout(std::time::Duration::from_secs(2));
    let projects = session_projects(&connection);
    let mut statement = connection
        .prepare("SELECT id, data, time_created FROM message ORDER BY time_created DESC LIMIT ?1")
        .or_else(|_| connection.prepare("SELECT id, data, 0 FROM message LIMIT ?1"))
        .map_err(|_| "OpenCode message schema is not recognized".to_string())?;
    let rows = statement
        .query_map([MAX_OPENCODE_ROWS], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
            ))
        })
        .map_err(|_| "OpenCode messages could not be read".to_string())?;
    let mut events = Vec::new();
    for row in rows.flatten() {
        let Ok(data) = serde_json::from_str::<serde_json::Value>(&row.1) else {
            continue;
        };
        let fallback = if row.2 < 10_000_000_000 && row.2 > 0 {
            row.2 as u64 * 1000
        } else {
            row.2.max(0) as u64
        };
        if let Some(event) = opencode_event(&row.0, &data, fallback, &projects) {
            events.push(event);
        }
    }
    events.sort_by_key(|event| event.at_ms);
    Ok(events)
}

/// Default Claude Code project log root, if it exists.
pub fn default_claude_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let root = home.join(".claude/projects");
    root.is_dir().then_some(root)
}

pub fn default_codex_root() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let root = home.join(".codex/sessions");
    root.is_dir().then_some(root)
}

pub fn default_opencode_db() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("OPENCODE_DATA_HOME").map(PathBuf::from) {
        for candidate in [root.join("opencode.db"), root.join("storage/opencode.db")] {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    if let Some(root) = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from) {
        let candidate = root.join("opencode/opencode.db");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    [
        home.join(".local/share/opencode/opencode.db"),
        home.join("Library/Application Support/opencode/opencode.db"),
        home.join(".opencode/opencode.db"),
    ]
    .into_iter()
    .find(|candidate| candidate.is_file())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Totals {
    pub events: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub priced_cost_usd: f64,
    pub unpriced_events: u64,
    pub recorded_cost_events: u64,
}

impl Totals {
    fn add(&mut self, event: &BillingEvent) {
        self.events += 1;
        self.input_tokens += event.input_tokens;
        self.output_tokens += event.output_tokens;
        self.reasoning_tokens += event.reasoning_tokens;
        self.cache_read_tokens += event.cache_read_tokens;
        self.cache_write_tokens += event.cache_write_tokens;
        match event.cost_usd {
            Some(cost) => {
                self.priced_cost_usd += cost;
                if event.cost_source == CostSource::Recorded {
                    self.recorded_cost_events += 1;
                }
            }
            None => self.unpriced_events += 1,
        }
    }
    pub fn tokens(&self) -> u64 {
        self.input_tokens
            + self.output_tokens
            + self.reasoning_tokens
            + self.cache_read_tokens
            + self.cache_write_tokens
    }
    /// Share of prompt-side tokens served from cache. Unknown when no
    /// prompt-side tokens were reported; never an invented zero.
    pub fn cache_reuse_percent(&self) -> Option<f32> {
        let prompt = self
            .input_tokens
            .saturating_add(self.cache_read_tokens)
            .saturating_add(self.cache_write_tokens);
        (prompt > 0).then(|| self.cache_read_tokens as f32 / prompt as f32 * 100.)
    }

    pub fn cost_label(&self) -> String {
        if self.events == 0 {
            return "no recorded events".into();
        }
        if self.unpriced_events == 0 {
            format!("${:.4}", self.priced_cost_usd)
        } else {
            format!(
                "≥ ${:.4} · {} unpriced",
                self.priced_cost_usd, self.unpriced_events
            )
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Slice {
    pub label: String,
    pub totals: Totals,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Day {
    pub date: String,
    pub totals: Totals,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Alert {
    pub level: AlertLevel,
    pub message: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Summary {
    pub generated_ms: u64,
    pub totals: Totals,
    pub by_provider: Vec<Slice>,
    pub by_harness: Vec<Slice>,
    pub by_model: Vec<Slice>,
    pub by_project: Vec<Slice>,
    pub by_day: Vec<Day>,
    pub unpriced_models: Vec<String>,
    pub warnings: Vec<String>,
    pub files: usize,
    pub bytes: u64,
    pub truncated: bool,
}

fn push_slice(map: &mut BTreeMap<String, Totals>, key: Option<&str>, event: &BillingEvent) {
    map.entry(key.unwrap_or("Unattributed").to_string())
        .or_default()
        .add(event);
}

fn slices(map: BTreeMap<String, Totals>) -> Vec<Slice> {
    let mut values = map
        .into_iter()
        .map(|(label, totals)| Slice { label, totals })
        .collect::<Vec<_>>();
    values.sort_by(|a, b| {
        b.totals
            .priced_cost_usd
            .total_cmp(&a.totals.priced_cost_usd)
            .then_with(|| b.totals.events.cmp(&a.totals.events))
            .then_with(|| a.label.cmp(&b.label))
    });
    values
}

pub fn summarize(report: ScanReport) -> Summary {
    let mut totals = Totals::default();
    let mut by_provider = BTreeMap::new();
    let mut by_harness = BTreeMap::new();
    let mut by_model = BTreeMap::new();
    let mut by_project = BTreeMap::new();
    let mut by_day = BTreeMap::new();
    let mut unpriced = BTreeSet::new();
    for event in &report.events {
        totals.add(event);
        push_slice(&mut by_provider, Some(&event.provider), event);
        push_slice(&mut by_harness, Some(&event.harness), event);
        push_slice(&mut by_model, Some(&event.model), event);
        push_slice(&mut by_project, event.project.as_deref(), event);
        if event.cost_usd.is_none() {
            unpriced.insert(event.model.clone());
        }
        let date = chrono::DateTime::from_timestamp_millis(event.at_ms as i64)
            .map(|at| at.format("%Y-%m-%d").to_string())
            .unwrap_or_else(|| "unknown".into());
        by_day
            .entry(date.clone())
            .or_insert_with(|| Day {
                date,
                totals: Totals::default(),
            })
            .totals
            .add(event);
    }
    let mut by_day = by_day.into_values().collect::<Vec<_>>();
    by_day.sort_by(|a, b| b.date.cmp(&a.date));
    by_day.truncate(90);
    Summary {
        generated_ms: now_ms(),
        totals,
        by_provider: slices(by_provider),
        by_harness: slices(by_harness),
        by_model: slices(by_model),
        by_project: slices(by_project),
        by_day,
        unpriced_models: unpriced.into_iter().collect(),
        warnings: report.warnings,
        files: report.files,
        bytes: report.bytes,
        truncated: report.truncated,
    }
}

impl Summary {
    /// Current-month priced spend. Unknown prices are never added as zero; the
    /// caller receives the count separately through `totals.unpriced_events`.
    pub fn month_to_date_cost(&self) -> f64 {
        let now = chrono::Utc::now();
        let prefix = now.format("%Y-%m").to_string();
        self.by_day
            .iter()
            .filter(|day| day.date.starts_with(&prefix))
            .map(|day| day.totals.priced_cost_usd)
            .sum()
    }

    pub fn alerts(&self, monthly_budget_usd: Option<f64>) -> Vec<Alert> {
        let mut alerts = Vec::new();
        if self.totals.events == 0 {
            alerts.push(Alert {
                level: AlertLevel::Info,
                message: "No Claude Code, Codex or OpenCode billing logs were found in the default locations.".into(),
            });
        }
        if self.totals.unpriced_events > 0 {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: format!(
                    "{} events are unpriced ({}). They are excluded from the known total, not treated as $0.",
                    self.totals.unpriced_events,
                    if self.unpriced_models.is_empty() {
                        "unknown model".into()
                    } else {
                        self.unpriced_models.join(", ")
                    }
                ),
            });
        }
        if self.truncated {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: "The scan reached a file, byte or event bound. The summary is incomplete."
                    .into(),
            });
        }
        for warning in &self.warnings {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: warning.clone(),
            });
        }
        if let Some(limit) = monthly_budget_usd.filter(|limit| limit.is_finite() && *limit > 0.) {
            let spent = self.month_to_date_cost();
            let percent = spent / limit * 100.;
            if percent >= 100. {
                alerts.push(Alert {
                    level: AlertLevel::Critical,
                    message: format!(
                        "Known month-to-date spend ${spent:.2} has reached the ${limit:.2} alert budget. Unpriced events can add unknown cost."
                    ),
                });
            } else if percent >= 80. {
                alerts.push(Alert {
                    level: AlertLevel::Warning,
                    message: format!(
                        "Known month-to-date spend ${spent:.2} is {percent:.0}% of the ${limit:.2} alert budget."
                    ),
                });
            }
        }
        alerts
    }
}

/// Scan every supported local source with the default bounds.
pub fn scan_all() -> ScanReport {
    let mut report = ScanReport::default();
    let mut checkpoint = Checkpoint::default();
    if let Some(root) = default_claude_root() {
        let mut claude = Checkpoint::default();
        let events = scan_claude_incremental(&root, &mut claude);
        checkpoint.files.extend(claude.files);
        checkpoint.seen.extend(claude.seen);
        report.events.extend(events);
    }
    if let Some(root) = default_codex_root() {
        let mut codex = Checkpoint::default();
        let events = scan_codex_incremental(&root, &mut codex);
        checkpoint.files.extend(codex.files);
        checkpoint.seen.extend(codex.seen);
        report.events.extend(events);
    }
    if let Some(path) = default_opencode_db() {
        match scan_opencode_db(&path) {
            Ok(events) => {
                for event in events {
                    if checkpoint.seen.insert(event.key.clone()) {
                        report.events.push(event);
                    }
                }
                if let Ok(metadata) = fs::metadata(&path) {
                    report.files += 1;
                    report.bytes = report.bytes.saturating_add(metadata.len());
                    checkpoint.files.insert(
                        path.to_string_lossy().to_string(),
                        FileCheckpoint {
                            offset: 0,
                            size: metadata.len(),
                            mtime_ms: mtime_ms(&metadata),
                        },
                    );
                }
            }
            Err(error) => report.warn(format!("OpenCode billing: {error}")),
        }
    }
    checkpoint.last_scan_ms = now_ms();
    report.checkpoint = checkpoint;
    report
        .events
        .sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then_with(|| a.key.cmp(&b.key)));
    report
}

/// Helper for tests and callers that only need a bounded instant.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dunlin-billing-{name}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn claude_logs_are_priced_deduped_and_unknown_models_stay_unpriced() {
        let root = fixture("claude");
        fs::create_dir_all(root.join("session")).unwrap();
        let mut file = fs::File::create(root.join("session/log.jsonl")).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "type":"assistant",
            "timestamp":"2026-11-02T10:00:00Z",
            "cwd":"/projects/one",
            "message":{"id":"m1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000000,"output_tokens":1000000,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}
        })).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "type":"assistant",
            "timestamp":"2026-11-02T10:01:00Z",
            "message":{"id":"m1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000000,"output_tokens":1000000}}
        })).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "type":"assistant",
            "timestamp":"2026-11-02T10:02:00Z",
            "message":{"id":"m2","model":"unknown-model","usage":{"input_tokens":10,"output_tokens":20}}
        })).unwrap();
        let events = scan_claude(&root);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].cost_usd, Some(28.5));
        assert_eq!(events[0].project.as_deref(), Some("/projects/one"));
        assert_eq!(events[1].cost_usd, None);
        assert_eq!(events[1].cost_source, CostSource::Unpriced);
        let summary = summarize(ScanReport {
            events,
            ..Default::default()
        });
        assert_eq!(summary.totals.unpriced_events, 1);
        assert_eq!(summary.by_project.len(), 2);
        assert_eq!(summary.unpriced_models, vec!["unknown-model"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_token_counts_use_deltas_and_cache_pricing() {
        let root = fixture("codex");
        fs::create_dir_all(root.join("2026/11/02")).unwrap();
        let path = root.join("2026/11/02/rollout-test.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        for value in [
            serde_json::json!({"timestamp":"2026-11-02T10:00:00Z","type":"session_meta","payload":{"id":"s","model":"gpt-5","cwd":"/code"}}),
            serde_json::json!({"timestamp":"2026-11-02T10:01:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1000,"cached_input_tokens":200,"output_tokens":300,"reasoning_output_tokens":50}}}}),
            serde_json::json!({"timestamp":"2026-11-02T10:02:00Z","type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":1500,"cached_input_tokens":300,"output_tokens":500,"reasoning_output_tokens":80}}}}),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        let events = scan_codex(&root);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].input_tokens, 800);
        assert_eq!(events[0].cache_read_tokens, 200);
        assert_eq!(events[1].input_tokens, 400);
        assert_eq!(events[1].cache_read_tokens, 100);
        assert!(events[0].cost_usd.unwrap() > 0.);
        assert_eq!(events[0].project.as_deref(), Some("/code"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opencode_sqlite_messages_keep_recorded_cost_and_unknown_models_unpriced() {
        let root = fixture("opencode");
        let path = root.join("opencode.db");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE session (id TEXT PRIMARY KEY, data TEXT NOT NULL);
                     CREATE TABLE message (id TEXT PRIMARY KEY, data TEXT NOT NULL, time_created INTEGER);
                     INSERT INTO session VALUES ('s1', '{\"directory\":\"/repo\"}');
                     INSERT INTO message VALUES ('a1', '{\"role\":\"assistant\",\"sessionID\":\"s1\",\"providerID\":\"anthropic\",\"modelID\":\"claude-sonnet-4-5\",\"tokens\":{\"input\":100,\"output\":20,\"cache\":{\"read\":5,\"write\":0}},\"cost\":0.25,\"time\":{\"created\":1762164000000}}', 0);
                     INSERT INTO message VALUES ('a2', '{\"role\":\"assistant\",\"sessionID\":\"s1\",\"providerID\":\"other\",\"modelID\":\"mystery\",\"tokens\":{\"input\":10,\"output\":2},\"time\":{\"created\":1762164060000}}', 0);",
                )
                .unwrap();
        }
        let events = scan_opencode_db(&path).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].cost_usd, Some(0.25));
        assert_eq!(events[0].cost_source, CostSource::Recorded);
        assert_eq!(events[0].project.as_deref(), Some("/repo"));
        assert_eq!(events[1].cost_usd, None);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checkpoint_resumes_after_complete_lines_and_does_not_duplicate() {
        let root = fixture("checkpoint");
        let path = root.join("log.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "{}", serde_json::json!({
            "type":"assistant","timestamp":"2026-11-02T10:00:00Z",
            "message":{"id":"m1","model":"claude-sonnet-4-5","usage":{"input_tokens":1000,"output_tokens":100}}
        })).unwrap();
        let mut checkpoint = Checkpoint::default();
        assert_eq!(scan_claude_incremental(&root, &mut checkpoint).len(), 1);
        let mut append = fs::OpenOptions::new().append(true).open(&path).unwrap();
        write!(append, "{{\"type\":\"assistant\",\"timestamp\":\"2026-11-02T10:01:00Z\",\"message\":{{\"id\":\"m2\",\"model\":\"gpt-5\",\"usage\":{{\"input_tokens\":5}}}}}}").unwrap();
        assert!(scan_claude_incremental(&root, &mut checkpoint).is_empty());
        writeln!(append).unwrap();
        let events = scan_claude_incremental(&root, &mut checkpoint);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key, "claude:m2");
        assert!(scan_claude_incremental(&root, &mut checkpoint).is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn long_context_and_alert_budgets_are_explicit() {
        assert!(
            price("claude-sonnet-4-5", true).unwrap().input
                > price("claude-sonnet-4-5", false).unwrap().input
        );
        let mut summary = Summary {
            totals: Totals {
                events: 1,
                priced_cost_usd: 90.,
                ..Default::default()
            },
            ..Default::default()
        };
        summary.by_day.push(Day {
            date: chrono::Utc::now().format("%Y-%m-%d").to_string(),
            totals: Totals {
                events: 1,
                priced_cost_usd: 90.,
                ..Default::default()
            },
        });
        let alerts = summary.alerts(Some(100.));
        assert!(
            alerts
                .iter()
                .any(|alert| alert.level == AlertLevel::Warning)
        );
        summary.totals.priced_cost_usd = 110.;
        summary.by_day[0].totals.priced_cost_usd = 110.;
        assert!(
            summary
                .alerts(Some(100.))
                .iter()
                .any(|alert| alert.level == AlertLevel::Critical)
        );
        assert!(summary.alerts(None).is_empty());
    }
}
