//! Static run-command discovery.
//!
//! This module reads a bounded set of manifests and script names. It never
//! evaluates manifest code, never expands shell variables and never executes a
//! discovered command. The UI shows the exact argv and asks before starting it.
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path, PathBuf},
};

mod dependencies;
pub use dependencies::Dependency;

pub const MAX_CANDIDATES: usize = 80;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_DIRS: usize = 40;
const MAX_SCRIPTS: usize = 24;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RunCandidate {
    pub id: String,
    pub label: String,
    pub program: String,
    pub args: Vec<String>,
    /// Project-relative working directory; `"."` is the project root.
    pub cwd: String,
    /// Project-relative manifest or script that produced this candidate.
    pub source: String,
    pub kind: String,
}

impl RunCandidate {
    pub fn command_line(&self) -> String {
        let mut parts = Vec::with_capacity(self.args.len() + 1);
        parts.push(shell_quote(&self.program));
        for arg in &self.args {
            parts.push(shell_quote(arg));
        }
        parts.join(" ")
    }
}

fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b':'))
    {
        return value.into();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Detection {
    pub candidates: Vec<RunCandidate>,
    pub warnings: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<Dependency>,
}

impl Detection {
    fn warn(&mut self, message: impl Into<String>) {
        let message = message.into();
        if self.warnings.len() < 24 && !self.warnings.contains(&message) {
            self.warnings.push(message);
        }
    }
}

fn relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.contains('\0')
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn source_for(root: &Path, path: &Path) -> Option<String> {
    let value = path.strip_prefix(root).ok()?.to_string_lossy().to_string();
    if value.is_empty() {
        Some(".".into())
    } else {
        relative(&value).then_some(value)
    }
}

fn id(source: &str, cwd: &str, program: &str, args: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(source.as_bytes());
    hash.update([0]);
    hash.update(cwd.as_bytes());
    hash.update([0]);
    hash.update(program.as_bytes());
    for arg in args {
        hash.update([0]);
        hash.update(arg.as_bytes());
    }
    format!("{:x}", hash.finalize())[..16].to_string()
}

fn candidate(
    root: &Path,
    source: &Path,
    cwd: &Path,
    kind: &str,
    label: impl Into<String>,
    program: impl Into<String>,
    args: Vec<String>,
) -> Option<RunCandidate> {
    let source = source_for(root, source)?;
    let cwd = source_for(root, cwd)?;
    let program = program.into();
    let label = label.into();
    if program.is_empty()
        || program.len() > 4096
        || program.contains('\0')
        || label.is_empty()
        || label.len() > 160
        || args.len() > 64
        || args
            .iter()
            .any(|arg| arg.len() > 4096 || arg.contains('\0'))
        || args.iter().map(String::len).sum::<usize>() > 16 * 1024
        || !relative(&cwd)
        || !relative(&source)
    {
        return None;
    }
    Some(RunCandidate {
        id: id(&source, &cwd, &program, &args),
        label,
        program,
        args,
        cwd,
        source,
        kind: kind.into(),
    })
}

fn read_limited(path: &Path) -> Result<String, String> {
    let metadata = fs::metadata(path).map_err(|_| "manifest is unavailable".to_string())?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err("manifest exceeds 1 MiB".into());
    }
    fs::read_to_string(path).map_err(|_| "manifest is not readable UTF-8".into())
}

fn skipped_dir(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "target"
            | "dist"
            | "build"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".next"
            | ".turbo"
            | ".gradle"
            | ".idea"
            | ".dart_tool"
    ) || name.starts_with('.')
}

fn manifest_dirs(root: &Path) -> Vec<PathBuf> {
    let mut out = vec![root.to_path_buf()];
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth >= 2 || out.len() >= MAX_DIRS {
            continue;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut children = entries
            .flatten()
            .filter_map(|entry| {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path).ok()?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return None;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                (!skipped_dir(&name)).then_some(path)
            })
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            if out.len() >= MAX_DIRS {
                break;
            }
            out.push(child.clone());
            stack.push((child, depth + 1));
        }
    }
    out
}

fn package_manager(dir: &Path) -> &'static str {
    if let Some(manager) = read_limited(&dir.join("package.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|value| {
            value
                .get("packageManager")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
    {
        match manager.split('@').next().unwrap_or("") {
            "npm" => return "npm",
            "pnpm" => return "pnpm",
            "yarn" => return "yarn",
            "bun" => return "bun",
            _ => {}
        }
    }
    if dir.join("pnpm-lock.yaml").is_file() {
        "pnpm"
    } else if dir.join("yarn.lock").is_file() {
        "yarn"
    } else if dir.join("bun.lock").is_file() || dir.join("bun.lockb").is_file() {
        "bun"
    } else {
        "npm"
    }
}

fn package_json(root: &Path, dir: &Path, detection: &mut Detection, seen: &mut BTreeSet<String>) {
    let path = dir.join("package.json");
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| "package.json".into())
            ));
            return;
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            detection.warn(format!(
                "{} is not valid JSON",
                source_for(root, &path).unwrap_or_else(|| "package.json".into())
            ));
            return;
        }
    };
    let manager = package_manager(&dependencies::install_dir(root, dir));
    dependencies::detect_node(root, dir, &value, detection);
    if let Some(scripts) = value.get("scripts").and_then(serde_json::Value::as_object) {
        for name in scripts.keys().take(64) {
            if name.is_empty()
                || name.len() > 128
                || name.starts_with('-')
                || name.chars().any(char::is_control)
            {
                continue;
            }
            let args = vec!["run".into(), name.clone()];
            let label = format!("{manager} run {name}");
            if let Some(candidate) = candidate(root, &path, dir, "node", label, manager, args)
                && seen.insert(format!(
                    "{}:{}:{}:{}",
                    candidate.cwd,
                    candidate.program,
                    candidate.args.join("\0"),
                    candidate.source
                ))
            {
                detection.candidates.push(candidate);
            }
        }
    }
}

fn cargo_toml(root: &Path, dir: &Path, detection: &mut Detection, seen: &mut BTreeSet<String>) {
    let path = dir.join("Cargo.toml");
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| "Cargo.toml".into())
            ));
            return;
        }
    };
    let value: toml::Value = match toml::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            detection.warn(format!(
                "{} is not valid TOML",
                source_for(root, &path).unwrap_or_else(|| "Cargo.toml".into())
            ));
            return;
        }
    };
    let has_package = value.get("package").is_some();
    let has_workspace = value.get("workspace").is_some();
    let mut commands: Vec<(&str, Vec<String>)> = Vec::new();
    if has_package {
        commands.extend([
            ("cargo run", vec!["run".into()]),
            ("cargo test", vec!["test".into()]),
            ("cargo check", vec!["check".into()]),
            ("cargo build", vec!["build".into()]),
            (
                "cargo clippy",
                vec!["clippy".into(), "--all-targets".into()],
            ),
        ]);
        if let Some(bins) = value.get("bin").and_then(toml::Value::as_array) {
            for bin in bins.iter().take(16) {
                if let Some(name) = bin.get("name").and_then(toml::Value::as_str)
                    && !name.is_empty()
                    && name.len() <= 128
                    && !name.chars().any(char::is_control)
                {
                    commands.push((
                        "cargo run --bin",
                        vec!["run".into(), "--bin".into(), name.into()],
                    ));
                }
            }
        }
        if let Some(examples) = value.get("example").and_then(toml::Value::as_array) {
            for example in examples.iter().take(16) {
                if let Some(name) = example.get("name").and_then(toml::Value::as_str)
                    && !name.is_empty()
                    && name.len() <= 128
                    && !name.chars().any(char::is_control)
                {
                    commands.push((
                        "cargo run --example",
                        vec!["run".into(), "--example".into(), name.into()],
                    ));
                }
            }
        }
    }
    if has_workspace && !has_package {
        commands.extend([
            (
                "cargo test --workspace",
                vec!["test".into(), "--workspace".into()],
            ),
            (
                "cargo check --workspace",
                vec!["check".into(), "--workspace".into()],
            ),
            (
                "cargo build --workspace",
                vec!["build".into(), "--workspace".into()],
            ),
        ]);
    }
    for (_, args) in commands {
        let label = std::iter::once("cargo")
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(candidate) = candidate(root, &path, dir, "rust", label, "cargo", args)
            && seen.insert(format!(
                "{}:{}:{}:{}",
                candidate.cwd,
                candidate.program,
                candidate.args.join("\0"),
                candidate.source
            ))
        {
            detection.candidates.push(candidate);
        }
    }
}

fn makefile(
    root: &Path,
    dir: &Path,
    name: &str,
    detection: &mut Detection,
    seen: &mut BTreeSet<String>,
) {
    let path = dir.join(name);
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| name.into())
            ));
            return;
        }
    };
    let mut target_count = 0;
    for line in text.lines().take(20_000) {
        let line = line.split('#').next().unwrap_or("");
        if line.starts_with('\t') || line.trim().is_empty() {
            continue;
        }
        let Some((target, _)) = line.split_once(':') else {
            continue;
        };
        let target = target.trim();
        if target.is_empty()
            || target.len() > 80
            || target.starts_with('.')
            || target.contains('%')
            || target.contains('=')
            || target.chars().any(char::is_control)
        {
            continue;
        }
        let args = vec![target.to_string()];
        if let Some(candidate) = candidate(
            root,
            &path,
            dir,
            "make",
            format!("make {target}"),
            "make",
            args,
        ) && seen.insert(format!(
            "{}:{}:{}:{}",
            candidate.cwd,
            candidate.program,
            candidate.args.join("\0"),
            candidate.source
        )) {
            detection.candidates.push(candidate);
            target_count += 1;
            if target_count >= 32 {
                break;
            }
        }
    }
    if target_count > 0
        && let Some(candidate) = candidate(root, &path, dir, "make", "make", "make", vec![])
        && seen.insert(format!(
            "{}:{}:{}:{}",
            candidate.cwd,
            candidate.program,
            candidate.args.join("\0"),
            candidate.source
        ))
    {
        detection.candidates.push(candidate);
    }
}

fn justfile(
    root: &Path,
    dir: &Path,
    name: &str,
    detection: &mut Detection,
    seen: &mut BTreeSet<String>,
) {
    let path = dir.join(name);
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| name.into())
            ));
            return;
        }
    };
    let mut count = 0;
    for line in text.lines().take(20_000) {
        if line.starts_with(' ') || line.starts_with('\t') || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((head, rest)) = line.split_once(':') else {
            continue;
        };
        if rest.starts_with('=') {
            continue;
        }
        let head = head.trim();
        let Some(name) = head.split_whitespace().next() else {
            continue;
        };
        if name.is_empty()
            || name.len() > 80
            || matches!(name, "set" | "alias" | "export" | "import" | "mod")
            || name.chars().any(char::is_control)
        {
            continue;
        }
        if let Some(candidate) = candidate(
            root,
            &path,
            dir,
            "just",
            format!("just {name}"),
            "just",
            vec![name.to_string()],
        ) && seen.insert(format!(
            "{}:{}:{}:{}",
            candidate.cwd,
            candidate.program,
            candidate.args.join("\0"),
            candidate.source
        )) {
            detection.candidates.push(candidate);
            count += 1;
            if count >= 32 {
                break;
            }
        }
    }
}

fn pyproject(root: &Path, dir: &Path, detection: &mut Detection, seen: &mut BTreeSet<String>) {
    let path = dir.join("pyproject.toml");
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| "pyproject.toml".into())
            ));
            return;
        }
    };
    let value: toml::Value = match toml::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            detection.warn(format!(
                "{} is not valid TOML",
                source_for(root, &path).unwrap_or_else(|| "pyproject.toml".into())
            ));
            return;
        }
    };
    let tool = value.get("tool");
    let mut commands = Vec::new();
    if tool.and_then(|tool| tool.get("pytest")).is_some() {
        commands.push(("pytest", vec!["-m".into(), "pytest".into()]));
    }
    if tool.and_then(|tool| tool.get("ruff")).is_some() {
        commands.push((
            "ruff check",
            vec!["-m".into(), "ruff".into(), "check".into(), ".".into()],
        ));
    }
    if tool.and_then(|tool| tool.get("mypy")).is_some() {
        commands.push(("mypy", vec!["-m".into(), "mypy".into(), ".".into()]));
    }
    for (label, args) in commands {
        if let Some(candidate) = candidate(root, &path, dir, "python", label, "python3", args)
            && seen.insert(format!(
                "{}:{}:{}:{}",
                candidate.cwd,
                candidate.program,
                candidate.args.join("\0"),
                candidate.source
            ))
        {
            detection.candidates.push(candidate);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn simple_manifest(
    root: &Path,
    dir: &Path,
    file: &str,
    kind: &str,
    program: &str,
    args: &[&str],
    label: &str,
    detection: &mut Detection,
    seen: &mut BTreeSet<String>,
) {
    let path = dir.join(file);
    if !path.is_file() {
        return;
    }
    if let Some(candidate) = candidate(
        root,
        &path,
        dir,
        kind,
        label,
        program,
        args.iter().map(|arg| arg.to_string()).collect(),
    ) && seen.insert(format!(
        "{}:{}:{}:{}",
        candidate.cwd,
        candidate.program,
        candidate.args.join("\0"),
        candidate.source
    )) {
        detection.candidates.push(candidate);
    }
}

#[allow(clippy::too_many_arguments)]
fn json_scripts(
    root: &Path,
    dir: &Path,
    file: &str,
    kind: &str,
    program: &str,
    prefix: &[&str],
    detection: &mut Detection,
    seen: &mut BTreeSet<String>,
) {
    let path = dir.join(file);
    if !path.is_file() {
        return;
    }
    let text = match read_limited(&path) {
        Ok(text) => text,
        Err(error) => {
            detection.warn(format!(
                "{}: {error}",
                source_for(root, &path).unwrap_or_else(|| file.into())
            ));
            return;
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            detection.warn(format!(
                "{} is not valid JSON",
                source_for(root, &path).unwrap_or_else(|| file.into())
            ));
            return;
        }
    };
    let scripts = value
        .get("tasks")
        .and_then(serde_json::Value::as_object)
        .or_else(|| value.get("scripts").and_then(serde_json::Value::as_object));
    let Some(scripts) = scripts else {
        return;
    };
    for name in scripts.keys().take(48) {
        if name.is_empty()
            || name.len() > 128
            || name.starts_with('-')
            || name.chars().any(char::is_control)
        {
            continue;
        }
        let mut args = prefix.iter().map(|arg| arg.to_string()).collect::<Vec<_>>();
        args.push(name.clone());
        if let Some(candidate) = candidate(
            root,
            &path,
            dir,
            kind,
            format!("{kind} {name}"),
            program,
            args,
        ) && seen.insert(format!(
            "{}:{}:{}:{}",
            candidate.cwd,
            candidate.program,
            candidate.args.join("\0"),
            candidate.source
        )) {
            detection.candidates.push(candidate);
        }
    }
}

fn scripts_dir(root: &Path, detection: &mut Detection, seen: &mut BTreeSet<String>) {
    for directory in ["scripts", "bin"] {
        let base = root.join(directory);
        if !base.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        let mut stack = vec![(base, 0usize)];
        while let Some((dir, depth)) = stack.pop() {
            if depth > 2 || files.len() >= MAX_SCRIPTS {
                continue;
            }
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                if files.len() >= MAX_SCRIPTS {
                    break;
                }
                let path = entry.path();
                let Ok(metadata) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if metadata.file_type().is_symlink() {
                    continue;
                }
                if metadata.is_dir() {
                    stack.push((path, depth + 1));
                } else if metadata.is_file() {
                    files.push(path);
                }
            }
        }
        files.sort();
        for path in files {
            let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };
            let (program, kind) = match extension {
                "sh" | "bash" | "zsh" => ("bash", "script"),
                "py" => ("python3", "script"),
                "js" | "mjs" | "cjs" => ("node", "script"),
                "rb" => ("ruby", "script"),
                "pl" => ("perl", "script"),
                _ => continue,
            };
            let Some(source) = source_for(root, &path) else {
                continue;
            };
            let label = format!("Run {source}");
            let args = vec![source.clone()];
            if let Some(candidate) = candidate(root, &path, root, kind, label, program, args)
                && seen.insert(format!(
                    "{}:{}:{}:{}",
                    candidate.cwd,
                    candidate.program,
                    candidate.args.join("\0"),
                    candidate.source
                ))
            {
                detection.candidates.push(candidate);
            }
        }
    }
}

/// Detect static run candidates without executing or evaluating anything.
pub fn detect(root: &str) -> Detection {
    let root = match fs::canonicalize(root) {
        Ok(root) if root.is_dir() => root,
        _ => {
            return Detection {
                warnings: vec!["Project folder is not available".into()],
                ..Default::default()
            };
        }
    };
    let mut detection = Detection::default();
    let mut seen = BTreeSet::new();
    for dir in manifest_dirs(&root) {
        package_json(&root, &dir, &mut detection, &mut seen);
        cargo_toml(&root, &dir, &mut detection, &mut seen);
        makefile(&root, &dir, "Makefile", &mut detection, &mut seen);
        makefile(&root, &dir, "makefile", &mut detection, &mut seen);
        justfile(&root, &dir, "justfile", &mut detection, &mut seen);
        justfile(&root, &dir, "Justfile", &mut detection, &mut seen);
        pyproject(&root, &dir, &mut detection, &mut seen);
        simple_manifest(
            &root,
            &dir,
            "go.mod",
            "go",
            "go",
            &["test", "./..."],
            "go test ./...",
            &mut detection,
            &mut seen,
        );
        simple_manifest(
            &root,
            &dir,
            "go.mod",
            "go",
            "go",
            &["build", "./..."],
            "go build ./...",
            &mut detection,
            &mut seen,
        );
        simple_manifest(
            &root,
            &dir,
            "go.mod",
            "go",
            "go",
            &["run", "."],
            "go run .",
            &mut detection,
            &mut seen,
        );
        json_scripts(
            &root,
            &dir,
            "deno.json",
            "deno",
            "deno",
            &["task"],
            &mut detection,
            &mut seen,
        );
        json_scripts(
            &root,
            &dir,
            "deno.jsonc",
            "deno",
            "deno",
            &["task"],
            &mut detection,
            &mut seen,
        );
        json_scripts(
            &root,
            &dir,
            "composer.json",
            "composer",
            "composer",
            &["run"],
            &mut detection,
            &mut seen,
        );
        simple_manifest(
            &root,
            &dir,
            "Rakefile",
            "ruby",
            "rake",
            &[],
            "rake",
            &mut detection,
            &mut seen,
        );
        simple_manifest(
            &root,
            &dir,
            "mix.exs",
            "elixir",
            "mix",
            &["test"],
            "mix test",
            &mut detection,
            &mut seen,
        );
        let gradlew = dir.join("gradlew");
        if gradlew.is_file() {
            for (label, args) in [
                ("gradlew test", vec!["test"]),
                ("gradlew build", vec!["build"]),
            ] {
                if let Some(candidate) = candidate(
                    &root,
                    &gradlew,
                    &dir,
                    "gradle",
                    label,
                    "./gradlew",
                    args.iter().map(|arg| arg.to_string()).collect(),
                ) && seen.insert(format!(
                    "{}:{}:{}:{}",
                    candidate.cwd,
                    candidate.program,
                    candidate.args.join("\0"),
                    candidate.source
                )) {
                    detection.candidates.push(candidate);
                }
            }
        }
        let mvnw = dir.join("mvnw");
        if mvnw.is_file() {
            for (label, args) in [
                ("mvnw test", vec!["test"]),
                ("mvnw package", vec!["package"]),
            ] {
                if let Some(candidate) = candidate(
                    &root,
                    &mvnw,
                    &dir,
                    "maven",
                    label,
                    "./mvnw",
                    args.iter().map(|arg| arg.to_string()).collect(),
                ) && seen.insert(format!(
                    "{}:{}:{}:{}",
                    candidate.cwd,
                    candidate.program,
                    candidate.args.join("\0"),
                    candidate.source
                )) {
                    detection.candidates.push(candidate);
                }
            }
        }
    }
    scripts_dir(&root, &mut detection, &mut seen);
    detection.candidates.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.source.cmp(&b.source))
            .then_with(|| a.label.cmp(&b.label))
            .then_with(|| a.args.cmp(&b.args))
    });
    detection.candidates.truncate(MAX_CANDIDATES);
    detection
}

/// Stable grouping label for the Run panel.
pub fn kind_label(kind: &str) -> &str {
    match kind {
        "node" => "Node",
        "rust" => "Rust",
        "make" => "Make",
        "just" => "Just",
        "python" => "Python",
        "go" => "Go",
        "deno" => "Deno",
        "composer" => "Composer",
        "ruby" => "Ruby",
        "elixir" => "Elixir",
        "gradle" => "Gradle",
        "maven" => "Maven",
        "script" => "Scripts",
        other => other,
    }
}

/// Merge manifest candidates for the UI while keeping a stable order.
pub fn grouped(candidates: &[RunCandidate]) -> BTreeMap<&str, Vec<&RunCandidate>> {
    let mut grouped = BTreeMap::new();
    for candidate in candidates {
        grouped
            .entry(kind_label(&candidate.kind))
            .or_insert_with(Vec::new)
            .push(candidate);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "dunlin-run-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn detects_node_cargo_make_just_python_and_scripts_without_executing() {
        let root = fixture("manifests");
        fs::write(
            root.join("package.json"),
            r#"{"scripts":{"dev":"vite","test":"vitest run","-bad":"x"}}"#,
        )
        .unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        fs::write(
            root.join("Cargo.toml"),
            r#"[package]
name = "demo"
version = "0.1.0"
[[bin]]
name = "server"
path = "src/main.rs"
"#,
        )
        .unwrap();
        fs::write(
            root.join("Makefile"),
            "build:\n\techo build\ntest:\n\techo test\n",
        )
        .unwrap();
        fs::write(
            root.join("justfile"),
            "fmt:\n  cargo fmt\ncheck:\n  cargo check\n",
        )
        .unwrap();
        fs::write(
            root.join("pyproject.toml"),
            "[tool.pytest.ini_options]\naddopts = '-q'\n[tool.ruff]\nline-length = 100\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("scripts")).unwrap();
        fs::write(root.join("scripts/deploy.sh"), "#!/bin/sh\necho hi\n").unwrap();
        fs::write(root.join("scripts/seed.py"), "print('seed')\n").unwrap();
        let detection = detect(root.to_str().unwrap());
        let labels = detection
            .candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"pnpm run dev"), "{labels:?}");
        assert!(labels.contains(&"cargo run --bin server"), "{labels:?}");
        assert!(labels.contains(&"make test"));
        assert!(labels.contains(&"just check"));
        assert!(labels.contains(&"pytest"));
        assert!(labels.contains(&"ruff check"));
        assert!(
            labels
                .iter()
                .any(|label| label.contains("scripts/deploy.sh"))
        );
        assert!(labels.iter().any(|label| label.contains("scripts/seed.py")));
        assert!(!labels.iter().any(|label| label.contains("-bad")));
        assert!(detection.candidates.len() <= MAX_CANDIDATES);
        assert!(detection.candidates.iter().all(|candidate| {
            !candidate.program.contains('\0')
                && candidate.args.iter().all(|arg| !arg.contains('\0'))
        }));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn package_manager_and_workspace_cargo_choices_are_static() {
        let root = fixture("workspaces");
        fs::write(root.join("yarn.lock"), "").unwrap();
        fs::write(root.join("package.json"), r#"{"scripts":{"build":"tsc"}}"#).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"one\"]\n",
        )
        .unwrap();
        let detection = detect(root.to_str().unwrap());
        let labels = detection
            .candidates
            .iter()
            .map(|candidate| candidate.label.as_str())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"yarn run build"), "{labels:?}");
        assert!(labels.contains(&"cargo test --workspace"), "{labels:?}");
        assert!(detection.warnings.is_empty());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_manifests_are_warnings_not_commands() {
        let root = fixture("bad");
        fs::write(root.join("package.json"), "{not json").unwrap();
        fs::write(root.join("Cargo.toml"), "not = [toml").unwrap();
        let detection = detect(root.to_str().unwrap());
        assert!(detection.candidates.is_empty());
        assert_eq!(detection.warnings.len(), 2);
        assert!(
            detection
                .warnings
                .iter()
                .all(|warning| warning.contains("not valid"))
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_manifests_are_bounded_and_skipped_in_build_folders() {
        let root = fixture("nested");
        fs::create_dir_all(root.join("web")).unwrap();
        fs::create_dir_all(root.join("node_modules/ignored")).unwrap();
        fs::write(
            root.join("web/package.json"),
            r#"{"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        fs::write(
            root.join("node_modules/ignored/package.json"),
            r#"{"scripts":{"hidden":"x"}}"#,
        )
        .unwrap();
        let detection = detect(root.to_str().unwrap());
        assert!(
            detection
                .candidates
                .iter()
                .any(|candidate| { candidate.label == "npm run dev" && candidate.cwd == "web" })
        );
        assert!(
            !detection
                .candidates
                .iter()
                .any(|candidate| { candidate.label == "npm run hidden" })
        );
        fs::remove_dir_all(root).unwrap();
    }
}
