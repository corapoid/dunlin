use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::cloudflare;
use crate::codex;
use crate::model::{AuditEngine, AuditSnapshot, CoverageSummary, RunHeader, RunStatus, SourceRef};
use crate::sarif;
use crate::util::{clip, read_json, read_text, safe_relative_path, string_field};

const MAX_RUNS: usize = 100;
const MAX_REPORT_BYTES: usize = 512 * 1024;

pub struct CreatedRun {
    pub id: String,
    pub dir: PathBuf,
}

fn sanitize_project_id(project_id: &str) -> String {
    let sanitized = project_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '-'
            }
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "project".into()
    } else {
        sanitized
    }
}

pub fn run_root(data_dir: &Path, project_id: &str) -> PathBuf {
    data_dir
        .join("audits")
        .join(sanitize_project_id(project_id))
}

fn write_run_value(dir: &Path, value: &Value) -> Result<(), String> {
    let path = dir.join("dunlin-run.json");
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    let temporary = dir.join(format!(".dunlin-run-{}.tmp", uuid::Uuid::now_v7()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        use std::io::Write;
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, &path).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}

pub fn attach_run_session(dir: &Path, session_id: &str) -> Result<(), String> {
    if session_id.is_empty() || session_id.len() > 256 || session_id.contains('\0') {
        return Err("invalid session id".into());
    }
    let path = dir.join("dunlin-run.json");
    let mut value = read_json(&path)?;
    let Some(object) = value.as_object_mut() else {
        return Err("dunlin-run.json is not an object".into());
    };
    object.insert("session_id".into(), Value::String(session_id.to_string()));
    write_run_value(dir, &value)
}

pub fn set_run_phase(dir: &Path, phase: &str) -> Result<(), String> {
    if !matches!(
        phase,
        "reconnaissance" | "hunting" | "validation" | "reporting" | "verification" | "complete"
    ) {
        return Err("unknown audit phase".into());
    }
    let path = dir.join("dunlin-run.json");
    let mut value = read_json(&path)?;
    let Some(object) = value.as_object_mut() else {
        return Err("dunlin-run.json is not an object".into());
    };
    object.insert("phase".into(), Value::String(phase.to_string()));
    write_run_value(dir, &value)
}

pub fn import_scanner_report(
    data_dir: &Path,
    project_id: &str,
    source: &Path,
) -> Result<CreatedRun, String> {
    let text = crate::util::read_text(source)?;
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    let kind = crate::adapters::detect(&value)
        .ok_or_else(|| "Unsupported scanner report format".to_string())?;
    let parsed = crate::adapters::parse(&value)
        .ok_or_else(|| "Unsupported scanner report format".to_string())?;
    if parsed.value.is_empty() && !parsed.errors.is_empty() {
        return Err(parsed.errors.join("; "));
    }
    let created = create_run(data_dir, project_id)?;
    let report = created.dir.join("scanner-report.json");
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&report).map_err(|error| error.to_string())?;
    use std::io::Write;
    file.write_all(text.as_bytes())
        .map_err(|error| error.to_string())?;
    file.sync_all().map_err(|error| error.to_string())?;
    let stub_path = created.dir.join("dunlin-run.json");
    let mut stub = read_json(&stub_path)?;
    if let Some(object) = stub.as_object_mut() {
        object.insert(
            "engine".into(),
            Value::String(
                match kind {
                    crate::adapters::AdapterKind::Semgrep => "semgrep",
                    crate::adapters::AdapterKind::Snyk => "snyk",
                    crate::adapters::AdapterKind::Trivy => "trivy",
                    crate::adapters::AdapterKind::Gitleaks => "gitleaks",
                    crate::adapters::AdapterKind::Bandit => "bandit",
                }
                .into(),
            ),
        );
        object.insert("status".into(), Value::String("incomplete".into()));
        object.insert("phase".into(), Value::String("reporting".into()));
    }
    write_run_value(&created.dir, &stub)?;
    Ok(created)
}

pub fn create_run(data_dir: &Path, project_id: &str) -> Result<CreatedRun, String> {
    let root = run_root(data_dir, project_id);
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&root, fs::Permissions::from_mode(0o700));
    }
    let id = format!("run-{}", uuid::Uuid::now_v7());
    let dir = root.join(&id);
    fs::create_dir(&dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&dir, fs::Permissions::from_mode(0o700));
    }
    let created = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let stub = serde_json::json!({
        "id": id,
        "project_id": project_id,
        "created_at": created,
        "status": "started",
        "phase": "reconnaissance",
        "engine": "cloudflare",
        "profile": "standard",
        "policy": "source-only"
    });
    let text = serde_json::to_string_pretty(&stub).map_err(|error| error.to_string())?;
    fs::write(dir.join("dunlin-run.json"), text).map_err(|error| error.to_string())?;
    Ok(CreatedRun { id, dir })
}

fn merge_dunlin_run(snapshot: &mut AuditSnapshot, value: &Value) {
    if let Some(id) = string_field(value, "id") {
        snapshot.run.id = id.to_string();
    }
    snapshot.run.project_id = string_field(value, "project_id").map(str::to_owned);
    if let Some(created) = value.get("created_at") {
        snapshot.run.created_at = match created {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        };
    }
    if let Some(status) = string_field(value, "status") {
        snapshot.run.status = RunStatus::parse(status);
    }
    snapshot.run.phase = string_field(value, "phase").map(str::to_owned);
    if let Some(profile) = string_field(value, "profile") {
        snapshot.run.profile = Some(profile.to_string());
    }
    if let Some(engine) = string_field(value, "engine") {
        snapshot.run.engine = match engine {
            "cloudflare" => AuditEngine::Cloudflare,
            "codex" => AuditEngine::Codex,
            "sarif" => AuditEngine::Sarif,
            "semgrep" => AuditEngine::Semgrep,
            "snyk" => AuditEngine::Snyk,
            "trivy" => AuditEngine::Trivy,
            "gitleaks" => AuditEngine::Gitleaks,
            "bandit" => AuditEngine::Bandit,
            _ => snapshot.run.engine,
        };
    }
    if let Some(source) = string_field(value, "source_ref") {
        snapshot.run.source.commit = Some(source.to_string());
    }
    snapshot.run.session_id = string_field(value, "session_id").map(str::to_owned);
}

fn merge_cloudflare_metadata(snapshot: &mut AuditSnapshot, value: &Value) {
    if let Some(id) = string_field(value, "run_id") {
        snapshot.run.id = id.to_string();
    }
    snapshot.run.repo = string_field(value, "repo").map(str::to_owned);
    snapshot.run.target = string_field(value, "target").map(str::to_owned);
    snapshot.run.profile = string_field(value, "profile")
        .map(str::to_owned)
        .or(snapshot.run.profile.take());
    if let Some(source_ref) = value.get("source_ref") {
        snapshot.run.source.commit = match source_ref {
            Value::String(text) => Some(text.clone()),
            Value::Object(object) => object
                .get("commit")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
        if let Value::Object(object) = source_ref {
            snapshot.run.source.dirty = object.get("dirty").and_then(Value::as_bool);
            snapshot.run.source.captured_at = object
                .get("captured_at")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }
    if let Some(status) = string_field(value, "run_status") {
        snapshot.run.status = RunStatus::parse(status);
    }
    snapshot.run.incomplete_reason = string_field(value, "incomplete_reason").map(str::to_owned);
}

fn merge_codex_manifest(snapshot: &mut AuditSnapshot, value: &Value) {
    let scan = value.get("scan").unwrap_or(value);
    if let Some(id) = string_field(scan, "scanId").or_else(|| string_field(scan, "scan_id")) {
        snapshot.run.id = id.to_string();
    }
    if let Some(status) = string_field(scan, "status") {
        snapshot.run.status = RunStatus::parse(status);
    }
    if let Some(mode) = string_field(scan, "mode") {
        snapshot.run.profile = Some(mode.to_string());
    }
    if let Some(target) = scan.get("target") {
        snapshot.run.target = match target {
            Value::String(text) => Some(text.clone()),
            Value::Object(object) => object
                .get("path")
                .and_then(Value::as_str)
                .map(str::to_owned),
            _ => None,
        };
    }
    snapshot.run.engine = AuditEngine::Codex;
}

fn read_optional_json(dir: &Path, name: &str, errors: &mut Vec<String>) -> Option<Value> {
    let path = dir.join(name);
    if !path.exists() {
        return None;
    }
    match read_json(&path) {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(format!("{name}: {error}"));
            None
        }
    }
}

fn read_optional_text(dir: &Path, name: &str, warnings: &mut Vec<String>) -> Option<String> {
    let path = dir.join(name);
    if !path.exists() {
        return None;
    }
    match read_text(&path) {
        Ok(text) => Some(text),
        Err(error) => {
            warnings.push(format!("{name}: {error}"));
            None
        }
    }
}

fn verification_records(
    value: &Value,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Vec<crate::model::VerificationRecord> {
    let records = match value {
        Value::Array(items) => items.clone(),
        Value::Object(object) => object
            .get("verifications")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_else(|| vec![value.clone()]),
        _ => {
            errors.push("verification record: expected object or array".into());
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for (index, record) in records.iter().take(1000).enumerate() {
        let Some(fingerprint) = string_field(record, "fingerprint") else {
            errors.push(format!("verification[{index}]: missing fingerprint"));
            continue;
        };
        let Some(decision) = string_field(record, "decision")
            .or_else(|| string_field(record, "verdict"))
            .and_then(crate::model::VerificationDecision::parse)
        else {
            errors.push(format!("verification[{index}]: invalid decision"));
            continue;
        };
        let reason = string_field(record, "reason")
            .unwrap_or_default()
            .to_string();
        if reason.is_empty() {
            warnings.push(format!(
                "verification[{index}] for {fingerprint} has no reason"
            ));
        }
        out.push(crate::model::VerificationRecord {
            fingerprint: fingerprint.to_string(),
            decision,
            reason,
            verifier: string_field(record, "verifier").map(str::to_owned),
        });
    }
    out
}

fn read_verifications(
    artifacts: &Path,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Vec<crate::model::VerificationRecord> {
    let mut records = Vec::new();
    let main = artifacts.join("verification.json");
    if main.exists() {
        match read_json(&main) {
            Ok(value) => records.extend(verification_records(&value, errors, warnings)),
            Err(error) => errors.push(format!("verification.json: {error}")),
        }
    }
    if let Ok(entries) = fs::read_dir(artifacts) {
        for entry in entries.take(256).filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if !name.starts_with("verification-") || !name.ends_with(".json") {
                continue;
            }
            let path = entry.path();
            if fs::symlink_metadata(&path)
                .map(|metadata| metadata.file_type().is_symlink() || !metadata.is_file())
                .unwrap_or(true)
            {
                continue;
            }
            match read_json(&path) {
                Ok(value) => records.extend(verification_records(&value, errors, warnings)),
                Err(error) => errors.push(format!("{name}: {error}")),
            }
        }
    }
    let mut latest = std::collections::BTreeMap::new();
    for record in records {
        latest.insert(record.fingerprint.clone(), record);
    }
    latest.into_values().collect()
}

fn read_scanner_reports(
    artifacts: &Path,
    errors: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> Option<(AuditEngine, Vec<crate::model::Finding>)> {
    let mut findings = Vec::new();
    let mut engine = AuditEngine::Unknown;
    for name in [
        "scanner-report.json",
        "semgrep.json",
        "snyk.json",
        "trivy.json",
        "gitleaks.json",
        "bandit.json",
    ] {
        let path = artifacts.join(name);
        if !path.exists() {
            continue;
        }
        match read_json(&path) {
            Ok(value) => match crate::adapters::parse(&value) {
                Some(parsed) => {
                    findings.extend(parsed.value);
                    errors.extend(parsed.errors);
                    warnings.extend(parsed.warnings);
                    if engine == AuditEngine::Unknown
                        && let Some(finding) = findings.first()
                    {
                        engine = finding.source;
                    }
                }
                None => warnings.push(format!("{name}: unknown scanner report format")),
            },
            Err(error) => errors.push(format!("{name}: {error}")),
        }
    }
    if findings.is_empty() {
        None
    } else {
        Some((engine, findings))
    }
}

fn apply_engine(snapshot: &mut AuditSnapshot, engine: AuditEngine) {
    if engine != AuditEngine::Unknown {
        snapshot.run.engine = engine;
    }
}

pub fn discover_run(dir: &Path) -> AuditSnapshot {
    let mut snapshot = AuditSnapshot {
        dir: dir.to_path_buf(),
        ..Default::default()
    };
    snapshot.run.id = dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("run")
        .to_string();
    if let Ok(metadata) = fs::symlink_metadata(dir)
        && (!metadata.is_dir() || metadata.file_type().is_symlink())
    {
        snapshot
            .errors
            .push("run path is not a regular directory".into());
        return snapshot;
    }

    if let Some(value) = read_optional_json(dir, "dunlin-run.json", &mut snapshot.errors) {
        merge_dunlin_run(&mut snapshot, &value);
        snapshot.files.push("dunlin-run.json".into());
    }
    // Codex CLI runs write into a nested directory so Dunlin's own stub can
    // coexist with an empty-output-directory requirement.
    let artifacts = if dir.join("codex-results").is_dir() {
        dir.join("codex-results")
    } else {
        dir.to_path_buf()
    };
    if let Some(value) = read_optional_json(&artifacts, "run-metadata.json", &mut snapshot.errors) {
        merge_cloudflare_metadata(&mut snapshot, &value);
        snapshot.files.push("run-metadata.json".into());
    }
    if let Some(value) = read_optional_json(&artifacts, "scan-manifest.json", &mut snapshot.errors)
    {
        merge_codex_manifest(&mut snapshot, &value);
        snapshot.files.push("scan-manifest.json".into());
    }

    let findings_path = artifacts.join("findings.json");
    if findings_path.exists() {
        match read_json(&findings_path) {
            Ok(value) => {
                snapshot.files.push("findings.json".into());
                if value.is_array() {
                    let parsed = cloudflare::parse_findings(&value);
                    snapshot.findings = parsed.value;
                    snapshot.errors.extend(parsed.errors);
                    snapshot.warnings.extend(parsed.warnings);
                    apply_engine(&mut snapshot, AuditEngine::Cloudflare);
                } else if value.get("documentType").and_then(Value::as_str)
                    == Some("codex-security.findings")
                {
                    let parsed = codex::parse_findings(&value);
                    snapshot.findings = parsed.value;
                    snapshot.errors.extend(parsed.errors);
                    snapshot.warnings.extend(parsed.warnings);
                    apply_engine(&mut snapshot, AuditEngine::Codex);
                } else {
                    snapshot.errors.push(
                        "findings.json: unknown document; expected Cloudflare array or Codex findings"
                            .into(),
                    );
                }
            }
            Err(error) => snapshot.errors.push(format!("findings.json: {error}")),
        }
    } else if let Some((engine, findings)) =
        read_scanner_reports(&artifacts, &mut snapshot.errors, &mut snapshot.warnings)
    {
        snapshot.findings = findings;
        apply_engine(&mut snapshot, engine);
    } else if let Some(report) =
        read_optional_text(&artifacts, "exports/results.sarif", &mut snapshot.warnings)
            .or_else(|| read_optional_text(&artifacts, "results.sarif", &mut snapshot.warnings))
    {
        if let Ok(value) = serde_json::from_str::<Value>(&report) {
            let parsed = sarif::parse(&value);
            snapshot.findings = parsed.value;
            snapshot.errors.extend(parsed.errors);
            snapshot.warnings.extend(parsed.warnings);
            apply_engine(&mut snapshot, AuditEngine::Sarif);
            snapshot.files.push("results.sarif".into());
        } else {
            snapshot.errors.push("results.sarif: invalid JSON".into());
        }
    } else if snapshot.run.status != RunStatus::Started {
        snapshot
            .warnings
            .push("No findings.json or SARIF export found yet".into());
    }

    let cloudflare_coverage = artifacts.join("coverage-ledger.json");
    let codex_coverage = artifacts.join("coverage.json");
    if cloudflare_coverage.exists() {
        match read_json(&cloudflare_coverage) {
            Ok(value) => {
                let parsed = cloudflare::parse_coverage(&value);
                snapshot.coverage = parsed.value;
                snapshot.errors.extend(parsed.errors);
                snapshot.warnings.extend(parsed.warnings);
                snapshot.files.push("coverage-ledger.json".into());
                apply_engine(&mut snapshot, AuditEngine::Cloudflare);
            }
            Err(error) => snapshot
                .errors
                .push(format!("coverage-ledger.json: {error}")),
        }
    } else if codex_coverage.exists() {
        match read_json(&codex_coverage) {
            Ok(value) => {
                let parsed = codex::parse_coverage(&value);
                snapshot.coverage = parsed.value;
                snapshot.errors.extend(parsed.errors);
                snapshot.warnings.extend(parsed.warnings);
                snapshot.files.push("coverage.json".into());
                apply_engine(&mut snapshot, AuditEngine::Codex);
            }
            Err(error) => snapshot.errors.push(format!("coverage.json: {error}")),
        }
    } else {
        snapshot.coverage = CoverageSummary {
            present: false,
            source: snapshot.run.engine,
            ..Default::default()
        };
        if !snapshot.findings.is_empty() || snapshot.run.status != RunStatus::Started {
            snapshot
                .warnings
                .push("No coverage artifact found; completeness is unknown".into());
        }
    }

    for name in [
        "REPORT.md",
        "report.md",
        "FINDINGS-DETAIL.md",
        "NEEDS-VALIDATION.md",
    ] {
        if let Some(text) = read_optional_text(&artifacts, name, &mut snapshot.warnings) {
            if snapshot.report.is_none() {
                snapshot.report = Some(clip(&text, MAX_REPORT_BYTES));
                snapshot.report_name = Some(name.to_string());
            }
            snapshot.files.push(name.to_string());
        }
    }

    snapshot.verifications =
        read_verifications(&artifacts, &mut snapshot.errors, &mut snapshot.warnings);
    if snapshot.run.engine == AuditEngine::Unknown {
        snapshot.run.engine = snapshot.coverage.source;
    }
    if snapshot.run.status == RunStatus::Started
        && (!snapshot.findings.is_empty() || snapshot.coverage.present || snapshot.report.is_some())
    {
        snapshot.run.status = RunStatus::Running;
    }
    if snapshot.run.status == RunStatus::Unknown && !snapshot.errors.is_empty() {
        snapshot.run.status = RunStatus::Incomplete;
    }
    snapshot
}

pub fn discover_runs(root: &Path) -> Vec<AuditSnapshot> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut runs = entries
        .take(MAX_RUNS + 1)
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).ok()?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return None;
            }
            Some(discover_run(&path))
        })
        .collect::<Vec<_>>();
    runs.sort_by(|a, b| {
        b.run
            .created_at
            .cmp(&a.run.created_at)
            .then_with(|| a.run.id.cmp(&b.run.id))
    });
    runs.truncate(MAX_RUNS);
    runs
}

/// Validate a path that came from an audit artifact before it is shown or
/// opened in the editor.
pub fn valid_source_path(path: &str) -> bool {
    safe_relative_path(path)
}

/// Summarize source references from a run for UI headers.
pub fn source_label(run: &RunHeader) -> String {
    if let Some(commit) = &run.source.commit {
        if run.source.dirty == Some(true) {
            format!("{commit} · dirty")
        } else {
            commit.clone()
        }
    } else {
        "source ref unknown".into()
    }
}

pub fn empty_source() -> SourceRef {
    SourceRef::default()
}
