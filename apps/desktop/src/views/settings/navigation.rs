use super::*;
use dunlin_ui::component::input::{InputEvent, InputState};

/// Transient presentation only; no preference keys or stored values are renamed.
#[derive(Default)]
pub(crate) struct State {
    pub search: Option<Entity<InputState>>,
    pub confirming_leave: bool,
    pub audits: bool,
    pub usage_report: bool,
    pub credential_details: Option<String>,
    pub backup_details: bool,
    pub available_agents: bool,
    pub project_focus: Option<FocusHandle>,
    pub recovery_notice: Option<String>,
}

pub(super) const GROUPS: &[(&str, &[SettingsTab])] = &[
    (
        "Application",
        &[
            SettingsTab::General,
            SettingsTab::Design,
            SettingsTab::Sidebar,
            SettingsTab::Chat,
            SettingsTab::Editor,
        ],
    ),
    (
        "Agents",
        &[
            SettingsTab::Harnesses,
            SettingsTab::Skills,
            SettingsTab::Extensions,
        ],
    ),
    (
        "Security & Privacy",
        &[
            SettingsTab::Permissions,
            SettingsTab::Secrets,
            SettingsTab::Security,
        ],
    ),
    (
        "System & Data",
        &[
            SettingsTab::Remote,
            SettingsTab::Usage,
            SettingsTab::Data,
            SettingsTab::Advanced,
        ],
    ),
];

impl SettingsTab {
    pub(crate) fn subtitle(self) -> &'static str {
        match self {
            Self::General => "Startup, session restoration and desktop notifications.",
            Self::Design => "Theme, interface typography, density and motion.",
            Self::Sidebar => "Dashboard, project list, Explorer and project discovery.",
            Self::Chat => "Response behavior and message appearance.",
            Self::Editor => "Code typography, editing and terminal integration.",
            Self::Harnesses => "Local ACP agents, session imports and connection defaults.",
            Self::Skills => "Install skills once globally and share them across harnesses.",
            Self::Extensions => "Install and enable extension packages on this Mac.",
            Self::Permissions => "Agent approval policy and separate macOS permissions.",
            Self::Secrets => "Keychain storage, scoped agent access and credential policies.",
            Self::Security => "Configure audit tools or inspect project audits and evidence.",
            Self::Remote => "Tailscale servers, connected users and server-managed task queues.",
            Self::Usage => "Subscription limits, local cost evidence and budget alerts.",
            Self::Data => "Portable export/import, local storage and recovery.",
            Self::Advanced => "Agent logs, environment diagnostics and application version.",
        }
    }

    fn keywords(self) -> &'static str {
        match self {
            Self::General => {
                "startup destination dashboard last project restore session history notification desktop macos allow"
            }
            Self::Design => {
                "design appearance theme skin default solid grain texture transparency opaque system light dark font interface density comfortable compact reduce motion animation shortcut hints keyboard accessibility gpu acceleration metal renderer graphics hardware"
            }
            Self::Sidebar => {
                "sidebar workspace dashboard live agents activity heatmap explorer file sizes project list branch language topics wrap information info preview sort import github gitlab gh glab cli tracked watched folder"
            }
            Self::Chat => {
                "chat stream responses timestamps message times duration logo reasoning thinking final answer only"
            }
            Self::Editor => {
                "editor terminal code font size typography word wrap autosave minimap inline blame shell integration zsh bash"
            }
            Self::Harnesses => {
                "harness adapter agents acp default opencode claude codex gemini copilot cursor qwen auggie kiro openclaw hermes qoder connect auto-connect restart crash import sessions portable free endpoints consent privacy training retention"
            }
            Self::Skills => {
                "skills instructions library catalog install uninstall disable enable edit global project github import folder bundle spec kit plugin builder"
            }
            Self::Extensions => {
                "extensions plugins packs wasm methodology governance proposals mcp scope spec intent tools"
            }
            Self::Permissions => {
                "permissions privacy approval ask read-only auto-approve sandbox home directory network redaction macos tcc accessibility automation screen recording full disk access notifications"
            }
            Self::Secrets => {
                "secrets credentials keys vault keychain broker token api policy grant expiry ttl host http operation cli recipe audit revoke"
            }
            Self::Security => {
                "security audits scanner findings artifacts sarif cloudflare codex sandbox check evidence"
            }
            Self::Remote => {
                "remote ssh relay server host pair login authorize tailscale clone copy tunnel bootstrap"
            }
            Self::Usage => {
                "usage limits subscription tokens billing costs budget alerts report logs evidence sync provider model project"
            }
            Self::Data => {
                "data backup storage directory sqlite cache recovery reset factory clear history export import json repository settings project structure download folder"
            }
            Self::Advanced => {
                "advanced diagnostics about version license logs acp zip export environment report checks troubleshooting qa"
            }
        }
    }

    pub(crate) fn matches_search(self, query: &str) -> bool {
        let names = match self {
            Self::General => "Startup destination Restore the last session Desktop notifications",
            Self::Design => {
                "UI font Interface density Show keyboard shortcut hints Reduce motion GPU acceleration Metal"
            }
            Self::Sidebar => {
                "Live agents Activity Show branch names Show primary language Show repository topics Wrap project names Project information Sort Show file sizes Project discovery Git hosting"
            }
            Self::Chat => {
                "Stream responses Message times Agent turn duration Response logo Show reasoning by default Final answer only"
            }
            Self::Editor => {
                "Code font Font size Word wrap Autosave File minimap Inline blame Shell integration"
            }
            Self::Harnesses => {
                "Local agents Install Default agent Set default Scan again Default for new conversations Auto-connect Connect automatically Restart on crash Import sessions Last import Refresh OpenCode Free privacy Allow Free endpoints consent training provider account settings"
            }
            Self::Skills => {
                "Available skills Installed skills Install Enable Disable Edit Install globally Add skill Scan harness folders Import from GitHub Import folder Project copies"
            }
            Self::Extensions => {
                "Intent Concepts Decisions Spec Extension packs Install pack Enable Disable"
            }
            Self::Permissions => {
                "Agent approval policy Default permission mode Ask every time Read-only Auto-approve Workspace writes Home directory Network access Secrets macOS permissions Notification permission"
            }
            Self::Secrets => {
                "Agent credential access Stored credentials Add credential Allowed API hosts Description for the agent Injection Header OAuth AWS Path prefix Time to live Require approval Projects Access policies & history"
            }
            Self::Security => {
                "Enable Security tools Audit skill and artifacts Explicit tools Configuration Project audits Tool availability Run sandboxed check"
            }
            Self::Remote => {
                "Tailscale Paired servers Connect to server Manage server Add project Remove remote host Server setup Linear tasks queue"
            }
            Self::Usage => {
                "Monthly alert budget Budget alerts Subscription limits Limits & alerts Cost report Local billing Scan logs Sync evidence Relay evidence"
            }
            Self::Data => {
                "Export and import Portable preferences Local storage Data directory Recovery Clear conversation history Reset Dunlin to factory defaults Download folder"
            }
            Self::Advanced => {
                "Agent logs View log Export ZIP Environment report Run diagnostics Copy report About"
            }
        };
        let haystack = format!(
            "{} {} {} {}",
            self.label(),
            self.subtitle(),
            self.keywords(),
            names
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|word| haystack.contains(&word.to_lowercase()))
    }
}

impl DunlinApp {
    pub(crate) fn settings_have_draft(&self) -> bool {
        self.vault.ui.draft.is_some()
            || self.vault.ui.recipe_draft.is_some()
            || self.vault.ui.operation_draft.is_some()
            || self.skills_ui.has_draft()
    }

    /// Every user-initiated tab change/close goes through the same discard gate.
    pub(crate) fn request_settings_location(
        &mut self,
        target: Option<SettingsTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_ui.confirming_leave
            || (self.settings_open && target == Some(self.settings_tab))
        {
            return;
        }
        if !self.settings_have_draft() {
            self.apply_settings_location(target, window, cx);
            return;
        }
        self.settings_ui.confirming_leave = true;
        let answer = window.prompt(PromptLevel::Warning, "Discard unsaved settings?",
            Some("This form has not been saved. Leaving discards its changes and clears any pasted credential. Saved settings are not affected."),
            &[PromptButton::cancel("Keep editing"), PromptButton::new("Discard changes")], cx);
        let answer = self.overlay_backdrop.track_native(answer, cx);
        cx.spawn_in(window, async move |this, cx| {
            let discard = answer.await == Ok(1);
            let _ = this.update_in(cx, |this, window, cx| {
                this.settings_ui.confirming_leave = false;
                if discard {
                    this.apply_settings_location(target, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_settings_location(
        &mut self,
        target: Option<SettingsTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.vault.ui.draft = None;
        self.vault.ui.recipe_draft = None;
        self.vault.ui.operation_draft = None;
        self.skills_ui.discard_draft();
        self.settings_drag = None;
        if let Some(tab) = target {
            self.settings_tab = tab;
            self.settings_open = true;
        } else {
            self.settings_open = false;
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    pub(super) fn ensure_settings_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_ui.search.is_none() {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find a setting…"));
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            })
            .detach();
            self.settings_ui.search = Some(search);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_covers_every_page_once() {
        let pages: Vec<_> = GROUPS
            .iter()
            .flat_map(|(_, tabs)| tabs.iter().copied())
            .collect();
        assert_eq!(pages.len(), 15);
        for tab in &pages {
            assert_eq!(pages.iter().filter(|other| *other == tab).count(), 1);
            assert!(!tab.subtitle().is_empty());
        }
        assert_eq!(pages[0], SettingsTab::General);
        assert_eq!(pages.last(), Some(&SettingsTab::Advanced));
    }

    #[test]
    fn search_finds_old_names_and_user_terms_without_case_sensitivity() {
        for (query, tab) in [
            ("harness", SettingsTab::Harnesses),
            ("secrets", SettingsTab::Secrets),
            ("backup export", SettingsTab::Data),
            ("SIDEBAR sort", SettingsTab::Sidebar),
            ("terminal shell", SettingsTab::Editor),
            ("skin grain", SettingsTab::Design),
            ("skin solid", SettingsTab::Design),
            ("skin default", SettingsTab::Design),
            ("transparency", SettingsTab::Design),
            ("logs zip", SettingsTab::Advanced),
        ] {
            assert!(tab.matches_search(query), "{query}");
        }
        assert!(SettingsTab::General.matches_search("  "));
        assert!(!SettingsTab::General.matches_search("backup"));
    }

    #[test]
    fn full_visible_setting_names_are_searchable() {
        for (query, tab) in [
            ("Show branch names", SettingsTab::Sidebar),
            ("Show file sizes", SettingsTab::Sidebar),
            ("Restore the last session", SettingsTab::General),
            ("Show reasoning by default", SettingsTab::Chat),
            ("Connect automatically", SettingsTab::Harnesses),
            ("Reset Dunlin to factory defaults", SettingsTab::Data),
            ("Agent credential access", SettingsTab::Secrets),
        ] {
            assert!(tab.matches_search(query), "{query}");
        }
    }
}
