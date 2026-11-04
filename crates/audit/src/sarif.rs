use std::fs;
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use crate::Parsed;
use crate::model::{AuditEngine, AuditSnapshot, Evidence, Finding, Location, Severity, Verdict};
use crate::util::{MAX_TEXT_BYTES, safe_relative_path, safe_text, string_field, u32_field};

fn severity(value: Option<&Value>) -> Option<Severity> {
    value.and_then(Value::as_str).and_then(Severity::parse)
}

fn message_text(result: &Value) -> String {
    result
        .get("message")
        .and_then(|message| message.get("text"))
        .and_then(Value::as_str)
        .or_else(|| {
            result
                .get("message")
                .and_then(|message| message.get("markdown"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string()
}

fn fingerprint(result: &Value, rule_id: &str, locations: &[Location]) -> String {
    if let Some(partial) = result.get("partialFingerprints").and_then(Value::as_object) {
        for value in partial.values() {
            if let Some(text) = value.as_str()
                && !text.is_empty()
            {
                return text.to_string();
            }
        }
    }
    let first = locations
        .first()
        .map(|location| format!("{}:{}", location.path, location.line))
        .unwrap_or_else(|| "unknown".to_string());
    format!("sarif:{rule_id}:{first}")
}

pub fn parse(value: &Value) -> Parsed<Vec<Finding>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Some(version) = value.get("version").and_then(Value::as_str) else {
        errors.push("SARIF: missing version".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    };
    if !version.starts_with("2.1") {
        errors.push(format!("SARIF: unsupported version `{version}`"));
    }
    let Some(runs) = value.get("runs").and_then(Value::as_array) else {
        errors.push("SARIF: expected a runs array".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    };
    let mut findings = Vec::new();
    for (run_index, run) in runs.iter().take(64).enumerate() {
        let tool = run
            .get("tool")
            .and_then(|tool| tool.get("driver"))
            .and_then(|driver| driver.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("SARIF scanner");
        let rules = run
            .get("tool")
            .and_then(|tool| tool.get("driver"))
            .and_then(|driver| driver.get("rules"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let Some(results) = run.get("results").and_then(Value::as_array) else {
            warnings.push(format!("SARIF run {run_index} has no results"));
            continue;
        };
        for (result_index, result) in results.iter().take(10_000).enumerate() {
            let rule_id = string_field(result, "ruleId")
                .or_else(|| {
                    result
                        .get("rule")
                        .and_then(|rule| rule.get("id"))
                        .and_then(Value::as_str)
                })
                .unwrap_or("sarif.unknown")
                .to_string();
            let title = rules
                .iter()
                .find(|rule| string_field(rule, "id") == Some(rule_id.as_str()))
                .and_then(|rule| rule.get("shortDescription"))
                .and_then(|description| description.get("text"))
                .and_then(Value::as_str)
                .or_else(|| string_field(result, "message"))
                .unwrap_or(&rule_id)
                .to_string();
            let description = message_text(result);
            if !safe_text(&description, MAX_TEXT_BYTES) && !description.is_empty() {
                errors.push(format!(
                    "SARIF run {run_index} result {result_index}: overlong message"
                ));
            }
            let level = string_field(result, "level")
                .or_else(|| string_field(result, "kind"))
                .map(str::to_owned);
            let severity = severity(
                result
                    .get("properties")
                    .and_then(|properties| properties.get("severity")),
            )
            .or_else(|| level.as_deref().and_then(Severity::parse));
            let mut locations = Vec::new();
            if let Some(items) = result.get("locations").and_then(Value::as_array) {
                for location in items.iter().take(64) {
                    let Some(physical) = location.get("physicalLocation") else {
                        continue;
                    };
                    let Some(path) = physical
                        .get("artifactLocation")
                        .and_then(|artifact| artifact.get("uri"))
                        .and_then(Value::as_str)
                    else {
                        continue;
                    };
                    let path = path.strip_prefix("file://").unwrap_or(path);
                    if !safe_relative_path(path) {
                        errors.push(format!(
                            "SARIF run {run_index} result {result_index}: non-relative path `{path}`"
                        ));
                        continue;
                    }
                    let line = physical
                        .get("region")
                        .and_then(|region| u32_field(region, "startLine"))
                        .unwrap_or(1);
                    locations.push(Location {
                        path: path.to_string(),
                        line,
                        end_line: physical
                            .get("region")
                            .and_then(|region| u32_field(region, "endLine")),
                        role: Some("sarif".into()),
                    });
                }
            }
            if locations.is_empty() {
                warnings.push(format!(
                    "SARIF run {run_index} result {result_index} has no source location"
                ));
            }
            let mut remediation = String::new();
            if let Some(fixes) = result.get("fixes").and_then(Value::as_array) {
                for fix in fixes.iter().take(8) {
                    if let Some(description) = fix
                        .get("description")
                        .and_then(|description| description.get("text"))
                        .and_then(Value::as_str)
                    {
                        remediation.push_str(description);
                        remediation.push('\n');
                    }
                }
            }
            let evidence = locations
                .iter()
                .map(|location| Evidence {
                    path: location.path.clone(),
                    line: location.line,
                    description: description.clone(),
                })
                .collect::<Vec<_>>();
            findings.push(Finding {
                fingerprint: fingerprint(result, &rule_id, &locations),
                external_id: Some(rule_id.clone()),
                title,
                description: if description.is_empty() {
                    rule_id.clone()
                } else {
                    description
                },
                verdict: Verdict::Reported,
                severity,
                confidence: None,
                cwe: Vec::new(),
                locations,
                root_cause: String::new(),
                remediation: remediation.trim().to_string(),
                validation: format!("Reported by {tool} SARIF; scanner-owned verification"),
                trace: Vec::new(),
                evidence,
                conditions: Vec::new(),
                blockers: Vec::new(),
                source: AuditEngine::Sarif,
            });
        }
    }
    findings.sort_by(|a, b| {
        b.severity_rank()
            .cmp(&a.severity_rank())
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    if findings.is_empty() {
        warnings.push("SARIF contains no results".into());
    }
    Parsed {
        value: findings,
        errors,
        warnings,
    }
}

fn level(finding: &Finding) -> &'static str {
    match finding.severity {
        Some(Severity::Critical | Severity::High) => "error",
        Some(Severity::Medium) => "warning",
        Some(Severity::Low | Severity::Informational) | None => "note",
    }
}

/// Export a native, report-only SARIF document. Rejected candidates are omitted;
/// confirmed, needs-validation and reported records retain their original
/// verdict in `properties` so a consumer cannot mistake them for one class.
pub fn export(snapshot: &AuditSnapshot) -> Result<String, String> {
    let mut rules = Vec::new();
    let mut results = Vec::new();
    for finding in snapshot
        .findings
        .iter()
        .filter(|finding| finding.verdict != Verdict::Rejected)
    {
        let rule_id = finding.fingerprint.clone();
        rules.push(json!({
            "id": rule_id.clone(),
            "name": finding.title.clone(),
            "shortDescription": { "text": finding.title.clone() },
            "fullDescription": { "text": finding.description.clone() },
            "properties": {
                "verdict": finding.verdict.label(),
                "engine": finding.source.label(),
                "cwe": finding.cwe.clone(),
            }
        }));
        let locations = finding
            .locations
            .iter()
            .map(|location| {
                json!({
                    "physicalLocation": {
                        "artifactLocation": { "uri": location.path },
                        "region": {
                            "startLine": location.line,
                            "endLine": location.end_line.unwrap_or(location.line)
                        }
                    },
                    "message": { "text": location.role.as_deref().unwrap_or("location") }
                })
            })
            .collect::<Vec<_>>();
        results.push(json!({
            "ruleId": rule_id,
            "level": level(finding),
            "message": {
                "text": if finding.description.is_empty() {
                    finding.title.clone()
                } else {
                    format!("{}\n\n{}", finding.title, finding.description)
                }
            },
            "locations": locations,
            "partialFingerprints": { "dunlin/v1": finding.fingerprint.clone() },
            "properties": {
                "verdict": finding.verdict.label(),
                "severity": finding.severity.map(Severity::label),
                "confidence": finding.confidence.clone(),
                "engine": finding.source.label(),
            }
        }));
    }
    let value = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "Dunlin Audit",
                    "informationUri": "https://github.com/corapoid/dunlin",
                    "rules": rules
                }
            },
            "results": results
        }]
    });
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

/// Write the export atomically beside the run artifacts. The destination parent
/// must already exist; Dunlin never creates or follows a symlinked destination.
pub fn write_export(snapshot: &AuditSnapshot, path: &Path) -> Result<(), String> {
    let text = export(snapshot)?;
    let parent = path
        .parent()
        .ok_or("export destination has no parent directory")?;
    if !parent.is_dir() {
        return Err("export destination directory does not exist".into());
    }
    let temporary = parent.join(format!(".dunlin-sarif-{}.tmp", uuid::Uuid::now_v7()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let result = (|| {
        let mut file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        file.write_all(text.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
