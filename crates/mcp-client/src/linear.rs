//! A small deterministic adapter over the tools actually advertised by Linear.
//! Unknown schemas/results fail explicitly rather than silently widening a tag
//! filter, dropping pagination or pretending that an agent verified success.
use crate::{Client, Error, Tool};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashSet;

pub const ENDPOINT: &str = "https://mcp.linear.app/mcp";
pub const MAX_ISSUES: usize = 500;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Filter {
    pub label: String,
    pub team: Option<String>,
}

impl Filter {
    pub fn validate(&self) -> Result<(), Error> {
        if !text(&self.label, 128) || self.team.as_deref().is_some_and(|team| !text(team, 128)) {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub description: String,
    pub url: String,
    pub status: String,
    pub labels: Vec<String>,
}
pub fn issue_digest(issue: &Issue) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(
        serde_json::to_vec(issue).expect("serializable issue"),
    ))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inbox {
    pub filter: Option<Filter>,
    pub issues: Vec<Issue>,
    pub synced_at: Option<i64>,
    pub error: Option<String>,
}

impl Inbox {
    pub fn validate(&self) -> Result<(), Error> {
        if let Some(filter) = &self.filter {
            filter.validate()?;
        }
        if self.issues.len() > MAX_ISSUES
            || (self.filter.is_none() && !self.issues.is_empty())
            || self.error.as_ref().is_some_and(|error| error.len() > 8192)
        {
            return Err(Error::Limit);
        }
        let mut ids = HashSet::new();
        for issue in &self.issues {
            if !text(&issue.id, 128)
                || !ids.insert(&issue.id)
                || !text(&issue.identifier, 128)
                || !text(&issue.title, 2048)
                || issue.description.len() > 32 * 1024
                || !is_issue_url(&issue.url)
                || !text(&issue.status, 256)
                || issue.labels.len() > 128
                || issue.labels.iter().any(|label| !text(label, 128))
            {
                return Err(Error::Protocol);
            }
            if self.filter.as_ref().is_some_and(|filter| {
                !issue
                    .labels
                    .iter()
                    .any(|label| label.eq_ignore_ascii_case(filter.label.trim()))
            }) {
                return Err(Error::Protocol);
            }
        }
        Ok(())
    }
}

fn text(value: &str, limit: usize) -> bool {
    !value.trim().is_empty() && value.len() <= limit && !value.chars().any(char::is_control)
}

fn output(result: Value) -> Result<Value, Error> {
    if let Some(structured) = result.get("structuredContent") {
        if structured.is_object() || structured.is_array() {
            return Ok(structured.clone());
        }
        return Err(Error::Protocol);
    }
    let content = result["content"].as_array().ok_or(Error::Protocol)?;
    let texts: Vec<_> = content
        .iter()
        .filter(|item| item["type"] == "text")
        .filter_map(|item| item["text"].as_str())
        .collect();
    if texts.len() != 1 {
        return Err(Error::Protocol);
    }
    serde_json::from_str(texts[0]).map_err(|_| Error::Protocol)
}

fn issue(value: &Value) -> Result<Issue, Error> {
    let required = |field: &str, limit| {
        value[field]
            .as_str()
            .filter(|value| text(value, limit))
            .map(str::to_string)
            .ok_or(Error::Protocol)
    };
    let url = required("url", 2048)?;
    if !is_issue_url(&url) {
        return Err(Error::Protocol);
    }
    let labels = value["labels"].as_array().ok_or(Error::Protocol)?;
    if labels.len() > 128 {
        return Err(Error::Limit);
    }
    let labels: Vec<String> = labels
        .iter()
        .map(|label| {
            label
                .as_str()
                .or_else(|| label["name"].as_str())
                .filter(|label| text(label, 128))
                .map(str::to_string)
                .ok_or(Error::Protocol)
        })
        .collect::<Result<_, _>>()?;
    let description = value["description"].as_str().unwrap_or("");
    if description.len() > 32 * 1024 {
        return Err(Error::Limit);
    }
    let status = value["status"]
        .as_str()
        .or_else(|| value["status"]["name"].as_str())
        .unwrap_or("Unknown");
    if !text(status, 256) {
        return Err(Error::Protocol);
    }
    Ok(Issue {
        id: required("id", 128)?,
        identifier: required("identifier", 128)?,
        title: required("title", 2048)?,
        description: description.into(),
        url,
        status: status.into(),
        labels,
    })
}

pub fn is_issue_url(value: &str) -> bool {
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    value.len() <= 2048
        && parsed.scheme() == "https"
        && parsed.host_str() == Some("linear.app")
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.port().is_none()
}

pub fn issues(
    client: &mut Client<'_>,
    tools: &[Tool],
    filter: &Filter,
) -> Result<Vec<Issue>, Error> {
    filter.validate()?;
    let mut fields = vec!["label", "limit"];
    if filter.team.is_some() {
        fields.push("team");
    }
    let tool = tools
        .iter()
        .find(|tool| tool.name == "list_issues" && tool.supports(&fields))
        .ok_or(Error::Unsupported)?;
    let mut all = Vec::new();
    let mut payload_bytes = 0usize;
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut cursor = None;
    for _ in 0..16 {
        let mut arguments = json!({"label":filter.label, "limit":100});
        if let Some(team) = &filter.team {
            arguments["team"] = json!(team);
        }
        if let Some(cursor) = &cursor {
            let mut paginated = fields.clone();
            paginated.push("cursor");
            if !tool.supports(&paginated) {
                return Err(Error::Unsupported);
            }
            arguments["cursor"] = json!(cursor);
        }
        let page = output(client.call(tool, arguments)?)?;
        let rows = page
            .get("issues")
            .unwrap_or(&page)
            .as_array()
            .ok_or(Error::Protocol)?;
        if rows.len() > 100 {
            return Err(Error::Limit);
        }
        for row in rows {
            let issue = issue(row)?;
            payload_bytes += serde_json::to_vec(&issue)
                .map_err(|_| Error::Protocol)?
                .len();
            // Leave headroom for envelope/filter metadata in the relay's 8 MiB
            // cache. Oversized syncs must retain the last complete snapshot.
            if payload_bytes > 7 * 1024 * 1024 {
                return Err(Error::Limit);
            }
            if !issue
                .labels
                .iter()
                .any(|label| label.eq_ignore_ascii_case(filter.label.trim()))
            {
                return Err(Error::Protocol);
            }
            if !ids.insert(issue.id.clone()) {
                return Err(Error::Protocol);
            }
            all.push(issue);
            if all.len() > MAX_ISSUES {
                return Err(Error::Limit);
            }
        }
        let more = page["hasNextPage"]
            .as_bool()
            .or_else(|| page["pageInfo"]["hasNextPage"].as_bool());
        match more {
            Some(false) => return Ok(all),
            None => return Err(Error::Unsupported),
            Some(true) => {
                let next = page["cursor"]
                    .as_str()
                    .or_else(|| page["endCursor"].as_str())
                    .or_else(|| page["pageInfo"]["endCursor"].as_str())
                    .filter(|cursor| text(cursor, 4096))
                    .ok_or(Error::Protocol)?;
                if !cursors.insert(next.to_string()) {
                    return Err(Error::Protocol);
                }
                cursor = Some(next.to_string());
            }
        }
    }
    Err(Error::Limit)
}

/// Status-only mutation, always with an existing issue id. No retries: an
/// interrupted response is ambiguous and must never replay a task execution.
pub fn set_status(
    client: &mut Client<'_>,
    tools: &[Tool],
    id: &str,
    state: &str,
) -> Result<(), Error> {
    if !text(id, 128) || !text(state, 128) {
        return Err(Error::Protocol);
    }
    let tool = ["save_issue", "update_issue"]
        .into_iter()
        .find_map(|name| {
            tools
                .iter()
                .find(|tool| tool.name == name && tool.supports(&["id", "state"]))
        })
        .ok_or(Error::Unsupported)?;
    client.call(tool, json!({"id":id,"state":state}))?;
    Ok(())
}

pub fn get_issue(client: &mut Client<'_>, tools: &[Tool], id: &str) -> Result<Issue, Error> {
    if !text(id, 128) {
        return Err(Error::Protocol);
    }
    let tool = tools
        .iter()
        .find(|tool| tool.name == "get_issue" && tool.supports(&["id"]))
        .ok_or(Error::Unsupported)?;
    let data = output(client.call(tool, json!({"id":id}))?)?;
    let issue = issue(data.get("issue").unwrap_or(&data))?;
    if issue.id != id {
        return Err(Error::Protocol);
    }
    Ok(issue)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{Mock, Reply};

    #[test]
    fn issue_snapshots_bind_the_entire_reviewed_task_and_rereads_bind_identity() {
        let original = issue(&row("issue-id")).unwrap();
        let mut changed = original.clone();
        changed.description.push_str(" changed");
        assert_ne!(issue_digest(&original), issue_digest(&changed));
        let tool = Tool {
            name: "get_issue".into(),
            input_schema: json!({"properties":{"id":{}}}),
        };
        for id in ["issue-id", "another-issue"] {
            let mock = Mock::new(vec![
                Reply::initialize(),
                Reply::notification(),
                Reply::result("tools/call", json!({"structuredContent":{"issue":row(id)}})),
            ]);
            let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
            let result = get_issue(&mut client, &[tool.clone()], "issue-id");
            assert_eq!(result.is_ok(), id == "issue-id");
            let requests = mock.finish();
            assert_eq!(
                requests[2].body["params"]["arguments"],
                json!({"id":"issue-id"})
            );
        }
    }

    fn row(id: &str) -> Value {
        json!({"id":id,"identifier":"APP-1","title":"Task", "description":"Requirements", "url":"https://linear.app/team/issue/APP-1/task", "status":"Todo", "labels":["dunlin"]})
    }
    fn tool() -> Tool {
        Tool {
            name: "list_issues".into(),
            input_schema: json!({"properties":{"label":{},"limit":{},"cursor":{},"team":{}}}),
        }
    }

    #[test]
    fn tagged_issues_are_paginated_without_an_llm() {
        let mock = Mock::new(vec![
            Reply::initialize(),
            Reply::notification(),
            Reply::result(
                "tools/call",
                json!({"structuredContent":{"issues":[row("a")],"hasNextPage":true,"cursor":"next"}}),
            ),
            Reply::result(
                "tools/call",
                json!({"content":[{"type":"text","text":json!({"issues":[row("b")],"hasNextPage":false}).to_string()}]}),
            ),
        ]);
        let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
        let result = issues(
            &mut client,
            &[tool()],
            &Filter {
                label: "dunlin".into(),
                team: Some("APP".into()),
            },
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        let requests = mock.finish();
        assert_eq!(requests[2].body["params"]["arguments"]["label"], "dunlin");
        assert_eq!(requests[3].body["params"]["arguments"]["cursor"], "next");
    }

    #[test]
    fn wrong_labels_and_untrusted_links_are_not_imported() {
        let mut wrong = row("a");
        wrong["labels"] = json!(["other"]);
        let mock = Mock::new(vec![
            Reply::initialize(),
            Reply::notification(),
            Reply::result(
                "tools/call",
                json!({"structuredContent":{"issues":[wrong],"hasNextPage":false}}),
            ),
        ]);
        let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
        assert_eq!(
            issues(
                &mut client,
                &[tool()],
                &Filter {
                    label: "dunlin".into(),
                    team: None
                }
            )
            .unwrap_err(),
            Error::Protocol
        );
        mock.finish();
        let mut wrong = row("a");
        wrong["url"] = json!("https://127.0.0.1/private");
        assert!(issue(&wrong).is_err());
    }

    #[test]
    fn missing_filter_capability_is_not_an_unfiltered_fallback() {
        let mock = Mock::new(vec![Reply::initialize(), Reply::notification()]);
        let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
        let tool = Tool {
            name: "list_issues".into(),
            input_schema: json!({"properties":{"limit":{}}}),
        };
        assert_eq!(
            issues(
                &mut client,
                &[tool],
                &Filter {
                    label: "dunlin".into(),
                    team: None
                }
            )
            .unwrap_err(),
            Error::Unsupported
        );
        mock.finish();
    }

    #[test]
    fn status_updates_use_an_advertised_tool_and_only_id_and_state() {
        let mock = Mock::new(vec![
            Reply::initialize(),
            Reply::notification(),
            Reply::result("tools/call", json!({"content":[]})),
        ]);
        let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
        let tool = Tool {
            name: "save_issue".into(),
            input_schema: json!({"properties":{"id":{},"state":{}}}),
        };
        set_status(&mut client, &[tool], "issue-id", "In Review").unwrap();
        let requests = mock.finish();
        assert_eq!(
            requests[2].body["params"]["arguments"],
            json!({"id":"issue-id","state":"In Review"})
        );
    }

    #[test]
    fn cached_inboxes_reject_unbounded_duplicate_or_mistagged_rows() {
        let value = issue(&row("a")).unwrap();
        let mut inbox = Inbox {
            filter: Some(Filter {
                label: "dunlin".into(),
                team: None,
            }),
            issues: vec![value.clone()],
            ..Inbox::default()
        };
        assert!(inbox.validate().is_ok());
        inbox.issues.push(value.clone());
        assert!(inbox.validate().is_err());
        inbox.issues = vec![value];
        inbox.issues[0].labels = vec!["other".into()];
        assert!(inbox.validate().is_err());
        inbox.issues.clear();
        inbox.error = Some("x".repeat(8193));
        assert!(inbox.validate().is_err());
    }

    #[test]
    fn missing_pagination_metadata_is_not_silently_treated_as_a_complete_inbox() {
        let mock = Mock::new(vec![
            Reply::initialize(),
            Reply::notification(),
            Reply::result(
                "tools/call",
                json!({"structuredContent":{"issues":[row("a")]}}),
            ),
        ]);
        let mut client = Client::connect(&mock.url, "fixture-key").unwrap();
        assert_eq!(
            issues(
                &mut client,
                &[tool()],
                &Filter {
                    label: "dunlin".into(),
                    team: None
                }
            )
            .unwrap_err(),
            Error::Unsupported
        );
        mock.finish();
    }
}
