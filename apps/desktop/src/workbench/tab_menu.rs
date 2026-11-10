//! The same context menu and close protection for every workspace document.
use super::*;
use crate::workspace_panes::TabKey;

impl DunlinApp {
    pub(crate) fn workspace_tab_label(&self, key: &TabKey) -> String {
        match key {
            TabKey::File(id) => self
                .workbench
                .tabs
                .iter()
                .find(|tab| &tab.id == id)
                .map(|tab| tab.inspector.name.clone())
                .unwrap_or_else(|| "File".into()),
            TabKey::Chat(id) => self
                .sessions
                .iter()
                .find(|s| &s.id == id)
                .map(|s| s.title.clone())
                .unwrap_or_else(|| "Chat".into()),
            TabKey::Terminal(id) => self
                .terminal
                .tabs
                .iter()
                .find(|tab| &tab.id == id)
                .map(|tab| tab.title.clone())
                .unwrap_or_else(|| "Terminal".into()),
            TabKey::Git(_) => self.git_history.tab_label().into(),
            TabKey::Spec(_) => "Spec".into(),
            TabKey::Tool(_, tool) => tool.label().into(),
        }
    }

    pub(crate) fn workspace_tab_locked(&self, key: &TabKey) -> bool {
        if let TabKey::File(id) = key {
            self.workbench
                .tabs
                .iter()
                .any(|tab| &tab.id == id && tab.locked)
        } else {
            self.workbench.panes.locked.contains(key)
        }
    }

    pub(crate) fn toggle_workspace_tab_lock(&mut self, key: &TabKey, cx: &mut Context<Self>) {
        if let TabKey::File(id) = key {
            if let Some(tab) = self.workbench.tabs.iter_mut().find(|tab| &tab.id == id) {
                tab.locked = !tab.locked;
            }
        } else if !self.workbench.panes.locked.remove(key) {
            self.workbench.panes.locked.insert(key.clone());
        }
        cx.notify();
    }

    pub(crate) fn guard_workspace_tab_close(
        &mut self,
        key: &TabKey,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.workspace_tab_locked(key) {
            return true;
        }
        self.file_notice("Unlock the tab before closing it", cx);
        false
    }

    pub(crate) fn request_close_workspace_tab(
        &mut self,
        key: &TabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workspace_tab_keys().contains(key) || !self.guard_workspace_tab_close(key, cx) {
            return;
        }
        match key {
            TabKey::File(id) => self.request_close_tab(id, window, cx),
            TabKey::Chat(id) => self.close_session_tab(id, window, cx),
            TabKey::Terminal(id) => self.close_terminal_tab(id, cx),
            TabKey::Git(_) => self.close_git_history(cx),
            TabKey::Spec(_) => self.close_spec_workspace(cx),
            TabKey::Tool(project, tool) => self.close_workspace_tool(project, *tool, cx),
        }
    }
}
