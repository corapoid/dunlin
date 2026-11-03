//! A local instruction library with explicit, ownership-checked global installs.
use crate::app::DunlinApp;
use dunlin_ui::{
    component::input::{Editor, EditorState, Input, InputState},
    *,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

pub(crate) const COMMIT_SKILL: &str = "conventional-commits";
const MAX_SKILL_BYTES: usize = 64 * 1024;
pub(crate) const COMMIT_INSTRUCTIONS: &str =
    include_str!("../../../assets/skills/conventional-commits.md");

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Scope {
    #[default]
    Global,
    Project,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Target {
    OpenCode,
    Claude,
    Codex,
    Other,
}
impl Target {
    pub const ALL: [Self; 3] = [Self::OpenCode, Self::Claude, Self::Codex];
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenCode => "OpenCode",
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
            Self::Other => "Other",
        }
    }
    pub(crate) fn project_directory(self, root: &Path) -> Result<PathBuf, String> {
        let relative = match self {
            Self::OpenCode => ".opencode/skills",
            Self::Claude => ".claude/skills",
            Self::Codex => ".agents/skills",
            Self::Other => return Err("Choose an explicit project skills directory".into()),
        };
        let path = root.join(relative);
        if !path.is_absolute() {
            return Err("Project skills directories must be absolute".into());
        }
        Ok(path)
    }
    fn directory(self) -> Result<PathBuf, String> {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or("Home directory is unavailable")?;
        let path = match self {
            Self::OpenCode => std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".config"))
                .join("opencode/skills"),
            Self::Claude => std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"))
                .join("skills"),
            Self::Codex => home.join(".agents/skills"),
            Self::Other => return Err("Choose this harness's global skills directory".into()),
        };
        if !path.is_absolute() {
            return Err("Global skill directories must be absolute".into());
        }
        Ok(path)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Installation {
    pub target: Target,
    pub path: String,
    pub contents: String,
    #[serde(default)]
    pub scope: Scope,
    #[serde(default)]
    pub project_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Skill {
    pub id: String,
    pub title: String,
    pub description: String,
    pub body: String,
    #[serde(default)]
    pub origin: String,
    #[serde(default)]
    pub source_hash: String,
    #[serde(default)]
    pub installations: Vec<Installation>,
}
impl Skill {
    pub fn rendered(&self) -> Result<String, String> {
        if self.id.is_empty()
            || self.id.len() > 48
            || !self
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        {
            return Err("Invalid skill ID".into());
        }
        if self.title.trim().is_empty()
            || self.description.trim().is_empty()
            || self.body.trim().is_empty()
        {
            return Err("A skill needs a name, description and instructions".into());
        }
        if self.title.len() > 120 || self.description.len() > 1024 {
            return Err("Use a name up to 120 bytes and a description up to 1,024 bytes".into());
        }
        let contents = format!(
            "---\nname: dunlin-{}\ndescription: {}\n{}---\n\n{}\n",
            self.id,
            serde_json::to_string(&self.description).map_err(|e| e.to_string())?,
            if self.id == COMMIT_SKILL {
                "disable-model-invocation: true\n"
            } else {
                ""
            },
            self.body.trim()
        );
        if contents.len() > MAX_SKILL_BYTES {
            return Err("A skill must fit in 64 KiB".into());
        }
        Ok(contents)
    }
    pub(crate) fn content_hash(&self) -> String {
        self.rendered()
            .map(|text| format!("{:x}", Sha256::digest(text.as_bytes())))
            .unwrap_or_default()
    }
}
pub(crate) fn defaults() -> Vec<Skill> {
    vec![Skill { id: COMMIT_SKILL.into(), title: "Conventional commits".into(), description: "Analyze repository changes and create focused Conventional Commits only when the user explicitly requests commits.".into(), body: COMMIT_INSTRUCTIONS.into(), origin: "Built-in".into(), source_hash: String::new(), installations: Vec::new() }]
}

fn frontmatter_value(front: &str, key: &str) -> Option<String> {
    let line = front.lines().find(|line| {
        line.strip_prefix(key)
            .is_some_and(|rest| rest.starts_with(':'))
    })?;
    let value = line.split_once(':')?.1.trim();
    Some(
        serde_json::from_str::<String>(value)
            .unwrap_or_else(|_| value.trim_matches('"').to_string()),
    )
}

fn parse_skill_contents(contents: &str, fallback_id: &str) -> Result<Skill, String> {
    let normalized = contents.replace("\r\n", "\n");
    let rest = normalized
        .strip_prefix("---\n")
        .ok_or("SKILL.md needs YAML frontmatter")?;
    let (front, body) = rest
        .split_once("\n---\n")
        .ok_or("SKILL.md frontmatter is not closed")?;
    let name = frontmatter_value(front, "name").ok_or("SKILL.md needs a name")?;
    let description =
        frontmatter_value(front, "description").ok_or("SKILL.md needs a description")?;
    let raw_id = name.strip_prefix("dunlin-").unwrap_or(fallback_id);
    let mut id = raw_id
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>();
    while id.starts_with('-') || id.ends_with('-') {
        id = id.trim_matches('-').to_string();
    }
    if id.is_empty() {
        id = "imported-skill".into();
    }
    id.truncate(48);
    let title = name
        .strip_prefix("dunlin-")
        .unwrap_or(&name)
        .replace('-', " ");
    Ok(Skill {
        id,
        title: if title.trim().is_empty() {
            fallback_id.replace('-', " ")
        } else {
            title
        },
        description,
        body: body.trim().to_string(),
        origin: String::new(),
        source_hash: format!("{:x}", Sha256::digest(contents.as_bytes())),
        installations: Vec::new(),
    })
}

fn skill_from_directory(directory: &Path) -> Result<Skill, String> {
    let path = directory.join("SKILL.md");
    regular_file(&path)?;
    let contents = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let fallback = directory
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("imported-skill");
    let fallback = fallback.strip_prefix("dunlin-").unwrap_or(fallback);
    let mut skill = parse_skill_contents(&contents, fallback)?;
    skill.origin = directory.to_string_lossy().to_string();
    Ok(skill)
}

/// Read bounded SKILL.md files from one skill directory or a directory of
/// skill directories. Never follows symlinks, executes scripts or reads
/// supporting files.
pub(crate) fn discover(path: &Path) -> Result<Vec<Skill>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err("Choose a regular skill directory".into());
    }
    let path = fs::canonicalize(path).map_err(|e| e.to_string())?;
    let mut directories = Vec::new();
    if path.join("SKILL.md").is_file() {
        directories.push(path);
    } else {
        for entry in fs::read_dir(&path).map_err(|e| e.to_string())?.take(128) {
            let entry = entry.map_err(|e| e.to_string())?;
            let child = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&child) else {
                continue;
            };
            if metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && child.join("SKILL.md").is_file()
            {
                directories.push(child);
            }
        }
    }
    let mut skills = Vec::new();
    for directory in directories {
        if let Ok(skill) = skill_from_directory(&directory)
            && !skills
                .iter()
                .any(|existing: &Skill| existing.id == skill.id)
        {
            skills.push(skill);
        }
    }
    if skills.is_empty() {
        return Err("No readable SKILL.md files were found".into());
    }
    Ok(skills)
}

/// Discover skills already installed in the three default global harness
/// directories. Results are library imports, not ownership claims.
pub(crate) fn discover_global() -> Vec<Skill> {
    let mut skills = Vec::new();
    for target in Target::ALL {
        let Ok(directory) = target.directory() else {
            continue;
        };
        if !directory.is_dir() {
            continue;
        }
        let Ok(found) = discover(&directory) else {
            continue;
        };
        for mut skill in found {
            skill.origin = format!("{} · {}", target.label(), skill.origin);
            if !skills
                .iter()
                .any(|existing: &Skill| existing.id == skill.id)
            {
                skills.push(skill);
            }
        }
    }
    skills
}

fn regular_file(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Refusing a symlink or non-regular skill file".into());
    }
    if metadata.len() > MAX_SKILL_BYTES as u64 {
        return Err("Installed skill is unexpectedly large".into());
    }
    Ok(())
}
pub(crate) fn install_at(
    base: &Path,
    skill: &Skill,
    target: Target,
) -> Result<Installation, String> {
    let contents = skill.rendered()?;
    fs::create_dir_all(base).map_err(|e| e.to_string())?;
    let base = fs::canonicalize(base).map_err(|e| e.to_string())?;
    let directory = base.join(format!("dunlin-{}", skill.id));
    match fs::symlink_metadata(&directory) {
        Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
            return Err("Refusing an existing skill-directory symlink or file".into());
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(&directory).map_err(|e| e.to_string())?
        }
        Err(e) => return Err(e.to_string()),
    }
    let path = directory.join("SKILL.md");
    let recorded = skill
        .installations
        .iter()
        .find(|i| Path::new(&i.path) == path);
    let check = || -> Result<(), String> {
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                regular_file(&path)?;
                let old =
                    recorded.ok_or("A skill already exists here and is not owned by Dunlin")?;
                if fs::read_to_string(&path).map_err(|e| e.to_string())? != old.contents {
                    return Err("Installed skill was changed outside Dunlin; preserve or move it before updating".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
        Ok(())
    };
    check()?;
    let temporary = directory.join(format!(".dunlin-{}.tmp", uuid::Uuid::now_v7()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
        file.write_all(contents.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        check()?;
        if recorded.is_none() {
            // Hard-link publication is exclusive: even a concurrent new file
            // cannot be overwritten on the first installation.
            fs::hard_link(&temporary, &path).map_err(|e| e.to_string())?;
        } else {
            fs::rename(&temporary, &path).map_err(|e| e.to_string())?;
        }
        Ok::<_, String>(())
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    Ok(Installation {
        target,
        path: path.to_string_lossy().into(),
        contents,
        scope: Scope::Global,
        project_id: String::new(),
    })
}

pub(crate) fn install_at_scoped(
    base: &Path,
    skill: &Skill,
    target: Target,
    scope: Scope,
    project_id: &str,
) -> Result<Installation, String> {
    let mut installation = install_at(base, skill, target)?;
    installation.scope = scope;
    installation.project_id = project_id.to_string();
    Ok(installation)
}

pub(crate) fn uninstall(installation: &Installation, id: &str) -> Result<(), String> {
    let path = Path::new(&installation.path);
    if !path.is_absolute()
        || path.file_name().is_none_or(|n| n != "SKILL.md")
        || path
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|n| n != format!("dunlin-{id}").as_str())
    {
        return Err("Invalid managed skill path".into());
    }
    let parent = path.parent().ok_or("Missing skill directory")?;
    if fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Refusing a symlinked skill directory".into());
    }
    match fs::canonicalize(parent) {
        Ok(resolved) if resolved != parent => {
            return Err(
                "Managed skill directory moved through a symlink; preserve it instead of deleting"
                    .into(),
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.to_string()),
        Ok(_) => {}
    }
    regular_file(path)?;
    if fs::read_to_string(path).map_err(|e| e.to_string())? != installation.contents {
        return Err("Installed skill was modified outside Dunlin and will not be deleted".into());
    }
    fs::remove_file(path).map_err(|e| e.to_string())?;
    let _ = fs::remove_dir(parent); // Non-recursive: keep any supporting/user files.
    Ok(())
}

#[derive(Default)]
pub(crate) struct Ui {
    draft: Option<Draft>,
    busy: bool,
    message: Option<String>,
}
struct Draft {
    id: String,
    title: Entity<InputState>,
    description: Entity<InputState>,
    body: Entity<EditorState>,
}
const MAX_GITHUB_SKILL_BYTES: usize = MAX_SKILL_BYTES;

fn github_skill_urls(value: &str) -> Result<Vec<Url>, String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 {
        return Err("Paste a GitHub link to SKILL.md or a skill folder".into());
    }
    let url = Url::parse(value).map_err(|_| "Invalid GitHub URL".to_string())?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err("Use a plain HTTPS github.com or raw.githubusercontent.com link".into());
    }
    let host = url.host_str().unwrap_or_default();
    let segments = url
        .path_segments()
        .map(|segments| segments.filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    if host == "raw.githubusercontent.com" {
        let mut raw = url.clone();
        if segments.last() != Some(&"SKILL.md") {
            let path = format!("{}/SKILL.md", url.path().trim_end_matches('/'));
            raw.set_path(&path);
        }
        return Ok(vec![raw]);
    }
    if host != "github.com" {
        return Err("Only github.com or raw.githubusercontent.com links are supported".into());
    }
    if segments.len() < 2 {
        return Err("The GitHub link must include an owner and repository".into());
    }
    let (owner, repo) = (segments[0], segments[1]);
    let mut urls = Vec::new();
    match segments.get(2).copied() {
        Some("blob") if segments.len() >= 5 => {
            if segments.last() != Some(&"SKILL.md") {
                return Err("Link directly to SKILL.md or to the skill folder".into());
            }
            urls.push(raw_url(owner, repo, segments[3], &segments[4..].join("/"))?);
        }
        Some("tree") if segments.len() >= 4 => {
            let path = if segments.len() == 4 {
                "SKILL.md".to_string()
            } else {
                format!("{}/SKILL.md", segments[4..].join("/"))
            };
            urls.push(raw_url(owner, repo, segments[3], &path)?);
        }
        Some(_) | None => {
            let path = if segments.len() == 2 {
                "SKILL.md".to_string()
            } else {
                format!("{}/SKILL.md", segments[2..].join("/"))
            };
            for branch in ["main", "master"] {
                urls.push(raw_url(owner, repo, branch, &path)?);
            }
        }
    }
    Ok(urls)
}

fn raw_url(owner: &str, repo: &str, reference: &str, path: &str) -> Result<Url, String> {
    Url::parse(&format!(
        "https://raw.githubusercontent.com/{owner}/{repo}/{reference}/{path}"
    ))
    .map_err(|_| "Could not build the GitHub raw URL".to_string())
}

fn fetch_github_skill(urls: &[Url]) -> Result<Skill, String> {
    let agent = ureq::AgentBuilder::new()
        .try_proxy_from_env(false)
        .redirects(0)
        .timeout(Duration::from_secs(20))
        .build();
    let mut last_error = None;
    for url in urls {
        let response = agent
            .get(url.as_str())
            .set("User-Agent", "Dunlin")
            .set("Accept", "text/plain")
            .call();
        match response {
            Ok(response) => {
                let mut text = String::new();
                response
                    .into_reader()
                    .take(MAX_GITHUB_SKILL_BYTES as u64 + 1)
                    .read_to_string(&mut text)
                    .map_err(|error| format!("Could not read SKILL.md: {error}"))?;
                if text.len() > MAX_GITHUB_SKILL_BYTES {
                    return Err("SKILL.md must fit in 64 KiB".into());
                }
                let mut skill = parse_skill_contents(&text, "github-skill")?;
                skill.rendered()?;
                skill.origin = url.to_string();
                return Ok(skill);
            }
            Err(ureq::Error::Status(404, _)) => {
                last_error = Some(format!("No SKILL.md at {url}"));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(format!("GitHub returned HTTP {status}"));
            }
            Err(ureq::Error::Transport(error)) => {
                return Err(format!("Could not reach GitHub: {error}"));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| "No SKILL.md found at the linked GitHub location".into()))
}

impl DunlinApp {
    fn edit_skill(&mut self, id: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let skill = id
            .and_then(|id| self.skills.iter().find(|s| s.id == id))
            .cloned();
        let title = cx.new(|cx| InputState::new(window, cx).placeholder("Skill name"));
        let description =
            cx.new(|cx| InputState::new(window, cx).placeholder("When to use this skill"));
        let body = cx.new(|cx| {
            EditorState::new(window, cx)
                .language("markdown")
                .soft_wrap(true)
        });
        if let Some(skill) = &skill {
            title.update(cx, |s, cx| s.set_value(skill.title.clone(), window, cx));
            description.update(cx, |s, cx| {
                s.set_value(skill.description.clone(), window, cx)
            });
            body.update(cx, |s, cx| s.set_value(skill.body.clone(), window, cx));
        }
        self.skills_ui.draft = Some(Draft {
            id: skill
                .map(|s| s.id)
                .unwrap_or_else(|| format!("skill-{}", uuid::Uuid::now_v7().simple())),
            title,
            description,
            body,
        });
        self.skills_ui.message = None;
        cx.notify();
    }
    fn import_skills(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Reading skill folders…".into());
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut found = Vec::new();
                    let mut errors = Vec::new();
                    for path in paths {
                        match discover(&path) {
                            Ok(skills) => found.extend(skills),
                            Err(error) => errors.push(format!("{}: {error}", path.display())),
                        }
                    }
                    (found, errors)
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let (found, mut errors) = result;
                let mut added = 0;
                let mut skipped = 0;
                for skill in found {
                    if this.skills.iter().any(|existing| existing.id == skill.id) {
                        skipped += 1;
                    } else if this.skills.len() < 128 {
                        this.skills.push(skill);
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                }
                if added > 0 {
                    this.persist_settings(cx);
                }
                if !errors.is_empty() {
                    errors.truncate(3);
                    this.skills_ui.message = Some(format!(
                        "Imported {added}, skipped {skipped}. {}",
                        errors.join(" · ")
                    ));
                } else {
                    this.skills_ui.message = Some(format!(
                        "Imported {added} skill(s), skipped {skipped} existing or over-limit entries. Imported files are not claimed as Dunlin-managed until installed."
                    ));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn import_skill_from_github(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let value = self.skills_import_url.read(cx).value().to_string();
        let urls = match github_skill_urls(&value) {
            Ok(urls) => urls,
            Err(error) => {
                self.skills_ui.message = Some(error);
                cx.notify();
                return;
            }
        };
        self.skills_import_url
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Fetching skill from GitHub…".into());
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { fetch_github_skill(&urls) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                match result {
                    Ok(skill) => {
                        if this.skills.iter().any(|existing| existing.id == skill.id) {
                            this.skills_ui.message = Some(format!(
                                "Skill “{}” already exists in the library",
                                skill.title
                            ));
                        } else if this.skills.len() >= 128 {
                            this.skills_ui.message =
                                Some("The skill library is limited to 128 entries".into());
                        } else {
                            let title = skill.title.clone();
                            this.skills.push(skill);
                            this.persist_settings(cx);
                            this.skills_ui.message =
                                Some(format!("Imported “{title}” from GitHub"));
                        }
                    }
                    Err(error) => this.skills_ui.message = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn scan_global_skills(&mut self, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        self.skills_ui.busy = true;
        self.skills_ui.message = Some("Scanning global harness skill folders…".into());
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { discover_global() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let mut added = 0;
                let mut skipped = 0;
                for skill in found {
                    if this.skills.iter().any(|existing| existing.id == skill.id) {
                        skipped += 1;
                    } else if this.skills.len() < 128 {
                        this.skills.push(skill);
                        added += 1;
                    } else {
                        skipped += 1;
                    }
                }
                if added > 0 {
                    this.persist_settings(cx);
                }
                this.skills_ui.message = Some(format!(
                    "Scanned global folders: imported {added}, skipped {skipped}. Discovery is read-only and does not claim ownership of existing files."
                ));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_skill(&mut self, cx: &mut Context<Self>) {
        if self.skills_ui.busy {
            return;
        }
        let Some(draft) = &self.skills_ui.draft else {
            return;
        };
        let mut skill = Skill {
            id: draft.id.clone(),
            title: draft.title.read(cx).value().trim().to_string(),
            description: draft.description.read(cx).value().trim().to_string(),
            body: draft.body.read(cx).value().to_string(),
            origin: String::new(),
            source_hash: String::new(),
            installations: Vec::new(),
        };
        if let Err(e) = skill.rendered() {
            self.skills_ui.message = Some(e);
            cx.notify();
            return;
        }
        if let Some(old) = self.skills.iter_mut().find(|s| s.id == skill.id) {
            skill.installations = old.installations.clone();
            skill.origin = if old.origin.is_empty() {
                "Manual".into()
            } else {
                old.origin.clone()
            };
            skill.source_hash = skill.content_hash();
            *old = skill;
        } else if self.skills.len() < 128 {
            skill.origin = "Manual".into();
            skill.source_hash = skill.content_hash();
            self.skills.push(skill);
        } else {
            self.skills_ui.message = Some("The library limit is 128 skills".into());
            cx.notify();
            return;
        }
        self.skills_ui.draft = None;
        self.skills_ui.message = Some(
            "Saved in the global library. Use the harness buttons to install or update its copies."
                .into(),
        );
        self.persist_settings(cx);
        cx.notify();
    }
    fn install_skill(
        &mut self,
        id: &str,
        target: Target,
        directory: Option<PathBuf>,
        cx: &mut Context<Self>,
    ) {
        self.install_skill_scoped(id, target, directory, Scope::Global, "", cx);
    }

    fn install_skill_scoped(
        &mut self,
        id: &str,
        target: Target,
        directory: Option<PathBuf>,
        scope: Scope,
        project_id: &str,
        cx: &mut Context<Self>,
    ) {
        if self.skills_ui.busy {
            return;
        }
        let Some(skill) = self.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let base = match directory.map(Ok).unwrap_or_else(|| {
            if scope == Scope::Project {
                let project = self.project();
                if project.is_remote() {
                    return Err("Project skill installation requires a local project".into());
                }
                target.project_directory(Path::new(&project.path))
            } else {
                target.directory()
            }
        }) {
            Ok(p) => p,
            Err(e) => {
                self.skills_ui.message = Some(e);
                cx.notify();
                return;
            }
        };
        let project_id = project_id.to_string();
        self.skills_ui.busy = true;
        self.skills_ui.message = Some(if scope == Scope::Project {
            "Installing project skill…".into()
        } else {
            "Installing skill…".into()
        });
        cx.spawn(async move |this, cx| {
            let id = skill.id.clone();
            let scope_for_result = scope;
            let result = cx.background_executor().spawn(async move {
                install_at_scoped(&base, &skill, target, scope, &project_id)
            }).await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                match result {
                    Ok(installation) => {
                        if let Some(skill) = this.skills.iter_mut().find(|s| s.id == id) {
                            skill.installations.retain(|i| i.path != installation.path);
                            skill.installations.push(installation);
                            this.persist_settings(cx);
                        }
                        this.skills_ui.message = Some(if scope_for_result == Scope::Project {
                            "Installed in this project. The harness may prefer it over the global copy; no harness program was installed or executed.".into()
                        } else {
                            "Installed globally. Project-level skills may override global definitions; no harness program was installed or executed.".into()
                        });
                    }
                    Err(e) => this.skills_ui.message = Some(e),
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }
    fn remove_skill_installations(
        &mut self,
        id: &str,
        target: Option<Target>,
        delete: bool,
        cx: &mut Context<Self>,
    ) {
        self.remove_skill_installations_scoped(id, target, None, None, delete, cx);
    }

    fn remove_skill_installations_scoped(
        &mut self,
        id: &str,
        target: Option<Target>,
        scope: Option<Scope>,
        project_id: Option<&str>,
        delete: bool,
        cx: &mut Context<Self>,
    ) {
        if self.skills_ui.busy {
            return;
        }
        let Some(skill) = self.skills.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        let project_id = project_id.map(str::to_string);
        self.skills_ui.busy = true;
        cx.spawn(async move |this, cx| {
            let id = skill.id.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    skill
                        .installations
                        .iter()
                        .filter(|i| target.is_none_or(|target| i.target == target))
                        .filter(|i| scope.is_none_or(|scope| i.scope == scope))
                        .filter(|i| {
                            project_id
                                .as_deref()
                                .is_none_or(|project| i.project_id == project)
                        })
                        .map(|i| (i.path.clone(), uninstall(i, &skill.id)))
                        .collect::<Vec<_>>()
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.skills_ui.busy = false;
                let mut errors = Vec::new();
                if let Some(skill) = this.skills.iter_mut().find(|s| s.id == id) {
                    for (path, result) in result {
                        match result {
                            Ok(()) => skill.installations.retain(|i| i.path != path),
                            Err(e) => errors.push(e),
                        }
                    }
                }
                if delete && errors.is_empty() {
                    this.skills.retain(|s| s.id != id);
                }
                this.skills_ui.message = Some(if errors.is_empty() {
                    "Removed managed skill copies".into()
                } else {
                    errors.join("\n")
                });
                this.persist_settings(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

fn action(id: String, label: impl Into<SharedString>, p: Palette) -> Stateful<Div> {
    div()
        .id(id)
        .h(px(28.))
        .px(px(11.))
        .rounded(px(6.))
        .border_1()
        .border_color(p.border)
        .bg(p.card)
        .text_color(p.fg)
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(p.hover).border_color(p.border_strong))
        .child(label.into())
}
pub(crate) fn render(this: &DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    let p = palette(cx);
    let mut body = div().p(px(16.)).flex().flex_col().gap(px(12.))
        .child(div().text_size(px(11.5)).text_color(p.muted_fg).child("One global instruction library on this Mac. Review instructions before installing: harnesses may execute tools they request. No credentials or harness settings are changed. Only unchanged Dunlin-managed files can be updated or removed."));
    if let Some(message) = &this.skills_ui.message {
        body = body.child(
            div()
                .text_size(px(11.5))
                .text_color(p.muted_fg)
                .child(message.clone()),
        );
    }
    if let Some(draft) = &this.skills_ui.draft {
        return body
            .child(Input::new(&draft.title))
            .child(Input::new(&draft.description))
            .child(
                div()
                    .h(px(240.))
                    .overflow_hidden()
                    .child(Editor::new(&draft.body).h_full()),
            )
            .child(
                div()
                    .flex()
                    .gap(px(6.))
                    .child(
                        action("save-skill".into(), "Save skill", p)
                            .on_click(cx.listener(|this, _, _, cx| this.save_skill(cx))),
                    )
                    .child(
                        action("cancel-skill".into(), "Cancel", p).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.skills_ui.draft = None;
                                cx.notify();
                            },
                        )),
                    ),
            )
            .into_any_element();
    }
    body = body.child(
        div()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .child(
                action("new-skill".into(), "Add skill", p)
                    .on_click(cx.listener(|this, _, window, cx| this.edit_skill(None, window, cx))),
            )
            .child(
                action("scan-skills".into(), "Scan harness folders", p)
                    .on_click(cx.listener(|this, _, _, cx| this.scan_global_skills(cx))),
            )
            .child(
                action("import-skills".into(), "Import folder…", p).on_click(cx.listener(
                    |this, _, _, cx| {
                        let selected = cx.prompt_for_paths(PathPromptOptions {
                            files: false,
                            directories: true,
                            multiple: false,
                            prompt: Some(
                                "Choose a folder containing SKILL.md or skill folders".into(),
                            ),
                        });
                        let selected = this.overlay_backdrop.track_native(selected, cx);
                        cx.spawn(async move |this, cx| {
                            if let Ok(Ok(Some(paths))) = selected.await
                                && let Some(path) = paths.into_iter().next()
                            {
                                let _ =
                                    this.update(cx, |this, cx| this.import_skills(vec![path], cx));
                            }
                        })
                        .detach();
                    },
                )),
            ),
    );
    body = body.child(
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(10.5))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(p.muted_fg)
                    .child("GitHub skill"),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .h(px(28.))
                            .flex_1()
                            .min_w(px(0.))
                            .px(px(9.))
                            .rounded(px(6.))
                            .border_1()
                            .border_color(p.border)
                            .bg(p.bg)
                            .flex()
                            .items_center()
                            .child(
                                Input::new(&this.skills_import_url)
                                    .appearance(false)
                                    .bordered(false)
                                    .h(px(26.)),
                            ),
                    )
                    .child(
                        action("import-github-skill".into(), "Import from GitHub", p).on_click(
                            cx.listener(|this, _, window, cx| {
                                this.import_skill_from_github(window, cx)
                            }),
                        ),
                    ),
            ),
    );
    if !this.skills.iter().any(|s| s.id == COMMIT_SKILL) {
        body = body.child(
            action(
                "restore-commit-skill".into(),
                "Add Conventional Commits skill",
                p,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.skills_ui.busy && this.skills.len() < 128 {
                    this.skills.extend(defaults());
                    this.persist_settings(cx);
                    cx.notify();
                }
            })),
        );
    }
    for skill in &this.skills {
        let id = skill.id.clone();
        let remove = id.clone();
        let mut card = div().p(px(12.)).rounded(px(8.)).bg(p.card).flex().flex_col().gap(px(8.))
            .child(div().flex().items_center().gap(px(6.)).child(div().flex_1().font_weight(FontWeight::MEDIUM).child(skill.title.clone()))
                .child(action(format!("edit-skill:{id}"), "Edit", p).on_click(cx.listener(move |this, _, window, cx| this.edit_skill(Some(&id), window, cx))))
                .child(action(format!("delete-skill:{remove}"), "Delete", p).on_click(cx.listener(move |this, _, window, cx| {
                    let id = remove.clone();
                    let answer = window.prompt(PromptLevel::Warning, "Delete skill?", Some("Delete the library entry and its unchanged Dunlin-managed installations? Externally modified files are preserved."), &["Cancel", "Delete"], cx);
                    let answer = this.overlay_backdrop.track_native(answer, cx);
                    cx.spawn(async move |this, cx| { if answer.await == Ok(1) { let _ = this.update(cx, |this, cx| this.remove_skill_installations(&id, None, true, cx)); } }).detach();
                }))))
            .child(div().text_size(px(11.5)).text_color(p.muted_fg).child(skill.description.clone()));
        let mut targets = div().flex().flex_wrap().gap(px(6.));
        for target in Target::ALL {
            let id = skill.id.clone();
            let uninstall_id = id.clone();
            let installed = skill
                .installations
                .iter()
                .filter(|i| i.target == target && i.scope == Scope::Global)
                .collect::<Vec<_>>();
            let current = skill.rendered().ok();
            let state = if installed.is_empty() {
                "Install"
            } else if installed
                .iter()
                .all(|i| Some(&i.contents) == current.as_ref())
            {
                "Installed"
            } else {
                "Update"
            };
            targets = targets.child(
                action(
                    format!("install-skill:{id}:{target:?}"),
                    format!("{} · {state}", target.label()),
                    p,
                )
                .on_click(
                    cx.listener(move |this, _, _, cx| this.install_skill(&id, target, None, cx)),
                ),
            );
            if !installed.is_empty() {
                targets = targets.child(
                    action(format!("uninstall-skill:{uninstall_id}:{target:?}"), "×", p)
                        .aria_label(format!("Uninstall from {}", target.label()))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_skill_installations_scoped(
                                &uninstall_id,
                                Some(target),
                                Some(Scope::Global),
                                None,
                                false,
                                cx,
                            )
                        })),
                );
            }
        }
        let id = skill.id.clone();
        targets = targets.child(
            action(format!("install-custom-skill:{id}"), "Other folder…", p).on_click(cx.listener(
                move |this, _, _, cx| {
                    let id = id.clone();
                    let selected = cx.prompt_for_paths(PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Choose the harness's global skills directory".into()),
                    });
                    let selected = this.overlay_backdrop.track_native(selected, cx);
                    cx.spawn(async move |this, cx| {
                        if let Ok(Ok(Some(paths))) = selected.await
                            && let Some(base) = paths.into_iter().next()
                        {
                            let _ = this.update(cx, |this, cx| {
                                this.install_skill(&id, Target::Other, Some(base), cx)
                            });
                        }
                    })
                    .detach();
                },
            )),
        );
        if skill
            .installations
            .iter()
            .any(|i| i.target == Target::Other && i.scope == Scope::Global)
        {
            let id = skill.id.clone();
            targets = targets.child(
                action(format!("uninstall-other-skill:{id}"), "Uninstall Other", p).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.remove_skill_installations_scoped(
                            &id,
                            Some(Target::Other),
                            Some(Scope::Global),
                            None,
                            false,
                            cx,
                        )
                    }),
                ),
            );
        }
        card = card.child(targets);
        let origin = if skill.origin.is_empty() {
            "Manual".to_string()
        } else {
            skill.origin.clone()
        };
        let hash = if skill.source_hash.is_empty() {
            skill.content_hash()
        } else {
            skill.source_hash.clone()
        };
        let hash = &hash[..hash.len().min(12)];
        card = card.child(
            div()
                .text_size(px(10.))
                .text_color(p.muted_fg)
                .child(format!("Origin: {origin} · SHA-256 {hash}")),
        );
        if let Some(project) = this
            .projects
            .iter()
            .find(|project| {
                project.id == this.project().id && !project.is_remote() && project.trusted
            })
            .cloned()
        {
            let root = PathBuf::from(&project.path);
            let mut project_row = div().flex().flex_wrap().gap(px(6.));
            for target in Target::ALL {
                let Ok(base) = target.project_directory(&root) else {
                    continue;
                };
                let id = skill.id.clone();
                let project_id = project.id.clone();
                let installed = skill.installations.iter().any(|i| {
                    i.target == target && i.scope == Scope::Project && i.project_id == project.id
                });
                project_row = project_row.child(
                    action(
                        format!("project-skill:{id}:{target:?}"),
                        if installed {
                            format!("Remove project {}", target.label())
                        } else {
                            format!("Install in project · {}", target.label())
                        },
                        p,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if installed {
                            this.remove_skill_installations_scoped(
                                &id,
                                Some(target),
                                Some(Scope::Project),
                                Some(&project_id),
                                false,
                                cx,
                            );
                        } else {
                            this.install_skill_scoped(
                                &id,
                                target,
                                Some(base.clone()),
                                Scope::Project,
                                &project_id,
                                cx,
                            );
                        }
                    })),
                );
            }
            card = card
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("Project copies · {}", project.name)),
                )
                .child(project_row);
        }
        body = body.child(card);
    }
    body.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{Scope, Target, defaults, discover, install_at, install_at_scoped, uninstall};
    use std::{fs, path::PathBuf};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("dunlin-skills-{}", uuid::Uuid::now_v7()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn portable_frontmatter_is_bounded_and_description_cannot_inject_fields() {
        let mut skill = defaults().remove(0);
        skill.description = "Commit rules\n---\nmodel: unwanted".into();
        let text = skill.rendered().unwrap();
        assert_eq!(text.lines().filter(|line| *line == "---").count(), 2);
        assert!(text.contains("disable-model-invocation: true"));
        assert!(text.contains(r#"\n---\nmodel: unwanted"#));
        skill.id = "../escape".into();
        assert!(skill.rendered().is_err());
        skill.id = "safe".into();
        skill.body = "x".repeat(64 * 1024);
        assert!(skill.rendered().is_err());
    }
    #[test]
    fn install_updates_only_owned_unchanged_files_and_refuses_user_edits() {
        let temp = Temp::new();
        let mut skill = defaults().remove(0);
        let installed = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        let original = fs::read_to_string(&installed.path).unwrap();
        assert!(install_at(&temp.0, &skill, Target::OpenCode).is_err()); // Not yet recorded as ours.
        assert_eq!(fs::read_to_string(&installed.path).unwrap(), original);
        skill.installations.push(installed);
        skill.body.push_str("\nAdditional reviewed instructions.\n");
        let updated = install_at(&temp.0, &skill, Target::OpenCode).unwrap();
        assert_ne!(updated.contents, original);
        assert_eq!(fs::read_to_string(&updated.path).unwrap(), updated.contents);
        skill.installations = vec![updated.clone()];
        fs::write(&updated.path, "externally modified").unwrap();
        assert!(install_at(&temp.0, &skill, Target::OpenCode).is_err());
        assert!(uninstall(&updated, &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(&updated.path).unwrap(),
            "externally modified"
        );
    }
    #[test]
    fn uninstall_never_recursively_removes_supporting_or_user_files() {
        let temp = Temp::new();
        let skill = defaults().remove(0);
        let installed = install_at(&temp.0, &skill, Target::Codex).unwrap();
        let extra = std::path::Path::new(&installed.path)
            .parent()
            .unwrap()
            .join("keep.txt");
        fs::write(&extra, "keep").unwrap();
        uninstall(&installed, &skill.id).unwrap();
        assert_eq!(fs::read_to_string(extra).unwrap(), "keep");
        assert!(!std::path::Path::new(&installed.path).exists());
        assert!(uninstall(&installed, "different-id").is_err());
    }
    #[cfg(unix)]
    #[test]
    fn skill_directory_and_file_symlinks_cannot_overwrite_external_content() {
        use std::os::unix::fs::symlink;
        let temp = Temp::new();
        let outside = Temp::new();
        let skill = defaults().remove(0);
        fs::write(outside.0.join("SKILL.md"), "external").unwrap();
        let directory = temp.0.join(format!("dunlin-{}", skill.id));
        symlink(&outside.0, &directory).unwrap();
        assert!(install_at(&temp.0, &skill, Target::Claude).is_err());
        fs::remove_file(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        symlink(outside.0.join("SKILL.md"), directory.join("SKILL.md")).unwrap();
        assert!(install_at(&temp.0, &skill, Target::Claude).is_err());
        assert_eq!(
            fs::read_to_string(outside.0.join("SKILL.md")).unwrap(),
            "external"
        );
    }
    #[cfg(unix)]
    #[test]
    fn uninstall_refuses_an_ancestor_replaced_with_a_symlink() {
        let temp = Temp::new();
        let outside = Temp::new();
        let base = temp.0.join("skills");
        let skill = defaults().remove(0);
        let installed = install_at(&base, &skill, Target::OpenCode).unwrap();
        fs::rename(&base, temp.0.join("moved-skills")).unwrap();
        let external = outside.0.join(format!("dunlin-{}", skill.id));
        fs::create_dir(&external).unwrap();
        fs::write(external.join("SKILL.md"), &installed.contents).unwrap();
        std::os::unix::fs::symlink(&outside.0, &base).unwrap();
        assert!(uninstall(&installed, &skill.id).is_err());
        assert_eq!(
            fs::read_to_string(external.join("SKILL.md")).unwrap(),
            installed.contents
        );
    }
    #[test]
    fn ordinary_skills_remain_discoverable_without_commit_only_invocation_restriction() {
        let mut skill = defaults().remove(0);
        skill.id = "review".into();
        assert!(
            !skill
                .rendered()
                .unwrap()
                .contains("disable-model-invocation")
        );
    }
    #[test]
    fn commit_skill_preserves_index_scope_and_repository_policy_without_bypasses() {
        let skill = defaults().remove(0);
        for rule in [
            "Conventional Commits",
            "AGENTS.md",
            "recent commit",
            "partial staging",
            "nonempty",
            "--no-verify",
            "never use blind",
            "Never infer authorization",
            "Repository policy wins",
        ] {
            assert!(skill.body.contains(rule), "missing {rule}");
        }
        let serialized = serde_json::to_string(&vec![skill]).unwrap();
        let restored: Vec<super::Skill> = serde_json::from_str(&serialized).unwrap();
        assert_eq!(restored[0].id, super::COMMIT_SKILL);
    }
    #[test]
    fn discovery_reads_frontmatter_and_rejects_unbounded_or_symlinked_skills() {
        let temp = Temp::new();
        let directory = temp.0.join("dunlin-docs");
        fs::create_dir(&directory).unwrap();
        fs::write(
            directory.join("SKILL.md"),
            "---\nname: dunlin-docs\ndescription: \"Use project docs\"\n---\n\nRead the docs.\n",
        )
        .unwrap();
        let found = discover(&temp.0).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, "docs");
        assert_eq!(found[0].description, "Use project docs");
        assert_eq!(found[0].body, "Read the docs.");
        assert_eq!(found[0].source_hash.len(), 64);
        #[cfg(unix)]
        {
            let link = temp.0.join("linked");
            std::os::unix::fs::symlink(&directory, &link).unwrap();
            assert!(discover(&link).is_err());
        }
        fs::write(directory.join("SKILL.md"), "x".repeat(64 * 1024 + 1)).unwrap();
        assert!(discover(&temp.0).is_err());
    }
    #[test]
    fn project_installation_records_scope_and_can_be_removed_without_global_scope() {
        let temp = Temp::new();
        let skill = defaults().remove(0);
        let base = Target::Claude.project_directory(&temp.0).unwrap();
        let installation =
            install_at_scoped(&base, &skill, Target::Claude, Scope::Project, "project-1").unwrap();
        assert_eq!(installation.scope, Scope::Project);
        assert_eq!(installation.project_id, "project-1");
        assert!(installation.path.contains(".claude"));
        uninstall(&installation, &skill.id).unwrap();
        assert!(!PathBuf::from(&installation.path).exists());
    }
    #[test]
    fn github_skill_links_map_to_raw_skill_md() {
        let repo = super::github_skill_urls("https://github.com/owner/repo").unwrap();
        assert_eq!(
            repo[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/SKILL.md"
        );
        assert_eq!(
            repo[1].as_str(),
            "https://raw.githubusercontent.com/owner/repo/master/SKILL.md"
        );
        let tree = super::github_skill_urls("https://github.com/owner/repo/tree/main/skills/demo")
            .unwrap();
        assert_eq!(
            tree[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/skills/demo/SKILL.md"
        );
        let blob = super::github_skill_urls(
            "https://github.com/owner/repo/blob/main/skills/demo/SKILL.md",
        )
        .unwrap();
        assert_eq!(
            blob[0].as_str(),
            "https://raw.githubusercontent.com/owner/repo/main/skills/demo/SKILL.md"
        );
        assert!(super::github_skill_urls("http://github.com/owner/repo").is_err());
        assert!(super::github_skill_urls("https://example.com/owner/repo").is_err());
        assert!(
            super::github_skill_urls("https://github.com/owner/repo/blob/main/README.md").is_err()
        );
    }
}
