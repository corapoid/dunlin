use super::*;
use std::{cell::RefCell, ffi::OsString, fs, path::Path, process::Command};
thread_local! {static GH:RefCell<Option<OsString>>=const{RefCell::new(None)};}
pub(super) fn executable() -> OsString {
    GH.with(|v| v.borrow().clone().unwrap_or_else(|| OsString::from("gh")))
}
struct Fixture {
    dir: tempfile::TempDir,
    repo: Repository,
}
impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let output = Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(args)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        git(&["init", "--template=", "-qb", "main"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "t@example.invalid"]);
        git(&["config", "commit.gpgsign", "false"]);
        fs::write(root.join("file"), "base").unwrap();
        git(&["add", "file"]);
        git(&["commit", "-qm", "base"]);
        git(&["remote", "add", "origin", "git@github.com:owner/repo.git"]);
        let repo = repository(root.to_str().unwrap(), "origin").unwrap();
        let script = dir.path().join("mock-gh");
        fs::write(&script,r##"#!/usr/bin/env python3
import json,sys,pathlib
root=pathlib.Path(__file__).parent
a=sys.argv[1:]
method=a[a.index('--method')+1] if '--method' in a else 'GET'
endpoint=next((s for s in a if s in ('user','graphql') or s.startswith('repos/')), '')
body=json.load(sys.stdin) if '--input' in a else None
with (root/'calls').open('a') as f: f.write(json.dumps({'args':a,'method':method,'endpoint':endpoint,'body':body})+'\n')
fixtures=json.loads((root/'responses').read_text())
key=method+' '+endpoint
value=fixtures.get(key, {'__error':True})
if isinstance(value,dict) and '__error' in value:
 print('private secret error token',file=sys.stderr);sys.exit(1)
if '--silent' not in a: print(json.dumps(value))
"##).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        GH.with(|v| *v.borrow_mut() = Some(script.into_os_string()));
        let f = Self { dir, repo };
        f.responses(json!({"GET user":{"login":"reviewer"}}));
        f
    }
    fn responses(&self, value: Value) {
        fs::write(
            self.dir.path().join("responses"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.path().join("calls"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
    fn pr(&self) -> Value {
        json!({"node_id":"PR_test","number":42,"title":"Example","body":"Original","state":"open","draft":false,"head":{"ref":"topic","sha":"a".repeat(40)},"base":{"ref":"main","sha":"b".repeat(40)},"updated_at":"2026-11-01T12:00:00Z","mergeable":true,"merged":false})
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        GH.with(|v| *v.borrow_mut() = None);
    }
}
#[test]
fn explicit_github_remotes_only() {
    for url in [
        "https://github.com/owner/repo.git",
        "git@github.com:owner/repo",
        "ssh://git@github.com/owner/repo.git",
    ] {
        assert_eq!(repository_name(url).unwrap(), "owner/repo");
    }
    for url in [
        "https://token@github.com/owner/repo",
        "https://github.com.evil/owner/repo",
        "https://github.com/../repo",
        "git@evil:owner/repo",
        "https://github.com/owner/repo?token=secret",
        "https://github.com/owner/repo/extra",
    ] {
        assert!(repository_name(url).is_err(), "{url}");
    }
}

#[test]
fn line_reviews_and_draft_transitions_use_typed_pinned_targets() {
    let f = Fixture::new();
    f.responses(json!({"GET user":{"login":"reviewer"},"GET repos/owner/repo/pulls/42":f.pr(),"POST repos/owner/repo/pulls/42/comments":{"id":1},"POST graphql":{"data":{"convertPullRequestToDraft":{"pullRequest":{"id":"PR_test","isDraft":true}}}}}));
    execute(
        prepare(
            &f.repo,
            Action::LineComment {
                number: 42,
                path: "src/main.rs".into(),
                line: 7,
                body: "Please test".into(),
            },
        )
        .unwrap(),
    )
    .unwrap();
    execute(
        prepare(
            &f.repo,
            Action::Draft {
                number: 42,
                draft: true,
            },
        )
        .unwrap(),
    )
    .unwrap();
    let calls = f.calls();
    let line = calls
        .iter()
        .find(|c| c["method"] == "POST" && c["endpoint"] == "repos/owner/repo/pulls/42/comments")
        .unwrap();
    assert_eq!(line["body"]["commit_id"], "a".repeat(40));
    assert_eq!(line["body"]["side"], "RIGHT");
    assert_eq!(calls.last().unwrap()["body"]["variables"]["id"], "PR_test");
    assert!(
        prepare(
            &f.repo,
            Action::LineComment {
                number: 42,
                path: "../outside".into(),
                line: 7,
                body: "bad".into()
            }
        )
        .is_err()
    );
}
#[test]
fn untrusted_log_escapes_cannot_forge_ui() {
    assert_eq!(
        display_text(
            "a\x1b[31mred\x1b[0m\x1b]8;;https://evil\x07link\x1b]8;;\x07\u{202e}\0\r\n",
            100
        ),
        "aredlink\n"
    );
    assert!(
        display_text(&"x".repeat(500), 10).ends_with("[truncated; inspect full content on GitHub]")
    );
}
#[test]
fn publication_is_typed_and_review_and_merge_pin_commit() {
    let snapshot = json!({"head":{"sha":"a".repeat(40)}});
    let (_, url, body) = mutations::request(
        &Action::Merge {
            number: 42,
            method: "squash".into(),
        },
        &snapshot,
    );
    assert_eq!(url, "pulls/42/merge");
    assert_eq!(body["sha"], "a".repeat(40));
    assert!(body.get("admin").is_none());
    let (_, _, body) = mutations::request(
        &Action::Review {
            number: 42,
            body: "review".into(),
            event: "APPROVE".into(),
        },
        &snapshot,
    );
    assert_eq!(body["commit_id"], "a".repeat(40));
    assert!(
        serde_json::from_value::<Action>(
            json!({"kind":"merge","number":42,"method":"merge","admin":true})
        )
        .is_err()
    );
}

#[test]
fn prepare_is_read_only_and_execute_submits_one_explicit_pinned_request() {
    let f = Fixture::new();
    f.responses(json!({"GET user":{"login":"reviewer"},"GET repos/owner/repo/pulls/42":f.pr(),"PUT repos/owner/repo/pulls/42/merge":{"merged":true}}));
    let plan = prepare(
        &f.repo,
        Action::Merge {
            number: 42,
            method: "squash".into(),
        },
    )
    .unwrap();
    assert!(f.calls().iter().all(|c| c["method"] == "GET"));
    execute(plan).unwrap();
    let calls = f.calls();
    let writes = calls
        .iter()
        .filter(|c| c["method"] != "GET")
        .collect::<Vec<_>>();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0]["body"],
        json!({"sha":"a".repeat(40),"merge_method":"squash"})
    );
    assert!(!serde_json::to_string(&calls).unwrap().contains("push"));
}
#[test]
fn changed_pr_or_account_never_publishes_and_remote_errors_hide_secrets() {
    let f = Fixture::new();
    let mut responses =
        json!({"GET user":{"login":"reviewer"},"GET repos/owner/repo/pulls/42":f.pr()});
    f.responses(responses.clone());
    let plan = prepare(
        &f.repo,
        Action::Comment {
            number: 42,
            body: "Review".into(),
        },
    )
    .unwrap();
    responses["GET repos/owner/repo/pulls/42"]["head"]["sha"] = json!("c".repeat(40));
    f.responses(responses.clone());
    assert!(execute(plan).is_err());
    let plan = prepare(
        &f.repo,
        Action::Comment {
            number: 42,
            body: "Review".into(),
        },
    )
    .unwrap();
    responses["GET user"]["login"] = json!("other");
    f.responses(responses);
    assert!(execute(plan).is_err());
    assert!(f.calls().iter().all(|c| c["method"] == "GET"));
    f.responses(json!({}));
    let e = account(&f.repo).unwrap_err();
    assert!(!e.contains("private secret"));
}
#[test]
fn create_requires_existing_branches_and_never_pushes_local_files() {
    let f = Fixture::new();
    f.responses(json!({"GET user":{"login":"reviewer"},"GET repos/owner/repo/git/ref/heads/main":{"object":{"sha":"b".repeat(40)}},"GET repos/owner/repo/git/ref/heads/topic":{"object":{"sha":"a".repeat(40)}},"POST repos/owner/repo/pulls":{}}));
    let plan = prepare(
        &f.repo,
        Action::Create {
            base: "main".into(),
            head: "topic".into(),
            title: "Title".into(),
            body: "Body".into(),
            draft: true,
        },
    )
    .unwrap();
    execute(plan).unwrap();
    let calls = f.calls();
    assert_eq!(calls.iter().filter(|c| c["method"] == "POST").count(), 1);
    assert_eq!(calls.last().unwrap()["body"]["draft"], true);
    assert!(
        prepare(
            &f.repo,
            Action::Create {
                base: "main".into(),
                head: "missing".into(),
                title: "Title".into(),
                body: "".into(),
                draft: true
            }
        )
        .is_err()
    );
}
#[test]
fn rejects_changed_local_repository_and_missing_gh_without_fallback() {
    let f = Fixture::new();
    let root = Path::new(&f.repo.root);
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "remote",
            "set-url",
            "origin",
            "https://github.com/other/repo",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(account(&f.repo).is_err());
    assert!(f.calls().is_empty());
}

#[test]
fn missing_cli_and_expired_plan_fail_closed() {
    let f = Fixture::new();
    f.responses(json!({"GET user":{"login":"reviewer"},"GET repos/owner/repo/pulls/42":f.pr()}));
    let mut plan = prepare(
        &f.repo,
        Action::Comment {
            number: 42,
            body: "note".into(),
        },
    )
    .unwrap();
    plan.prepared = Instant::now() - Duration::from_secs(301);
    assert!(execute(plan).is_err());
    GH.with(|v| *v.borrow_mut() = Some(f.dir.path().join("missing-cli").into_os_string()));
    assert!(account(&f.repo).is_err());
    assert!(f.calls().iter().all(|c| c["method"] == "GET"));
}
#[test]
fn jobs_are_bound_to_exact_attempt_and_status_and_logs_fail_while_running() {
    let f = Fixture::new();
    let run = json!({"id":17,"run_attempt":2,"name":"CI","head_sha":"a".repeat(40),"head_branch":"topic","status":"in_progress","conclusion":null,"event":"pull_request"});
    let job = json!({"id":19,"run_id":17,"run_attempt":2,"head_sha":"a".repeat(40),"name":"Tests","status":"in_progress","conclusion":null,"steps":[{"number":1,"name":"Build","status":"completed","conclusion":"success"},{"number":2,"name":"Test","status":"in_progress","conclusion":null}]});
    let mut responses = json!({"GET repos/owner/repo/actions/runs/17":run,"GET repos/owner/repo/actions/runs/17/attempts/2/jobs?per_page=100":{"total_count":1,"jobs":[job]}});
    f.responses(responses.clone());
    let run: Run = serde_json::from_value(run).unwrap();
    let jobs = self::super::jobs(&f.repo, &run).unwrap();
    assert_eq!(jobs[0].steps[1].status, "in_progress");
    assert!(
        logs(&f.repo, &run, &jobs[0])
            .unwrap_err()
            .contains("not a guaranteed live")
    );
    assert!(f.calls().iter().all(|c| c["args"][0] == "api"));
    responses["GET repos/owner/repo/actions/runs/17"]["run_attempt"] = json!(3);
    f.responses(responses);
    assert!(self::super::jobs(&f.repo, &run).is_err());
}
#[cfg(unix)]
#[test]
fn subprocess_limits_cover_large_output_and_unread_stdin() {
    let mut cmd = Command::new("/bin/sh");
    cmd.args(["-c", "while :; do printf abcdefghijklmnopqrstuvwxyz; done"]);
    assert!(process::run(cmd, None, Duration::from_millis(500), 100).is_err());
    let mut cmd = Command::new("/bin/sleep");
    cmd.arg("30");
    let start = Instant::now();
    assert!(
        process::run(
            cmd,
            Some(vec![b'x'; 1024 * 1024]),
            Duration::from_millis(100),
            100
        )
        .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(3));
}
