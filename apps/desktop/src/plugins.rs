//! Settings surface for declarative plugin packs. Packs contribute metadata,
//! commands and skill names; no plugin code is linked into Dunlin in v1.

use crate::app::DunlinApp;
use dunlin_plugin::{LoadedPack, Source};
use dunlin_plugin_host::InstalledPlugin;
use dunlin_ui::prelude::FluentBuilder as _;
use dunlin_ui::*;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub(crate) struct PackCommandEntry {
    pub pack_id: String,
    pub pack_name: String,
    pub command_id: String,
    pub label: String,
    pub description: String,
}

impl DunlinApp {
    pub(crate) fn pack_command_entries(&self) -> Vec<PackCommandEntry> {
        if self.projects.is_empty() {
            return Vec::new();
        }
        let project_id = self.project().id.clone();
        let mut entries = Vec::new();
        for pack in self
            .plugins
            .packs
            .iter()
            .filter(|pack| self.plugins.is_enabled_for(Some(&project_id), pack))
        {
            if !self.spec_tools_enabled
                && matches!(pack.manifest.id.as_str(), "spec-kit" | "agent-contract")
            {
                continue;
            }
            for command in &pack.manifest.contributes.commands {
                if command.prompt.trim().is_empty() {
                    continue;
                }
                entries.push(PackCommandEntry {
                    pack_id: pack.manifest.id.clone(),
                    pack_name: pack.manifest.name.clone(),
                    command_id: command.id.clone(),
                    label: command.label.clone(),
                    description: command.description.clone(),
                });
            }
        }
        entries
    }

    pub(crate) fn run_pack_command(
        &mut self,
        pack_id: &str,
        command_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_id = self.project().id.clone();
        let Some(prompt) = self
            .plugins
            .packs
            .iter()
            .find(|pack| {
                pack.manifest.id == pack_id && self.plugins.is_enabled_for(Some(&project_id), pack)
            })
            .and_then(|pack| pack.command_prompt(command_id).map(str::to_string))
        else {
            self.file_notice(
                "This pack command is disabled for the current project or has no prompt",
                cx,
            );
            return;
        };
        let mut next = self.composer.read(cx).value().to_string();
        if !next.trim().is_empty() {
            next.push_str("\n\n");
        }
        next.push_str(prompt.trim_end());
        self.composer
            .update(cx, |state, cx| state.set_value(next, window, cx));
        self.set_notice(
            format!("Added the {pack_id}/{command_id} prompt to the composer"),
            cx,
        );
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.composer
            .update(cx, |state, cx| state.focus(window, cx));
    }
}

pub(crate) struct State {
    pub packs: Vec<LoadedPack>,
    pub warnings: Vec<String>,
    pub loaded: bool,
    pub error: Option<String>,
    pub scope_project: bool,
    disabled: Vec<String>,
    enabled: Vec<String>,
    project_disabled: HashMap<String, Vec<String>>,
    project_enabled: HashMap<String, Vec<String>>,
    pub(crate) wasm_status: HashMap<String, String>,
    pub(crate) available: Vec<dunlin_plugin::CatalogPlugin>,
}

impl Default for State {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new(), HashMap::new(), HashMap::new())
    }
}

fn governance_mcp_config(
    pack: &LoadedPack,
    project: &dunlin_domain::Project,
    binary: &str,
) -> Option<dunlin_agent::McpServerConfig> {
    if project.is_remote() || !project.trusted {
        return None;
    }
    let directory = pack.directory.as_deref()?;
    Some(dunlin_agent::McpServerConfig::stdio(
        "dunlin-knowledge",
        binary,
        vec![
            "--project".into(),
            project.path.clone(),
            "--plugin".into(),
            directory.to_string(),
        ],
        Vec::new(),
    ))
}

impl DunlinApp {
    pub(crate) fn start_plugin_builder_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.add_catalog_skill(crate::skills::PLUGIN_BUILDER_SKILL, cx);
        self.install_catalog_skill(
            crate::skills::PLUGIN_BUILDER_SKILL,
            crate::skills::Target::Shared,
            crate::skills::Scope::Global,
            "",
            None,
            cx,
        );
        let local_project = !self.projects.is_empty() && !self.project().is_remote();
        if local_project {
            self.new_session(cx);
        } else {
            self.new_chat_space(cx);
        }
        let prompt = r#"Use the `plugin-builder` skill. We are going to build a fully working Dunlin WASM plugin together. If the skill is not visible in this harness yet, tell me and I will install it from Settings → Skills.

Start by interviewing me one question at a time. Do not write any files yet. Ask me, in order:
1. What should the plugin do, in one sentence?
2. Plugin id, name, version and author.
3. Which knowledge kinds should it contribute (id, label, directory, statuses, fields, relations)?
4. Which context sections should it add to the intent envelope?
5. Which MCP tools should it expose, and which are proposal-only?
6. Which project paths may it read or write?
7. How should we test it (sample files and expected output)?
8. Should it be bundled with Dunlin or installed as an external pack?

After my answers, propose the manifest v2, the Rust WASM ABI implementation, build/install commands, tests and a security checklist. Follow the skill exactly: sandboxed WASM only, no WASI/network/process/Keychain, disabled by default, installed under Application Support/Dunlin/plugins, never inside Dunlin.app. Ask before writing any file."#;
        self.composer.update(cx, |state, cx| {
            state.set_value(prompt.to_string(), window, cx)
        });
        self.set_view(dunlin_domain::MainView::Chat, cx);
        self.set_notice(
            "Plugin builder chat started. The agent will interview you first.",
            cx,
        );
        cx.notify();
    }
}

impl State {
    pub(crate) fn new(
        disabled: Vec<String>,
        enabled: Vec<String>,
        project_disabled: HashMap<String, Vec<String>>,
        project_enabled: HashMap<String, Vec<String>>,
    ) -> Self {
        let mut state = Self {
            packs: Vec::new(),
            warnings: Vec::new(),
            loaded: false,
            error: None,
            scope_project: false,
            disabled,
            enabled,
            project_disabled,
            project_enabled,
            wasm_status: HashMap::new(),
            available: Vec::new(),
        };
        state.load();
        state
    }
    pub(crate) fn disabled(&self) -> Vec<String> {
        self.disabled.clone()
    }
    pub(crate) fn enabled(&self) -> Vec<String> {
        self.enabled.clone()
    }
    pub(crate) fn project_disabled(&self) -> HashMap<String, Vec<String>> {
        self.project_disabled.clone()
    }
    pub(crate) fn project_enabled(&self) -> HashMap<String, Vec<String>> {
        self.project_enabled.clone()
    }
    pub(crate) fn command_prompt_for(&self, project_id: &str, command_id: &str) -> Option<String> {
        self.packs
            .iter()
            .filter(|pack| self.is_enabled_for(Some(project_id), pack))
            .find_map(|pack| pack.command_prompt(command_id).map(str::to_string))
    }
    fn is_enabled(&self, pack: &LoadedPack) -> bool {
        match pack.source {
            Source::Bundled => !self.disabled.contains(&pack.manifest.id),
            Source::External => self.enabled.contains(&pack.manifest.id),
        }
    }
    pub(crate) fn is_enabled_for(&self, project: Option<&str>, pack: &LoadedPack) -> bool {
        let id = &pack.manifest.id;
        let base = self.is_enabled(pack);
        let Some(project) = project else {
            return base;
        };
        if self
            .project_disabled
            .get(project)
            .is_some_and(|ids| ids.contains(id))
        {
            return false;
        }
        if self
            .project_enabled
            .get(project)
            .is_some_and(|ids| ids.contains(id))
        {
            return true;
        }
        base
    }
    pub(crate) fn toggle_for(&mut self, project: Option<&str>, pack: &LoadedPack) -> bool {
        let id = pack.manifest.id.clone();
        if let Some(project) = project {
            let base = self.is_enabled(pack);
            let currently = self.is_enabled_for(Some(project), pack);
            let next = !currently;
            let disabled = self
                .project_disabled
                .entry(project.to_string())
                .or_default();
            let enabled = self.project_enabled.entry(project.to_string()).or_default();
            if next {
                disabled.retain(|entry| entry != &id);
                enabled.retain(|entry| entry != &id);
                if !base {
                    enabled.push(id);
                }
            } else {
                enabled.retain(|entry| entry != &id);
                disabled.retain(|entry| entry != &id);
                if base {
                    disabled.push(id);
                }
            }
            return next;
        }
        let next = !self.is_enabled(pack);
        match pack.source {
            Source::Bundled => {
                if next {
                    self.disabled.retain(|entry| entry != &id);
                } else if !self.disabled.contains(&id) {
                    self.disabled.push(id);
                }
            }
            Source::External => {
                if next {
                    if !self.enabled.contains(&id) {
                        self.enabled.push(id);
                    }
                } else {
                    self.enabled.retain(|entry| entry != &id);
                }
            }
        }
        next
    }
    pub(crate) fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let directory = crate::app::data_dir().map(|data| data.join("plugins"));
        let (packs, warnings) = dunlin_plugin::discover(directory.as_deref());
        self.available = dunlin_plugin::catalog()
            .into_iter()
            .filter(|catalog| !packs.iter().any(|pack| pack.manifest.id == catalog.id))
            .collect();
        self.packs = packs;
        self.warnings = warnings;
        self.error = None;
    }
    pub(crate) fn attach_governance_mcp(
        &self,
        config: &mut dunlin_agent::HarnessConfig,
        project: &dunlin_domain::Project,
    ) {
        if project.is_remote() || !project.trusted {
            return;
        }
        let Some(pack) = self.packs.iter().find(|pack| {
            pack.manifest.id == "dunlin-governance" && self.is_enabled_for(Some(&project.id), pack)
        }) else {
            return;
        };
        let binary = Self::mcp_binary();
        if !Path::new(&binary).is_file() && !crate::harness::command_available(&binary) {
            crate::logging::warn(
                "governance pack is enabled but dunlin-knowledge-mcp is not available",
            );
            return;
        }
        if let Some(server) = governance_mcp_config(pack, project, &binary) {
            config.mcp_servers.push(server);
        }
    }

    pub(crate) fn install_bundled(&mut self, id: &str) -> Result<String, String> {
        let catalog = dunlin_plugin::catalog_plugin(id)
            .ok_or_else(|| format!("Unknown bundled plugin {id}"))?;
        let root = Self::directory().ok_or("Application data directory is unavailable")?;
        let directory = root.join(&catalog.id);
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("Create plugin directory: {error}"))?;
        let wasm = catalog
            .manifest
            .wasm
            .as_ref()
            .ok_or("Bundled plugin has no WASM module")?;
        let actual = hex::encode(Sha256::digest(catalog.wasm));
        if !actual.eq_ignore_ascii_case(&wasm.sha256) {
            return Err("Bundled WASM failed its SHA-256 check".into());
        }
        let manifest_path = directory.join("plugin.json");
        let wasm_path = directory.join("plugin.wasm");
        std::fs::write(&manifest_path, catalog.manifest_text)
            .map_err(|error| format!("Write plugin.json: {error}"))?;
        std::fs::write(&wasm_path, catalog.wasm)
            .map_err(|error| format!("Write plugin.wasm: {error}"))?;
        self.loaded = false;
        self.load();
        Ok(format!(
            "Installed {} in {}. Enable it explicitly to load its WASM module.",
            catalog.id,
            directory.display()
        ))
    }

    pub(crate) fn validate_wasm_by_id(&mut self, id: &str) -> Result<String, String> {
        let pack = self
            .packs
            .iter()
            .find(|pack| pack.manifest.id == id)
            .cloned()
            .ok_or_else(|| "Plugin is no longer discovered".to_string())?;
        self.validate_wasm(&pack)
    }

    pub(crate) fn validate_wasm(&mut self, pack: &LoadedPack) -> Result<String, String> {
        let wasm = pack
            .manifest
            .wasm
            .as_ref()
            .ok_or_else(|| "This pack has no WASM module".to_string())?;
        let directory = pack.directory.as_deref().ok_or_else(|| {
            "Bundled WASM packs are not installed from a directory yet".to_string()
        })?;
        let directory = PathBuf::from(directory);
        let installed = InstalledPlugin {
            manifest: pack.manifest.clone(),
            manifest_hash: pack.manifest_hash.clone(),
            directory: directory.clone(),
            wasm_path: Some(directory.join(&wasm.path)),
        };
        let plugin = installed.load().map_err(|error| error.to_string())?;
        let schema = plugin.describe().map_err(|error| error.to_string())?;
        let status = format!(
            "WASM ok · {} kind(s) · {} relation(s) · fuel {}",
            schema.kinds.len(),
            schema.relations.len(),
            wasm.fuel
        );
        self.wasm_status
            .insert(pack.manifest.id.clone(), status.clone());
        Ok(status)
    }

    fn directory() -> Option<PathBuf> {
        crate::app::data_dir().map(|data| data.join("plugins"))
    }

    fn proposals_directory() -> Option<PathBuf> {
        crate::app::data_dir().map(|data| data.join("proposals"))
    }

    pub(crate) fn mcp_binary() -> String {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join("dunlin-knowledge-mcp")))
            .filter(|path| path.is_file())
            .map(|path| path.to_string_lossy().to_string())
            .unwrap_or_else(|| "dunlin-knowledge-mcp".into())
    }

    fn pending_proposals() -> usize {
        let Some(directory) = Self::proposals_directory() else {
            return 0;
        };
        std::fs::read_dir(directory)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter(|entry| {
                        entry.path().extension().and_then(|ext| ext.to_str()) == Some("json")
                    })
                    .count()
            })
            .unwrap_or(0)
    }
}

fn card(p: &Palette) -> Div {
    dunlin_ui::settings::card(*p)
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(10.))
}

fn badge(label: impl Into<SharedString>, color: Hsla, p: &Palette) -> Div {
    div()
        .h(px(19.))
        .px(px(7.))
        .rounded(px(dunlin_ui::radii::CODE))
        .bg(p.muted)
        .flex()
        .items_center()
        .text_size(px(9.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(color)
        .child(label.into())
}

fn permissions(pack: &LoadedPack) -> String {
    let permissions = &pack.manifest.permissions;
    let mut parts = Vec::new();
    if !permissions.project_read.is_empty() {
        parts.push(format!("read {}", permissions.project_read.join(", ")));
    }
    if !permissions.project_write.is_empty() {
        parts.push(format!("write {}", permissions.project_write.join(", ")));
    }
    if permissions.agent_prompt {
        parts.push("prompt: explicit only".into());
    }
    if !permissions.network.is_empty() {
        parts.push(format!("network {}", permissions.network.join(", ")));
    }
    if permissions.process {
        parts.push("process execution".into());
    }
    if pack.manifest.wasm.is_some() {
        parts.push("sandboxed WASM".into());
    }
    if parts.is_empty() {
        "no host capabilities requested".into()
    } else {
        parts.join(" · ")
    }
}

pub(crate) fn render(this: &mut DunlinApp, cx: &mut Context<DunlinApp>) -> AnyElement {
    this.plugins.load();
    let p = palette(cx);
    let project = this.project().clone();
    let scope = this.plugins.scope_project.then(|| project.id.clone());
    let directory = State::directory()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "unavailable".into());
    let mut content = dunlin_ui::settings::page()
        .child(dunlin_ui::settings::section("Built-in workspace tools · this Mac", p))
        .child(dunlin_ui::settings::card(p).child(dunlin_ui::settings::row("Spec & intent tools",
            "Show Spec, Intent Inbox and agent-contract commands. Disabling leaves existing files untouched.",
            dunlin_ui::switch("spec-tools", "Spec & intent tools", this.spec_tools_enabled)
                .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                    this.spec_tools_enabled = *enabled;
                    if !enabled { this.spec.active = false; this.spec.view = None; }
                    this.persist_settings(cx); cx.notify();
                })).into_any_element(), p)))
        .child(dunlin_ui::settings::section("Extension packs", p))
        .child(dunlin_ui::settings::description("Preferences and installed packs live on this Mac. Project overrides do not install packs or authorize tools on a remote server.", p))
        .child(
            card(&p)
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Methodology packs"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child("Packs add methodology, commands and optional sandboxed WASM validation. They cannot run native app code. Installation and enabling are separate actions; review capabilities before enabling."),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("External pack folder: {directory}")),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            div()
                                .text_size(px(10.))
                                .text_color(p.muted_fg)
                                .flex_1().child(format!(
                                    "Pending governance proposals: {}",
                                    State::pending_proposals()
                                )),
                        )
                        .child(
                            dunlin_ui::settings::button("extensions-open-proposals")
                                .label("Open proposals folder")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    if let Some(directory) = State::proposals_directory()
                                        && std::fs::create_dir_all(&directory).is_ok()
                                    {
                                        let _ = std::process::Command::new("open")
                                            .arg(&directory)
                                            .spawn();
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            dunlin_ui::settings::segments(p)
                                .child(dunlin_ui::settings::segment("extensions-scope-global", "This Mac · global", !this.plugins.scope_project)
                                    .on_click(cx.listener(|this, _, _, cx| { this.plugins.scope_project = false; cx.notify(); })))
                                .child(dunlin_ui::settings::segment("extensions-scope-project", format!("Project: {}", project.name), this.plugins.scope_project).max_w(px(260.))
                                    .on_click(cx.listener(|this, _, _, cx| { this.plugins.scope_project = true; cx.notify(); }))),
                        )
                        .child(div().flex_1())
                        .child(
                            dunlin_ui::settings::button("extensions-refresh")
                                .label("Refresh")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.plugins.loaded = false;
                                    this.plugins.load();
                                    cx.notify();
                                })),
                        )
                        .child(
                            dunlin_ui::settings::button("extensions-open-folder")
                                .label("Open extensions folder")
                                .on_click(cx.listener(|_, _, _, cx| {
                                    if let Some(directory) = State::directory()
                                        && std::fs::create_dir_all(&directory).is_ok()
                                    {
                                        let _ = std::process::Command::new("open")
                                            .arg(&directory)
                                            .spawn();
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        );
    if let Some(error) = &this.plugins.error {
        content = content.child(
            div()
                .p(px(10.))
                .rounded(px(dunlin_ui::radii::PANEL))
                .bg(p.red_soft)
                .text_color(p.red)
                .text_size(px(11.))
                .child(error.clone()),
        );
    }
    for warning in &this.plugins.warnings {
        content = content.child(
            div()
                .p(px(9.))
                .rounded(px(dunlin_ui::radii::PANEL))
                .bg(p.amber_soft)
                .text_color(p.amber)
                .text_size(px(10.5))
                .child(warning.clone()),
        );
    }
    let available = this.plugins.available.clone();
    if !available.is_empty() {
        content = content.child(
            div()
                .text_size(px(11.))
                .font_family(dunlin_ui::theme::code_font())
                .font_weight(FontWeight::MEDIUM)
                .text_color(p.muted_fg)
                .child("AVAILABLE · NOT INSTALLED"),
        );
        for (index, catalog) in available.iter().enumerate() {
            let id = catalog.id.clone();
            content = content.child(
                card(&p)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(7.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        div()
                                            .text_size(px(12.5))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(catalog.manifest.name.clone()),
                                    )
                                    .child(div().text_size(px(10.)).text_color(p.muted_fg).child(
                                        format!(
                                            "{} · v{} · bundled WASM · disabled after install",
                                            catalog.id, catalog.manifest.version
                                        ),
                                    )),
                            )
                            .child(
                                dunlin_ui::settings::button(format!("extension-install:{index}"))
                                    .label("Install pack")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        match this.plugins.install_bundled(&id) {
                                            Ok(message) => this.set_notice(message, cx),
                                            Err(error) => this
                                                .set_notice(format!("Install failed: {error}"), cx),
                                        }
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(10.5))
                            .text_color(p.muted_fg)
                            .child(catalog.manifest.description.clone()),
                    )
                    .child(
                        div()
                            .text_size(px(10.))
                            .text_color(p.muted_fg)
                            .child(format!(
                                "Permissions: {}",
                                permissions(&LoadedPack {
                                    manifest: catalog.manifest.clone(),
                                    source: Source::Bundled,
                                    directory: None,
                                    manifest_hash: catalog.manifest_hash.clone(),
                                })
                            )),
                    ),
            );
        }
    }
    content = content.child(
        card(&p)
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Build your own extension"),
            )
            .child(
                div()
                    .text_size(px(10.5))
                    .text_color(p.muted_fg)
                    .child("The plugin-builder skill contains the full manifest v2, sandboxed WASM ABI, knowledge schema, MCP proposal tools, security checklist, build and install steps. Start a guided chat and the agent will interview you one question at a time."),
            )
            .child(
                div().flex().justify_end().child(dunlin_ui::settings::button("plugins-build-with-skill")
                    .label("Use skills and build plugins")
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.start_plugin_builder_chat(window, cx);
                    }))),
            ),
    );
    for (index, pack) in this.plugins.packs.iter().enumerate() {
        let enabled = this.plugins.is_enabled_for(scope.as_deref(), pack);
        let id = pack.manifest.id.clone();
        let scope_id = scope.clone();
        let source = match pack.source {
            Source::Bundled => "bundled",
            Source::External => "external",
        };
        let color = if enabled { p.green } else { p.muted_fg };
        let details = this.settings_ui.extension_details.as_deref() == Some(&id);
        let details_id = id.clone();
        let mut item =
            card(&p)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(7.))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .child(
                                    div()
                                        .text_size(px(12.5))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(pack.manifest.name.clone()),
                                )
                                .child(div().text_size(px(10.)).text_color(p.muted_fg).child(
                                    format!(
                                        "{} · v{} · {} · API {}",
                                        pack.manifest.id,
                                        pack.manifest.version,
                                        source,
                                        pack.manifest.api
                                    ),
                                )),
                        )
                        .child(badge(if enabled { "enabled" } else { "off" }, color, &p))
                        .child(
                            dunlin_ui::switch(format!("extension-toggle:{index}"), format!("Enable {} for the selected scope", pack.manifest.name), enabled)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let pack = this
                                        .plugins
                                        .packs
                                        .iter()
                                        .find(|pack| pack.manifest.id == id)
                                        .cloned();
                                    if let Some(pack) = pack {
                                        let enabled =
                                            this.plugins.toggle_for(scope_id.as_deref(), &pack);
                                        this.persist_settings(cx);
                                        if this.section == dunlin_domain::SidebarSection::Knowledge
                                        {
                                            this.refresh_knowledge(cx);
                                        }
                                        if this.spec.is_active_for(this.project()) {
                                            this.refresh_spec_workspace(cx);
                                        }
                                        this.set_notice(
                                            format!(
                                                "{} {} · restart the harness to attach or detach its MCP server",
                                                id,
                                                if enabled { "enabled" } else { "disabled" }
                                            ),
                                            cx,
                                        );
                                    }
                                    cx.notify();
                                })),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(p.muted_fg)
                        .child(pack.manifest.description.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("Contributes: {}", pack.summary())),
                )
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!("Permissions: {}", permissions(pack))),
                )
                .child(div().flex().justify_end().child(dunlin_ui::settings::button(format!("extension-details:{index}")).label(if details { "Hide technical details" } else { "Technical details…" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.settings_ui.extension_details = if details { None } else { Some(details_id.clone()) };
                        cx.notify();
                    }))));
        if !details {
            content = content.child(item);
            continue;
        }
        if let Some(wasm) = &pack.manifest.wasm {
            let wasm_id = pack.manifest.id.clone();
            let status = this.plugins.wasm_status.get(&wasm_id).cloned();
            let mcp_command = pack.directory.as_ref().map(|directory| {
                format!(
                    "{:?} --project {:?} --plugin {:?}",
                    State::mcp_binary(),
                    project.path,
                    directory
                )
            });
            item = item
                .child(
                    div()
                        .text_size(px(10.))
                        .text_color(p.muted_fg)
                        .child(format!(
                            "WASM: {} · sha256 {}… · fuel {} · max {} MiB",
                            wasm.path,
                            &wasm.sha256[..wasm.sha256.len().min(12)],
                            wasm.fuel,
                            wasm.max_memory_bytes / 1024 / 1024
                        )),
                )
                .when_some(mcp_command, |item, command| {
                    item.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(
                                div()
                                    .text_size(px(10.))
                                    .text_color(p.muted_fg)
                                    .truncate()
                                    .child(command.clone()),
                            )
                            .child(
                                dunlin_ui::settings::button(format!("extension-copy-mcp:{index}"))
                                    .label("Copy MCP command")
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            command.clone(),
                                        ));
                                        cx.notify();
                                    })),
                            ),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(
                            dunlin_ui::settings::button(format!("extension-validate-wasm:{index}"))
                                .label("Validate WASM")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match this.plugins.validate_wasm_by_id(&wasm_id) {
                                        Ok(status) => this.set_notice(status, cx),
                                        Err(error) => this.set_notice(
                                            format!("WASM validation failed: {error}"),
                                            cx,
                                        ),
                                    }
                                    cx.notify();
                                })),
                        )
                        .when_some(status, |row, status| {
                            row.child(div().text_size(px(10.)).text_color(p.green).child(status))
                        }),
                );
        }
        if !pack.manifest.contributes.commands.is_empty() {
            item = item.child(
                div()
                    .text_size(px(10.))
                    .text_color(p.muted_fg)
                    .child(format!(
                        "Commands: {}",
                        pack.manifest
                            .contributes
                            .commands
                            .iter()
                            .map(|command| command.label.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
            );
        }
        if !pack.manifest.contributes.mcp_servers.is_empty() {
            item = item.child(
                div()
                    .text_size(px(10.))
                    .text_color(p.amber)
                    .child("MCP declarations are metadata only in this build; connector launch is not implemented."),
            );
        }
        content = content.child(item);
    }
    if this.plugins.packs.is_empty() && this.plugins.loaded {
        content = content.child(
            card(&p)
                .text_size(px(11.))
                .text_color(p.muted_fg)
                .child("No packs discovered."),
        );
    }
    content.into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{State, governance_mcp_config};
    use std::collections::HashMap;

    #[test]
    fn governance_mcp_config_is_project_scoped_and_trusted() {
        let manifest = dunlin_plugin::parse(
            r#"{"id":"dunlin-governance","name":"Governance","version":"1.0.0","api":2,"wasm":{"path":"plugin.wasm","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","exports":["dunlin_alloc","dunlin_free","dunlin_call"]}}"#,
        )
        .unwrap();
        let pack = dunlin_plugin::LoadedPack {
            manifest,
            source: dunlin_plugin::Source::External,
            directory: Some("/tmp/governance".into()),
            manifest_hash: "hash".into(),
        };
        let project = dunlin_domain::Project {
            id: "project".into(),
            name: "Project".into(),
            path: "/tmp/project".into(),
            branch: None,
            tone: dunlin_domain::ProjectTone::Blue,
            pinned: false,
            trusted: true,
            running_sessions: 0,
            kind: Default::default(),
            remote: None,
            group_id: None,
            settings: Default::default(),
        };
        let config = governance_mcp_config(&pack, &project, "/tmp/dunlin-knowledge-mcp").unwrap();
        assert_eq!(config.name, "dunlin-knowledge");
        assert_eq!(config.args[0], "--project");
        assert_eq!(config.args[1], "/tmp/project");
        let mut untrusted = project.clone();
        untrusted.trusted = false;
        assert!(governance_mcp_config(&pack, &untrusted, "/tmp/mcp").is_none());
    }

    #[test]
    fn external_wasm_packs_are_disabled_by_default() {
        let manifest = dunlin_plugin::parse(
            r#"{"id":"dunlin-governance","name":"Governance","version":"1.0.0","api":2,"wasm":{"path":"plugin.wasm","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","exports":["dunlin_alloc","dunlin_free","dunlin_call"]}}"#,
        )
        .unwrap();
        let pack = dunlin_plugin::LoadedPack {
            manifest,
            source: dunlin_plugin::Source::External,
            directory: Some("/tmp/governance".into()),
            manifest_hash: "hash".into(),
        };
        let mut state = State::new(Vec::new(), Vec::new(), HashMap::new(), HashMap::new());
        assert!(!state.is_enabled_for(Some("project-a"), &pack));
        assert!(state.toggle_for(Some("project-a"), &pack));
        assert!(state.is_enabled_for(Some("project-a"), &pack));
    }

    #[test]
    fn project_override_can_enable_or_disable_a_bundled_pack() {
        let (packs, _) = dunlin_plugin::discover(None);
        let spec_kit = packs
            .iter()
            .find(|pack| pack.manifest.id == "spec-kit")
            .unwrap();
        let mut state = State::new(Vec::new(), Vec::new(), HashMap::new(), HashMap::new());
        assert!(state.is_enabled_for(Some("project-a"), spec_kit));
        assert!(!state.toggle_for(Some("project-a"), spec_kit));
        assert!(!state.is_enabled_for(Some("project-a"), spec_kit));
        assert!(state.is_enabled_for(Some("project-b"), spec_kit));
        assert!(state.toggle_for(Some("project-a"), spec_kit));
        assert!(state.is_enabled_for(Some("project-a"), spec_kit));
    }
}
