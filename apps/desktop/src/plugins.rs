//! Settings surface for declarative plugin packs. Packs contribute metadata,
//! commands and skill names; no plugin code is linked into Dunlin in v1.

use crate::app::DunlinApp;
use dunlin_plugin::{LoadedPack, Source};
use dunlin_ui::component::Sizable;
use dunlin_ui::component::button::ButtonVariants;
use dunlin_ui::*;
use std::collections::HashMap;
use std::path::PathBuf;

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
}

impl Default for State {
    fn default() -> Self {
        Self::new(Vec::new(), Vec::new(), HashMap::new(), HashMap::new())
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
        self.packs = packs;
        self.warnings = warnings;
        self.error = None;
    }
    fn directory() -> Option<PathBuf> {
        crate::app::data_dir().map(|data| data.join("plugins"))
    }
}

fn card(p: &Palette) -> Div {
    card_surface(*p).p(px(14.)).flex().flex_col().gap(px(7.))
}

fn badge(label: impl Into<SharedString>, color: Hsla, p: &Palette) -> Div {
    div()
        .h(px(19.))
        .px(px(7.))
        .rounded(px(4.))
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
    let mut content = div()
        .id("extensions-scroll")
        .flex_1()
        .min_h(px(0.))
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap(px(10.))
        .p(px(14.))
        .child(
            card(&p)
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Declarative packs"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(p.muted_fg)
                        .child("A pack can contribute methodology metadata, commands, skill names, view names and MCP server declarations. It cannot link GPUI, inject context silently or execute code. MCP connectors and sidecar processes are later phases."),
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
                        .gap(px(6.))
                        .child(
                            dunlin_ui::button("extensions-scope")
                                .ghost()
                                .small()
                                .label(if this.plugins.scope_project {
                                    format!("Scope: {}", project.name)
                                } else {
                                    "Scope: global".into()
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.plugins.scope_project = !this.plugins.scope_project;
                                    cx.notify();
                                })),
                        )
                        .child(
                            dunlin_ui::button("extensions-refresh")
                                .ghost()
                                .small()
                                .label("Refresh")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.plugins.loaded = false;
                                    this.plugins.load();
                                    cx.notify();
                                })),
                        )
                        .child(
                            dunlin_ui::button("extensions-open-folder")
                                .ghost()
                                .small()
                                .label("Open plugins folder")
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
                .rounded(px(8.))
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
                .rounded(px(8.))
                .bg(p.amber_soft)
                .text_color(p.amber)
                .text_size(px(10.5))
                .child(warning.clone()),
        );
    }
    for (index, pack) in this.plugins.packs.iter().enumerate() {
        let enabled = this.plugins.is_enabled_for(scope.as_deref(), pack);
        let id = pack.manifest.id.clone();
        let scope_id = scope.clone();
        let source = match pack.source {
            Source::Bundled => "bundled",
            Source::External => "external",
        };
        let color = if enabled { p.green } else { p.muted_fg };
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
                            dunlin_ui::button(format!("extension-toggle:{index}"))
                                .ghost()
                                .small()
                                .label(if enabled { "Disable" } else { "Enable" })
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
                                        this.set_notice(
                                            format!(
                                                "{} {}",
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
                );
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
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_h(px(0.))
        .child(content)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::State;
    use std::collections::HashMap;

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
