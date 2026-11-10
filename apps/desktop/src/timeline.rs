//! A deterministic file-edit projection of the persisted conversation. ACP
//! snapshots replace edits; neither renders nor repeated updates add activity.
use crate::harness::ChatItem;
use dunlin_agent::FileChange;
use dunlin_domain::{ToolKind, ToolStatus};
use std::collections::HashSet;

#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub id: String,
    pub path: String,
    pub at_ms: u64,
    pub timing_known: bool,
    pub additions: Option<u32>,
    pub deletions: Option<u32>,
    pub diff: Option<String>,
    pub source: usize,
    pub turn: usize,
    pub agent: String,
}

#[derive(Clone, Debug)]
pub(crate) struct Prompt {
    pub at_ms: u64,
    pub label: String,
}

#[derive(Default)]
pub(crate) struct Snapshot {
    pub edits: Vec<Edit>,
    pub prompts: Vec<Prompt>,
    pub active: Vec<String>,
    pub duration_ms: u64,
    pub origin_ms: u64,
}

impl Snapshot {
    pub fn totals(&self, turn: Option<usize>) -> (u64, u64, usize) {
        let mut added = 0;
        let mut removed = 0;
        let mut files = HashSet::new();
        for edit in self
            .edits
            .iter()
            .filter(|edit| turn.is_none_or(|turn| edit.turn == turn))
        {
            added += u64::from(edit.additions.unwrap_or(0));
            removed += u64::from(edit.deletions.unwrap_or(0));
            files.insert(&edit.path);
        }
        (added, removed, files.len())
    }
}

pub(crate) fn project(items: &[ChatItem]) -> Snapshot {
    let mut snapshot = Snapshot::default();
    let mut turn = 0;
    let mut seen = HashSet::new();
    for (index, item) in items.iter().enumerate() {
        if let ChatItem::User {
            text,
            at_ms,
            timing,
        } = item
        {
            turn = snapshot.prompts.len();
            let start = timing
                .as_ref()
                .map(|timing| timing.started_at_ms)
                .filter(|at| *at > 0)
                .unwrap_or(*at_ms);
            observe(&mut snapshot, start);
            if let Some(end) = timing.as_ref().and_then(|timing| timing.finished_at_ms) {
                observe(&mut snapshot, end);
            }
            snapshot.prompts.push(Prompt {
                at_ms: start,
                label: text
                    .lines()
                    .next()
                    .unwrap_or("Prompt")
                    .chars()
                    .take(100)
                    .collect(),
            });
        }
        collect(item, index, turn, "", "Agent", &mut snapshot, &mut seen);
    }
    let origin = snapshot.origin_ms;
    snapshot.duration_ms = snapshot.duration_ms.saturating_sub(origin);
    for edit in &mut snapshot.edits {
        edit.at_ms = edit.at_ms.saturating_sub(origin);
    }
    for prompt in &mut snapshot.prompts {
        prompt.at_ms = prompt.at_ms.saturating_sub(origin);
    }
    snapshot
        .edits
        .sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.id.cmp(&b.id)));
    snapshot.active.sort();
    snapshot.active.dedup();
    snapshot
}

fn observe(snapshot: &mut Snapshot, at: u64) {
    if at > 0 {
        snapshot.origin_ms = if snapshot.origin_ms == 0 {
            at
        } else {
            snapshot.origin_ms.min(at)
        };
        snapshot.duration_ms = snapshot.duration_ms.max(at);
    }
}

fn collect(
    item: &ChatItem,
    source: usize,
    turn: usize,
    scope: &str,
    agent: &str,
    snapshot: &mut Snapshot,
    seen: &mut HashSet<String>,
) {
    match item {
        ChatItem::Tool {
            view,
            duration_ms,
            at_ms,
        } => {
            observe(snapshot, *at_ms);
            let at = if *at_ms == 0 {
                0
            } else {
                at_ms.saturating_add(*duration_ms)
            };
            observe(snapshot, at);
            if view.kind == ToolKind::Edit
                && matches!(view.status, ToolStatus::Running | ToolStatus::Waiting)
            {
                snapshot.active.extend(view.locations.iter().cloned());
                snapshot
                    .active
                    .extend(view.file_changes.iter().map(|change| change.path.clone()));
            }
            // A proposed diff can precede approval. Count only successful tools.
            if view.status != ToolStatus::Done {
                return;
            }
            let changes = if !view.file_changes.is_empty() {
                view.file_changes.clone()
            } else {
                view.diff.as_deref().map(parse_patch).unwrap_or_default()
            };
            let covered: HashSet<_> = changes.iter().map(|change| change.path.clone()).collect();
            if !changes.is_empty() {
                for (ordinal, change) in changes.into_iter().enumerate() {
                    if change.additions == 0 && change.deletions == 0 {
                        continue;
                    }
                    let id = format!("{scope}/{turn}/{}:{ordinal}", view.id);
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    snapshot.edits.push(Edit {
                        id,
                        path: change.path,
                        at_ms: at,
                        timing_known: *at_ms > 0,
                        additions: Some(change.additions),
                        deletions: Some(change.deletions),
                        diff: (!change.diff.is_empty()).then_some(change.diff),
                        source,
                        turn,
                        agent: agent.into(),
                    });
                }
            }
            if view.kind == ToolKind::Edit {
                // Binary/legacy edits have real paths but no trustworthy line
                // counts. Never divide a tool's totals across multiple files.
                for path in &view.locations {
                    if covered.contains(path) {
                        continue;
                    }
                    let id = format!("{scope}/{turn}/{}:{path}", view.id);
                    if !seen.insert(id.clone()) {
                        continue;
                    }
                    snapshot.edits.push(Edit {
                        id,
                        path: path.clone(),
                        at_ms: at,
                        timing_known: *at_ms > 0,
                        additions: None,
                        deletions: None,
                        diff: None,
                        source,
                        turn,
                        agent: agent.into(),
                    });
                }
            }
        }
        ChatItem::Child {
            id, title, items, ..
        } => {
            let scope = format!("{scope}/{id}");
            for item in items {
                collect(
                    item,
                    source,
                    turn,
                    &scope,
                    title.as_deref().unwrap_or("Subagent"),
                    snapshot,
                    seen,
                );
            }
        }
        ChatItem::AgentText { at_ms, .. }
        | ChatItem::Thought { at_ms, .. }
        | ChatItem::Plan { at_ms, .. }
        | ChatItem::Permission { at_ms, .. }
        | ChatItem::Error { at_ms, .. } => observe(snapshot, *at_ms),
        _ => {}
    }
}

/// Legacy imports contain unified patches instead of structured per-file data.
/// Track hunk lengths so source lines resembling file headers stay source lines.
pub(crate) fn parse_patch(patch: &str) -> Vec<FileChange> {
    if patch.ends_with("… [truncated]") {
        return Vec::new();
    }
    let mut changes = Vec::<FileChange>::new();
    let mut old_path = String::new();
    let mut section = String::new();
    let mut current: Option<FileChange> = None;
    let mut remaining = (0usize, 0usize);
    for line in patch.lines() {
        if remaining == (0, 0) && line.starts_with("--- ") {
            if let Some(change) = current.take() {
                changes.push(change);
            }
            old_path = patch_path(&line[4..]);
            section = format!("{line}\n");
            continue;
        }
        if remaining == (0, 0) && line.starts_with("+++ ") {
            let path = patch_path(&line[4..]);
            current = Some(FileChange {
                path: if path == "/dev/null" {
                    old_path.clone()
                } else {
                    path
                },
                additions: 0,
                deletions: 0,
                diff: format!("{section}{line}\n"),
            });
            continue;
        }
        let Some(change) = current.as_mut() else {
            continue;
        };
        if line.starts_with("diff --git ") {
            remaining = (0, 0);
            continue;
        }
        change.diff.push_str(line);
        change.diff.push('\n');
        if line.starts_with("@@ ") {
            let mut ranges = line.split_whitespace().skip(1);
            let length = |range: Option<&str>| {
                range
                    .and_then(|range| {
                        range
                            .split_once(',')
                            .map(|(_, len)| len.parse().unwrap_or(0))
                            .or(Some(1))
                    })
                    .unwrap_or(0)
            };
            remaining = (length(ranges.next()), length(ranges.next()));
        } else if remaining != (0, 0) {
            match line.as_bytes().first() {
                Some(b'+') => {
                    change.additions += 1;
                    remaining.1 = remaining.1.saturating_sub(1);
                }
                Some(b'-') => {
                    change.deletions += 1;
                    remaining.0 = remaining.0.saturating_sub(1);
                }
                Some(b' ') => {
                    remaining.0 = remaining.0.saturating_sub(1);
                    remaining.1 = remaining.1.saturating_sub(1);
                }
                _ => {}
            }
        }
    }
    if let Some(change) = current {
        changes.push(change);
    }
    changes
}

fn patch_path(header: &str) -> String {
    let path = header.split('\t').next().unwrap_or(header);
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

pub(crate) fn lanes(edits: &[&Edit], active: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    edits
        .iter()
        .map(|edit| &edit.path)
        .chain(active.iter())
        .filter(|path| seen.insert((*path).clone()))
        .cloned()
        .collect()
}

pub(crate) fn time_label(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// Axis windows use human-sized buckets rather than continuously rescaling.
pub(crate) fn axis_end(ms: u64) -> u64 {
    let step = match ms {
        0..=60_000 => 15_000,
        60_001..=300_000 => 60_000,
        300_001..=1_800_000 => 300_000,
        _ => 900_000,
    };
    ms.max(step).div_ceil(step).saturating_mul(step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dunlin_agent::ToolCallView;
    fn tool(id: &str, at: u64, status: ToolStatus) -> ChatItem {
        ChatItem::Tool {
            view: ToolCallView {
                id: id.into(),
                title: "Edit".into(),
                kind: ToolKind::Edit,
                status,
                detail: String::new(),
                locations: vec!["a.rs".into()],
                additions: 999,
                deletions: 999,
                diff: Some("--- a/a.rs\n+++ b/a.rs\n@@ -1,2 +1,2 @@\n same\n-old\n+new\n".into()),
                file_changes: Vec::new(),
            },
            at_ms: at,
            duration_ms: 50,
        }
    }
    #[test]
    fn edits_use_actual_patch_counts_and_stable_completion_times() {
        let data = project(&[
            tool("one", 1000, ToolStatus::Done),
            tool("two", 2000, ToolStatus::Done),
        ]);
        assert_eq!(data.totals(None), (2, 2, 1));
        assert_eq!(data.edits[0].at_ms, 50);
        assert_eq!(data.edits[1].at_ms, 1050);
        let restored = serde_json::from_str::<Vec<ChatItem>>(
            &serde_json::to_string(&vec![tool("one", 1000, ToolStatus::Done)]).unwrap(),
        )
        .unwrap();
        assert_eq!(project(&restored).totals(None), (1, 1, 1));
    }
    #[test]
    fn failed_waiting_and_running_proposals_never_inflate_totals() {
        let data = project(&[
            tool("fail", 1000, ToolStatus::Failed),
            tool("wait", 2000, ToolStatus::Waiting),
            tool("run", 3000, ToolStatus::Running),
        ]);
        assert!(data.edits.is_empty());
        assert_eq!(data.active, ["a.rs"]);
    }
    #[test]
    fn multiple_files_deletions_and_header_like_source_are_counted() {
        let changes = parse_patch(
            "--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n--- source\n+++ source\n--- a/deleted.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-bye\n",
        );
        assert_eq!(changes.len(), 2);
        assert_eq!((changes[0].additions, changes[0].deletions), (1, 1));
        assert_eq!(changes[1].path, "deleted.txt");
        assert_eq!((changes[1].additions, changes[1].deletions), (0, 1));
    }
    #[test]
    fn subagents_are_scoped_and_duplicate_snapshots_are_not_summed() {
        let data = project(&[
            tool("same", 1000, ToolStatus::Done),
            tool("same", 1000, ToolStatus::Done),
            ChatItem::Child {
                id: "child".into(),
                parent_id: "root".into(),
                title: None,
                items: vec![tool("same", 2000, ToolStatus::Done)],
                at_ms: 2000,
            },
        ]);
        assert_eq!(data.totals(None), (2, 2, 1));
        assert_eq!(data.edits[1].source, 2);
        assert_eq!(data.edits[1].agent, "Subagent");
    }
    #[test]
    fn unknown_metadata_stays_unknown_and_no_edit_is_invented() {
        assert!(project(&[]).edits.is_empty());
        let mut item = tool("legacy", 0, ToolStatus::Done);
        if let ChatItem::Tool { view, .. } = &mut item {
            view.diff = None;
        }
        let data = project(&[item]);
        assert_eq!(data.totals(None), (0, 0, 1));
        assert!(!data.edits[0].timing_known);
        assert!(data.edits[0].additions.is_none());
    }
    #[test]
    fn prompt_scope_does_not_merge_reused_tool_ids_or_invent_edits_in_idle_gaps() {
        let data = project(&[
            ChatItem::User {
                text: "First".into(),
                timing: None,
                at_ms: 1000,
            },
            tool("same", 1000, ToolStatus::Done),
            ChatItem::User {
                text: "Second".into(),
                timing: None,
                at_ms: 3_601_000,
            },
            tool("same", 3_601_000, ToolStatus::Done),
        ]);
        assert_eq!(data.edits.len(), 2);
        assert_eq!(data.totals(Some(0)), (1, 1, 1));
        assert_eq!(data.totals(Some(1)), (1, 1, 1));
        assert_eq!(data.edits[1].at_ms, 3_600_050);
    }

    #[test]
    fn partial_legacy_patches_never_claim_full_counts_or_hide_other_files() {
        let mut item = tool("partial", 1000, ToolStatus::Done);
        if let ChatItem::Tool { view, .. } = &mut item {
            view.locations.push("image.png".into());
        }
        let data = project(&[item.clone()]);
        assert_eq!(data.totals(None), (1, 1, 2));
        assert!(
            data.edits
                .iter()
                .any(|edit| edit.path == "image.png" && edit.additions.is_none())
        );
        if let ChatItem::Tool { view, .. } = &mut item {
            view.diff.as_mut().unwrap().push_str("… [truncated]");
        }
        let data = project(&[item]);
        assert!(data.edits.iter().all(|edit| edit.additions.is_none()));
    }

    #[test]
    fn large_sessions_keep_one_lane_per_file_and_bucket_the_axis() {
        let items: Vec<_> = (0..10_000)
            .map(|i| tool(&i.to_string(), 1000 + i * 100, ToolStatus::Done))
            .collect();
        let data = project(&items);
        assert_eq!(data.totals(None), (10_000, 10_000, 1));
        assert_eq!(lanes(&data.edits.iter().collect::<Vec<_>>(), &[]).len(), 1);
        assert!(axis_end(data.duration_ms) >= data.duration_ms);
        assert_eq!(axis_end(16_000), 30_000);
    }
}
