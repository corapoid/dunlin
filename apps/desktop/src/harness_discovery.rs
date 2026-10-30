//! Read-only local discovery. Never runs a CLI, downloads an adapter or signs in.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Detected {
    pub name: &'static str,
    pub cli: Option<PathBuf>,
    pub adapter: Option<PathBuf>,
    pub docs: &'static str,
}

pub(crate) fn search_paths() -> Vec<PathBuf> {
    let mut paths: Vec<_> = std::env::var_os("PATH")
        .map(|path| {
            std::env::split_paths(&path)
                .filter(|path| path.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    paths.extend(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        paths.extend(
            [
                ".local/bin",
                ".cargo/bin",
                ".bun/bin",
                ".npm-global/bin",
                ".volta/bin",
                ".opencode/bin",
                "Library/pnpm",
            ]
            .map(|dir| PathBuf::from(&home).join(dir)),
        );
    }
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn executable(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn find_in(name: &str, paths: &[PathBuf]) -> Option<PathBuf> {
    if name.contains('/') {
        let path = PathBuf::from(name);
        return executable(&path).then_some(path);
    }
    paths
        .iter()
        .map(|dir| dir.join(name))
        .find(|path| executable(path))
}

fn split_command(command: &str) -> Option<(&str, &str)> {
    let command = command.trim_start();
    if let Some(quote) = command.chars().next().filter(|c| matches!(c, '\'' | '"')) {
        let end = command[1..].find(quote)? + 1;
        Some((&command[1..end], &command[end + 1..]))
    } else {
        let end = command.find(char::is_whitespace).unwrap_or(command.len());
        (end > 0).then_some((&command[..end], &command[end..]))
    }
}

pub(crate) fn resolved_command(command: &str) -> Option<String> {
    let (name, args) = split_command(command)?;
    let path = find_in(name, &search_paths())?;
    let path = path.to_str()?.replace('\'', "'\"'\"'");
    Some(format!("'{path}'{args}"))
}

pub(crate) fn child_path() -> Option<String> {
    std::env::join_paths(search_paths())
        .ok()?
        .into_string()
        .ok()
}

pub(crate) fn local_command(name: &str) -> std::process::Command {
    let mut command =
        std::process::Command::new(find_in(name, &search_paths()).unwrap_or_else(|| name.into()));
    if let Some(path) = child_path() {
        command.env("PATH", path);
    }
    command
}

pub(crate) fn discover() -> Vec<Detected> {
    let paths = search_paths();
    [
        (
            "OpenCode",
            "opencode",
            "opencode",
            "https://opencode.ai/docs/acp/",
        ),
        (
            "Claude Code",
            "claude",
            "claude-agent-acp",
            "https://github.com/zed-industries/claude-agent-acp",
        ),
        (
            "OpenAI Codex",
            "codex",
            "codex-acp",
            "https://github.com/zed-industries/codex-acp",
        ),
    ]
    .into_iter()
    .map(|(name, cli, adapter, docs)| Detected {
        name,
        cli: find_in(cli, &paths),
        adapter: find_in(adapter, &paths),
        docs,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_requires_an_executable_and_preserves_quoted_command_arguments() {
        assert_eq!(
            split_command("'/path with space/agent' --acp"),
            Some(("/path with space/agent", " --acp"))
        );
        assert!(split_command("'broken").is_none());
        assert!(split_command("").is_none());
        assert!(find_in("no-such-dunlin-agent", &[]).is_none());
        assert!(search_paths().iter().all(|path| path.is_absolute()));
        let path = std::env::temp_dir().join(format!("dunlin-discovery-{}", uuid::Uuid::now_v7()));
        std::fs::write(&path, b"not an executable").unwrap();
        assert!(!executable(&path));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            assert!(executable(&path));
        }
        std::fs::remove_file(path).unwrap();
    }
}
