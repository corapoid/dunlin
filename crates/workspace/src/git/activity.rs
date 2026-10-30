//! Branch dates and non-mutating upstream checks. Never fetches objects/refs,
//! opens an auth prompt intentionally, merges, or exposes remote URLs/errors.
use super::GitChange;
use serde::{Deserialize, Serialize};
use std::{
    hash::{Hash, Hasher},
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant, UNIX_EPOCH},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct BranchActivity {
    pub last_commit_ms: Option<u64>,
    pub modified_ms: u64,
    pub local_oid: Option<String>,
    pub canonical_root: Option<String>,
    pub head_ref: Option<String>,
    pub remote_fingerprint: Option<u64>,
    pub upstream: Option<String>,
    pub tracking_oid: Option<String>,
    pub remote_name: Option<String>,
    pub remote_ref: Option<String>,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}
impl BranchActivity {
    pub fn same_target(&self, other: &Self) -> bool {
        self.local_oid == other.local_oid
            && self.canonical_root == other.canonical_root
            && self.head_ref == other.head_ref
            && self.remote_fingerprint == other.remote_fingerprint
            && self.upstream == other.upstream
            && self.remote_name == other.remote_name
            && self.remote_ref == other.remote_ref
            && self.tracking_oid == other.tracking_oid
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteState {
    Known,
    Changed,
    Missing,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoteStatus {
    pub state: RemoteState,
    pub ahead: Option<u64>,
    pub behind: Option<u64>,
}

// Unlike general Git operations these reads must never trigger a promisor
// object's lazy fetch, a credential dialog, or an unbounded child process.
pub(super) fn git(path: &str, args: &[&str]) -> Result<String, String> {
    git_limited(path, args, 65536)
}
pub(super) fn git_limited(path: &str, args: &[&str], limit: u64) -> Result<String, String> {
    let mut command = Command::new("git");
    command
        .args(["-C", path])
        .args(args)
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C");
    let (status, bytes) = bounded_command(command, Duration::from_secs(2), limit)?;
    if !status.success() {
        return Err("Local Git metadata is unavailable".into());
    }
    String::from_utf8(bytes).map_err(|_| "Invalid Git metadata".into())
}

fn mtime(path: &Path) -> u64 {
    std::fs::symlink_metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}
fn counts(path: &str, local: &str, other: &str) -> Option<(u64, u64)> {
    let result = git(
        path,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{local}...{other}"),
        ],
    )
    .ok()?;
    let mut values = result.split_whitespace();
    Some((values.next()?.parse().ok()?, values.next()?.parse().ok()?))
}
pub fn branch_activity(path: &str, changes: &[GitChange]) -> BranchActivity {
    let local_oid: Option<String> = git(path, &["rev-parse", "--verify", "HEAD"])
        .ok()
        .map(|s| s.trim().into());
    let last_commit_ms = local_oid
        .as_ref()
        .and_then(|oid| git(path, &["log", "-1", "--format=%ct", oid]).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .and_then(|s| s.checked_mul(1000));
    let modified_ms = changes
        .iter()
        .filter_map(|change| crate::fs::resolve(path, &change.path).ok())
        .map(|p| mtime(&p).max(p.parent().map(mtime).unwrap_or(0)))
        .chain([mtime(Path::new(path)), last_commit_ms.unwrap_or(0)])
        .max()
        .unwrap_or(0);
    let mut result = BranchActivity {
        last_commit_ms,
        modified_ms,
        canonical_root: std::fs::canonicalize(path)
            .ok()
            .map(|p| p.to_string_lossy().into_owned()),
        local_oid,
        ..Default::default()
    };
    let Some(head_ref) = git(path, &["symbolic-ref", "--quiet", "HEAD"]).ok() else {
        return result;
    };
    result.head_ref = Some(head_ref.trim().into());
    let Ok(upstream) = git(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream:short)%00%(upstream)%00%(upstream:remotename)%00%(upstream:remoteref)",
            head_ref.trim(),
        ],
    ) else {
        return result;
    };
    let fields: Vec<_> = upstream.trim_end_matches('\n').split('\0').collect();
    if fields.len() == 4 && !fields[1].is_empty() {
        result.upstream = Some(fields[0].into());
        result.tracking_oid = git(
            path,
            &[
                "rev-parse",
                "--verify",
                &format!("{}^{{commit}}", fields[1]),
            ],
        )
        .ok()
        .map(|s| s.trim().into());
        result.remote_name = (!fields[2].is_empty() && fields[2] != ".").then(|| fields[2].into());
        result.remote_ref = fields[3]
            .starts_with("refs/heads/")
            .then(|| fields[3].into());
        if let Some(remote) = &result.remote_name
            && !remote.starts_with('-')
            && let Ok(urls) = git(path, &["remote", "get-url", "--all", remote])
        {
            // Configuration identity only; never send credentials/URLs to UI.
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            urls.hash(&mut hash);
            result.remote_fingerprint = Some(hash.finish());
        }
        if let Some((ahead, behind)) = result
            .local_oid
            .as_deref()
            .and_then(|local| counts(path, local, fields[1]))
        {
            result.ahead = Some(ahead);
            result.behind = Some(behind);
        }
    }
    result
}

fn bounded_command(
    mut command: Command,
    timeout: Duration,
    limit: u64,
) -> Result<(std::process::ExitStatus, Vec<u8>), String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command
        .spawn()
        .map_err(|_| "Remote status is unavailable; check Git installation and authentication")?;
    struct Guard {
        child: std::process::Child,
        done: bool,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            if !self.done {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(self.child.id() as i32), libc::SIGKILL);
                }
                let _ = self.child.kill();
                let _ = self.child.wait();
            }
        }
    }
    let mut guard = Guard { child, done: false };
    let mut stdout = guard
        .child
        .stdout
        .take()
        .ok_or("Remote status output is unavailable")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .by_ref()
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let start = Instant::now();
    let status = loop {
        if let Some(status) = guard
            .child
            .try_wait()
            .map_err(|_| "Could not read remote status")?
        {
            break status;
        }
        if start.elapsed() >= timeout {
            return Err("Remote check timed out; cached tracking information may be stale".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let bytes = rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| "Remote check did not finish")?
        .map_err(|_| "Could not read remote status")?;
    if bytes.len() as u64 > limit {
        return Err("Remote status response exceeded its limit".into());
    }
    guard.done = true;
    Ok((status, bytes))
}
pub fn check_remote_status(path: &str, expected: &BranchActivity) -> Result<RemoteStatus, String> {
    let current = branch_activity(path, &[]);
    if current.local_oid.is_none()
        || current.canonical_root.is_none()
        || !current.same_target(expected)
    {
        return Err("The branch changed; refresh repository information".into());
    }
    let remote = current
        .remote_name
        .as_deref()
        .ok_or("No remote upstream configured")?;
    let remote_ref = current
        .remote_ref
        .as_deref()
        .ok_or("No remote branch configured")?;
    if remote.starts_with('-') || !remote_ref.starts_with("refs/heads/") {
        return Err("Unsupported upstream configuration".into());
    }
    let mut command = Command::new("git");
    command
        .args([
            "-C",
            path,
            "-c",
            "credential.interactive=false",
            "ls-remote",
            "--exit-code",
            "--refs",
            "--",
            remote,
            remote_ref,
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_ASKPASS", "/usr/bin/false")
        .env("SSH_ASKPASS", "/usr/bin/false")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0");
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env(
            "GIT_SSH_COMMAND",
            "ssh -o BatchMode=yes -o ConnectTimeout=5",
        );
    }
    let (status, bytes) = bounded_command(command, Duration::from_secs(8), 65536)?;
    if !branch_activity(path, &[]).same_target(&current) {
        return Err("The branch changed during the remote check".into());
    }
    if status.code() == Some(2) {
        return Ok(RemoteStatus {
            state: RemoteState::Missing,
            ahead: None,
            behind: None,
        });
    }
    if !status.success() {
        return Err(
            "Remote status is unavailable; check the connection and Git authentication".into(),
        );
    }
    let output = std::str::from_utf8(&bytes).map_err(|_| "Invalid remote status")?;
    let oid = output
        .lines()
        .find_map(|line| {
            let (oid, name) = line.split_once('\t')?;
            (name == remote_ref
                && matches!(oid.len(), 40 | 64)
                && oid.bytes().all(|c| c.is_ascii_hexdigit()))
            .then_some(oid)
        })
        .ok_or("Remote branch was not advertised")?;
    if !branch_activity(path, &[]).same_target(&current) {
        return Err("The branch changed during the remote check".into());
    }
    if current.local_oid.as_deref() == Some(oid) {
        return Ok(RemoteStatus {
            state: RemoteState::Known,
            ahead: Some(0),
            behind: Some(0),
        });
    }
    if let Some((ahead, behind)) = current
        .local_oid
        .as_deref()
        .and_then(|local| counts(path, local, oid))
    {
        return Ok(RemoteStatus {
            state: RemoteState::Known,
            ahead: Some(ahead),
            behind: Some(behind),
        });
    }
    Ok(RemoteStatus {
        state: RemoteState::Changed,
        ahead: None,
        behind: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{RemoteState, bounded_command, branch_activity, check_remote_status};
    use std::{
        fs,
        path::{Path, PathBuf},
        process::Command,
        time::{Duration, Instant},
    };

    struct Fixture {
        root: PathBuf,
        repo: PathBuf,
        bare: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("dunlin-activity-{}", uuid::Uuid::now_v7()));
            let repo = root.join("repo");
            let bare = root.join("origin.git");
            fs::create_dir_all(&repo).unwrap();
            fs::create_dir(&bare).unwrap();
            run(&repo, &["init", "-q", "-b", "main"]);
            configure(&repo);
            fs::write(repo.join("file.txt"), "original\n").unwrap();
            run(&repo, &["add", "file.txt"]);
            let output = Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["commit", "-qm", "initial"])
                .env("GIT_AUTHOR_DATE", "2001-01-01T00:00:00Z")
                .env("GIT_COMMITTER_DATE", "2002-01-01T00:00:00Z")
                .output()
                .unwrap();
            assert!(output.status.success());
            run(&bare, &["init", "--bare", "-q"]);
            run(&repo, &["remote", "add", "origin", bare.to_str().unwrap()]);
            run(&repo, &["push", "-q", "-u", "origin", "main"]);
            Self { root, repo, bare }
        }
        fn path(&self) -> &str {
            self.repo.to_str().unwrap()
        }
        fn activity(&self) -> super::BranchActivity {
            branch_activity(self.path(), &super::super::load(self.path()).changes)
        }
        fn writer(&self) -> PathBuf {
            let writer = self.root.join("writer");
            fs::create_dir(&writer).unwrap();
            run(
                &writer,
                &[
                    "clone",
                    "-q",
                    "-b",
                    "main",
                    self.bare.to_str().unwrap(),
                    ".",
                ],
            );
            configure(&writer);
            writer
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
    fn run(path: &Path, args: &[&str]) -> String {
        super::super::git(path.to_str().unwrap(), args).unwrap()
    }
    fn configure(path: &Path) {
        run(path, &["config", "user.name", "Dunlin Test"]);
        run(path, &["config", "user.email", "test@example.invalid"]);
        run(path, &["config", "commit.gpgsign", "false"]);
        run(path, &["config", "core.hooksPath", "/dev/null"]);
    }
    fn commit(path: &Path, text: &str) {
        fs::write(path.join("file.txt"), text).unwrap();
        run(path, &["add", "file.txt"]);
        run(path, &["commit", "-qm", "fixture update"]);
    }
    #[test]
    fn committer_date_and_readonly_check_preserve_tracking_refs_index_and_dirty_work() {
        let f = Fixture::new();
        fs::write(f.repo.join("file.txt"), "staged\n").unwrap();
        run(&f.repo, &["add", "file.txt"]);
        fs::write(f.repo.join("file.txt"), "unstaged\n").unwrap();
        fs::write(f.repo.join("private.txt"), "untracked\n").unwrap();
        let activity = f.activity();
        assert_eq!(activity.last_commit_ms, Some(1_009_843_200_000));
        assert!(activity.modified_ms >= activity.last_commit_ms.unwrap());
        let index = fs::read(f.repo.join(".git/index")).unwrap();
        let refs = run(&f.repo, &["show-ref"]);
        let status = check_remote_status(f.path(), &activity).unwrap();
        assert_eq!(status.state, RemoteState::Known);
        assert_eq!(status.behind, Some(0));
        assert_eq!(fs::read(f.repo.join(".git/index")).unwrap(), index);
        assert_eq!(run(&f.repo, &["show-ref"]), refs);
        assert!(!f.repo.join(".git/FETCH_HEAD").exists());
        assert_eq!(
            fs::read_to_string(f.repo.join("file.txt")).unwrap(),
            "unstaged\n"
        );
        assert_eq!(
            fs::read_to_string(f.repo.join("private.txt")).unwrap(),
            "untracked\n"
        );
    }
    #[test]
    fn unknown_remote_commit_requires_fetch_without_inventing_a_behind_count() {
        let f = Fixture::new();
        let writer = f.writer();
        let expected = f.activity();
        let refs = run(&f.repo, &["show-ref"]);
        commit(&writer, "remote update\n");
        run(&writer, &["push", "-q", "origin", "main"]);
        let status = check_remote_status(f.path(), &expected).unwrap();
        assert_eq!(status.state, RemoteState::Changed);
        assert_eq!(status.behind, None);
        assert_eq!(run(&f.repo, &["show-ref"]), refs);
        assert!(!f.repo.join(".git/FETCH_HEAD").exists());
        let remote_oid = run(&writer, &["rev-parse", "HEAD"]);
        assert!(super::git(f.path(), &["cat-file", "-e", remote_oid.trim()]).is_err());
    }
    #[test]
    fn known_objects_allow_exact_behind_and_diverged_counts_without_fetching() {
        let f = Fixture::new();
        let base = run(&f.repo, &["rev-parse", "HEAD"]);
        run(&f.repo, &["switch", "-q", "-c", "future"]);
        commit(&f.repo, "future\n");
        run(&f.repo, &["push", "-q", "origin", "HEAD:main"]);
        run(&f.repo, &["switch", "-q", "main"]);
        run(
            &f.repo,
            &["update-ref", "refs/remotes/origin/main", base.trim()],
        );
        let status = check_remote_status(f.path(), &f.activity()).unwrap();
        assert_eq!((status.ahead, status.behind), (Some(0), Some(1)));
        commit(&f.repo, "local diverged\n");
        let status = check_remote_status(f.path(), &f.activity()).unwrap();
        assert_eq!((status.ahead, status.behind), (Some(1), Some(1)));
        assert!(!f.repo.join(".git/FETCH_HEAD").exists());
    }
    #[test]
    fn deleted_upstream_failure_detached_and_changed_configuration_are_distinct() {
        let f = Fixture::new();
        let expected = f.activity();
        run(&f.bare, &["update-ref", "-d", "refs/heads/main"]);
        assert_eq!(
            check_remote_status(f.path(), &expected).unwrap().state,
            RemoteState::Missing
        );
        fs::rename(&f.bare, f.root.join("offline.git")).unwrap();
        let error = check_remote_status(f.path(), &expected).unwrap_err();
        assert!(!error.contains(f.path()));
        assert!(!error.contains("origin.git"));
        run(
            &f.repo,
            &["remote", "set-url", "origin", "/not/a/real/remote"],
        );
        assert!(
            check_remote_status(f.path(), &expected)
                .unwrap_err()
                .contains("branch changed")
        );
        run(&f.repo, &["switch", "-q", "--detach"]);
        let detached = f.activity();
        assert!(detached.head_ref.is_none());
        assert!(detached.upstream.is_none());
        assert!(check_remote_status(f.path(), &detached).is_err());
    }
    #[test]
    fn unborn_repository_has_no_invented_commit_date() {
        let f = Fixture::new();
        let empty = f.root.join("empty");
        fs::create_dir(&empty).unwrap();
        run(&empty, &["init", "-q", "-b", "main"]);
        let activity = branch_activity(empty.to_str().unwrap(), &[]);
        assert!(activity.local_oid.is_none());
        assert!(activity.last_commit_ms.is_none());
    }
    #[test]
    fn child_process_timeout_and_output_limit_are_bounded() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 30 & wait"]);
        let start = Instant::now();
        assert!(
            bounded_command(command, Duration::from_millis(40), 65536)
                .unwrap_err()
                .contains("timed out")
        );
        assert!(start.elapsed() < Duration::from_secs(2));
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "head -c 70000 /dev/zero"]);
        assert!(bounded_command(command, Duration::from_secs(2), 65536).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn retargeted_project_symlink_cannot_reuse_an_observation() {
        let f = Fixture::new();
        let other = f.root.join("other");
        fs::create_dir(&other).unwrap();
        run(
            &other,
            &["clone", "-q", "-b", "main", f.bare.to_str().unwrap(), "."],
        );
        let link = f.root.join("project");
        std::os::unix::fs::symlink(&f.repo, &link).unwrap();
        let expected = branch_activity(link.to_str().unwrap(), &[]);
        fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(check_remote_status(link.to_str().unwrap(), &expected).is_err());
    }
}
