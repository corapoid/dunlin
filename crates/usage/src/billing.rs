//! Bounded, read-only local billing evidence from provider logs.
//!
//! Missing prices or malformed records are never priced as zero. The scanners
//! read Claude Code JSONL, Codex rollout JSONL and OpenCode's local SQLite
//! database. Only documented fields are interpreted; unknown formats and
//! unknown models stay explicitly `unpriced`.
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
mod jsonl;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

const MAX_FILES: usize = 2_000;
const MAX_BYTES: u64 = 32 * 1024 * 1024;
const MAX_LINE: usize = 32 * 1024 * 1024;
const MAX_EVENTS: usize = 200_000;
// shortcut: dedupe retains at most the relay's million-event capacity; use a disk index for larger archives.
const MAX_SEEN: usize = 1_000_000;

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

/// How a provider log record relates to visible user work.
///
/// The default is deliberately `Unknown`: an unrecognized record is never
/// silently attributed to the top-level conversation or priced as zero.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Main,
    Subagent,
    /// Provider/harness bookkeeping such as compaction or synthetic messages.
    Internal,
    #[default]
    Unknown,
}

impl WorkKind {
    pub fn label(self) -> &'static str {
        match self {
            WorkKind::Main => "Main conversation",
            WorkKind::Subagent => "Subagents",
            WorkKind::Internal => "Internal / synthetic",
            WorkKind::Unknown => "Unknown attribution",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Attribution {
    pub kind: WorkKind,
    #[serde(default)]
    pub session: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub task: Option<String>,
}

impl Attribution {
    fn new(kind: WorkKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
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
    #[serde(default)]
    pub attribution: Attribution,
}

#[derive(Clone, Copy, Debug)]
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

/// USD per million tokens. Unknown models deliberately return `None`.
fn price(model: &str, context_tokens: u64) -> Option<Price> {
    let normalized = model.to_ascii_lowercase().replace('.', "-");
    let model = normalized.strip_prefix("anthropic/").unwrap_or(&normalized);
    let model = model
        .rsplit_once('-')
        .filter(|(_, suffix)| suffix.len() == 8 && suffix.bytes().all(|b| b.is_ascii_digit()))
        .map_or(model, |(base, _)| base);
    // platform.claude.com/docs/en/models/{sonnet,haiku}-5-5/overview, verified 2026-11-06.
    if matches!(model, "claude-opus-5-5" | "opus-5-5") {
        Some(Price {
            input: 4.,
            output: 20.,
            cache_read: 0.2,
            cache_write: 5.,
        })
    } else if matches!(model, "claude-sonnet-5-5" | "sonnet-5-5") {
        Some(Price {
            input: 2.,
            output: 10.,
            cache_read: 0.1,
            cache_write: 2.5,
        })
    } else if matches!(model, "claude-haiku-5-5" | "haiku-5-5") {
        let multiplier = if context_tokens > 100_000 { 5. } else { 1. };
        Some(Price {
            input: 0.1 * multiplier,
            output: 0.5 * multiplier,
            cache_read: 0.01 * multiplier,
            cache_write: 0.125 * multiplier,
        })
    } else if matches!(
        model,
        "claude-opus-5"
            | "opus-5"
            | "claude-opus-4-5"
            | "claude-opus-4-6"
            | "claude-opus-4-7"
            | "claude-opus-4-8"
            | "opus-4-5"
    ) {
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
        if context_tokens > 200_000 {
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
    let price = price(
        model,
        input.saturating_add(cache_read).saturating_add(cache_write),
    )?;
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

fn truthy(value: Option<&serde_json::Value>) -> bool {
    value.is_some_and(|value| {
        value.as_bool() == Some(true)
            || value.as_u64().is_some_and(|number| number != 0)
            || value
                .as_str()
                .is_some_and(|text| matches!(text, "true" | "1" | "yes"))
    })
}

fn first_string(value: &serde_json::Value, paths: &[&str]) -> Option<String> {
    paths.iter().find_map(|path| string(value.pointer(path)))
}

fn contains_word(value: Option<&serde_json::Value>, word: &str) -> bool {
    match value {
        Some(serde_json::Value::String(text)) => text.to_ascii_lowercase().contains(word),
        Some(serde_json::Value::Object(map)) => map.iter().any(|(key, value)| {
            key.to_ascii_lowercase().contains(word) || contains_word(Some(value), word)
        }),
        Some(serde_json::Value::Array(values)) => {
            values.iter().any(|value| contains_word(Some(value), word))
        }
        _ => false,
    }
}

fn claude_attribution(value: &serde_json::Value, model: &str) -> Attribution {
    let mut attribution = Attribution::new(WorkKind::Unknown);
    attribution.session = first_string(value, &["/sessionId", "/session_id"]);
    attribution.parent = first_string(value, &["/parentSessionId", "/parent_session_id"]);
    attribution.agent = first_string(value, &["/agentId", "/agent_id"]);
    if attribution.session.is_some() {
        attribution.kind = WorkKind::Main;
    }
    if truthy(value.get("isSidechain")) || attribution.agent.is_some() {
        attribution.kind = WorkKind::Subagent;
    } else if model == "<synthetic>"
        || truthy(value.get("isCompactSummary"))
        || truthy(value.get("isApiErrorMessage"))
    {
        attribution.kind = WorkKind::Internal;
    }
    attribution
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
    )
    .map(|cost| {
        let one_hour = number(usage.pointer("/cache_creation/ephemeral_1h_input_tokens"))
            .min(cache_write_tokens);
        let price = price(
            &model,
            input_tokens
                .saturating_add(cache_read_tokens)
                .saturating_add(cache_write_tokens),
        )
        .unwrap();
        cost + one_hour as f64 * (price.input * 2. - price.cache_write).max(0.) / 1_000_000.
    });
    let attribution = claude_attribution(value, &model);
    let cost = if model == "<synthetic>"
        && input_tokens == 0
        && output_tokens == 0
        && cache_read_tokens == 0
        && cache_write_tokens == 0
    {
        Some(0.)
    } else {
        cost
    };
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
        attribution,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
struct CodexState {
    model: String,
    project: Option<String>,
    last_total: Option<(u64, u64, u64, u64)>,
    attribution: Attribution,
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
        if kind == Some("session_meta") {
            let session =
                first_string(payload, &["/id", "/session_id", "/sessionId", "/thread_id"]);
            let parent = first_string(
                payload,
                &[
                    "/parent_thread_id",
                    "/parent_id",
                    "/parent_session_id",
                    "/forked_from_id",
                ],
            );
            let agent = first_string(payload, &["/agent", "/agent_id", "/subagent"]);
            let subagent = parent.is_some()
                || agent.is_some()
                || contains_word(payload.get("source"), "subagent")
                || contains_word(payload.get("originator"), "subagent");
            let internal = contains_word(payload.get("source"), "internal")
                || contains_word(payload.get("originator"), "internal");
            state.attribution = Attribution {
                kind: if subagent {
                    WorkKind::Subagent
                } else if internal {
                    WorkKind::Internal
                } else if session.is_some() {
                    WorkKind::Main
                } else {
                    WorkKind::Unknown
                },
                session,
                parent,
                agent,
                task: first_string(payload, &["/task", "/title"]),
            };
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
    let attribution = if state.attribution.kind == WorkKind::Unknown {
        Attribution::new(WorkKind::Unknown)
    } else {
        state.attribution.clone()
    };
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
        attribution,
    })
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileCheckpoint {
    pub offset: u64,
    pub size: u64,
    pub mtime_ms: u64,
    #[serde(default)]
    line: u64,
    #[serde(default)]
    state: CodexState,
    #[serde(default)]
    skipping_line: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Checkpoint {
    pub files: BTreeMap<String, FileCheckpoint>,
    #[serde(default)]
    pub seen: BTreeSet<String>,
    #[serde(default)]
    pub last_scan_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceFile {
    pub path: String,
    pub provider: String,
    pub bytes: u64,
    pub mtime_ms: u64,
    pub offset: u64,
    pub events: u64,
    /// `scanned`, `cached`, `skipped` or `error`.
    pub state: String,
}

#[derive(Clone, Debug, Default)]
pub struct ScanReport {
    pub events: Vec<BillingEvent>,
    pub checkpoint: Checkpoint,
    pub sources: Vec<SourceFile>,
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

fn files(
    root: &Path,
    extension: &str,
    out: &mut Vec<PathBuf>,
    depth: usize,
    checkpoint: &Checkpoint,
) {
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
            files(&path, extension, out, depth + 1, checkpoint);
        } else if metadata.is_file() && path.extension().is_some_and(|ext| ext == extension) {
            if checkpoint
                .files
                .get(path.to_string_lossy().as_ref())
                .is_some_and(|previous| {
                    previous.offset == metadata.len()
                        && previous.size == metadata.len()
                        && previous.mtime_ms == mtime_ms(&metadata)
                })
            {
                continue;
            }
            out.push(path);
        }
    }
}

fn scan_jsonl(
    provider: &str,
    paths: Vec<PathBuf>,
    checkpoint: &mut Checkpoint,
    report: &mut ScanReport,
    mut parse: impl FnMut(&serde_json::Value, &Path, u64, &mut CodexState) -> Option<BillingEvent>,
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
        report.files += 1;
        let key = path.to_string_lossy().to_string();
        let previous = checkpoint.files.get(&key).cloned().unwrap_or_default();
        let size = metadata.len();
        let modified = mtime_ms(&metadata);
        let mut source = SourceFile {
            path: key.clone(),
            provider: provider.into(),
            bytes: size,
            mtime_ms: modified,
            offset: previous.offset,
            events: 0,
            state: "scanned".into(),
        };
        if previous.size == size && previous.mtime_ms == modified && previous.offset == size {
            source.state = "cached".into();
            report.sources.push(source);
            continue;
        }
        let mut offset = if previous.offset <= size {
            previous.offset
        } else {
            0
        };
        let Ok(mut file) = fs::File::open(&path) else {
            source.state = "error".into();
            report.sources.push(source);
            report.warn(format!("Could not read {provider} log {key}"));
            continue;
        };
        if file.seek(SeekFrom::Start(offset)).is_err() {
            offset = 0;
            let _ = file.seek(SeekFrom::Start(0));
        }
        let mut reader = BufReader::new(file);
        let resume = previous.offset <= size;
        let mut line_number = if resume { previous.line } else { 0 };
        let mut state = if resume {
            previous.state.clone()
        } else {
            CodexState::default()
        };
        let mut skipping = resume && previous.skipping_line;
        loop {
            if report.bytes >= MAX_BYTES
                || report.events.len() >= MAX_EVENTS
                || checkpoint.seen.len() >= MAX_SEEN
            {
                report.truncated = true;
                report.warn("Scan batch limit reached; additional provider logs remain.");
                break;
            }
            let mut line = jsonl::Line::new(&mut reader);
            let value = if skipping {
                None
            } else {
                serde_json::from_reader::<_, jsonl::Metadata>(
                    line.by_ref().take(MAX_LINE as u64 + 1),
                )
                .ok()
            };
            let remaining = (MAX_LINE as u64 + 1).saturating_sub(line.bytes);
            if std::io::copy(&mut line.by_ref().take(remaining), &mut std::io::sink()).is_err() {
                report.warn(format!("Could not decode {provider} log"));
                break;
            }
            let read = line.bytes;
            report.bytes += read;
            if read == 0 {
                break;
            }
            if !line.complete {
                if read > MAX_LINE as u64 || skipping {
                    offset += read;
                    skipping = true;
                    report.warn(
                        "Oversized provider log line skipped; billing evidence may be incomplete.",
                    );
                    continue;
                }
                // Keep a partial last line for the next incremental scan.
                break;
            }
            line_number += 1;
            offset = offset.saturating_add(read);
            if read > MAX_LINE as u64 || skipping {
                skipping = false;
                report.warn(
                    "Oversized provider log line skipped; billing evidence may be incomplete.",
                );
                continue;
            }
            let Some(value) = value.and_then(|value| serde_json::to_value(value).ok()) else {
                continue;
            };
            let Some(event) = parse(&value, &path, line_number, &mut state) else {
                continue;
            };
            if checkpoint.seen.len() >= MAX_SEEN {
                report.truncated = true;
                report.warn("Dedupe checkpoint limit reached; remaining events were not counted.");
                break;
            }
            if checkpoint.seen.insert(event.key.clone()) && report.events.len() < MAX_EVENTS {
                report.events.push(event);
                source.events += 1;
            }
        }
        source.offset = offset;
        checkpoint.files.insert(
            key,
            FileCheckpoint {
                offset,
                size,
                mtime_ms: modified,
                line: line_number,
                state,
                skipping_line: skipping,
            },
        );
        report.sources.push(source);
    }
}

fn claude_paths(root: &Path, checkpoint: &Checkpoint) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    files(root, "jsonl", &mut paths, 0, checkpoint);
    paths
}

/// Scan `~/.claude/projects`-style JSONL logs.
pub fn scan_claude(root: &Path) -> Vec<BillingEvent> {
    let mut checkpoint = Checkpoint::default();
    scan_claude_incremental(root, &mut checkpoint)
}

/// Bounded incremental Claude scan. Reuses complete-line offsets, never reads a
/// partial final line and dedupes provider message ids across files.
pub fn scan_claude_report(root: &Path, checkpoint: &mut Checkpoint) -> ScanReport {
    let mut report = ScanReport::default();
    let paths = claude_paths(root, checkpoint);
    if paths.len() == MAX_FILES {
        report.truncated = true;
        report.warn("File discovery limit reached; additional logs may remain.");
    }
    scan_jsonl(
        "Claude Code",
        paths,
        checkpoint,
        &mut report,
        |value, _, _, _| claude_event(value),
    );
    checkpoint.last_scan_ms = now_ms();
    report.events.sort_by_key(|event| event.at_ms);
    report
}

pub fn scan_claude_incremental(root: &Path, checkpoint: &mut Checkpoint) -> Vec<BillingEvent> {
    scan_claude_report(root, checkpoint).events
}

/// Scan Codex rollout JSONL logs. `token_count` records are read as per-turn
/// deltas when available; cumulative counters are differenced per file.
pub fn scan_codex_report(root: &Path, checkpoint: &mut Checkpoint) -> ScanReport {
    let mut report = ScanReport::default();
    let mut paths = Vec::new();
    files(root, "jsonl", &mut paths, 0, checkpoint);
    if paths.len() == MAX_FILES {
        report.truncated = true;
        report.warn("File discovery limit reached; additional logs may remain.");
    }
    scan_jsonl("OpenAI Codex", paths, checkpoint, &mut report, codex_event);
    checkpoint.last_scan_ms = now_ms();
    report.events.sort_by_key(|event| event.at_ms);
    report
}

pub fn scan_codex_incremental(root: &Path, checkpoint: &mut Checkpoint) -> Vec<BillingEvent> {
    scan_codex_report(root, checkpoint).events
}

pub fn scan_codex(root: &Path) -> Vec<BillingEvent> {
    scan_codex_incremental(root, &mut Checkpoint::default())
}

#[derive(Clone, Debug, Default)]
struct OpenCodeSession {
    project: Option<String>,
    parent: Option<String>,
    title: Option<String>,
    model: Option<String>,
}

fn session_projects(connection: &Connection, v2: bool) -> BTreeMap<String, OpenCodeSession> {
    let mut result = BTreeMap::new();
    let table = if v2 { "session_v2" } else { "session" };
    if let Ok(mut statement) = connection
        .prepare(&format!(
            "SELECT id, directory, parent_id, title, model FROM {table}"
        ))
        .or_else(|_| {
            connection.prepare(&format!(
                "SELECT id, directory, parent_id, title, NULL FROM {table}"
            ))
        })
    {
        if let Ok(rows) = statement.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                OpenCodeSession {
                    project: row.get(1)?,
                    parent: row.get(2)?,
                    title: row.get(3)?,
                    model: row.get::<_, Option<String>>(4)?.map(|model| {
                        serde_json::from_str::<serde_json::Value>(&model)
                            .ok()
                            .and_then(|v| v.get("id").and_then(|s| s.as_str()).map(str::to_owned))
                            .unwrap_or(model)
                    }),
                },
            ))
        }) {
            result.extend(rows.flatten());
        }
        return result;
    }
    let Ok(mut statement) = connection.prepare("SELECT id, data FROM session") else {
        return result;
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return result;
    };
    for row in rows.flatten() {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&row.1) {
            result.insert(
                row.0,
                OpenCodeSession {
                    project: string(value.get("directory"))
                        .or_else(|| string(value.get("path")))
                        .or_else(|| string(value.pointer("/project/path"))),
                    parent: first_string(&value, &["/parentID", "/parent_id", "/parentId"]),
                    title: string(value.get("title")),
                    model: string(value.get("modelID")).or_else(|| string(value.get("model"))),
                },
            );
        }
    }
    result
}

fn opencode_event(
    id: &str,
    data: &serde_json::Value,
    at_fallback: u64,
    projects: &BTreeMap<String, OpenCodeSession>,
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
    let session = data
        .get("sessionID")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let session_info = session.as_deref().and_then(|session| projects.get(session));
    let model = string(data.get("modelID"))
        .or_else(|| string(data.get("model")))
        .or_else(|| string(data.pointer("/model/id")))
        .or_else(|| session_info.and_then(|session| session.model.clone()))
        .unwrap_or_else(|| "unknown".into());
    let provider = string(data.get("providerID")).unwrap_or_else(|| "OpenCode".into());
    let at_ms = timestamp_value(data.pointer("/time/created"))
        .or_else(|| timestamp_value(data.get("created")))
        .unwrap_or(at_fallback);
    let project = string(data.get("directory"))
        .or_else(|| string(data.get("path")))
        .or_else(|| session_info.and_then(|session| session.project.clone()));
    let recorded = float(data.get("cost"));
    let cost = recorded.or_else(|| priced_cost(&model, input, output, cache_read, cache_write));
    let mut attribution = Attribution::new(if session_info.is_some() {
        WorkKind::Main
    } else {
        WorkKind::Unknown
    });
    attribution.session = session;
    if let Some(session_info) = session_info {
        attribution.parent = session_info.parent.clone();
        attribution.task = session_info.title.clone();
        if session_info.parent.is_some() {
            attribution.kind = WorkKind::Subagent;
        }
    }
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
        attribution,
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
    connection
        .execute_batch("BEGIN")
        .map_err(|_| "Could not open an OpenCode billing snapshot".to_string())?;
    let v2 = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM session_message WHERE type = 'assistant')",
            [],
            |row| row.get::<_, bool>(0),
        )
        .unwrap_or(false);
    let projects = session_projects(&connection, v2);
    let queries = if v2 {
        vec![
            "SELECT id, data, time_created, session_id, type FROM session_message WHERE type = 'assistant'",
            "SELECT CAST(rowid AS TEXT), data, 0, session_id, type FROM session_message WHERE type = 'assistant'",
        ]
    } else {
        vec![
            "SELECT id, data, time_created, session_id, '' FROM message",
            "SELECT id, data, 0, session_id, '' FROM message",
            "SELECT id, data, time_created, NULL, '' FROM message",
            "SELECT id, data, 0, NULL, '' FROM message",
        ]
    };
    let mut statement = queries
        .into_iter()
        .find_map(|query| connection.prepare(query).ok())
        .ok_or_else(|| "OpenCode message schema is not recognized".to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<i64>>(2)?.unwrap_or(0),
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|_| "OpenCode messages could not be read".to_string())?;
    let mut events = Vec::new();
    for row in rows {
        let row = row.map_err(|_| "An OpenCode billing row could not be read".to_string())?;
        let Ok(mut data) = serde_json::from_str::<serde_json::Value>(&row.1) else {
            continue;
        };
        if !data.is_object() {
            continue;
        }
        if data.get("sessionID").and_then(|s| s.as_str()).is_none() {
            data["sessionID"] = row.3.clone().into();
        }
        if data.get("role").is_none() && !row.4.is_empty() {
            data["role"] = row.4.clone().into();
        }
        let fallback = if row.2 < 10_000_000_000 && row.2 > 0 {
            row.2 as u64 * 1000
        } else {
            row.2.max(0) as u64
        };
        if let Some(event) = opencode_event(&row.0, &data, fallback, &projects) {
            if events.len() >= MAX_SEEN {
                return Err("OpenCode billing exceeds the million-event safety limit".into());
            }
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
    #[serde(default)]
    pub main_events: u64,
    #[serde(default)]
    pub subagent_events: u64,
    #[serde(default)]
    pub internal_events: u64,
    #[serde(default)]
    pub unknown_attribution_events: u64,
    #[serde(default)]
    pub subagent_cost_usd: f64,
    #[serde(default)]
    pub internal_cost_usd: f64,
    #[serde(default)]
    pub unknown_attribution_cost_usd: f64,
}

impl Totals {
    fn add(&mut self, event: &BillingEvent) {
        self.events += 1;
        self.input_tokens += event.input_tokens;
        self.output_tokens += event.output_tokens;
        self.reasoning_tokens += event.reasoning_tokens;
        self.cache_read_tokens += event.cache_read_tokens;
        self.cache_write_tokens += event.cache_write_tokens;
        match event.attribution.kind {
            WorkKind::Main => self.main_events += 1,
            WorkKind::Subagent => self.subagent_events += 1,
            WorkKind::Internal => self.internal_events += 1,
            WorkKind::Unknown => self.unknown_attribution_events += 1,
        }
        match event.cost_usd {
            Some(cost) => {
                self.priced_cost_usd += cost;
                match event.attribution.kind {
                    WorkKind::Subagent => self.subagent_cost_usd += cost,
                    WorkKind::Internal => self.internal_cost_usd += cost,
                    WorkKind::Unknown => self.unknown_attribution_cost_usd += cost,
                    WorkKind::Main => {}
                }
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimeRange {
    Day,
    Week,
    Month,
    #[default]
    All,
}

impl TimeRange {
    pub const ALL: [Self; 4] = [Self::Day, Self::Week, Self::Month, Self::All];

    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "1d",
            Self::Week => "7d",
            Self::Month => "30d",
            Self::All => "all",
        }
    }

    fn includes(self, at_ms: u64, now: u64) -> bool {
        let days = match self {
            Self::Day => 1,
            Self::Week => 7,
            Self::Month => 30,
            Self::All => return true,
        };
        at_ms > 0 && at_ms <= now && at_ms >= now.saturating_sub(days * 86_400_000)
    }
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
    #[serde(default)]
    pub by_attribution: Vec<Slice>,
    pub by_day: Vec<Day>,
    #[serde(default)]
    pub sources: Vec<SourceFile>,
    #[serde(default)]
    pub last_scan_ms: u64,
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
    summarize_range(&report, TimeRange::All, now_ms())
}

pub fn summarize_range(report: &ScanReport, range: TimeRange, now: u64) -> Summary {
    let mut totals = Totals::default();
    let mut by_provider = BTreeMap::new();
    let mut by_harness = BTreeMap::new();
    let mut by_model = BTreeMap::new();
    let mut by_project = BTreeMap::new();
    let mut by_attribution = BTreeMap::new();
    let mut by_day = BTreeMap::new();
    let mut unpriced = BTreeSet::new();
    for event in report
        .events
        .iter()
        .filter(|event| range.includes(event.at_ms, now))
    {
        totals.add(event);
        push_slice(&mut by_provider, Some(&event.provider), event);
        push_slice(&mut by_harness, Some(&event.harness), event);
        push_slice(&mut by_model, Some(&event.model), event);
        push_slice(&mut by_project, event.project.as_deref(), event);
        push_slice(
            &mut by_attribution,
            Some(event.attribution.kind.label()),
            event,
        );
        if event.cost_usd.is_none() {
            unpriced.insert(event.model.clone());
        }
        let date = (event.at_ms > 0)
            .then(|| chrono::DateTime::from_timestamp_millis(event.at_ms as i64))
            .flatten()
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
    Summary {
        generated_ms: now,
        totals,
        by_provider: slices(by_provider),
        by_harness: slices(by_harness),
        by_model: slices(by_model),
        by_project: slices(by_project),
        by_attribution: slices(by_attribution),
        by_day,
        sources: report.sources.clone(),
        last_scan_ms: report.checkpoint.last_scan_ms,
        unpriced_models: unpriced.into_iter().collect(),
        warnings: report.warnings.clone(),
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
                message: "No billing events found in this period.".into(),
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
        if self.totals.unknown_attribution_events > 0 {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: format!(
                    "{} events have no reliable main/subagent attribution. They are reported separately and are not assumed to be main work.",
                    self.totals.unknown_attribution_events
                ),
            });
        }
        if self.truncated
            && !self
                .warnings
                .iter()
                .any(|warning| warning.contains("limit reached"))
        {
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: "The scan reached a file, byte or event bound. The summary is incomplete."
                    .into(),
            });
        }
        for warning in &self.warnings {
            if alerts.iter().any(|alert| &alert.message == warning) {
                continue;
            }
            alerts.push(Alert {
                level: AlertLevel::Warning,
                message: warning.clone(),
            });
        }
        alerts.extend(self.budget_alerts(monthly_budget_usd));
        alerts
    }

    pub fn budget_alerts(&self, monthly_budget_usd: Option<f64>) -> Vec<Alert> {
        let mut alerts = Vec::new();
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

impl Summary {
    /// Copyable manual-QA evidence. It contains aggregate counts, file paths and
    /// warnings only; never prompts, responses, environment values or secrets.
    pub fn qa_report(&self) -> String {
        let generated = chrono::DateTime::from_timestamp_millis(self.generated_ms as i64)
            .map(|at| at.to_rfc3339())
            .unwrap_or_else(|| "unknown".into());
        let mut out = String::new();
        out.push_str("# Billing evidence QA report\n");
        out.push_str(&format!("Generated: {generated}\n"));
        out.push_str(&format!(
            "Totals: {} events, {} priced cost, {} unpriced, {} files, {} bytes, truncated={}\n",
            self.totals.events,
            self.totals.cost_label(),
            self.totals.unpriced_events,
            self.files,
            self.bytes,
            self.truncated
        ));
        out.push_str(&format!(
            "Attribution: {} main, {} subagent, {} internal, {} unknown\n",
            self.totals.main_events,
            self.totals.subagent_events,
            self.totals.internal_events,
            self.totals.unknown_attribution_events
        ));
        out.push_str(&format!(
            "Known attributed cost: subagents ${:.4}, internal ${:.4}, unknown ${:.4}\n",
            self.totals.subagent_cost_usd,
            self.totals.internal_cost_usd,
            self.totals.unknown_attribution_cost_usd
        ));
        out.push_str("\n## Sources\n");
        if self.sources.is_empty() {
            out.push_str("No supported provider logs were found.\n");
        }
        for source in self.sources.iter().take(200) {
            out.push_str(&format!(
                "- {} · {} · {} bytes · offset {} · {} events · {}\n",
                source.provider,
                source.path,
                source.bytes,
                source.offset,
                source.events,
                source.state
            ));
        }
        if self.sources.len() > 200 {
            out.push_str(&format!(
                "- … {} more sources omitted\n",
                self.sources.len() - 200
            ));
        }
        out.push_str("\n## Unpriced models\n");
        if self.unpriced_models.is_empty() {
            out.push_str("None.\n");
        } else {
            for model in &self.unpriced_models {
                out.push_str(&format!("- {model}\n"));
            }
        }
        out.push_str("\n## Warnings\n");
        if self.warnings.is_empty() {
            out.push_str("None.\n");
        } else {
            for warning in &self.warnings {
                out.push_str(&format!("- {warning}\n"));
            }
        }
        out
    }
}

/// Bounded aggregate payload that a user may explicitly sync to a self-hosted
/// relay. It never contains raw provider log lines or prompts.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Evidence {
    pub version: u32,
    pub device: String,
    pub generated_ms: u64,
    pub summary: Summary,
}

impl Evidence {
    pub const VERSION: u32 = 1;

    pub fn new(device: impl Into<String>, mut summary: Summary) -> Self {
        // The relay accepts bounded diagnostics; aggregate totals still cover the full archive.
        summary.by_day.truncate(400);
        summary.sources.truncate(4_000);
        Self {
            version: Self::VERSION,
            device: device.into(),
            generated_ms: summary.generated_ms,
            summary,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.version != Self::VERSION {
            return Err("Unsupported billing evidence version".into());
        }
        if self.device.trim().is_empty()
            || self.device.len() > 128
            || self.device.chars().any(char::is_control)
        {
            return Err("Invalid billing evidence device name".into());
        }
        if self.summary.sources.len() > 4_000
            || self.summary.by_day.len() > 400
            || self.summary.by_attribution.len() > 16
            || self.summary.by_provider.len() > 100
            || self.summary.by_harness.len() > 100
            || self.summary.by_model.len() > 500
            || self.summary.by_project.len() > 500
            || self.summary.warnings.len() > 64
            || self.summary.unpriced_models.len() > 500
        {
            return Err("Billing evidence exceeds the relay limits".into());
        }
        if self.summary.totals.events > 1_000_000 || self.summary.files > 10_000 {
            return Err("Billing evidence exceeds the event/file limits".into());
        }
        Ok(())
    }
}

fn scan_batches(root: &Path, scanner: fn(&Path, &mut Checkpoint) -> ScanReport) -> ScanReport {
    let mut checkpoint = Checkpoint::default();
    let mut result = ScanReport::default();
    let mut sources = BTreeMap::<String, SourceFile>::new();
    loop {
        let before = (
            checkpoint.files.len(),
            checkpoint.files.values().map(|f| f.offset).sum::<u64>(),
        );
        let mut batch = scanner(root, &mut checkpoint);
        let pending = batch.truncated;
        result.bytes += batch.bytes;
        result.events.append(&mut batch.events);
        for mut source in batch.sources {
            if let Some(previous) = sources.get(&source.path) {
                source.events += previous.events;
            }
            sources.insert(source.path.clone(), source);
        }
        for warning in batch.warnings {
            if !matches!(
                warning.as_str(),
                "Scan batch limit reached; additional provider logs remain."
                    | "File discovery limit reached; additional logs may remain."
                    | "Event limit reached; older or additional log lines were not read."
                    | "File limit reached; additional provider logs were not read."
            ) {
                result.warn(warning);
            }
        }
        let after = (
            checkpoint.files.len(),
            checkpoint.files.values().map(|f| f.offset).sum::<u64>(),
        );
        if !pending || after == before || checkpoint.seen.len() >= MAX_SEEN {
            result.truncated = pending;
            if pending {
                result.warn("Scan limit reached; the summary is incomplete. Remaining logs were not included.");
            }
            break;
        }
    }
    result.files = sources.len();
    result.sources = sources.into_values().collect();
    result.checkpoint = checkpoint;
    result
}

/// Scan supported sources in bounded batches; cached bytes never consume the next batch.
pub fn scan_all() -> ScanReport {
    let mut report = ScanReport::default();
    let mut checkpoint = Checkpoint::default();
    if let Some(root) = default_claude_root() {
        let mut claude = scan_batches(&root, scan_claude_report);
        checkpoint.files.append(&mut claude.checkpoint.files);
        checkpoint.seen.append(&mut claude.checkpoint.seen);
        report.events.append(&mut claude.events);
        report.sources.append(&mut claude.sources);
        report.warnings.append(&mut claude.warnings);
        report.files += claude.files;
        report.bytes = report.bytes.saturating_add(claude.bytes);
        report.truncated |= claude.truncated;
    }
    if let Some(root) = default_codex_root() {
        let mut codex = scan_batches(&root, scan_codex_report);
        checkpoint.files.append(&mut codex.checkpoint.files);
        checkpoint.seen.append(&mut codex.checkpoint.seen);
        report.events.append(&mut codex.events);
        report.sources.append(&mut codex.sources);
        report.warnings.append(&mut codex.warnings);
        report.files += codex.files;
        report.bytes = report.bytes.saturating_add(codex.bytes);
        report.truncated |= codex.truncated;
    }
    if let Some(path) = default_opencode_db() {
        let key = path.to_string_lossy().to_string();
        let metadata = fs::metadata(&path).ok();
        match scan_opencode_db(&path) {
            Ok(events) => {
                let mut counted = 0;
                for event in events {
                    if checkpoint.seen.insert(event.key.clone()) {
                        report.events.push(event);
                        counted += 1;
                    }
                }
                if let Some(metadata) = metadata {
                    report.files += 1;
                    report.bytes = report.bytes.saturating_add(metadata.len());
                    checkpoint.files.insert(
                        key.clone(),
                        FileCheckpoint {
                            offset: 0,
                            size: metadata.len(),
                            mtime_ms: mtime_ms(&metadata),
                            ..Default::default()
                        },
                    );
                    report.sources.push(SourceFile {
                        path: key,
                        provider: "OpenCode".into(),
                        bytes: metadata.len(),
                        mtime_ms: mtime_ms(&metadata),
                        offset: 0,
                        events: counted,
                        state: "scanned".into(),
                    });
                }
            }
            Err(error) => {
                report.warn(format!("OpenCode billing: {error}"));
                if let Some(metadata) = metadata {
                    report.sources.push(SourceFile {
                        path: key,
                        provider: "OpenCode".into(),
                        bytes: metadata.len(),
                        mtime_ms: mtime_ms(&metadata),
                        offset: 0,
                        events: 0,
                        state: "error".into(),
                    });
                }
            }
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
    fn opus_55_uses_verified_prices_and_one_hour_cache_writes() {
        let value = serde_json::json!({"type":"assistant", "sessionId":"main", "timestamp":"2026-11-05T10:00:00Z",
            "message":{"id":"opus55", "model":"claude-opus-5-5", "usage":{"input_tokens":1000000, "output_tokens":1000000,
                "cache_read_input_tokens":1000000, "cache_creation_input_tokens":1000000,
                "cache_creation":{"ephemeral_1h_input_tokens":1000000}}}});
        assert!((claude_event(&value).unwrap().cost_usd.unwrap() - 32.2).abs() < 1e-9);
        assert_eq!(price("claude-opus-4-6", 0).unwrap().input, 5.);
        assert!(price("claude-opus-5-6", 0).is_none());
    }

    #[test]
    fn sonnet_and_haiku_55_prices_include_cache_and_context_tiers() {
        let sonnet = price("anthropic/claude-sonnet-5.5-20260928", 1_000_000).unwrap();
        assert_eq!(
            (
                sonnet.input,
                sonnet.output,
                sonnet.cache_read,
                sonnet.cache_write
            ),
            (2., 10., 0.1, 2.5)
        );
        let low = price("claude-haiku-5-5", 100_000).unwrap();
        let high = price("claude-haiku-5-5", 100_001).unwrap();
        assert_eq!(
            (low.input, low.output, low.cache_read, low.cache_write),
            (0.1, 0.5, 0.01, 0.125)
        );
        assert_eq!(
            (high.input, high.output, high.cache_read, high.cache_write),
            (0.5, 2.5, 0.05, 0.625)
        );
        let value = serde_json::json!({"type":"assistant","timestamp":"2026-11-06T10:00:00Z",
            "message":{"id":"cache","model":"claude-haiku-5-5","usage":{
                "input_tokens":1,"cache_creation_input_tokens":100000,
                "cache_creation":{"ephemeral_1h_input_tokens":100000}}}});
        assert!((claude_event(&value).unwrap().cost_usd.unwrap() - 0.1000005).abs() < 1e-9);
        assert!(
            price("codex-auto-review", 100).is_none(),
            "a backend alias is not proof of API pricing"
        );
    }

    #[test]
    fn only_zero_token_synthetic_events_are_known_to_be_free() {
        let mut value = serde_json::json!({"type":"assistant","timestamp":"2026-11-06T10:00:00Z",
            "message":{"id":"synthetic","model":"<synthetic>","usage":{}}});
        let free = claude_event(&value).unwrap();
        assert_eq!(free.cost_usd, Some(0.));
        assert_eq!(free.attribution.kind, WorkKind::Internal);
        value["message"]["usage"]["input_tokens"] = 1.into();
        assert!(claude_event(&value).unwrap().cost_usd.is_none());
        value["message"]["model"] = "unknown-model".into();
        value["message"]["usage"]["input_tokens"] = 0.into();
        assert!(claude_event(&value).unwrap().cost_usd.is_none());
    }

    #[test]
    fn ranges_filter_every_breakdown_at_exact_rolling_boundaries() {
        let now = timestamp_ms("2026-11-06T12:00:00Z").unwrap();
        let day = 86_400_000;
        let event = claude_event(
            &serde_json::json!({"type":"assistant","timestamp":"2026-11-06T10:00:00Z",
            "cwd":"/fixture","sessionId":"s","message":{"id":"m","model":"claude-sonnet-5-5",
            "usage":{"input_tokens":1000}}}),
        )
        .unwrap();
        let dates = [
            now,
            now - day,
            now - day - 1,
            now - 7 * day,
            now - 7 * day - 1,
            now - 30 * day,
            now - 30 * day - 1,
            0,
            now + 1,
        ];
        let report = ScanReport {
            events: dates
                .into_iter()
                .map(|at_ms| BillingEvent {
                    at_ms,
                    ..event.clone()
                })
                .collect(),
            ..Default::default()
        };
        for (range, count) in [
            (TimeRange::Day, 2),
            (TimeRange::Week, 4),
            (TimeRange::Month, 6),
            (TimeRange::All, 9),
        ] {
            let summary = summarize_range(&report, range, now);
            assert_eq!(summary.totals.events, count);
            for slices in [
                &summary.by_harness,
                &summary.by_model,
                &summary.by_provider,
                &summary.by_project,
                &summary.by_attribution,
            ] {
                assert_eq!(
                    slices.iter().map(|slice| slice.totals.events).sum::<u64>(),
                    count
                );
            }
            assert_eq!(
                summary
                    .by_day
                    .iter()
                    .map(|day| day.totals.events)
                    .sum::<u64>(),
                count
            );
        }
        assert!(
            summarize_range(&report, TimeRange::All, now)
                .by_day
                .iter()
                .any(|day| day.date == "unknown")
        );
    }

    #[test]
    fn all_keeps_old_daily_totals_while_relay_diagnostics_stay_bounded() {
        let now = timestamp_ms("2026-11-06T12:00:00Z").unwrap();
        let event = claude_event(
            &serde_json::json!({"type":"assistant","timestamp":"2026-11-06T10:00:00Z",
            "message":{"id":"m","model":"claude-sonnet-5-5","usage":{"input_tokens":1}}}),
        )
        .unwrap();
        let report = ScanReport {
            events: (0..450)
                .map(|days| BillingEvent {
                    at_ms: now - days * 86_400_000,
                    ..event.clone()
                })
                .collect(),
            ..Default::default()
        };
        let summary = summarize_range(&report, TimeRange::All, now);
        assert_eq!(summary.by_day.len(), 450);
        let totals = summary.totals.clone();
        let evidence = Evidence::new("fixture", summary);
        evidence.validate().unwrap();
        assert_eq!(evidence.summary.by_day.len(), 400);
        assert_eq!(evidence.summary.totals, totals);
    }

    #[test]
    fn metadata_projection_reads_large_claude_records_without_retaining_content() {
        let root = fixture("large-record");
        let value = serde_json::json!({"type":"assistant","timestamp":"2026-11-06T10:00:00Z",
            "session_id":"child","agent_id":"worker","cwd":"/fixture",
            "message":{"id":"large","model":"claude-sonnet-5-5","usage":{"input_tokens":100},
                "content":[{"type":"text","text":"private text".repeat(200_000)}]}});
        let text = value.to_string();
        assert!(text.len() > 1024 * 1024);
        let metadata: jsonl::Metadata = serde_json::from_str(&text).unwrap();
        let projected = serde_json::to_value(metadata).unwrap();
        assert!(!projected.to_string().contains("private text"));
        assert!(projected.get("payload").is_none());
        assert_eq!(claude_event(&value), claude_event(&projected));
        fs::write(root.join("large.jsonl"), format!("{text}\n")).unwrap();
        let report = scan_batches(&root, scan_claude_report);
        assert_eq!(report.events.len(), 1);
        assert_eq!(report.events[0].input_tokens, 100);
        assert!(report.warnings.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_discovery_continues_past_one_batch_without_duplicates() {
        let root = fixture("file-pages");
        for id in 0..MAX_FILES + 3 {
            fs::write(
                root.join(format!("{id}.jsonl")),
                format!(
                    "{}\n",
                    serde_json::json!({
                "type":"assistant","timestamp":"2026-11-06T10:00:00Z", "message":{
                    "id":id.to_string(),"model":"claude-sonnet-5-5","usage":{"input_tokens":1}}})
                ),
            )
            .unwrap();
        }
        let report = scan_batches(&root, scan_claude_report);
        assert_eq!(report.events.len(), MAX_FILES + 3);
        assert_eq!(report.files, MAX_FILES + 3);
        assert!(!report.truncated);
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn sqlite_billing_streams_past_100k_rows_even_with_identical_timestamps() {
        let root = fixture("sqlite-pages");
        let path = root.join("opencode.db");
        let mut connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE message (id TEXT PRIMARY KEY, data TEXT, time_created INTEGER);",
            )
            .unwrap();
        let transaction = connection.transaction().unwrap();
        let data = serde_json::json!({"role":"assistant","modelID":"claude-sonnet-5-5","tokens":{"input":1},"cost":0.1}).to_string();
        for id in 0..100_007 {
            transaction
                .execute(
                    "INSERT INTO message VALUES (?1,?2,1762426800000)",
                    [id.to_string(), data.clone()],
                )
                .unwrap();
        }
        transaction.commit().unwrap();
        let events = scan_opencode_db(&path).unwrap();
        assert_eq!(events.len(), 100_007);
        assert_eq!(
            events
                .iter()
                .map(|event| &event.key)
                .collect::<BTreeSet<_>>()
                .len(),
            events.len()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn large_jsonl_files_resume_in_bounded_batches_and_cached_bytes_are_free() {
        let root = fixture("large-batches");
        let mut file = fs::File::create(root.join("large.jsonl")).unwrap();
        let padding = "x".repeat(256 * 1024);
        for id in 0..160 {
            writeln!(file, "{}", serde_json::json!({"type":"assistant", "timestamp":"2026-11-05T10:00:00Z", "padding":padding,
                "message":{"id":format!("m{id}"),"model":"claude-opus-5-5","usage":{"input_tokens":1}}})).unwrap();
        }
        let mut checkpoint = Checkpoint::default();
        let first = scan_claude_report(&root, &mut checkpoint);
        assert!(
            first.truncated
                && !first.events.is_empty()
                && first.bytes <= MAX_BYTES + MAX_LINE as u64
        );
        let second = scan_claude_report(&root, &mut checkpoint);
        assert_eq!(first.events.len() + second.events.len(), 160);
        assert!(!second.truncated);
        assert_eq!(scan_claude_report(&root, &mut checkpoint).bytes, 0);
        let all = scan_batches(&root, scan_claude_report);
        assert_eq!(all.events.len(), 160);
        assert!(!all.truncated);
        assert!(all.warnings.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn incremental_codex_preserves_model_attribution_totals_and_line_identity() {
        let root = fixture("codex-resume");
        let path = root.join("rollout.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        writeln!(file, "{}", serde_json::json!({"type":"session_meta","payload":{"id":"main","model":"gpt-5","cwd":"/code"}})).unwrap();
        let mut checkpoint = Checkpoint::default();
        for total in [100, 120, 140] {
            writeln!(file, "{}", serde_json::json!({"type":"event_msg", "timestamp":"2026-11-05T10:00:00Z",
                "payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":total,"output_tokens":total}}}})).unwrap();
            let events = scan_codex_report(&root, &mut checkpoint).events;
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].model, "gpt-5");
            assert_eq!(events[0].input_tokens, if total == 100 { 100 } else { 20 });
            assert_eq!(events[0].attribution.kind, WorkKind::Main);
        }
        assert_eq!(checkpoint.seen.len(), 3);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn oversized_and_partial_lines_resume_without_losing_the_next_event() {
        let root = fixture("oversized-partial");
        let path = root.join("log.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(&vec![b'x'; MAX_LINE + 100]).unwrap();
        let mut checkpoint = Checkpoint::default();
        let first = scan_claude_report(&root, &mut checkpoint);
        assert!(first.events.is_empty());
        assert!(
            first
                .warnings
                .iter()
                .any(|warning| warning.contains("Oversized"))
        );
        let event = serde_json::json!({"type":"assistant", "timestamp":"2026-11-05T10:00:00Z",
            "message":{"id":"after-oversized","model":"claude-opus-5-5","usage":{"input_tokens":1}}}).to_string();
        write!(file, "\n{event}").unwrap();
        assert!(scan_claude_report(&root, &mut checkpoint).events.is_empty());
        writeln!(file).unwrap();
        let events = scan_claude_report(&root, &mut checkpoint).events;
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].input_tokens, 1);
        assert_eq!(scan_claude_report(&root, &mut checkpoint).bytes, 0);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn relational_opencode_v1_and_v2_supply_session_identity_and_parent() {
        let root = fixture("relational-opencode");
        for v2 in [false, true] {
            let path = root.join(if v2 { "v2.db" } else { "v1.db" });
            let c = Connection::open(&path).unwrap();
            let table = if v2 { "session_v2" } else { "session" };
            c.execute_batch(&format!("CREATE TABLE {table} (id TEXT, directory TEXT, parent_id TEXT, title TEXT);
                INSERT INTO {table} VALUES ('main','/repo',NULL,'Main'),('child','/repo','main','Child');")).unwrap();
            let data = serde_json::json!({"role":"assistant", "modelID":"claude-opus-5-5", "tokens":{"input":1000000,"output":1000000}}).to_string();
            if v2 {
                c.execute_batch("CREATE TABLE session_message (id TEXT, session_id TEXT, type TEXT, seq INTEGER, data TEXT, time_created INTEGER);").unwrap();
                c.execute("INSERT INTO session_message VALUES ('m','child','assistant',1,?1,1762336800000)", [&data]).unwrap();
            } else {
                c.execute_batch("CREATE TABLE message (id TEXT, data TEXT, time_created INTEGER, session_id TEXT);").unwrap();
                c.execute(
                    "INSERT INTO message VALUES ('m',?1,1762336800000,'child')",
                    [&data],
                )
                .unwrap();
            }
            let events = scan_opencode_db(&path).unwrap();
            assert_eq!(events.len(), 1);
            assert_eq!(events[0].attribution.kind, WorkKind::Subagent);
            assert_eq!(events[0].attribution.parent.as_deref(), Some("main"));
            assert_eq!(events[0].project.as_deref(), Some("/repo"));
            assert_eq!(events[0].cost_usd, Some(24.));
        }
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn duplicate_limit_alerts_are_not_repeated_or_hidden() {
        let warning = "Byte limit reached; remaining provider logs were not read.".to_string();
        let summary = summarize(ScanReport {
            truncated: true,
            warnings: vec![warning.clone(), warning.clone()],
            ..Default::default()
        });
        let alerts = summary.alerts(None);
        assert_eq!(
            alerts
                .iter()
                .filter(|alert| alert.message == warning)
                .count(),
            1
        );
        assert!(
            !alerts
                .iter()
                .any(|alert| alert.message.contains("file, byte or event bound"))
        );
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
    fn sidechains_synthetic_messages_and_unknown_records_keep_explicit_attribution() {
        let root = fixture("attribution-claude");
        let mut file = fs::File::create(root.join("log.jsonl")).unwrap();
        for value in [
            serde_json::json!({
                "type":"assistant","timestamp":"2026-11-02T10:00:00Z","sessionId":"main-1",
                "message":{"id":"main","model":"claude-sonnet-4-5","usage":{"input_tokens":10}}
            }),
            serde_json::json!({
                "type":"assistant","timestamp":"2026-11-02T10:01:00Z","sessionId":"main-1","isSidechain":true,
                "message":{"id":"child","model":"claude-sonnet-4-5","usage":{"input_tokens":10}}
            }),
            serde_json::json!({
                "type":"assistant","timestamp":"2026-11-02T10:02:00Z","sessionId":"main-1",
                "message":{"id":"synthetic","model":"<synthetic>","usage":{"input_tokens":10}}
            }),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        let events = scan_claude(&root);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].attribution.kind, WorkKind::Main);
        assert_eq!(events[1].attribution.kind, WorkKind::Subagent);
        assert_eq!(events[1].attribution.session.as_deref(), Some("main-1"));
        assert_eq!(events[2].attribution.kind, WorkKind::Internal);
        let summary = summarize(ScanReport {
            events,
            ..Default::default()
        });
        assert_eq!(summary.totals.main_events, 1);
        assert_eq!(summary.totals.subagent_events, 1);
        assert_eq!(summary.totals.internal_events, 1);
        assert_eq!(summary.totals.unknown_attribution_events, 0);
        assert!(
            summary
                .by_attribution
                .iter()
                .any(|slice| slice.label == "Subagents")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn codex_subagent_session_meta_marks_child_events_and_qa_report_is_copyable() {
        let root = fixture("attribution-codex");
        fs::create_dir_all(root.join("2026/11/02")).unwrap();
        let path = root.join("2026/11/02/rollout-child.jsonl");
        let mut file = fs::File::create(&path).unwrap();
        for value in [
            serde_json::json!({
                "timestamp":"2026-11-02T10:00:00Z","type":"session_meta",
                "payload":{"id":"child-1","parent_thread_id":"main-1","model":"gpt-5","cwd":"/code","source":{"subagent":"review"}}
            }),
            serde_json::json!({
                "timestamp":"2026-11-02T10:01:00Z","type":"event_msg",
                "payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"output_tokens":20}}}
            }),
        ] {
            writeln!(file, "{value}").unwrap();
        }
        let events = scan_codex(&root);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].attribution.kind, WorkKind::Subagent);
        assert_eq!(events[0].attribution.parent.as_deref(), Some("main-1"));
        let summary = summarize(ScanReport {
            events,
            sources: vec![SourceFile {
                path: path.to_string_lossy().into(),
                provider: "OpenAI Codex".into(),
                bytes: 10,
                mtime_ms: 0,
                offset: 10,
                events: 1,
                state: "scanned".into(),
            }],
            ..Default::default()
        });
        let report = summary.qa_report();
        assert!(report.contains("subagent"));
        assert!(report.contains("rollout-child.jsonl"));
        let evidence = Evidence::new("test-device", summary);
        evidence.validate().unwrap();
        let json = serde_json::to_string(&evidence).unwrap();
        assert!(!json.contains("prompt"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn opencode_child_sessions_are_attributed_to_their_parent() {
        let root = fixture("attribution-opencode");
        let path = root.join("opencode.db");
        {
            let connection = Connection::open(&path).unwrap();
            connection
                .execute_batch(
                    "CREATE TABLE session (id TEXT PRIMARY KEY, data TEXT NOT NULL);
                     CREATE TABLE message (id TEXT PRIMARY KEY, data TEXT NOT NULL, time_created INTEGER);
                     INSERT INTO session VALUES ('parent', '{\"directory\":\"/repo\",\"title\":\"Main\"}');
                     INSERT INTO session VALUES ('child', '{\"directory\":\"/repo\",\"title\":\"Review\",\"parentID\":\"parent\"}');
                     INSERT INTO message VALUES ('a1', '{\"role\":\"assistant\",\"sessionID\":\"child\",\"providerID\":\"anthropic\",\"modelID\":\"claude-sonnet-4-5\",\"tokens\":{\"input\":10,\"output\":2},\"cost\":0.1}', 0);",
                )
                .unwrap();
        }
        let events = scan_opencode_db(&path).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].attribution.kind, WorkKind::Subagent);
        assert_eq!(events[0].attribution.parent.as_deref(), Some("parent"));
        assert_eq!(events[0].attribution.task.as_deref(), Some("Review"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn old_summary_and_event_json_default_to_unknown_attribution_not_main() {
        let old_event: BillingEvent = serde_json::from_value(serde_json::json!({
            "key":"old","at_ms":1,"provider":"Claude Code","harness":"Claude Code",
            "model":"claude-sonnet-4-5","input_tokens":1,"output_tokens":2,
            "cache_read_tokens":0,"cache_write_tokens":0,"cost_usd":0.1,"cost_source":"priced"
        }))
        .unwrap();
        assert_eq!(old_event.attribution.kind, WorkKind::Unknown);
        let old_summary: Summary = serde_json::from_value(serde_json::json!({
            "generated_ms":1,
            "totals":{"events":0,"input_tokens":0,"output_tokens":0,"reasoning_tokens":0,
                "cache_read_tokens":0,"cache_write_tokens":0,"priced_cost_usd":0.0,
                "unpriced_events":0,"recorded_cost_events":0},
            "by_provider":[],"by_harness":[],"by_model":[],"by_project":[],"by_day":[],
            "unpriced_models":[],"warnings":[],"files":0,"bytes":0,"truncated":false
        }))
        .unwrap();
        assert!(old_summary.by_attribution.is_empty());
        assert_eq!(old_summary.totals.unknown_attribution_events, 0);
    }

    #[test]
    fn long_context_and_alert_budgets_are_explicit() {
        assert!(
            price("claude-sonnet-4-5", 200_001).unwrap().input
                > price("claude-sonnet-4-5", 200_000).unwrap().input
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
