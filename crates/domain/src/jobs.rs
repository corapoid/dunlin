//! Durable relay job contracts. Enqueue authorizes one bounded ACP turn, not
//! unlimited model spending, a merge, a push or verified task success.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub harness: String,
    pub model: String,
    pub mode: String,
    pub timeout_secs: u64,
    pub max_tool_requests: u32,
    pub allow_tools: bool,
    pub review_state: String,
}
impl Profile {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(self.harness.as_str(), "opencode" | "codex" | "claude")
            || [&self.model, &self.mode, &self.review_state]
                .iter()
                .any(|value| {
                    value.trim().is_empty()
                        || value.len() > 256
                        || value.chars().any(char::is_control)
                })
            || !(30..=3600).contains(&self.timeout_secs)
            || self.review_state.len() > 128
            || !(1..=256).contains(&self.max_tool_requests)
        {
            return Err("Choose a supported harness, exact model/mode, review status, 30–3600 seconds and 1–256 tool requests".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Enqueue {
    pub issue_id: String,
    pub expected_issue_digest: String,
    pub project_id: String,
    pub idempotency_key: String,
    pub profile: Profile,
    pub approved: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Preparing,
    Running,
    Review,
    Failed,
    Blocked,
    Cancelled,
    Unknown,
}
impl Status {
    pub fn active(self) -> bool {
        matches!(self, Self::Preparing | Self::Running)
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Preparing => "Preparing",
            Self::Running => "Running",
            Self::Review => "Review required",
            Self::Failed => "Failed",
            Self::Blocked => "Blocked",
            Self::Cancelled => "Cancelled",
            Self::Unknown => "Interrupted · needs reconciliation",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub owner: String,
    pub owner_name: String,
    pub project_id: String,
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub profile: Profile,
    pub status: Status,
    pub cancel_requested: bool,
    pub report: String,
    pub checkout_project: Option<String>,
    pub branch: Option<String>,
    pub sync_state: String,
    pub sync_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Queue {
    pub paused: bool,
    pub jobs: Vec<Job>,
    #[serde(default)]
    pub self_user_id: Option<String>,
}
impl Job {
    pub fn validate(&self) -> Result<(), String> {
        let identifier = |value: &str| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        };
        if !identifier(&self.id)
            || !identifier(&self.owner)
            || !identifier(&self.project_id)
            || self
                .checkout_project
                .as_deref()
                .is_some_and(|id| !identifier(id))
            || self.issue_id.is_empty()
            || self.issue_id.len() > 128
            || self.identifier.len() > 128
            || self.title.len() > 2048
            || self.owner_name.len() > 256
            || self.report.len() > 16 * 1024
            || self
                .sync_error
                .as_ref()
                .is_some_and(|error| error.len() > 8192)
            || self
                .branch
                .as_ref()
                .is_some_and(|branch| branch.len() > 256)
            || !matches!(
                self.sync_state.as_str(),
                "none" | "pending" | "sending" | "unknown" | "sent" | "resolved"
            )
        {
            return Err("Invalid queue job".into());
        }
        self.profile.validate()
    }
}
impl Queue {
    pub fn validate(&self) -> Result<(), String> {
        if self.jobs.len() > 512 {
            return Err("Queue is too large".into());
        }
        let mut ids = std::collections::HashSet::new();
        for job in &self.jobs {
            job.validate()?;
            if !ids.insert(&job.id) {
                return Err("Duplicate queue job".into());
            }
        }
        Ok(())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub action: String,
    #[serde(default)]
    pub approved: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn budgets_and_explicit_modes_are_required() {
        let mut profile = Profile {
            harness: "opencode".into(),
            model: "model/id".into(),
            mode: "build".into(),
            timeout_secs: 900,
            max_tool_requests: 32,
            allow_tools: false,
            review_state: "In Review".into(),
        };
        assert!(profile.validate().is_ok());
        profile.timeout_secs = 0;
        assert!(profile.validate().is_err());
        profile.timeout_secs = 900;
        profile.model.clear();
        assert!(profile.validate().is_err());
    }
}
