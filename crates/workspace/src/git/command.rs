//! Per-process repository selection. Account/SSH credentials and author identity
//! remain intact; inherited repository paths/config injections cannot retarget Git.
use std::{
    collections::BTreeSet,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::Command,
};
fn selector(key: &OsStr) -> bool {
    let key = key.to_string_lossy().to_ascii_uppercase();
    key.starts_with("GIT_CONFIG")
        || [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_INDEX_FILE",
            "GIT_NAMESPACE",
            "GIT_SHALLOW_FILE",
            "GIT_GRAFT_FILE",
            "GIT_REPLACE_REF_BASE",
            "GIT_QUARANTINE_PATH",
            "GIT_CEILING_DIRECTORIES",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_PREFIX",
            "GIT_INTERNAL_SUPER_PREFIX",
            "GIT_SUPER_PREFIX",
            "GIT_LITERAL_PATHSPECS",
            "GIT_GLOB_PATHSPECS",
            "GIT_NOGLOB_PATHSPECS",
            "GIT_ICASE_PATHSPECS",
        ]
        .contains(&key.as_str())
}
pub(crate) fn scrub(command: &mut Command) {
    let keys: BTreeSet<_> = std::env::vars_os()
        .map(|(key, _)| key)
        .chain(command.get_envs().map(|(key, _)| key.to_os_string()))
        .filter(|key| selector(key))
        .collect();
    for key in keys {
        command.env_remove(key);
    }
}
/// Discovery follows literal .git entries, never repo/global core.worktree.
/// Retain -C at the requested subdirectory so relative pathspec semantics stay
/// unchanged for desktop callers. Native callers validate an exact root first.
fn worktree_root(path: &Path) -> PathBuf {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_owned()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        }
    });
    canonical
        .ancestors()
        .find(|root| std::fs::symlink_metadata(root.join(".git")).is_ok())
        .unwrap_or(&canonical)
        .to_owned()
}
pub(super) fn bind_worktree(command: &mut Command, path: &Path) {
    scrub(command);
    let root = worktree_root(path);
    command
        .arg("--work-tree")
        .arg(&root)
        .arg("-c")
        .arg(format!("core.worktree={}", root.to_string_lossy()))
        .arg("-C")
        .arg(path);
}
pub(super) fn worktree(path: &str) -> Command {
    let mut command = Command::new("git");
    bind_worktree(&mut command, Path::new(path));
    command
}
/// Explicit object-only/bare or clone/init helper: no synthetic working tree.
/// Caller supplies its intended --git-dir/--bare or destination operands.
pub(super) fn directory(path: &str) -> Command {
    let mut command = Command::new("git");
    scrub(&mut command);
    command.args(["-C", path]);
    command
}
#[cfg(test)]
mod tests;
