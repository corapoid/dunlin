//! Read-only local discovery. Never runs a CLI, downloads an adapter or signs in.
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub(crate) struct Detected {
    pub name: &'static str,
    pub cli: Option<PathBuf>,
    pub adapter: Option<PathBuf>,
}

impl Detected {
    pub(crate) fn visible_in_onboarding(&self, show_all: bool) -> bool {
        show_all || self.adapter.is_some() || self.cli.is_some()
    }

    pub(crate) fn status(&self) -> &'static str {
        if self.adapter.is_some() {
            "Adapter detected"
        } else if self.cli.is_some() {
            "CLI detected · adapter needed"
        } else {
            "Not installed"
        }
    }
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
        paths.extend(home_bins(Path::new(&home)));
        // Finder-launched apps do not inherit the shell's NVM initialization.
        let nvm = std::env::var_os("NVM_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| PathBuf::from(&home).join(".nvm"));
        paths.extend(nvm_bins(&nvm));
    }
    let mut unique = Vec::new();
    for path in paths {
        if !unique.contains(&path) {
            unique.push(path);
        }
    }
    unique
}

fn home_bins(home: &Path) -> Vec<PathBuf> {
    [
        ".local/bin",
        ".cargo/bin",
        ".bun/bin",
        ".npm-global/bin",
        ".volta/bin",
        ".opencode/bin",
        "Library/pnpm",
        "node_modules/.bin",
    ]
    .map(|dir| home.join(dir))
    .into()
}

fn nvm_bins(root: &Path) -> Vec<PathBuf> {
    let mut bins: Vec<_> = std::fs::read_dir(root.join("versions/node"))
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("bin")))
        .filter(|path| path.is_dir())
        .collect();
    bins.sort();
    bins.reverse();
    bins
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
        docs: "https://github.com/agentclientprotocol/claude-agent-acp",
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

pub(crate) fn install_command(name: &str) -> Option<&'static str> {
    (name == "Claude Code")
        .then_some("npm install -g @agentclientprotocol/claude-agent-acp@preview")
}

impl crate::app::DunlinApp {
    pub(crate) fn show_harness_setup(&mut self, name: &str, cx: &mut dunlin_ui::Context<Self>) {
        self.harness_setup = Some(name.into());
        self.settings_tab = crate::app::SettingsTab::Harnesses;
        self.settings_open = true;
        cx.notify();
    }

    pub(crate) fn rescan_harnesses(&mut self, cx: &mut dunlin_ui::Context<Self>) {
        if self.harness_scanning {
            return;
        }
        self.harness_scanning = true;
        self.harness_setup_notice = Some("Scanning installed ACP adapters…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let detected = cx.background_executor().spawn(async { discover() }).await;
            let _ = this.update(cx, |this, cx| {
                let count = detected
                    .iter()
                    .filter(|entry| entry.adapter.is_some())
                    .count();
                this.harness_discovery = detected;
                this.harness_scanning = false;
                let message = format!("Scan complete · {count} ACP adapters detected");
                this.harness_setup_notice = Some(message.clone());
                this.set_notice(message, cx);
            });
        })
        .detach();
    }

    pub(crate) fn open_harness_docs(&mut self, name: &str, cx: &mut dunlin_ui::Context<Self>) {
        let Some(url) = catalog_docs(name) else {
            return;
        };
        self.harness_setup_notice = Some("Opening the setup guide…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut command = std::process::Command::new(if cfg!(target_os = "macos") {
                        "/usr/bin/open"
                    } else {
                        "xdg-open"
                    });
                    crate::providers::command_output(
                        command.arg(url),
                        std::time::Duration::from_secs(5),
                    )
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.harness_setup_notice =
                    Some(if result.is_ok_and(|output| output.status.success()) {
                        "Setup guide opened in your browser; nothing was installed.".into()
                    } else {
                        format!("Could not open the browser. Open this URL manually: {url}")
                    });
                cx.notify();
            });
        })
        .detach();
    }
}

pub(crate) fn discover() -> Vec<Detected> {
    discover_in(&search_paths())
}

fn discover_in(paths: &[PathBuf]) -> Vec<Detected> {
    SPECS
        .iter()
        .map(|spec| Detected {
            name: spec.name,
            cli: find_in(spec.cli, paths),
            adapter: split_command(spec.command).and_then(|(name, _)| find_in(name, paths)),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(unix)]
    fn finds_home_local_npm_install_without_scanning_project_dependencies() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join("node_modules/.bin");
        std::fs::create_dir_all(&bin).unwrap();
        let target = home.path().join("node_modules/adapter.js");
        std::fs::write(
            &target,
            b"#!/usr/bin/env node\nthrow new Error('must not execute');\n",
        )
        .unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adapter = bin.join("claude-agent-acp");
        std::os::unix::fs::symlink(&target, &adapter).unwrap();
        let detected = discover_in(&home_bins(home.path()));
        assert_eq!(
            detected
                .iter()
                .find(|entry| entry.name == "Claude Code")
                .unwrap()
                .adapter
                .as_ref(),
            Some(&adapter)
        );
        assert!(
            !home_bins(home.path())
                .iter()
                .any(|path| path.starts_with(home.path().join("project")))
        );
        assert_eq!(
            install_command("Claude Code"),
            Some("npm install -g @agentclientprotocol/claude-agent-acp@preview")
        );
        assert!(install_command("Unknown").is_none());
    }

    #[test]
    #[cfg(unix)]
    fn rescan_detects_new_nvm_adapters_without_executing_them() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("versions/node/v24.18.0/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let paths = nvm_bins(dir.path());
        assert_eq!(paths, vec![bin.clone()]);
        assert!(
            discover_in(&paths)
                .iter()
                .all(|entry| entry.adapter.is_none())
        );
        let adapter = bin.join("claude-agent-acp");
        std::fs::write(&adapter, b"#!/bin/sh\nexit 99\n").unwrap();
        assert!(find_in("claude-agent-acp", &paths).is_none());
        std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
        let entries = discover_in(&nvm_bins(dir.path()));
        let claude = entries
            .iter()
            .find(|entry| entry.name == "Claude Code")
            .unwrap();
        assert_eq!(claude.adapter.as_ref(), Some(&adapter));
        assert!(claude.cli.is_none());
        assert!(nvm_bins(&dir.path().join("missing")).is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn onboarding_keeps_cli_only_agents_visible_without_advertising_them_as_ready() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for name in ["opencode", "claude", "codex"] {
            let path = dir.path().join(name);
            std::fs::write(&path, b"#!/bin/sh\n").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let entries = discover_in(&[dir.path().to_path_buf()]);
        let visible = entries
            .iter()
            .filter(|entry| entry.visible_in_onboarding(false))
            .map(|entry| (entry.name, entry.status(), entry.adapter.is_some()))
            .collect::<Vec<_>>();
        assert_eq!(
            visible,
            vec![
                ("OpenCode", "Adapter detected", true),
                ("Claude Code", "CLI detected · adapter needed", false),
                ("OpenAI Codex", "CLI detected · adapter needed", false),
            ]
        );
        assert!(
            entries
                .iter()
                .all(|entry| entry.visible_in_onboarding(true))
        );
        let hermes = entries
            .iter()
            .find(|entry| entry.name == "Hermes Agent")
            .unwrap();
        assert_eq!(hermes.status(), "Not installed");
        assert!(!hermes.visible_in_onboarding(false));

        let adapter = dir.path().join("claude-agent-acp");
        std::fs::write(&adapter, b"#!/bin/sh\n").unwrap();
        std::fs::set_permissions(adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::remove_file(dir.path().join("claude")).unwrap();
        let entries = discover_in(&[dir.path().to_path_buf()]);
        let claude = entries
            .iter()
            .find(|entry| entry.name == "Claude Code")
            .unwrap();
        assert!(claude.adapter.is_some());
        assert!(claude.cli.is_none());
        assert!(claude.visible_in_onboarding(false));
        assert_eq!(claude.status(), "Adapter detected");
    }

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
