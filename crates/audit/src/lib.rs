//! Bounded, read-only projection of external security audit artifacts.
//!
//! Dunlin does not execute the scanners. It reads files written by the
//! Cloudflare `security-audit-skill`, Codex Security, or a SARIF producer,
//! normalizes them for the native Security workspace, and independently checks
//! structural invariants before calling an artifact validated.

pub mod adapters;
pub mod cloudflare;
pub mod codex;
pub mod codex_cli;
pub mod discover;
pub mod export;
pub mod model;
pub mod sandbox;
pub mod sarif;
pub mod util;

pub use adapters::{AdapterKind, detect as detect_adapter, parse as parse_adapter};
pub use codex_cli::{CliOutput, CodexCli, discover as discover_codex_cli};
pub use discover::{
    CreatedRun, attach_run_session, create_run, discover_run, discover_runs, import_scanner_report,
    run_root, set_run_phase, valid_source_path,
};
pub use export::{export_csv, export_json, export_markdown, write_export as write_export_file};
pub use model::{
    AuditEngine, AuditSnapshot, CoverageEntry, CoverageStatus, CoverageSummary, Evidence, Finding,
    Location, RunHeader, RunStatus, Severity, SourceRef, TraceStep, Verdict, VerificationDecision,
    VerificationRecord,
};
pub use sandbox::{Limits as SandboxLimits, Output as SandboxOutput, run as run_sandboxed};
pub use sarif::{export as export_sarif, write_export as write_sarif};

/// Result of parsing one artifact plus bounded diagnostics. A parser keeps
/// valid records when other records fail so the UI can still show evidence.
#[derive(Clone, Debug, Default)]
pub struct Parsed<T> {
    pub value: T,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn confirmed_finding() -> serde_json::Value {
        json!({
            "verdict": "confirmed",
            "fingerprint": "src/auth.ts:42:missing-check",
            "title": "Missing authorization check",
            "description": "A lower-trust user can read another tenant's invoice.",
            "root_cause": "The handler loads by id without an owner predicate.",
            "intended_behavior": "Only the invoice owner or an administrator may read it.",
            "trace": [
                {"kind": "entrypoint", "file": "src/routes.ts", "line": 10, "scope": "handler", "description": "public request"},
                {"kind": "propagation", "file": "src/invoices.ts", "line": 24, "scope": "service", "description": "id reaches the query"},
                {"kind": "sink", "file": "src/store.ts", "line": 42, "scope": "query", "description": "row returned"}
            ],
            "evidence": [
                {"file": "src/store.ts", "line": 42, "description": "query has no tenant predicate"}
            ],
            "conditions": [{"kind": "authentication_level", "description": "any signed-in user"}],
            "execution": {
                "attacker_perspective": "tenant A user",
                "payloads": ["invoice id from tenant B"],
                "instructions": ["call GET /invoices/:id"],
                "observed_result": "tenant B invoice is returned"
            },
            "remediation": {"strategy": "Add owner predicate before the query."},
            "severity": {
                "likelihood": {"score": "high", "reason": "simple request"},
                "impact": {"score": "high", "reason": "cross-tenant read"},
                "overall_severity": "high"
            },
            "confidence": {"score": "high", "reason": "source and local check agree"}
        })
    }

    #[test]
    fn cloudflare_findings_are_normalized_and_confirmed_severity_is_kept() {
        let parsed = cloudflare::parse_findings(&json!([confirmed_finding()]));
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        let findings = parsed.value;
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].verdict, Verdict::Confirmed);
        assert_eq!(findings[0].severity, Some(Severity::High));
        assert_eq!(findings[0].locations.len(), 3);
        assert!(findings[0].validation.contains("tenant B invoice"));
    }

    #[test]
    fn needs_validation_cannot_carry_severity_or_execution() {
        let mut record = confirmed_finding();
        record["verdict"] = json!("needs_validation");
        record["claimed_root_cause"] = record["root_cause"].clone();
        record["blockers"] = json!(["missing gateway configuration"]);
        record["validation_plan"] = json!({"deployment": "owner checks the gateway"});
        record.as_object_mut().unwrap().remove("root_cause");
        record.as_object_mut().unwrap().remove("execution");
        record.as_object_mut().unwrap().remove("remediation");
        record.as_object_mut().unwrap().remove("severity");
        let parsed = cloudflare::parse_findings(&json!([record]));
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.value[0].verdict, Verdict::NeedsValidation);
        assert_eq!(parsed.value[0].severity, None);
    }

    #[test]
    fn coverage_rejects_duplicate_unsorted_and_unowned_covered_units() {
        let unit = json!({
            "coverage_id": "b",
            "surface": "HTTP",
            "boundary": "auth",
            "subsystem": "api",
            "attack_class": "Access control",
            "status": "covered",
            "reviewed_paths": [],
            "local_checks": [],
            "result_fingerprints": [],
            "unresolved": []
        });
        let mut duplicate = unit.clone();
        duplicate["coverage_id"] = json!("a");
        let parsed = cloudflare::parse_coverage(&json!([unit, duplicate]));
        assert!(!parsed.errors.is_empty());
        assert!(parsed.errors.iter().any(|error| error.contains("sorted")));
    }

    #[test]
    fn codex_findings_and_coverage_are_reported_not_cloudflare_confirmed() {
        let findings = json!({
            "documentType": "codex-security.findings",
            "schemaVersion": "1.0",
            "scanId": "scan_1",
            "findings": [{
                "findingId": "csf_aaaaaaaaaaaaaaaaaaaaaaaa",
                "occurrenceId": "occ_aaaaaaaaaaaaaaaaaaaaaaaa",
                "ruleId": "auth.missing-tenant",
                "fingerprints": {"algorithm": "codex-security/v1", "primary": "codex-security/v1:sha256:abc"},
                "title": "Missing tenant filter",
                "summary": "A query lacks tenant scoping.",
                "severity": {"level": "high"},
                "confidence": {"level": "high", "rationale": "source trace"},
                "taxonomy": {"category": "auth", "cwe": ["CWE-639"]},
                "locations": [{"path": "src/query.ts", "startLine": 12}],
                "remediation": "Add a tenant predicate.",
                "provenance": {"source": "local_plugin"}
            }]
        });
        let parsed = codex::parse_findings(&findings);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.value[0].verdict, Verdict::Reported);
        assert_eq!(parsed.value[0].severity, Some(Severity::High));
        let coverage = codex::parse_coverage(&json!({
            "documentType": "codex-security.coverage",
            "schemaVersion": "1.0",
            "scanId": "scan_1",
            "mode": "repository",
            "inventoryStrategy": "repository",
            "includePaths": [],
            "excludePaths": [],
            "surfaces": [{"id": "auth", "label": "Auth", "disposition": "needs_follow_up", "receiptRefs": [], "notes": "gateway config missing"}],
            "explicitExclusions": [],
            "deferred": []
        }));
        assert!(coverage.errors.is_empty(), "{:?}", coverage.errors);
        assert_eq!(coverage.value.entries[0].status, CoverageStatus::Deferred);
        assert!(coverage.value.present);
    }

    #[test]
    fn json_csv_and_markdown_exports_are_bounded_and_escaped() {
        let mut finding = cloudflare::parse_findings(&json!([confirmed_finding()]))
            .value
            .remove(0);
        finding.title = "Comma, quote \" and newline\nvalue".into();
        let snapshot = AuditSnapshot {
            findings: vec![finding],
            ..Default::default()
        };
        let json = export_json(&snapshot).unwrap();
        assert!(json.contains("\"fingerprint\""));
        let csv = export_csv(&snapshot);
        assert!(csv.starts_with("fingerprint,verdict"));
        assert!(csv.contains("\"Comma, quote \"\" and newline value\""));
        let markdown = export_markdown(&snapshot);
        assert!(markdown.contains("# Dunlin security audit"));
        assert!(markdown.contains("Confirmed"));
        assert!(markdown.contains("Root cause"));
    }

    #[test]
    fn native_sarif_export_preserves_verdict_and_locations() {
        let parsed = cloudflare::parse_findings(&json!([confirmed_finding()]));
        let snapshot = AuditSnapshot {
            findings: parsed.value,
            ..Default::default()
        };
        let text = export_sarif(&snapshot).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["version"], "2.1.0");
        assert_eq!(value["runs"][0]["results"][0]["level"], "error");
        assert_eq!(
            value["runs"][0]["results"][0]["properties"]["verdict"],
            "Confirmed"
        );
        assert_eq!(
            value["runs"][0]["results"][0]["locations"][0]["physicalLocation"]["artifactLocation"]
                ["uri"],
            "src/routes.ts"
        );
    }

    #[test]
    fn sarif_results_are_bounded_and_relative() {
        let value = json!({
            "version": "2.1.0",
            "runs": [{
                "tool": {"driver": {"name": "Semgrep", "rules": [{"id": "rule-1", "shortDescription": {"text": "Rule one"}}]}},
                "results": [{
                    "ruleId": "rule-1",
                    "level": "warning",
                    "message": {"text": "Potential issue"},
                    "locations": [{"physicalLocation": {"artifactLocation": {"uri": "src/main.rs"}, "region": {"startLine": 7}}}]
                }]
            }]
        });
        let parsed = sarif::parse(&value);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.value.len(), 1);
        assert_eq!(parsed.value[0].severity, Some(Severity::Medium));
        assert_eq!(parsed.value[0].locations[0].path, "src/main.rs");
        assert_eq!(parsed.value[0].verdict, Verdict::Reported);
    }

    #[test]
    fn discover_run_reads_stub_and_rejects_invalid_json() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::write(
            dir.join("dunlin-run.json"),
            r#"{"id":"run-1","status":"started","profile":"standard"}"#,
        )
        .unwrap();
        std::fs::write(dir.join("findings.json"), "not json").unwrap();
        let snapshot = discover_run(dir);
        assert_eq!(snapshot.run.id, "run-1");
        assert!(!snapshot.errors.is_empty());
    }

    #[test]
    fn discover_run_normalizes_cloudflare_findings_coverage_and_report() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::write(
            dir.join("run-metadata.json"),
            json!({
                "run_id": "run-cloudflare",
                "repo": "example",
                "target": "/tmp/example",
                "source_ref": "abc123",
                "profile": "standard",
                "execution_policy": "sandboxed-source-and-local-only",
                "run_status": "complete"
            })
            .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("findings.json"),
            json!([confirmed_finding()]).to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("coverage-ledger.json"),
            json!([{
                "coverage_id": "a",
                "surface": "HTTP",
                "boundary": "auth",
                "subsystem": "api",
                "attack_class": "Access control",
                "status": "planned",
                "reviewed_paths": [],
                "local_checks": [],
                "result_fingerprints": [],
                "unresolved": []
            }])
            .to_string(),
        )
        .unwrap();
        std::fs::write(dir.join("REPORT.md"), "# Audit report").unwrap();
        std::fs::write(
            dir.join("verification-src-auth.ts-42-missing-check.json"),
            json!({
                "fingerprint": "src/auth.ts:42:missing-check",
                "decision": "verified",
                "reason": "independent source check"
            })
            .to_string(),
        )
        .unwrap();
        let snapshot = discover_run(dir);
        assert!(snapshot.errors.is_empty(), "{:?}", snapshot.errors);
        assert_eq!(snapshot.run.status, RunStatus::Complete);
        assert_eq!(snapshot.run.source.commit.as_deref(), Some("abc123"));
        assert_eq!(snapshot.findings.len(), 1);
        assert!(snapshot.coverage.present);
        assert_eq!(snapshot.report.as_deref(), Some("# Audit report"));
        assert_eq!(snapshot.verifications.len(), 1);
        assert_eq!(
            snapshot.verifications[0].decision,
            VerificationDecision::Verified
        );
        assert!(snapshot.has_artifacts());
    }

    #[test]
    fn imported_scanner_reports_are_discovered_without_a_shell() {
        let temp = tempfile::tempdir().unwrap();
        let report = temp.path().join("semgrep.json");
        std::fs::write(
            &report,
            json!({
                "results": [{
                    "check_id": "rules.auth",
                    "path": "src/auth.ts",
                    "start": {"line": 9},
                    "end": {"line": 9},
                    "extra": {
                        "message": "Missing authorization check",
                        "severity": "ERROR",
                        "metadata": {"cwe": ["CWE-862"]}
                    }
                }]
            })
            .to_string(),
        )
        .unwrap();
        let created = import_scanner_report(temp.path(), "project", &report).unwrap();
        let snapshot = discover_run(&created.dir);
        assert!(snapshot.errors.is_empty(), "{:?}", snapshot.errors);
        assert_eq!(snapshot.findings.len(), 1);
        assert_eq!(snapshot.findings[0].source, AuditEngine::Semgrep);
        assert_eq!(snapshot.findings[0].severity, Some(Severity::High));
        assert_eq!(snapshot.run.engine, AuditEngine::Semgrep);
    }

    #[test]
    fn codex_cli_output_in_nested_directory_is_discovered() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir(dir.join("codex-results")).unwrap();
        std::fs::write(
            dir.join("dunlin-run.json"),
            r#"{"id":"run-codex","status":"started"}"#,
        )
        .unwrap();
        std::fs::write(
            dir.join("codex-results/scan-manifest.json"),
            json!({"scan": {"scanId": "scan-1", "status": "completed", "mode": "standard"}})
                .to_string(),
        )
        .unwrap();
        std::fs::write(
            dir.join("codex-results/findings.json"),
            json!({
                "documentType": "codex-security.findings",
                "schemaVersion": "1.0",
                "scanId": "scan-1",
                "findings": [{
                    "findingId": "csf_aaaaaaaaaaaaaaaaaaaaaaaa",
                    "occurrenceId": "occ_aaaaaaaaaaaaaaaaaaaaaaaa",
                    "ruleId": "auth.missing-tenant",
                    "fingerprints": {"algorithm": "codex-security/v1", "primary": "primary-1"},
                    "title": "Missing tenant filter",
                    "summary": "A query lacks tenant scoping.",
                    "severity": {"level": "high"},
                    "confidence": {"level": "high", "rationale": "source trace"},
                    "taxonomy": {"category": "auth", "cwe": ["CWE-639"]},
                    "locations": [{"path": "src/query.ts", "startLine": 12}],
                    "remediation": "Add a tenant predicate.",
                    "provenance": {"source": "local_plugin"}
                }]
            })
            .to_string(),
        )
        .unwrap();
        let snapshot = discover_run(dir);
        assert!(snapshot.errors.is_empty(), "{:?}", snapshot.errors);
        assert_eq!(snapshot.run.engine, AuditEngine::Codex);
        assert_eq!(snapshot.run.status, RunStatus::Complete);
        assert_eq!(snapshot.findings.len(), 1);
        assert_eq!(
            snapshot.findings[0].external_id.as_deref(),
            Some("occ_aaaaaaaaaaaaaaaaaaaaaaaa")
        );
        assert_eq!(snapshot.run.id, "scan-1");
    }

    #[test]
    fn run_directories_are_created_outside_the_project_and_contain_a_stub() {
        let temp = tempfile::tempdir().unwrap();
        let created = create_run(temp.path(), "project/with spaces").unwrap();
        assert!(created.dir.is_dir());
        assert!(created.dir.join("dunlin-run.json").is_file());
        attach_run_session(&created.dir, "ses-1").unwrap();
        let runs = discover_runs(&run_root(temp.path(), "project/with spaces"));
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run.status, RunStatus::Started);
        assert_eq!(runs[0].run.session_id.as_deref(), Some("ses-1"));
    }
}
