//! Project-owned utility documents in the ordinary tab strip, never overlay windows.
use crate::{app::DunlinApp, workspace_panes::TabKey};
use dunlin_ui::{assets::IconName, *};
use std::collections::HashSet;

#[cfg(debug_assertions)]
pub(crate) mod smoke;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Tool {
    Knowledge,
    Artifacts,
    Security,
}

impl Tool {
    pub fn label(self) -> &'static str {
        match self {
            Self::Knowledge => "Knowledge",
            Self::Artifacts => "Artifacts",
            Self::Security => "Security Audit",
        }
    }
    pub fn icon(self) -> IconName {
        match self {
            Self::Knowledge => IconName::BookOpen,
            Self::Artifacts => IconName::FileText,
            Self::Security => IconName::ShieldCheck,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub opened: HashSet<(String, Tool)>,
    pub current: Option<(String, Tool)>,
    pub knowledge_mode: crate::knowledge::Mode,
}

impl DunlinApp {
    pub(crate) fn current_workspace_tool(&self) -> Option<Tool> {
        self.workspace_tools
            .current
            .as_ref()
            .filter(|(project, _)| project == &self.project().id)
            .map(|(_, tool)| *tool)
    }

    pub(crate) fn open_workspace_tool(
        &mut self,
        tool: Tool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.projects.is_empty() {
            self.file_notice("Open a project first", cx);
            return;
        }
        if !self.dismiss_history_for_menu(cx) {
            return;
        }
        if tool == Tool::Security && !self.security_workspace_enabled() {
            self.file_notice(
                "Install and enable a security audit skill in Settings → Skills first",
                cx,
            );
            return;
        }
        self.remember_workspace_tab();
        self.stash_editor_tab();
        self.workbench.show_file = false;
        self.workbench.terminal = None;
        self.git_history.active = false;
        self.spec.active = false;
        self.settings_open = false;
        self.section = self.sidebar_tab.section();
        let project = self.project().id.clone();
        self.workspace_tools.opened.insert((project.clone(), tool));
        self.workspace_tools.current = Some((project.clone(), tool));
        self.workbench.panes.opened(TabKey::Tool(project, tool));
        match tool {
            Tool::Knowledge => {
                self.knowledge.mode = self.workspace_tools.knowledge_mode;
                self.knowledge.show_proposals = false;
                self.refresh_knowledge_if_needed(cx);
            }
            Tool::Artifacts => {
                self.knowledge.mode = crate::knowledge::Mode::Artifacts;
                self.knowledge.show_proposals = false;
                self.refresh_knowledge_if_needed(cx);
            }
            Tool::Security => self.refresh_security_runs(cx),
        }
        self.remember_workspace_tab();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_workspace_tool(
        &mut self,
        project: &str,
        tool: Tool,
        cx: &mut Context<Self>,
    ) {
        if !self.guard_workspace_tab_close(&TabKey::Tool(project.into(), tool), cx) {
            return;
        }
        self.workspace_tools.opened.remove(&(project.into(), tool));
        if self.workspace_tools.current.as_ref() == Some(&(project.into(), tool)) {
            self.workspace_tools.current = None;
        }
        self.workbench
            .panes
            .closed(&TabKey::Tool(project.into(), tool));
        cx.notify();
    }

    pub(crate) fn render_workspace_tool(
        &mut self,
        tool: Tool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match tool {
            Tool::Knowledge | Tool::Artifacts => crate::knowledge::render(self, window, cx),
            Tool::Security => crate::security::workspace::render(self, window, cx),
        }
    }
}
