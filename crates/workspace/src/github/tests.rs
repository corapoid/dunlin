use super::*;
use serde_json::{Value, json};
use std::{cell::RefCell, ffi::OsString, fs, process::Command, time::Instant};
thread_local! {static GH:RefCell<Option<OsString>>=const{RefCell::new(None)};}
pub(super) fn executable() -> OsString {
    GH.with(|v| v.borrow().clone().unwrap_or_else(|| OsString::from("gh")))
}
struct Fixture {
    dir: tempfile::TempDir,
    root: String,
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
        git(&["remote", "add", "origin", "git@github.com:owner/repo.git"]);
        let script = dir.path().join("mock-gh");
        fs::write(&script, r##"#!/usr/bin/env python3
import json,sys,pathlib
root=pathlib.Path(__file__).parent
a=sys.argv[1:]
method=a[a.index('--method')+1] if '--method' in a else 'GET'
endpoint=next((s for s in a if s=='search/repositories' or s.startswith('repos/')), '')
if a==['--version']: endpoint='version'
if a[:2]==['auth','status']: endpoint='auth'
if a[:2]==['repo','clone']: endpoint='clone'
with (root/'calls').open('a') as f: f.write(json.dumps({'args':a,'method':method,'endpoint':endpoint})+'\n')
fixtures=json.loads((root/'responses').read_text())
key=method+' '+endpoint
if endpoint=='search/repositories': key+=' '+next(s for s in a if s.startswith('page='))
value=fixtures.get(key, {'__error':True})
if isinstance(value,dict) and '__error' in value:
 print('private secret error token',file=sys.stderr);sys.exit(1)
if endpoint=='clone':
 import subprocess
 subprocess.run(['git','init','-q','--template=',a[3]],check=True)
if '--include' in a:
 print('HTTP/2.0 200 OK')
 if endpoint=='search/repositories':
  page=int(next(s for s in a if s.startswith('page=')).split('=')[1])
  if value.pop('__next', value.get('total_count',0)>((page-1)*100+len(value.get('items',[])))):
   print('Link: <https://api.github.com/search/repositories?page='+str(page+1)+'>; rel="next"')
 print()
if '--silent' not in a: print(json.dumps(value))
"##).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        GH.with(|v| *v.borrow_mut() = Some(script.into_os_string()));
        let f = Self {
            root: root.to_string_lossy().into_owned(),
            dir,
        };
        f.responses(json!({}));
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
fn repository_metadata_uses_exact_read_only_identity_and_validates_topics() {
    let f = Fixture::new();
    f.responses(json!({"GET repos/owner/repo": {"full_name":"Owner/Repo", "language":"Rust", "topics":["rust", "cloud", "rust"]}}));
    let result = metadata(&f.root).unwrap();
    assert_eq!(result.topics, ["cloud", "rust"]);
    assert_eq!(result.language.as_deref(), Some("Rust"));
    let calls = f.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0]["method"], "GET");
    assert_eq!(calls[0]["endpoint"], "repos/owner/repo");
    assert!(
        metadata(&f.root).is_ok(),
        "metadata does not require a HEAD commit"
    );
    for response in [
        json!({"full_name":"different/repo", "language":null, "topics":[]}),
        json!({"full_name":"owner/repo", "language":null, "topics":["bad\nlabel"]}),
        json!({"__error":true}),
    ] {
        f.responses(json!({"GET repos/owner/repo":response}));
        assert!(!metadata(&f.root).unwrap_err().contains("private secret"));
    }
}
#[test]
fn import_preview_uses_native_query_and_all_pages_without_shell_interpolation() {
    let f = Fixture::new();
    let items: Vec<_> = (0..100)
        .map(|index| json!({"full_name":format!("owner/repo-{index}")}))
        .collect();
    f.responses(json!({"GET search/repositories page=1":{"total_count":101,"incomplete_results":false,"items":items}, "GET search/repositories page=2":{"total_count":101,"incomplete_results":false,"items":[{"full_name":"other/repo"}]}, "GET repos/owner/repo":{"full_name":"Owner/Repo"}}));
    let query = "org:owner archived:false stars:>=10 $(touch SHOULD_NOT_EXIST)";
    let repos = imports::preview(&imports::Source::Search(query.into())).unwrap();
    assert_eq!(repos.len(), 101);
    assert_eq!(repos[100], "other/repo");
    let calls = f.calls();
    assert_eq!(calls.len(), 2);
    assert!(
        calls[0]["args"]
            .as_array()
            .unwrap()
            .contains(&json!(format!("q={query}")))
    );
    assert!(
        calls[1]["args"]
            .as_array()
            .unwrap()
            .contains(&json!("page=2"))
    );
    assert_eq!(
        imports::preview(&imports::Source::Repository("owner/repo".into())).unwrap(),
        ["Owner/Repo"]
    );
}
#[test]
fn import_preview_refuses_partial_oversized_duplicate_and_invalid_search_results() {
    let f = Fixture::new();
    for page in [
        json!({"total_count":1,"incomplete_results":true,"items":[{"full_name":"o/r"}]}),
        json!({"total_count":1001,"incomplete_results":false,"items":[]}),
        json!({"total_count":2,"incomplete_results":false,"items":[{"full_name":"o/r"},{"full_name":"O/R"}]}),
        json!({"total_count":1,"incomplete_results":false,"items":[{"full_name":"../escape"}]}),
        json!({"total_count":2,"incomplete_results":false,"items":[{"full_name":"o/r"}]}),
    ] {
        f.responses(json!({"GET search/repositories page=1":page}));
        assert!(imports::preview(&imports::Source::Search("org:owner".into())).is_err());
    }
    f.responses(json!({"GET search/repositories page=1":{"total_count":0,"incomplete_results":false,"items":[]}}));
    assert!(
        imports::preview(&imports::Source::Search("org:owner".into()))
            .unwrap()
            .is_empty()
    );
    f.responses(json!({}));
    assert!(
        !imports::preview(&imports::Source::Repository("owner/repo".into()))
            .unwrap_err()
            .contains("secret")
    );
}
#[test]
fn import_status_distinguishes_installation_from_authorization_without_returning_tokens() {
    let f = Fixture::new();
    let missing = imports::status();
    assert!(!missing.installed && !missing.authenticated);
    f.responses(json!({"GET version":"gh mock"}));
    let unauthenticated = imports::status();
    assert!(unauthenticated.installed && !unauthenticated.authenticated);
    f.responses(json!({"GET version":"gh mock", "GET auth":{"token":"not-returned"}}));
    let ready = imports::status();
    assert!(ready.installed && ready.authenticated);
    assert!(!ready.message.contains("not-returned"));
}
#[test]
fn import_preview_handles_the_full_github_limit_and_refuses_changing_page_counts() {
    let f = Fixture::new();
    let mut responses = serde_json::Map::new();
    for page in 1..=10 {
        let items: Vec<_> = (0..100)
            .map(|index| json!({"full_name":format!("owner/repo-{}", (page - 1) * 100 + index)}))
            .collect();
        responses.insert(
            format!("GET search/repositories page={page}"),
            json!({"total_count":1000,"incomplete_results":false,"items":items}),
        );
    }
    f.responses(Value::Object(responses.clone()));
    assert_eq!(
        imports::preview(&imports::Source::Search("org:owner".into()))
            .unwrap()
            .len(),
        1000
    );
    responses.get_mut("GET search/repositories page=2").unwrap()["total_count"] = json!(999);
    f.responses(Value::Object(responses));
    assert!(
        imports::preview(&imports::Source::Search("org:owner".into()))
            .unwrap_err()
            .contains("changed")
    );
}

#[test]
fn search_uses_the_actual_final_page_even_when_the_reported_total_is_larger() {
    let f = Fixture::new();
    let items: Vec<_> = (0..42)
        .map(|index| json!({"full_name":format!("owner/matching-{index}")}))
        .collect();
    f.responses(json!({"GET search/repositories page=1":{"total_count":54072,"incomplete_results":false,"items":items,"__next":false}}));
    let query = "org:owner in:name matching archived:false";
    assert_eq!(
        imports::preview(&imports::Source::Search(query.into()))
            .unwrap()
            .len(),
        42
    );
    assert_eq!(f.calls().len(), 1);
    assert!(
        f.calls()[0]["args"]
            .as_array()
            .unwrap()
            .contains(&json!(format!("q={query}")))
    );
}

#[test]
fn search_follows_next_links_even_when_a_page_has_fewer_than_100_repositories() {
    let f = Fixture::new();
    f.responses(json!({
        "GET search/repositories page=1":{"total_count":54072,"incomplete_results":false,"items":[{"full_name":"owner/one"}],"__next":true},
        "GET search/repositories page=2":{"total_count":54072,"incomplete_results":false,"items":[{"full_name":"owner/two"}],"__next":false}
    }));
    assert_eq!(
        imports::preview(&imports::Source::Search(
            "org:owner in:name matching".into()
        ))
        .unwrap(),
        ["owner/one", "owner/two"]
    );
    assert_eq!(f.calls().len(), 2);
}
#[test]
fn imports_clone_with_gh_and_preserve_existing_or_partial_destinations() {
    let f = Fixture::new();
    f.responses(json!({"GET clone":{}}));
    let destination = f.dir.path().join("clones with spaces");
    let path = imports::clone_repository("owner/imported", &destination).unwrap();
    assert!(path.join(".git").is_dir());
    assert!(imports::clone_repository("owner/imported", &destination).is_err());
    let calls = f.calls();
    assert_eq!(calls.len(), 1);
    let args = calls[0]["args"].as_array().unwrap();
    assert_eq!(args[2], "https://github.com/owner/imported");
    assert!(args.contains(&json!("core.hooksPath=/dev/null")));
    f.responses(json!({}));
    let error = imports::clone_repository("owner/failed", &destination).unwrap_err();
    assert!(destination.join("owner/failed").is_dir());
    assert!(error.contains("partial files remain"));
    assert!(!error.contains("secret"));
}
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
