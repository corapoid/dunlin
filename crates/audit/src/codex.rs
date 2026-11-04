use serde_json::Value;

use crate::Parsed;
use crate::model::{
    AuditEngine, CoverageEntry, CoverageStatus, CoverageSummary, Evidence, Finding, Location,
    Severity, Verdict,
};
use crate::util::{
    MAX_TEXT_BYTES, safe_relative_path, safe_text, string_field, string_list, u32_field,
};

fn text(value: Option<&Value>, field: &str, index: usize, errors: &mut Vec<String>) -> String {
    let Some(text) = value.and_then(Value::as_str) else {
        errors.push(format!("$[{index}].{field}: expected non-empty text"));
        return String::new();
    };
    if !safe_text(text, MAX_TEXT_BYTES) {
        errors.push(format!("$[{index}].{field}: invalid or overlong text"));
        return String::new();
    }
    text.to_string()
}

fn severity(value: Option<&Value>) -> Option<Severity> {
    value.and_then(Value::as_str).and_then(Severity::parse)
}

fn parse_locations(value: Option<&Value>, index: usize, errors: &mut Vec<String>) -> Vec<Location> {
    let Some(items) = value.and_then(Value::as_array) else {
        errors.push(format!("$[{index}].locations: expected an array"));
        return Vec::new();
    };
    if items.is_empty() || items.len() > 256 {
        errors.push(format!("$[{index}].locations: expected 1-256 entries"));
        return Vec::new();
    }
    let mut locations = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        let path = text(item.get("path"), "path", index, errors);
        if !path.is_empty() && !safe_relative_path(&path) {
            errors.push(format!(
                "$[{index}].locations[{item_index}].path: expected repository-relative path"
            ));
        }
        let line = u32_field(item, "startLine").unwrap_or(0);
        if line == 0 {
            errors.push(format!(
                "$[{index}].locations[{item_index}].startLine: expected positive integer"
            ));
        }
        locations.push(Location {
            path,
            line,
            end_line: u32_field(item, "endLine"),
            role: item.get("role").and_then(Value::as_str).map(str::to_owned),
        });
    }
    locations
}

fn parse_code_evidence(value: Option<&Value>) -> Vec<Evidence> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let path = item.get("path")?.as_str()?.to_string();
                    let line = u32_field(item, "startLine")?;
                    let explanation = item
                        .get("explanation")
                        .and_then(Value::as_str)
                        .or_else(|| item.get("label").and_then(Value::as_str))
                        .unwrap_or("Code evidence");
                    Some(Evidence {
                        path,
                        line,
                        description: explanation.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn validation_text(record: &Value) -> String {
    let Some(validation) = record.get("validation") else {
        return String::new();
    };
    let mut parts = Vec::new();
    for key in ["summary", "result", "status", "disposition", "method"] {
        if let Some(value) = validation.get(key).and_then(Value::as_str) {
            parts.push(format!("{key}: {value}"));
        }
    }
    for item in string_list(validation.get("assertions")) {
        parts.push(format!("Assertion: {item}"));
    }
    for item in string_list(validation.get("limitations")) {
        parts.push(format!("Limit: {item}"));
    }
    parts.join("\n")
}

fn attack_path_text(record: &Value) -> (String, Vec<String>) {
    let Some(path) = record.get("attackPath") else {
        return (String::new(), Vec::new());
    };
    let mut parts = Vec::new();
    if let Some(summary) = path.get("summary").and_then(Value::as_str) {
        parts.push(summary.to_string());
    }
    if let Some(reachability) = path.get("reachability") {
        if let Some(summary) = reachability.as_str() {
            parts.push(summary.to_string());
        } else {
            for key in ["attacker", "entrypoint", "source", "sink", "outcome"] {
                if let Some(value) = reachability.get(key).and_then(Value::as_str) {
                    parts.push(format!("{key}: {value}"));
                }
            }
        }
    }
    let steps = string_list(path.get("steps"));
    (parts.join("\n"), steps)
}

fn parse_finding(record: &Value, index: usize, errors: &mut Vec<String>) -> Option<Finding> {
    if record.get("title").and_then(Value::as_str).is_none() {
        errors.push(format!("$[{index}]: missing `title`"));
    }
    let title = text(record.get("title"), "title", index, errors);
    let description = text(record.get("summary"), "summary", index, errors);
    let severity = severity(
        record
            .get("severity")
            .and_then(|severity| severity.get("level")),
    );
    let confidence = record
        .get("confidence")
        .and_then(|confidence| confidence.get("level"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cwe = string_list(
        record
            .get("taxonomy")
            .and_then(|taxonomy| taxonomy.get("cwe")),
    );
    let locations = parse_locations(record.get("locations"), index, errors);
    let evidence = parse_code_evidence(record.get("codeEvidence"));
    let fingerprint = record
        .get("fingerprints")
        .and_then(|fingerprints| fingerprints.get("primary"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            record
                .get("findingId")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| format!("codex-{index}"));
    let root_cause = record
        .get("rootCause")
        .and_then(|root| root.get("summary"))
        .and_then(Value::as_str)
        .or_else(|| record.get("rootCause").and_then(Value::as_str))
        .unwrap_or_default()
        .to_string();
    let remediation = record
        .get("remediation")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let (mut validation, steps) = attack_path_text(record);
    if !steps.is_empty() {
        validation.push_str(&format!("\nAttack path: {}", steps.join(" → ")));
    }
    let validation_from_record = validation_text(record);
    if !validation_from_record.is_empty() {
        if !validation.is_empty() {
            validation.push('\n');
        }
        validation.push_str(&validation_from_record);
    }
    let mut trace = locations
        .iter()
        .map(|location| crate::model::TraceStep {
            kind: location
                .role
                .clone()
                .unwrap_or_else(|| "location".to_string()),
            path: location.path.clone(),
            line: location.line,
            scope: "Codex Security finding".into(),
            description: title.clone(),
        })
        .collect::<Vec<_>>();
    if trace.is_empty() {
        for item in &evidence {
            trace.push(crate::model::TraceStep {
                kind: "evidence".into(),
                path: item.path.clone(),
                line: item.line,
                scope: "Codex Security finding".into(),
                description: item.description.clone(),
            });
        }
    }
    Some(Finding {
        fingerprint,
        external_id: record
            .get("occurrenceId")
            .and_then(Value::as_str)
            .map(str::to_owned),
        title,
        description,
        verdict: Verdict::Reported,
        severity,
        confidence,
        cwe,
        locations,
        root_cause,
        remediation,
        validation: validation.trim().to_string(),
        trace,
        evidence,
        conditions: Vec::new(),
        blockers: Vec::new(),
        source: AuditEngine::Codex,
    })
}

pub fn parse_findings(value: &Value) -> Parsed<Vec<Finding>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if value.get("documentType").and_then(Value::as_str) != Some("codex-security.findings") {
        errors.push("findings.json: expected codex-security.findings".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    }
    if value.get("schemaVersion").and_then(Value::as_str) != Some("1.0") {
        errors.push("findings.json: unsupported Codex Security schemaVersion".into());
    }
    let Some(records) = value.get("findings").and_then(Value::as_array) else {
        errors.push("findings.json: expected a findings array".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    };
    if records.len() > 10_000 {
        errors.push("findings.json: exceeds 10,000 records".into());
    }
    let mut findings = Vec::new();
    for (index, record) in records.iter().take(10_000).enumerate() {
        if !record.is_object() {
            errors.push(format!("$[{index}]: expected an object"));
            continue;
        }
        if let Some(finding) = parse_finding(record, index, &mut errors) {
            findings.push(finding);
        }
    }
    findings.sort_by(|a, b| {
        b.severity_rank()
            .cmp(&a.severity_rank())
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    if records.is_empty() {
        warnings.push("findings.json contains no findings".into());
    }
    Parsed {
        value: findings,
        errors,
        warnings,
    }
}

pub fn parse_coverage(value: &Value) -> Parsed<CoverageSummary> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if value.get("documentType").and_then(Value::as_str) != Some("codex-security.coverage") {
        errors.push("coverage.json: expected codex-security.coverage".into());
        return Parsed {
            value: CoverageSummary {
                present: false,
                source: AuditEngine::Codex,
                ..Default::default()
            },
            errors,
            warnings,
        };
    }
    let mut entries = Vec::new();
    if let Some(surfaces) = value.get("surfaces").and_then(Value::as_array) {
        for (index, surface) in surfaces.iter().take(10_000).enumerate() {
            let id = string_field(surface, "id").unwrap_or("surface").to_string();
            let label = string_field(surface, "label")
                .unwrap_or("Unnamed surface")
                .to_string();
            let disposition = string_field(surface, "disposition").unwrap_or("needs_follow_up");
            let status = match disposition {
                "reported" => CoverageStatus::Candidate,
                "no_issue_found" => CoverageStatus::Covered,
                "rejected" => CoverageStatus::OutOfScope,
                "not_applicable" => CoverageStatus::NotApplicable,
                "needs_follow_up" => CoverageStatus::Deferred,
                other => {
                    errors.push(format!(
                        "coverage.json.surfaces[{index}].disposition: unknown `{other}`"
                    ));
                    CoverageStatus::Deferred
                }
            };
            let notes = string_field(surface, "notes").unwrap_or_default();
            let unresolved = if status == CoverageStatus::Deferred && !notes.is_empty() {
                vec![notes.to_string()]
            } else {
                Vec::new()
            };
            entries.push(CoverageEntry {
                id,
                label,
                detail: notes.to_string(),
                status,
                reviewed_paths: Vec::new(),
                unresolved,
            });
        }
    }
    let mut open_questions = Vec::new();
    if let Some(questions) = value.get("openQuestions").and_then(Value::as_array) {
        for question in questions {
            if let Some(text) = question.get("question").and_then(Value::as_str) {
                open_questions.push(text.to_string());
            }
        }
    }
    if entries.is_empty() {
        warnings.push("coverage.json contains no surfaces".into());
    }
    Parsed {
        value: CoverageSummary {
            present: true,
            entries,
            open_questions,
            source: AuditEngine::Codex,
        },
        errors,
        warnings,
    }
}
