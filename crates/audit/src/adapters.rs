//! Importers for common scanner JSON reports.
//!
//! These adapters never execute the scanner. They normalize a saved report into
//! the same report-only finding model as Codex and SARIF.

use std::collections::HashSet;

use serde_json::Value;

use crate::Parsed;
use crate::model::{AuditEngine, Evidence, Finding, Location, Severity, Verdict};
use crate::util::{
    MAX_TEXT_BYTES, safe_relative_path, safe_text, string_field, string_list, u32_field,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdapterKind {
    Semgrep,
    Snyk,
    Trivy,
    Gitleaks,
    Bandit,
}

impl AdapterKind {
    pub fn engine(self) -> AuditEngine {
        match self {
            Self::Semgrep => AuditEngine::Semgrep,
            Self::Snyk => AuditEngine::Snyk,
            Self::Trivy => AuditEngine::Trivy,
            Self::Gitleaks => AuditEngine::Gitleaks,
            Self::Bandit => AuditEngine::Bandit,
        }
    }
    pub fn label(self) -> &'static str {
        self.engine().label()
    }
}

pub fn detect(value: &Value) -> Option<AdapterKind> {
    if value.get("vulnerabilities").is_some_and(Value::is_array) {
        return Some(AdapterKind::Snyk);
    }
    if value.get("Results").is_some_and(Value::is_array) {
        return Some(AdapterKind::Trivy);
    }
    if let Some(results) = value.get("results").and_then(Value::as_array) {
        if results
            .first()
            .is_some_and(|item| item.get("check_id").is_some())
        {
            return Some(AdapterKind::Semgrep);
        }
        if results
            .first()
            .is_some_and(|item| item.get("issue_text").is_some())
        {
            return Some(AdapterKind::Bandit);
        }
    }
    if value.as_array().is_some_and(|items| {
        items
            .first()
            .is_some_and(|item| item.get("RuleID").is_some() && item.get("File").is_some())
    }) {
        return Some(AdapterKind::Gitleaks);
    }
    None
}

fn text(value: Option<&Value>, max: usize) -> Option<String> {
    let value = value.and_then(Value::as_str)?;
    if value.is_empty() || !safe_text(value, max.min(MAX_TEXT_BYTES)) {
        return None;
    }
    Some(value.to_string())
}

fn location(path: Option<&Value>, line: Option<u32>) -> Option<Location> {
    let path = path.and_then(Value::as_str)?;
    if !safe_relative_path(path) {
        return None;
    }
    Some(Location {
        path: path.to_string(),
        line: line.unwrap_or(1).max(1),
        end_line: None,
        role: Some("scanner".into()),
    })
}

#[allow(clippy::too_many_arguments)]
fn make_finding(
    engine: AuditEngine,
    fingerprint: String,
    external_id: Option<String>,
    title: String,
    description: String,
    severity: Option<Severity>,
    confidence: Option<String>,
    cwe: Vec<String>,
    locations: Vec<Location>,
    remediation: String,
) -> Finding {
    let trace = locations
        .iter()
        .map(|location| crate::model::TraceStep {
            kind: "sink".into(),
            path: location.path.clone(),
            line: location.line,
            scope: "scanner report".into(),
            description: title.clone(),
        })
        .collect();
    let evidence = locations
        .iter()
        .map(|location| Evidence {
            path: location.path.clone(),
            line: location.line,
            description: description.clone(),
        })
        .collect();
    Finding {
        fingerprint,
        external_id,
        title,
        description,
        verdict: Verdict::Reported,
        severity,
        confidence,
        cwe,
        locations,
        root_cause: String::new(),
        remediation,
        validation: format!("Reported by {}; scanner-owned verification", engine.label()),
        trace,
        evidence,
        conditions: Vec::new(),
        blockers: Vec::new(),
        source: engine,
    }
}

fn finish(mut findings: Vec<Finding>) -> Parsed<Vec<Finding>> {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut seen = HashSet::new();
    findings.retain(|finding| {
        if seen.insert(finding.fingerprint.clone()) {
            true
        } else {
            warnings.push(format!(
                "duplicate fingerprint ignored: {}",
                finding.fingerprint
            ));
            false
        }
    });
    findings.sort_by(|a, b| {
        b.severity_rank()
            .cmp(&a.severity_rank())
            .then_with(|| a.fingerprint.cmp(&b.fingerprint))
    });
    if findings.is_empty() {
        warnings.push("scanner report contains no findings".into());
    }
    Parsed {
        value: findings,
        errors: std::mem::take(&mut errors),
        warnings,
    }
}

fn parse_semgrep(value: &Value) -> Parsed<Vec<Finding>> {
    let Some(results) = value.get("results").and_then(Value::as_array) else {
        return Parsed {
            value: Vec::new(),
            errors: vec!["Semgrep report has no results array".into()],
            warnings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for (index, result) in results.iter().take(10_000).enumerate() {
        let extra = result.get("extra").cloned().unwrap_or(Value::Null);
        let check = string_field(result, "check_id").unwrap_or("semgrep.unknown");
        let path = result.get("path");
        let start_line = result
            .get("start")
            .and_then(|start| u32_field(start, "line"));
        let end_line = result.get("end").and_then(|end| u32_field(end, "line"));
        let title = text(extra.get("message"), 4096)
            .or_else(|| text(result.get("check_id"), 4096))
            .unwrap_or_else(|| format!("Semgrep finding {index}"));
        let severity = extra
            .get("severity")
            .and_then(Value::as_str)
            .and_then(Severity::parse);
        let fingerprint = extra
            .get("fingerprint")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "semgrep:{}:{}:{}",
                    check,
                    path.and_then(Value::as_str).unwrap_or(""),
                    start_line.unwrap_or(1)
                )
            });
        let cwe = string_list(
            extra
                .get("metadata")
                .and_then(|metadata| metadata.get("cwe")),
        );
        let mut locations = Vec::new();
        if let Some(location) = location(path, start_line) {
            locations.push(Location {
                end_line,
                ..location
            });
        }
        findings.push(make_finding(
            AuditEngine::Semgrep,
            fingerprint,
            Some(check.to_string()),
            title.clone(),
            title,
            severity,
            None,
            cwe,
            locations,
            String::new(),
        ));
    }
    finish(findings)
}

fn parse_snyk(value: &Value) -> Parsed<Vec<Finding>> {
    let Some(items) = value.get("vulnerabilities").and_then(Value::as_array) else {
        return Parsed {
            value: Vec::new(),
            errors: vec!["Snyk report has no vulnerabilities array".into()],
            warnings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for (index, item) in items.iter().take(10_000).enumerate() {
        let id = string_field(item, "id").unwrap_or("snyk.unknown");
        let title =
            text(item.get("title"), 4096).unwrap_or_else(|| format!("Snyk finding {index}"));
        let description =
            text(item.get("description"), MAX_TEXT_BYTES).unwrap_or_else(|| title.clone());
        let severity = string_field(item, "severity").and_then(Severity::parse);
        let confidence = string_field(item, "confidence").map(str::to_owned);
        let cwe = item
            .get("identifiers")
            .and_then(|identifiers| identifiers.get("CWE"))
            .map(|value| string_list(Some(value)))
            .unwrap_or_default();
        let mut locations = Vec::new();
        if let Some(from) = item.get("from").and_then(Value::as_array) {
            for candidate in from.iter().rev().filter_map(Value::as_str) {
                if candidate.contains('/')
                    && !candidate.starts_with('@')
                    && let Some(location) =
                        location(Some(&Value::String(candidate.into())), Some(1))
                {
                    locations.push(location);
                    break;
                }
            }
        }
        let remediation = text(item.get("remediation"), MAX_TEXT_BYTES).unwrap_or_default();
        let fingerprint = format!(
            "snyk:{id}:{}",
            string_field(item, "packageName").unwrap_or("")
        );
        findings.push(make_finding(
            AuditEngine::Snyk,
            fingerprint,
            Some(id.to_string()),
            title,
            description,
            severity,
            confidence,
            cwe,
            locations,
            remediation,
        ));
    }
    finish(findings)
}

fn parse_trivy(value: &Value) -> Parsed<Vec<Finding>> {
    let Some(results) = value.get("Results").and_then(Value::as_array) else {
        return Parsed {
            value: Vec::new(),
            errors: vec!["Trivy report has no Results array".into()],
            warnings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for (result_index, result) in results.iter().take(1000).enumerate() {
        let target = result.get("Target").and_then(Value::as_str);
        for (index, item) in result
            .get("Vulnerabilities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .take(10_000)
            .enumerate()
        {
            let id = string_field(item, "VulnerabilityID").unwrap_or("trivy.unknown");
            let title = text(item.get("Title"), 4096).unwrap_or_else(|| {
                format!(
                    "{id} in {}",
                    string_field(item, "PkgName").unwrap_or("dependency")
                )
            });
            let description =
                text(item.get("Description"), MAX_TEXT_BYTES).unwrap_or_else(|| title.clone());
            let severity = string_field(item, "Severity").and_then(Severity::parse);
            let cwe = item
                .get("CweIDs")
                .map(|value| string_list(Some(value)))
                .unwrap_or_default();
            let mut locations = Vec::new();
            if let Some(location) = location(target.map(Value::from).as_ref(), Some(1)) {
                locations.push(location);
            }
            let remediation = string_field(item, "FixedVersion")
                .map(|version| format!("Upgrade to {version}"))
                .unwrap_or_default();
            let package = string_field(item, "PkgName").unwrap_or("");
            let fingerprint = format!("trivy:{id}:{target:?}:{package}:{result_index}:{index}");
            findings.push(make_finding(
                AuditEngine::Trivy,
                fingerprint,
                Some(id.to_string()),
                title,
                description,
                severity,
                None,
                cwe,
                locations,
                remediation,
            ));
        }
    }
    finish(findings)
}

fn parse_gitleaks(value: &Value) -> Parsed<Vec<Finding>> {
    let Some(items) = value.as_array() else {
        return Parsed {
            value: Vec::new(),
            errors: vec!["Gitleaks report must be an array".into()],
            warnings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for (index, item) in items.iter().take(10_000).enumerate() {
        let rule = string_field(item, "RuleID").unwrap_or("gitleaks.unknown");
        let title = text(item.get("Description"), 4096)
            .unwrap_or_else(|| format!("Potential secret {rule}"));
        let path = item.get("File");
        let start = u32_field(item, "StartLine");
        let end = u32_field(item, "EndLine");
        let mut locations = Vec::new();
        if let Some(location) = location(path, start) {
            locations.push(Location {
                end_line: end,
                ..location
            });
        }
        // Never copy the matched secret into a finding description.
        let description = format!(
            "Potential secret detected by Gitleaks rule `{rule}`. The matched secret is intentionally omitted."
        );
        let fingerprint = string_field(item, "Fingerprint")
            .map(str::to_owned)
            .unwrap_or_else(|| {
                format!(
                    "gitleaks:{rule}:{}:{}:{index}",
                    path.and_then(Value::as_str).unwrap_or(""),
                    start.unwrap_or(1)
                )
            });
        findings.push(make_finding(
            AuditEngine::Gitleaks,
            fingerprint,
            Some(rule.to_string()),
            title,
            description,
            None,
            None,
            vec!["CWE-798".into()],
            locations,
            "Remove the secret from history and rotate the credential.".into(),
        ));
    }
    finish(findings)
}

fn parse_bandit(value: &Value) -> Parsed<Vec<Finding>> {
    let Some(items) = value.get("results").and_then(Value::as_array) else {
        return Parsed {
            value: Vec::new(),
            errors: vec!["Bandit report has no results array".into()],
            warnings: Vec::new(),
        };
    };
    let mut findings = Vec::new();
    for (index, item) in items.iter().take(10_000).enumerate() {
        let test = string_field(item, "test_id").unwrap_or("bandit.unknown");
        let name = string_field(item, "test_name").unwrap_or("bandit");
        let title = text(item.get("issue_text"), 4096).unwrap_or_else(|| format!("Bandit {test}"));
        let severity = string_field(item, "issue_severity").and_then(Severity::parse);
        let confidence = string_field(item, "issue_confidence").map(str::to_owned);
        let path = item.get("filename");
        let line = u32_field(item, "line_number");
        let mut locations = Vec::new();
        if let Some(location) = location(path, line) {
            locations.push(location);
        }
        let fingerprint = format!(
            "bandit:{test}:{}:{}:{index}",
            path.and_then(Value::as_str).unwrap_or(""),
            line.unwrap_or(1)
        );
        findings.push(make_finding(
            AuditEngine::Bandit,
            fingerprint,
            Some(test.to_string()),
            title.clone(),
            format!("{title} ({name})"),
            severity,
            confidence,
            Vec::new(),
            locations,
            String::new(),
        ));
    }
    finish(findings)
}

pub fn parse(value: &Value) -> Option<Parsed<Vec<Finding>>> {
    match detect(value)? {
        AdapterKind::Semgrep => Some(parse_semgrep(value)),
        AdapterKind::Snyk => Some(parse_snyk(value)),
        AdapterKind::Trivy => Some(parse_trivy(value)),
        AdapterKind::Gitleaks => Some(parse_gitleaks(value)),
        AdapterKind::Bandit => Some(parse_bandit(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn semgrep_and_bandit_are_distinguished_by_result_shape() {
        assert_eq!(
            detect(
                &json!({"results": [{"check_id": "rule", "path": "a.ts", "start": {"line": 1}}]})
            ),
            Some(AdapterKind::Semgrep)
        );
        assert_eq!(
            detect(
                &json!({"results": [{"test_id": "B101", "issue_text": "assert", "filename": "a.py", "line_number": 1}]})
            ),
            Some(AdapterKind::Bandit)
        );
    }

    #[test]
    fn gitleaks_never_copies_the_secret() {
        let parsed = parse(&json!([{
            "RuleID": "generic-api-key",
            "Description": "Generic API key",
            "File": "src/config.ts",
            "StartLine": 7,
            "Secret": "super-secret",
            "Match": "api_key=super-secret",
            "Fingerprint": "gitleaks-1"
        }]))
        .unwrap();
        assert_eq!(parsed.value.len(), 1);
        assert!(!parsed.value[0].description.contains("super-secret"));
        assert_eq!(parsed.value[0].severity, None);
    }

    #[test]
    fn snyk_and_trivy_keep_scanner_identity() {
        let snyk = parse(&json!({
            "vulnerabilities": [{
                "id": "SNYK-JS-1",
                "title": "Prototype pollution",
                "description": "A package is vulnerable.",
                "severity": "high",
                "packageName": "left-pad",
                "from": ["app@1.0", "left-pad@1.0"],
                "identifiers": {"CWE": ["CWE-1321"]}
            }]
        }))
        .unwrap();
        assert_eq!(snyk.value[0].external_id.as_deref(), Some("SNYK-JS-1"));
        assert_eq!(snyk.value[0].severity, Some(Severity::High));
        let trivy = parse(&json!({
            "Results": [{"Target": "package-lock.json", "Vulnerabilities": [{
                "VulnerabilityID": "CVE-2024-1",
                "Title": "Bad package",
                "Description": "A package is vulnerable.",
                "Severity": "CRITICAL",
                "PkgName": "bad",
                "FixedVersion": "2.0.0",
                "CweIDs": ["CWE-79"]
            }]}]
        }))
        .unwrap();
        assert_eq!(trivy.value[0].severity, Some(Severity::Critical));
        assert!(trivy.value[0].remediation.contains("2.0.0"));
    }
}
