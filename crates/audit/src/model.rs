//! Canonical, target-neutral audit vocabulary.
//!
//! Cloudflare findings, Codex Security findings and SARIF results all normalize
//! into this model. The original artifacts remain the source of truth; this is
//! only a bounded projection for the native Security workspace.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuditEngine {
    #[default]
    Unknown,
    Cloudflare,
    Codex,
    Sarif,
    Semgrep,
    Snyk,
    Trivy,
    Gitleaks,
    Bandit,
}

impl AuditEngine {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Cloudflare => "Cloudflare security-audit",
            Self::Codex => "Codex Security",
            Self::Sarif => "SARIF",
            Self::Semgrep => "Semgrep",
            Self::Snyk => "Snyk",
            Self::Trivy => "Trivy",
            Self::Gitleaks => "Gitleaks",
            Self::Bandit => "Bandit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Verdict {
    /// Independently verified by the audit workflow.
    Confirmed,
    /// Source-grounded but blocked from confirmation.
    NeedsValidation,
    /// A disproved candidate retained for coverage history.
    Rejected,
    /// A scanner-reported result whose independent verification is owned by
    /// that scanner (for example Codex Security or SARIF).
    Reported,
}

impl Verdict {
    pub fn label(self) -> &'static str {
        match self {
            Self::Confirmed => "Confirmed",
            Self::NeedsValidation => "Needs validation",
            Self::Rejected => "Rejected",
            Self::Reported => "Reported",
        }
    }
    pub fn rank(self) -> u8 {
        match self {
            Self::Confirmed => 0,
            Self::NeedsValidation => 1,
            Self::Reported => 2,
            Self::Rejected => 3,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Severity {
    Informational,
    Low,
    Medium,
    High,
    Critical,
}

impl Severity {
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "informational" | "info" | "note" | "none" => Some(Self::Informational),
            "low" => Some(Self::Low),
            "medium" | "moderate" | "warning" => Some(Self::Medium),
            "high" | "error" => Some(Self::High),
            "critical" => Some(Self::Critical),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Informational => "Informational",
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
            Self::Critical => "Critical",
        }
    }
    pub fn rank(self) -> u8 {
        match self {
            Self::Informational => 0,
            Self::Low => 1,
            Self::Medium => 2,
            Self::High => 3,
            Self::Critical => 4,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    pub path: String,
    pub line: u32,
    pub end_line: Option<u32>,
    pub role: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceStep {
    pub kind: String,
    pub path: String,
    pub line: u32,
    pub scope: String,
    pub description: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub path: String,
    pub line: u32,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Finding {
    pub fingerprint: String,
    /// Scanner-owned identity (Codex occurrence id, SARIF result id, etc.).
    pub external_id: Option<String>,
    pub title: String,
    pub description: String,
    pub verdict: Verdict,
    pub severity: Option<Severity>,
    pub confidence: Option<String>,
    pub cwe: Vec<String>,
    pub locations: Vec<Location>,
    pub root_cause: String,
    pub remediation: String,
    pub validation: String,
    pub trace: Vec<TraceStep>,
    pub evidence: Vec<Evidence>,
    pub conditions: Vec<String>,
    pub blockers: Vec<String>,
    pub source: AuditEngine,
}

impl Finding {
    pub fn primary_location(&self) -> Option<&Location> {
        self.locations.first()
    }
    pub fn severity_rank(&self) -> i16 {
        self.severity.map(|s| s.rank() as i16).unwrap_or(-1)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageStatus {
    #[default]
    Planned,
    InProgress,
    Covered,
    Candidate,
    Blocked,
    Deferred,
    OutOfScope,
    NotApplicable,
}

impl CoverageStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "planned" => Some(Self::Planned),
            "in_progress" => Some(Self::InProgress),
            "covered" => Some(Self::Covered),
            "candidate" => Some(Self::Candidate),
            "blocked" => Some(Self::Blocked),
            "deferred" => Some(Self::Deferred),
            "out_of_scope" => Some(Self::OutOfScope),
            "not_applicable" => Some(Self::NotApplicable),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Planned => "Planned",
            Self::InProgress => "In progress",
            Self::Covered => "Covered",
            Self::Candidate => "Candidate",
            Self::Blocked => "Blocked",
            Self::Deferred => "Deferred",
            Self::OutOfScope => "Out of scope",
            Self::NotApplicable => "Not applicable",
        }
    }
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            Self::Covered | Self::OutOfScope | Self::NotApplicable | Self::Deferred
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoverageEntry {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub status: CoverageStatus,
    pub reviewed_paths: Vec<String>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoverageSummary {
    /// False when no compatible coverage artifact was found.
    pub present: bool,
    pub entries: Vec<CoverageEntry>,
    pub open_questions: Vec<String>,
    pub source: AuditEngine,
}

impl CoverageSummary {
    pub fn count(&self, status: CoverageStatus) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.status == status)
            .count()
    }
    pub fn total(&self) -> usize {
        self.entries.len()
    }
    pub fn is_finished(&self) -> bool {
        !self.entries.is_empty() && self.entries.iter().all(|entry| entry.status.is_finished())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationDecision {
    Verified,
    NeedsValidation,
    Rejected,
}

impl VerificationDecision {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "verified" | "confirmed" => Some(Self::Verified),
            "needs_validation" | "needs-validation" => Some(Self::NeedsValidation),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Verified => "Verified",
            Self::NeedsValidation => "Needs validation",
            Self::Rejected => "Rejected",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationRecord {
    pub fingerprint: String,
    pub decision: VerificationDecision,
    pub reason: String,
    pub verifier: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RunStatus {
    #[default]
    Unknown,
    Started,
    Running,
    Complete,
    Incomplete,
    Failed,
    Cancelled,
}

impl RunStatus {
    pub fn parse(value: &str) -> Self {
        match value.to_ascii_lowercase().as_str() {
            "started" | "draft" | "pending" => Self::Started,
            "running" | "in_progress" | "in-progress" => Self::Running,
            "complete" | "completed" | "done" => Self::Complete,
            "incomplete" => Self::Incomplete,
            "failed" | "error" => Self::Failed,
            "cancelled" | "canceled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Started => "Started",
            Self::Running => "Running",
            Self::Complete => "Complete",
            Self::Incomplete => "Incomplete",
            Self::Failed => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Complete | Self::Incomplete | Self::Failed | Self::Cancelled
        )
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceRef {
    pub commit: Option<String>,
    pub dirty: Option<bool>,
    pub captured_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RunHeader {
    pub id: String,
    pub project_id: Option<String>,
    pub created_at: Option<String>,
    pub status: RunStatus,
    pub phase: Option<String>,
    pub profile: Option<String>,
    pub engine: AuditEngine,
    pub source: SourceRef,
    pub repo: Option<String>,
    pub target: Option<String>,
    pub incomplete_reason: Option<String>,
    /// Dunlin conversation that launched this run, when known.
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AuditSnapshot {
    pub run: RunHeader,
    /// Absolute run directory outside the project checkout.
    pub dir: std::path::PathBuf,
    pub findings: Vec<Finding>,
    pub verifications: Vec<VerificationRecord>,
    pub coverage: CoverageSummary,
    pub report: Option<String>,
    pub report_name: Option<String>,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub files: Vec<String>,
}

impl AuditSnapshot {
    /// Structural validation only. This proves that Dunlin could parse and
    /// cross-check the artifacts; it does not prove the security conclusion.
    pub fn is_structurally_valid(&self) -> bool {
        self.errors.is_empty()
    }
    pub fn has_artifacts(&self) -> bool {
        !self.findings.is_empty() || self.coverage.present || self.report.is_some()
    }
    pub fn can_report_confirmed(&self) -> bool {
        self.is_structurally_valid()
            && self.run.status == RunStatus::Complete
            && self.coverage.present
            && self.has_artifacts()
    }
    pub fn confirmed_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.verdict == Verdict::Confirmed)
            .count()
    }
    pub fn needs_validation_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.verdict == Verdict::NeedsValidation)
            .count()
    }
    pub fn reported_count(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.verdict == Verdict::Reported)
            .count()
    }
}
