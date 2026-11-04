use std::collections::HashSet;

use serde_json::Value;

use crate::Parsed;
use crate::model::{
    AuditEngine, CoverageEntry, CoverageStatus, CoverageSummary, Evidence, Finding, Location,
    Severity, TraceStep, Verdict,
};
use crate::util::{
    MAX_TEXT_BYTES, clip, safe_relative_path, safe_text, string_field, string_list, u32_field,
};

fn text(value: Option<&Value>, field: &str, index: usize, errors: &mut Vec<String>) -> String {
    let Some(text) = value.and_then(Value::as_str) else {
        errors.push(format!("$[{index}].{field}: expected non-empty text"));
        return String::new();
    };
    if !safe_text(text, MAX_TEXT_BYTES) {
        errors.push(format!("$[{index}].{field}: invalid or overlong text"));
        return clip(text, 1024);
    }
    text.to_string()
}

fn path(value: Option<&Value>, field: &str, index: usize, errors: &mut Vec<String>) -> String {
    let value = text(value, field, index, errors);
    if !value.is_empty() && !safe_relative_path(&value) {
        errors.push(format!(
            "$[{index}].{field}: expected a repository-relative path"
        ));
    }
    value
}

fn parse_trace(record: &Value, index: usize, errors: &mut Vec<String>) -> Vec<TraceStep> {
    let Some(items) = record.get("trace").and_then(Value::as_array) else {
        errors.push(format!("$[{index}].trace: expected an array"));
        return Vec::new();
    };
    if items.is_empty() || items.len() > 256 {
        errors.push(format!("$[{index}].trace: expected 1-256 entries"));
        return Vec::new();
    }
    let mut steps = Vec::new();
    for (step_index, item) in items.iter().enumerate() {
        let base = format!("$[{index}].trace[{step_index}]");
        let kind = string_field(item, "kind").unwrap_or_default().to_string();
        if !matches!(kind.as_str(), "entrypoint" | "propagation" | "sink") {
            errors.push(format!(
                "{base}.kind: expected entrypoint, propagation or sink"
            ));
        }
        let file = path(item.get("file"), "file", index, errors);
        let line = u32_field(item, "line").unwrap_or(0);
        if line == 0 {
            errors.push(format!("{base}.line: expected a positive integer"));
        }
        let scope = text(item.get("scope"), "scope", index, errors);
        let description = text(item.get("description"), "description", index, errors);
        steps.push(TraceStep {
            kind,
            path: file,
            line,
            scope,
            description,
        });
    }
    if steps.len() == 1 && !matches!(steps[0].kind.as_str(), "entrypoint" | "sink") {
        errors.push(format!(
            "$[{index}].trace: a single-step trace must be entrypoint or sink"
        ));
    }
    if steps.len() > 1 {
        if steps.first().map(|step| step.kind.as_str()) != Some("entrypoint") {
            errors.push(format!("$[{index}].trace: must begin with entrypoint"));
        }
        if steps.last().map(|step| step.kind.as_str()) != Some("sink") {
            errors.push(format!("$[{index}].trace: must end with sink"));
        }
        for step in &steps[1..steps.len().saturating_sub(1)] {
            if step.kind != "propagation" {
                errors.push(format!(
                    "$[{index}].trace: middle steps must be propagation"
                ));
                break;
            }
        }
    }
    steps
}

fn parse_evidence(record: &Value, index: usize, errors: &mut Vec<String>) -> Vec<Evidence> {
    let Some(items) = record.get("evidence").and_then(Value::as_array) else {
        errors.push(format!("$[{index}].evidence: expected an array"));
        return Vec::new();
    };
    if items.is_empty() || items.len() > 256 {
        errors.push(format!("$[{index}].evidence: expected 1-256 entries"));
        return Vec::new();
    }
    let mut evidence = Vec::new();
    for (item_index, item) in items.iter().enumerate() {
        let file = path(item.get("file"), "file", index, errors);
        let line = u32_field(item, "line").unwrap_or(0);
        if line == 0 {
            errors.push(format!(
                "$[{index}].evidence[{item_index}].line: expected a positive integer"
            ));
        }
        let description = text(item.get("description"), "description", index, errors);
        evidence.push(Evidence {
            path: file,
            line,
            description,
        });
    }
    evidence
}

fn parse_locations(trace: &[TraceStep], evidence: &[Evidence]) -> Vec<Location> {
    let mut locations = Vec::new();
    for step in trace {
        if locations
            .iter()
            .any(|location: &Location| location.path == step.path && location.line == step.line)
        {
            continue;
        }
        locations.push(Location {
            path: step.path.clone(),
            line: step.line,
            end_line: None,
            role: Some(step.kind.clone()),
        });
    }
    for item in evidence {
        if locations
            .iter()
            .any(|location| location.path == item.path && location.line == item.line)
        {
            continue;
        }
        locations.push(Location {
            path: item.path.clone(),
            line: item.line,
            end_line: None,
            role: Some("evidence".into()),
        });
    }
    locations
}

fn parse_conditions(record: &Value) -> Vec<String> {
    record
        .get("conditions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let kind = item.get("kind").and_then(Value::as_str)?;
                    let description = item.get("description").and_then(Value::as_str)?;
                    Some(format!("{kind}: {description}"))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_execution(record: &Value) -> String {
    let Some(execution) = record.get("execution") else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(attacker) = execution
        .get("attacker_perspective")
        .and_then(Value::as_str)
    {
        parts.push(format!("Attacker perspective: {attacker}"));
    }
    let payloads = string_list(execution.get("payloads"));
    if !payloads.is_empty() {
        parts.push(format!("Payloads: {}", payloads.join(" | ")));
    }
    let instructions = string_list(execution.get("instructions"));
    if !instructions.is_empty() {
        parts.push(format!("Instructions: {}", instructions.join(" ")));
    }
    if let Some(result) = execution.get("observed_result").and_then(Value::as_str) {
        parts.push(format!("Observed result: {result}"));
    }
    parts.join("\n")
}

fn parse_remediation(record: &Value) -> String {
    let Some(remediation) = record.get("remediation") else {
        return String::new();
    };
    let mut parts = Vec::new();
    if let Some(strategy) = remediation.get("strategy").and_then(Value::as_str) {
        parts.push(strategy.to_string());
    }
    if let Some(changes) = remediation.get("code_changes").and_then(Value::as_array) {
        for change in changes {
            let file = change.get("file_name").and_then(Value::as_str);
            let code = change.get("fixed_code").and_then(Value::as_str);
            if let (Some(file), Some(code)) = (file, code) {
                parts.push(format!("{file}:\n{code}"));
            }
        }
    }
    parts.join("\n\n")
}

fn parse_severity(record: &Value) -> Option<Severity> {
    record
        .get("severity")
        .and_then(|severity| severity.get("overall_severity"))
        .and_then(Value::as_str)
        .and_then(Severity::parse)
}

fn parse_confidence(record: &Value) -> Option<String> {
    let confidence = record.get("confidence")?;
    let score = confidence.get("score").and_then(Value::as_str)?;
    let reason = confidence.get("reason").and_then(Value::as_str);
    Some(match reason {
        Some(reason) => format!("{score}: {reason}"),
        None => score.to_string(),
    })
}

fn validate_verdict_fields(
    record: &Value,
    verdict: Verdict,
    index: usize,
    errors: &mut Vec<String>,
) {
    let has = |key: &str| record.get(key).is_some();
    match verdict {
        Verdict::Confirmed => {
            for field in [
                "root_cause",
                "intended_behavior",
                "conditions",
                "execution",
                "remediation",
                "severity",
                "confidence",
            ] {
                if !has(field) {
                    errors.push(format!("$[{index}]: confirmed record is missing `{field}`"));
                }
            }
            for field in [
                "claimed_root_cause",
                "blockers",
                "validation_plan",
                "reason",
            ] {
                if has(field) {
                    errors.push(format!(
                        "$[{index}].{field}: not valid on a confirmed record"
                    ));
                }
            }
        }
        Verdict::NeedsValidation => {
            for field in ["claimed_root_cause", "blockers", "validation_plan"] {
                if !has(field) {
                    errors.push(format!(
                        "$[{index}]: needs_validation record is missing `{field}`"
                    ));
                }
            }
            for field in ["severity", "execution", "remediation", "reason"] {
                if has(field) {
                    errors.push(format!(
                        "$[{index}].{field}: not valid on a needs_validation record"
                    ));
                }
            }
        }
        Verdict::Rejected => {
            for field in ["claimed_root_cause", "reason"] {
                if !has(field) {
                    errors.push(format!("$[{index}]: rejected record is missing `{field}`"));
                }
            }
            for field in [
                "severity",
                "execution",
                "remediation",
                "blockers",
                "validation_plan",
            ] {
                if has(field) {
                    errors.push(format!(
                        "$[{index}].{field}: not valid on a rejected record"
                    ));
                }
            }
        }
        Verdict::Reported => {}
    }
}

fn parse_record(record: &Value, index: usize, errors: &mut Vec<String>) -> Option<Finding> {
    let verdict = match string_field(record, "verdict") {
        Some("confirmed") => Verdict::Confirmed,
        Some("needs_validation") => Verdict::NeedsValidation,
        Some("rejected") => Verdict::Rejected,
        other => {
            errors.push(format!(
                "$[{index}].verdict: expected confirmed, needs_validation or rejected (got {:?})",
                other
            ));
            return None;
        }
    };
    validate_verdict_fields(record, verdict, index, errors);
    let fingerprint = text(record.get("fingerprint"), "fingerprint", index, errors);
    if fingerprint.is_empty() {
        return None;
    }
    if !fingerprint
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-@+".contains(&byte))
    {
        errors.push(format!(
            "$[{index}].fingerprint: invalid fingerprint characters"
        ));
    }
    let title = text(record.get("title"), "title", index, errors);
    let description = text(record.get("description"), "description", index, errors);
    let root_cause = text(
        record.get(if verdict == Verdict::Confirmed {
            "root_cause"
        } else {
            "claimed_root_cause"
        }),
        if verdict == Verdict::Confirmed {
            "root_cause"
        } else {
            "claimed_root_cause"
        },
        index,
        errors,
    );
    let trace = parse_trace(record, index, errors);
    let evidence = parse_evidence(record, index, errors);
    let locations = parse_locations(&trace, &evidence);
    let mut validation = parse_execution(record);
    if verdict == Verdict::NeedsValidation
        && let Some(plan) = record.get("validation_plan")
    {
        if let Some(local) = plan.get("local").and_then(Value::as_str) {
            validation.push_str(&format!("Local validation: {local}\n"));
        }
        if let Some(deployment) = plan.get("deployment").and_then(Value::as_str) {
            validation.push_str(&format!("Deployment validation: {deployment}\n"));
        }
    }
    if verdict == Verdict::Rejected
        && let Some(reason) = string_field(record, "reason")
    {
        validation.push_str(&format!("Rejected: {reason}"));
    }
    Some(Finding {
        fingerprint,
        external_id: None,
        title,
        description,
        verdict,
        severity: (verdict == Verdict::Confirmed)
            .then(|| parse_severity(record))
            .flatten(),
        confidence: parse_confidence(record),
        cwe: Vec::new(),
        locations,
        root_cause,
        remediation: parse_remediation(record),
        validation: validation.trim().to_string(),
        trace,
        evidence,
        conditions: parse_conditions(record),
        blockers: string_list(record.get("blockers")),
        source: AuditEngine::Cloudflare,
    })
}

pub fn parse_findings(value: &Value) -> Parsed<Vec<Finding>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Some(records) = value.as_array() else {
        errors.push("findings.json: expected a top-level array".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    };
    if records.len() > 10_000 {
        errors.push("findings.json: exceeds 10,000 records".into());
        return Parsed {
            value: Vec::new(),
            errors,
            warnings,
        };
    }
    let mut findings = Vec::new();
    let mut fingerprints = HashSet::new();
    for (index, record) in records.iter().enumerate() {
        if !record.is_object() {
            errors.push(format!("$[{index}]: expected an object"));
            continue;
        }
        if let Some(finding) = parse_record(record, index, &mut errors) {
            if !fingerprints.insert(finding.fingerprint.clone()) {
                errors.push(format!(
                    "$[{index}].fingerprint: duplicate fingerprint `{}`",
                    finding.fingerprint
                ));
                continue;
            }
            findings.push(finding);
        }
    }
    findings.sort_by(|a, b| {
        a.verdict
            .rank()
            .cmp(&b.verdict.rank())
            .then_with(|| b.severity_rank().cmp(&a.severity_rank()))
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    if findings.is_empty() && records.is_empty() {
        warnings.push("findings.json contains no records".into());
    }
    Parsed {
        value: findings,
        errors,
        warnings,
    }
}

fn coverage_entry(unit: &Value, index: usize, errors: &mut Vec<String>) -> Option<CoverageEntry> {
    let id = text(unit.get("coverage_id"), "coverage_id", index, errors);
    if id.is_empty() {
        return None;
    }
    let status_text = text(unit.get("status"), "status", index, errors);
    let Some(status) = CoverageStatus::parse(&status_text) else {
        errors.push(format!("$[{index}].status: unknown status `{status_text}`"));
        return None;
    };
    let surface = text(unit.get("surface"), "surface", index, errors);
    let boundary = text(unit.get("boundary"), "boundary", index, errors);
    let subsystem = text(unit.get("subsystem"), "subsystem", index, errors);
    let attack_class = text(unit.get("attack_class"), "attack_class", index, errors);
    let reviewed_paths = string_list(unit.get("reviewed_paths"));
    for path in &reviewed_paths {
        if !safe_relative_path(path) {
            errors.push(format!(
                "$[{index}].reviewed_paths: invalid repository-relative path `{path}`"
            ));
        }
    }
    let unresolved = string_list(unit.get("unresolved"));
    let local_checks = unit
        .get("local_checks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if local_checks.len() > 1000 {
        errors.push(format!("$[{index}].local_checks: too many entries"));
    }
    for (check_index, check) in local_checks.iter().enumerate() {
        let Some(object) = check.as_object() else {
            errors.push(format!(
                "$[{index}].local_checks[{check_index}]: expected object"
            ));
            continue;
        };
        for field in [
            "agent_id",
            "reviewed_paths",
            "invariant",
            "method",
            "result",
            "artifact",
        ] {
            if !object.contains_key(field) {
                errors.push(format!(
                    "$[{index}].local_checks[{check_index}]: missing `{field}`"
                ));
            }
        }
        for path in string_list(check.get("reviewed_paths")) {
            if !safe_relative_path(&path) {
                errors.push(format!(
                    "$[{index}].local_checks[{check_index}].reviewed_paths: invalid path"
                ));
            }
        }
        if let Some(artifact) = check.get("artifact").and_then(Value::as_str)
            && !safe_relative_path(artifact)
        {
            errors.push(format!(
                "$[{index}].local_checks[{check_index}].artifact: invalid path"
            ));
        }
    }
    match status {
        CoverageStatus::Covered | CoverageStatus::Candidate | CoverageStatus::Blocked => {
            if reviewed_paths.is_empty() || local_checks.is_empty() {
                errors.push(format!(
                    "$[{index}]: status `{status_text}` requires reviewed_paths and local_checks"
                ));
            }
            if status == CoverageStatus::Candidate
                && string_list(unit.get("result_fingerprints")).is_empty()
            {
                errors.push(format!(
                    "$[{index}]: candidate status requires result_fingerprints"
                ));
            }
            if status == CoverageStatus::Blocked && unresolved.is_empty() {
                errors.push(format!("$[{index}]: blocked status requires unresolved"));
            }
        }
        CoverageStatus::Deferred | CoverageStatus::OutOfScope | CoverageStatus::NotApplicable => {
            if unresolved.is_empty() {
                errors.push(format!(
                    "$[{index}]: status `{status_text}` requires an unresolved reason"
                ));
            }
        }
        CoverageStatus::Planned | CoverageStatus::InProgress => {}
    }
    Some(CoverageEntry {
        id,
        label: if surface.is_empty() {
            "Unnamed surface".into()
        } else {
            surface
        },
        detail: [subsystem, attack_class, boundary]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" · "),
        status,
        reviewed_paths,
        unresolved,
    })
}

pub fn parse_coverage(value: &Value) -> Parsed<CoverageSummary> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let Some(records) = value.as_array() else {
        errors.push("coverage-ledger.json: expected a top-level array".into());
        return Parsed {
            value: CoverageSummary {
                present: false,
                source: AuditEngine::Cloudflare,
                ..Default::default()
            },
            errors,
            warnings,
        };
    };
    if records.len() > 10_000 {
        errors.push("coverage-ledger.json: exceeds 10,000 units".into());
    }
    let mut entries = Vec::new();
    let mut ids = HashSet::new();
    let mut previous: Option<String> = None;
    for (index, unit) in records.iter().take(10_000).enumerate() {
        if !unit.is_object() {
            errors.push(format!("$[{index}]: expected an object"));
            continue;
        }
        if let Some(entry) = coverage_entry(unit, index, &mut errors) {
            if !ids.insert(entry.id.clone()) {
                errors.push(format!(
                    "$[{index}].coverage_id: duplicate id `{}`",
                    entry.id
                ));
                continue;
            }
            if let Some(previous) = &previous
                && previous > &entry.id
            {
                errors.push(format!(
                    "$[{index}].coverage_id: units must be sorted lexicographically"
                ));
            }
            previous = Some(entry.id.clone());
            entries.push(entry);
        }
    }
    if entries.is_empty() {
        warnings.push("coverage-ledger.json contains no units".into());
    }
    Parsed {
        value: CoverageSummary {
            present: true,
            entries,
            open_questions: Vec::new(),
            source: AuditEngine::Cloudflare,
        },
        errors,
        warnings,
    }
}
