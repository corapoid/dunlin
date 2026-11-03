//! Read-only local discovery. Never runs a CLI, downloads an adapter or signs in.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Detected {
    pub name: &'static str,
    pub cli: Option<PathBuf>,
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

/// One known ACP agent. The adapter executable is what Dunlin spawns; the
/// optional CLI is only used to describe a CLI-only installation.
pub(crate) struct Spec {
    pub name: &'static str,
    pub cli: &'static str,
    pub command: &'static str,
    pub docs: &'static str,
}

/// A deliberately curated catalog. The ACP registry is much larger, but these
/// agents expose stable local executables and documented ACP entry points.
pub(crate) const SPECS: &[Spec] = &[
    Spec {
        name: "OpenCode",
        cli: "opencode",
        command: "opencode acp",
        docs: "https://opencode.ai/docs/acp/",
    },
    Spec {
        name: "Claude Code",
        cli: "claude",
        command: "claude-agent-acp",
        docs: "https://github.com/zed-industries/claude-agent-acp",
    },
    Spec {
        name: "OpenAI Codex",
        cli: "codex",
        command: "codex-acp",
        docs: "https://github.com/zed-industries/codex-acp",
    },
    Spec {
        name: "Gemini CLI",
        cli: "gemini",
        command: "gemini --acp",
        docs: "https://geminicli.com/docs/cli/acp-mode",
    },
    Spec {
        name: "Qwen Code",
        cli: "qwen",
        command: "qwen --acp",
        docs: "https://qwenlm.github.io/qwen-code-docs/",
    },
    Spec {
        name: "GitHub Copilot",
        cli: "copilot",
        command: "copilot --acp",
        docs: "https://docs.github.com/en/copilot/reference/copilot-cli-reference/acp-server",
    },
    Spec {
        name: "Cursor",
        cli: "cursor-agent",
        command: "cursor-agent acp",
        docs: "https://cursor.com/docs/cli/acp",
    },
    Spec {
        name: "Auggie CLI",
        cli: "auggie",
        command: "auggie --acp",
        docs: "https://docs.augmentcode.com/cli/acp",
    },
    Spec {
        name: "Kiro CLI",
        cli: "kiro-cli",
        command: "kiro-cli acp",
        docs: "https://kiro.dev/docs/cli/acp/",
    },
    Spec {
        name: "OpenClaw",
        cli: "openclaw",
        command: "openclaw acp",
        docs: "https://docs.openclaw.ai/cli/acp",
    },
    Spec {
        name: "Hermes Agent",
        cli: "hermes",
        command: "hermes acp",
        docs: "https://hermes-agent.nousresearch.com/docs/user-guide/features/acp",
    },
    Spec {
        name: "Qoder CLI",
        cli: "qodercli",
        command: "qodercli --acp",
        docs: "https://docs.qoder.com/cli/acp",
    },
];

pub(crate) fn catalog_names() -> impl Iterator<Item = &'static str> {
    SPECS.iter().map(|spec| spec.name)
}

pub(crate) fn catalog_command(name: &str) -> Option<&'static str> {
    SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.command)
}

pub(crate) fn catalog_docs(name: &str) -> Option<&'static str> {
    SPECS
        .iter()
        .find(|spec| spec.name == name)
        .map(|spec| spec.docs)
}

pub(crate) fn discover() -> Vec<Detected> {
    let paths = search_paths();
    SPECS
        .iter()
        .map(|spec| Detected {
            name: spec.name,
            cli: find_in(spec.cli, &paths),
            docs: spec.docs,
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
