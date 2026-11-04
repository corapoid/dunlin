use std::fs;
use std::io::Write;
use std::path::Path;

use crate::model::{AuditSnapshot, Verdict};

fn csv_cell(value: &str) -> String {
    let value = value.replace(['\r', '\n'], " ");
    if value.contains([',', '"', ';']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value
    }
}

pub fn export_json(snapshot: &AuditSnapshot) -> Result<String, String> {
    serde_json::to_string_pretty(snapshot).map_err(|error| error.to_string())
}

pub fn export_csv(snapshot: &AuditSnapshot) -> String {
    let mut output = String::from(
        "fingerprint,verdict,severity,confidence,title,path,line,source,remediation\n",
    );
    for finding in &snapshot.findings {
        let (path, line) = finding
            .primary_location()
            .map(|location| (location.path.as_str(), location.line))
            .unwrap_or(("", 0));
        output.push_str(&format!(
            "{},{},{},{},{},{},{},{},{}\n",
            csv_cell(&finding.fingerprint),
            csv_cell(finding.verdict.label()),
            csv_cell(
                finding
                    .severity
                    .map(|severity| severity.label())
                    .unwrap_or("")
            ),
            csv_cell(finding.confidence.as_deref().unwrap_or("")),
            csv_cell(&finding.title),
            csv_cell(path),
            line,
            csv_cell(finding.source.label()),
            csv_cell(&finding.remediation),
        ));
    }
    output
}

pub fn export_markdown(snapshot: &AuditSnapshot) -> String {
    let mut output = String::new();
    output.push_str("# Dunlin security audit\n\n");
    output.push_str(&format!("- Run: `{}`\n", snapshot.run.id));
    output.push_str(&format!("- Status: {}\n", snapshot.run.status.label()));
    output.push_str(&format!("- Engine: {}\n", snapshot.run.engine.label()));
    if let Some(profile) = &snapshot.run.profile {
        output.push_str(&format!("- Profile: {profile}\n"));
    }
    if let Some(source) = &snapshot.run.source.commit {
        output.push_str(&format!("- Source: `{source}`\n"));
    }
    output.push_str(&format!(
        "- Coverage: {} units\n",
        snapshot.coverage.total()
    ));
    output.push('\n');
    if !snapshot.errors.is_empty() {
        output.push_str("## Validation errors\n\n");
        for error in &snapshot.errors {
            output.push_str(&format!("- {error}\n"));
        }
        output.push('\n');
    }
    if !snapshot.warnings.is_empty() {
        output.push_str("## Coverage and artifact gaps\n\n");
        for warning in &snapshot.warnings {
            output.push_str(&format!("- {warning}\n"));
        }
        output.push('\n');
    }
    output.push_str("## Findings\n\n");
    if snapshot.findings.is_empty() {
        output.push_str(
            "No findings were parsed. This does not establish that the repository is secure.\n",
        );
    } else {
        output.push_str("| Verdict | Severity | Title | Source |\n|---|---|---|---|\n");
        for finding in &snapshot.findings {
            output.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                finding.verdict.label(),
                finding
                    .severity
                    .map(|severity| severity.label())
                    .unwrap_or("—"),
                finding.title.replace('|', "\\|"),
                finding.source.label(),
            ));
        }
        output.push('\n');
        for finding in snapshot
            .findings
            .iter()
            .filter(|finding| finding.verdict != Verdict::Rejected)
        {
            output.push_str(&format!("### {}\n\n", finding.title));
            output.push_str(&format!(
                "- Verdict: {}\n- Source: {}\n",
                finding.verdict.label(),
                finding.source.label()
            ));
            if let Some(severity) = finding.severity {
                output.push_str(&format!("- Severity: {}\n", severity.label()));
            }
            if let Some(confidence) = &finding.confidence {
                output.push_str(&format!("- Confidence: {confidence}\n"));
            }
            output.push('\n');
            output.push_str(&format!("{}\n\n", finding.description));
            if !finding.root_cause.is_empty() {
                output.push_str(&format!("**Root cause:** {}\n\n", finding.root_cause));
            }
            if !finding.remediation.is_empty() {
                output.push_str(&format!("**Remediation:** {}\n\n", finding.remediation));
            }
        }
    }
    if let Some(report) = &snapshot.report {
        output.push_str("## Saved report\n\n");
        output.push_str(report);
    }
    output
}

/// Write one export atomically without following a destination symlink.
pub fn write_export(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("export destination has no parent directory")?;
    if !parent.is_dir() {
        return Err("export destination directory does not exist".into());
    }
    let temporary = parent.join(format!(".dunlin-export-{}.tmp", uuid::Uuid::now_v7()));
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
        file.write_all(contents.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        fs::rename(&temporary, path).map_err(|error| error.to_string())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
