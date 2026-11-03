use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Create {
        base: String,
        head: String,
        title: String,
        body: String,
        draft: bool,
    },
    Edit {
        number: u64,
        title: String,
        body: String,
    },
    Comment {
        number: u64,
        body: String,
    },
    Review {
        number: u64,
        body: String,
        event: String,
    },
    LineComment {
        number: u64,
        path: String,
        line: u64,
        body: String,
    },
    Draft {
        number: u64,
        draft: bool,
    },
    State {
        number: u64,
        open: bool,
    },
    Merge {
        number: u64,
        method: String,
    },
    Cancel {
        run: u64,
    },
    Rerun {
        run: u64,
        failed_only: bool,
    },
}
impl Action {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Create { .. } => "Create pull request",
            Self::Edit { .. } => "Edit pull request",
            Self::Comment { .. } => "Publish comment",
            Self::Review { .. } => "Submit review",
            Self::LineComment { .. } => "Publish line review comment",
            Self::Draft { draft: true, .. } => "Convert PR to draft",
            Self::Draft { draft: false, .. } => "Mark PR ready for review",
            Self::State { open: true, .. } => "Reopen pull request",
            Self::State { open: false, .. } => "Close pull request",
            Self::Merge { .. } => "Merge pull request",
            Self::Cancel { .. } => "Cancel workflow",
            Self::Rerun { .. } => "Rerun workflow (may consume CI minutes)",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Plan {
    pub repository: Repository,
    pub account: String,
    pub action: Action,
    pub summary: String,
    snapshot: Value,
    pub(super) prepared: Instant,
}
fn text(value: &str, max: usize, required: bool) -> Result<(), String> {
    if value.len() > max
        || (required && value.trim().is_empty())
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err("Title/body is empty, contains unsupported control characters or exceeds its size limit".into());
    }
    Ok(())
}
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn branch(repo: &Repository, name: &str) -> Result<String, String> {
    if name.is_empty()
        || name.len() > 256
        || name.starts_with('-')
        || name.chars().any(char::is_control)
    {
        return Err("Choose an existing remote branch; Dunlin does not push one implicitly".into());
    }
    let value: Value = api(
        repo,
        &path(repo, &format!("git/ref/heads/{}", encode(name))),
    )?;
    let sha = value["object"]["sha"]
        .as_str()
        .filter(|v| oid(v))
        .ok_or("Remote branch SHA unavailable")?;
    Ok(sha.into())
}
fn snapshot(repo: &Repository, action: &Action) -> Result<Value, String> {
    match action {
        Action::Create {
            base,
            head,
            title,
            body,
            ..
        } => {
            text(title, 256, true)?;
            text(body, 60_000, false)?;
            if base == head {
                return Err("PR base and head must differ".into());
            }
            Ok(json!({"base":branch(repo,base)?,"head":branch(repo,head)?}))
        }
        Action::Cancel { run: run_id } | Action::Rerun { run: run_id, .. } => {
            let run = run(repo, *run_id)?;
            if matches!(action, Action::Cancel { .. }) && run.status == "completed" {
                return Err("Workflow already completed".into());
            }
            if matches!(action, Action::Rerun { .. }) && run.status != "completed" {
                return Err("Wait for this attempt to complete".into());
            }
            Ok(serde_json::to_value(run).map_err(|_| "Invalid run")?)
        }
        _ => {
            let number = match action {
                Action::Edit { number, .. }
                | Action::Comment { number, .. }
                | Action::Review { number, .. }
                | Action::LineComment { number, .. }
                | Action::Draft { number, .. }
                | Action::State { number, .. }
                | Action::Merge { number, .. } => *number,
                _ => unreachable!(),
            };
            let pr = pull(repo, number)?;
            match action {
                Action::Edit { title, body, .. } => {
                    text(title, 256, true)?;
                    text(body, 60_000, false)?;
                }
                Action::Comment { body, .. } => text(body, 60_000, true)?,
                Action::LineComment {
                    path, line, body, ..
                } => {
                    text(body, 60_000, true)?;
                    if *line == 0
                        || path.is_empty()
                        || path.len() > 4096
                        || !std::path::Path::new(path)
                            .components()
                            .all(|c| matches!(c, std::path::Component::Normal(_)))
                        || path.chars().any(char::is_control)
                    {
                        return Err("Choose a repository-relative filename and a positive RIGHT-side diff line".into());
                    }
                    if pr.state != "open" {
                        return Err("Line review requires an open PR".into());
                    }
                }
                Action::Draft { .. } if pr.state != "open" || pr.node_id.is_empty() => {
                    return Err("Draft transitions require an open PR with a node identity".into());
                }
                Action::Review { body, event, .. } => {
                    if !["APPROVE", "REQUEST_CHANGES", "COMMENT"].contains(&event.as_str()) {
                        return Err("Choose a supported review event".into());
                    }
                    text(body, 60_000, event != "APPROVE")?;
                    if pr.state != "open" || pr.draft {
                        return Err("Review requires an open, ready PR".into());
                    }
                }
                Action::State { .. } if pr.merged == Some(true) => {
                    return Err("Merged PRs cannot be reopened or closed".into());
                }
                Action::Merge { method, .. } => {
                    if !["merge", "squash", "rebase"].contains(&method.as_str()) {
                        return Err("Choose merge, squash or rebase".into());
                    }
                    if pr.state != "open" || pr.draft || pr.mergeable != Some(true) {
                        return Err("PR is draft, closed, conflicting or mergeability is still unknown. Refresh first.".into());
                    }
                }
                _ => {}
            }
            serde_json::to_value(pr).map_err(|_| "Invalid PR".into())
        }
    }
}
pub fn prepare(repo: &Repository, action: Action) -> Result<Plan, String> {
    current(repo)?;
    let account = account(repo)?;
    let snapshot = snapshot(repo, &action)?;
    let summary = format!(
        "{}\nRepository: github.com/{}\nAccount: {}\nLocal HEAD: {}\nRequest:\n{}\n\nNo push, checkout, branch deletion, auto-merge or admin override. GitHub branch protections remain authoritative. External changes can still race; inspect GitHub after any failed/lost response.",
        action.label(),
        repo.name,
        account,
        repo.local_head,
        serde_json::to_string_pretty(&action).map_err(|_| "Invalid request")?
    );
    if summary.len() > 65_000 {
        return Err("Encoded publication preview exceeds 65 KiB; shorten the title/body before preparing confirmation".into());
    }
    Ok(Plan {
        repository: repo.clone(),
        account,
        action,
        summary,
        snapshot,
        prepared: Instant::now(),
    })
}
pub(super) fn request(action: &Action, snapshot: &Value) -> (String, String, Value) {
    match action {
        Action::Create {
            base,
            head,
            title,
            body,
            draft,
        } => (
            "POST".into(),
            "pulls".into(),
            json!({"base":base,"head":head,"title":title,"body":body,"draft":draft}),
        ),
        Action::Edit {
            number,
            title,
            body,
        } => (
            "PATCH".into(),
            format!("pulls/{number}"),
            json!({"title":title,"body":body}),
        ),
        Action::Comment { number, body } => (
            "POST".into(),
            format!("issues/{number}/comments"),
            json!({"body":body}),
        ),
        Action::Review {
            number,
            body,
            event,
        } => (
            "POST".into(),
            format!("pulls/{number}/reviews"),
            json!({"body":body,"event":event,"commit_id":snapshot["head"]["sha"]}),
        ),
        Action::LineComment {
            number,
            path,
            line,
            body,
        } => (
            "POST".into(),
            format!("pulls/{number}/comments"),
            json!({"body":body,"path":path,"line":line,"side":"RIGHT","commit_id":snapshot["head"]["sha"]}),
        ),
        Action::Draft { draft, .. } => (
            "POST".into(),
            "graphql".into(),
            json!({"query":if *draft {"mutation($id:ID!){convertPullRequestToDraft(input:{pullRequestId:$id}){pullRequest{id isDraft}}}"}else{"mutation($id:ID!){markPullRequestReadyForReview(input:{pullRequestId:$id}){pullRequest{id isDraft}}}"},"variables":{"id":snapshot["node_id"]}}),
        ),
        Action::State { number, open } => (
            "PATCH".into(),
            format!("pulls/{number}"),
            json!({"state":if *open {"open"}else{"closed"}}),
        ),
        Action::Merge { number, method } => (
            "PUT".into(),
            format!("pulls/{number}/merge"),
            json!({"sha":snapshot["head"]["sha"],"merge_method":method}),
        ),
        Action::Cancel { run } => (
            "POST".into(),
            format!("actions/runs/{run}/cancel"),
            json!({}),
        ),
        Action::Rerun { run, failed_only } => (
            "POST".into(),
            format!(
                "actions/runs/{run}/{}",
                if *failed_only {
                    "rerun-failed-jobs"
                } else {
                    "rerun"
                }
            ),
            json!({}),
        ),
    }
}
/// Consumes the approval; caller must never retry on a transport/process failure.
pub fn execute(plan: Plan) -> Result<String, String> {
    if plan.prepared.elapsed() > Duration::from_secs(300) {
        return Err("Confirmation expired. Prepare a fresh request.".into());
    }
    current(&plan.repository)?;
    if account(&plan.repository)? != plan.account
        || snapshot(&plan.repository, &plan.action)? != plan.snapshot
    {
        return Err("GitHub account, PR, workflow or branch changed after preflight. Nothing was submitted; review again.".into());
    }
    let (method, suffix, body) = request(&plan.action, &plan.snapshot);
    // Body travels via stdin, never a shell/argv or a persistent temporary file.
    let mut cmd = command(&plan.repository);
    let endpoint = if suffix == "graphql" {
        suffix
    } else {
        path(&plan.repository, &suffix)
    };
    cmd.args([
        "api",
        "--hostname",
        "github.com",
        "--method",
        &method,
        &endpoint,
        "--input",
        "-",
    ]);
    let bytes = serde_json::to_vec(&body).map_err(|_| "Invalid GitHub request")?;
    let (status, bytes) = process::run(cmd, Some(bytes), Duration::from_secs(30), LIMIT)?;
    if !status.success() {
        return Err("GitHub refused or could not complete the request. Inspect permissions, branch protections and remote state; no automatic retry.".into());
    }
    if bytes.is_empty() && matches!(plan.action, Action::Merge { .. } | Action::Draft { .. }) {
        return Err("No GitHub result body; inspect remote state before any new submission".into());
    }
    if !bytes.is_empty() {
        let value: Value = serde_json::from_slice(&bytes).map_err(
            |_| "Unrecognized GitHub result; inspect remote state before any new request",
        )?;
        if value.get("errors").is_some()
            || (matches!(plan.action, Action::Merge { .. }) && value["merged"] != true)
        {
            return Err(
                "GitHub did not confirm this operation. Inspect remote state; nothing was retried."
                    .into(),
            );
        }
    }
    Ok("GitHub accepted the request. Refresh remote state to verify the result.".into())
}
