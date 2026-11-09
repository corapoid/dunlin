//! Two tab groups over canonical buffers and session runtimes, not duplicate apps.
use crate::{
    app::{DunlinApp, TraceFilter},
    harness::ChatItem,
};
use dunlin_domain::MainView;
use dunlin_ui::{
    assets::IconName,
    component::Icon,
    component::input::{InputEvent, InputState, TextareaState},
    *,
};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum TabKey {
    Chat(String),
    File(String),
    Git(String),
    Spec(String),
    Terminal(String),
}

#[derive(Clone)]
pub(crate) struct TabDrag {
    pub key: TabKey,
    pub project: String,
}
pub(crate) struct TabPreview {
    pub label: String,
    pub icon: IconName,
}
impl Render for TabPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = palette(cx);
        let preview = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(12.))
            .py(px(7.))
            .max_w(px(260.))
            .rounded(px(dunlin_ui::radii::DRAG_PREVIEW))
            .bg(p.card)
            .shadow_md()
            .opacity(0.92)
            .text_size(px(12.))
            .text_color(p.fg)
            .child(Icon::new(self.icon).size(px(14.)))
            .child(div().min_w(px(0.)).truncate().child(self.label.clone()));
        #[cfg(debug_assertions)]
        let preview = preview
            .relative()
            .children(crate::workspace_ui_smoke::probe("dragged-tab-preview"));
        preview
    }
}

pub(crate) struct State {
    pub enabled: bool,
    pub focused: usize,
    pub active: [Option<TabKey>; 2],
    pub views: [MainView; 2],
    pub order: Vec<TabKey>,
    recent: Vec<TabKey>,
    pub restore: Option<TabKey>,
    restoring: bool,
    owners: HashMap<TabKey, usize>,
    project: String,
    last: Option<TabKey>,
    ui: [Option<ActivityUi>; 2],
    pub composers: HashMap<String, Entity<TextareaState>>,
    pub transcripts: HashMap<String, Vec<ChatItem>>,
    subscriptions: Vec<Subscription>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            enabled: false,
            focused: 0,
            active: [None, None],
            views: [MainView::Chat; 2],
            order: Vec::new(),
            recent: Vec::new(),
            restore: None,
            restoring: false,
            owners: HashMap::new(),
            project: String::new(),
            last: None,
            ui: [None, None],
            composers: HashMap::new(),
            transcripts: HashMap::new(),
            subscriptions: Vec::new(),
        }
    }
}
impl State {
    pub fn opened(&mut self, key: TabKey) {
        if !self.order.contains(&key) {
            self.order.push(key);
        }
    }
    pub fn closed(&mut self, key: &TabKey) {
        let current = self.recent.last() == Some(key);
        let pane = self.owner(key);
        self.order.retain(|tab| tab != key);
        self.recent.retain(|tab| tab != key);
        if self.restore.as_ref() == Some(key) {
            self.restore = None;
        }
        let previous = self
            .recent
            .iter()
            .rev()
            .chain(self.order.iter().rev())
            .find(|tab| self.belongs(tab, pane))
            .cloned();
        if self.active[pane].as_ref() == Some(key) {
            self.active[pane] = previous.clone();
        }
        if current {
            self.restore = previous;
        }
    }
    fn selected(&mut self, key: TabKey) {
        self.recent.retain(|tab| tab != &key);
        self.recent.push(key);
    }
    fn reorder(&mut self, key: &TabKey, position: Option<&TabKey>) {
        if position == Some(key) {
            return;
        }
        let index = position.and_then(|target| self.order.iter().position(|key| key == target));
        self.order.retain(|tab| tab != key);
        self.order.insert(
            index.unwrap_or(self.order.len()).min(self.order.len()),
            key.clone(),
        );
    }
    fn move_tab(&mut self, key: &TabKey, target: usize, keys: &[TabKey]) {
        let source = self.owner(key);
        self.owners.insert(key.clone(), target);
        if self.active[source].as_ref() == Some(key) {
            self.active[source] = keys.iter().find(|key| self.belongs(key, source)).cloned();
        }
        self.active[target] = Some(key.clone());
    }
    pub fn rekey_chat(&mut self, old: &str, new: &str) {
        if let Some(input) = self.composers.remove(old) {
            self.composers.insert(new.into(), input);
        }
        if let Some(transcript) = self.transcripts.remove(old) {
            self.transcripts.insert(new.into(), transcript);
        }
        let old = TabKey::Chat(old.into());
        let new = TabKey::Chat(new.into());
        for key in &mut self.order {
            if *key == old {
                *key = new.clone();
            }
        }
        for key in &mut self.recent {
            if *key == old {
                *key = new.clone();
            }
        }
        if self.restore.as_ref() == Some(&old) {
            self.restore = Some(new.clone());
        }
        if let Some(owner) = self.owners.remove(&old) {
            self.owners.insert(new.clone(), owner);
        }
        for selected in &mut self.active {
            if selected.as_ref() == Some(&old) {
                *selected = Some(new.clone());
            }
        }
        if self.last.as_ref() == Some(&old) {
            self.last = Some(new);
        }
    }
    pub fn owner(&self, tab: &TabKey) -> usize {
        if self.enabled {
            self.owners.get(tab).copied().unwrap_or(0)
        } else {
            0
        }
    }
    pub fn belongs(&self, tab: &TabKey, pane: usize) -> bool {
        self.owner(tab) == pane
    }
}
struct ActivityUi {
    activity: crate::activity::State,
    activity_search: Entity<InputState>,
    activity_query: String,
    selected_node: Option<String>,
    selected_span: Option<String>,
    trace_filter: TraceFilter,
    expanded_tool: Option<usize>,
    queue_open: bool,
    chat_visible_items: usize,
}
impl ActivityUi {
    fn exchange(self, app: &mut DunlinApp) -> Self {
        macro_rules! swap { ($($field:ident),+) => { Self { $($field: std::mem::replace(&mut app.$field, self.$field)),+ } }; }
        swap!(
            activity,
            activity_search,
            activity_query,
            selected_node,
            selected_span,
            trace_filter,
            expanded_tool,
            queue_open,
            chat_visible_items
        )
    }
}
impl DunlinApp {
    pub(crate) fn remember_workspace_tab(&mut self) {
        if self.workbench.panes.restoring {
            return;
        }
        self.workbench.panes.restore = None;
        if let Some(key) = self.current_workspace_tab() {
            self.workbench.panes.selected(key);
        }
    }
    pub(crate) fn current_workspace_tab(&self) -> Option<TabKey> {
        if self.spec.is_active_for(self.project()) {
            Some(TabKey::Spec(self.project().id.clone()))
        } else if self.git_history.is_active_for(self.project()) {
            Some(TabKey::Git(self.project().id.clone()))
        } else if self.workbench.show_file {
            self.workbench.active.clone().map(TabKey::File)
        } else if let Some(id) = self.workbench.terminal.as_ref().filter(|id| {
            self.terminal
                .tabs
                .iter()
                .any(|tab| &tab.id == *id && tab.project == self.project().id && tab.workspace)
        }) {
            Some(TabKey::Terminal(id.clone()))
        } else if !self.workbench.closed_chats.contains(&self.session().id) {
            Some(TabKey::Chat(self.session().id.clone()))
        } else {
            None
        }
    }
    pub(crate) fn workspace_tab_keys(&self) -> Vec<TabKey> {
        let mut keys: Vec<_> = self
            .workbench
            .chat_tabs
            .iter()
            .filter(|id| self.sessions.iter().any(|s| &s.id == *id))
            .cloned()
            .map(TabKey::Chat)
            .collect();
        keys.extend(
            self.workbench
                .tabs
                .iter()
                .filter(|t| t.project == self.project().id)
                .map(|t| TabKey::File(t.id.clone())),
        );
        if self.git_history.is_open_for(self.project()) {
            keys.push(TabKey::Git(self.project().id.clone()));
        }
        if self.spec_tools_enabled && self.spec.is_open_for(self.project()) {
            keys.push(TabKey::Spec(self.project().id.clone()));
        }
        keys.extend(
            self.terminal
                .tabs
                .iter()
                .filter(|tab| tab.workspace && tab.project == self.project().id)
                .map(|tab| TabKey::Terminal(tab.id.clone())),
        );
        keys.sort_by_key(|key| {
            self.workbench
                .panes
                .order
                .iter()
                .position(|tab| tab == key)
                .unwrap_or(usize::MAX)
        });
        keys
    }
    fn exchange_pane_ui(&mut self, pane: usize, window: &mut Window, cx: &mut Context<Self>) {
        let previous = self.workbench.panes.focused;
        if previous == pane {
            return;
        }
        let ui = self.workbench.panes.ui[pane].take().unwrap_or_else(|| {
            let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search activity"));
            let subscription = cx.subscribe(&search, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.activity_query = input.read(cx).value().to_string();
                    // Match the original pane's search subscription: filtering
                    // must not leave an invisible selected/active descendant.
                    this.activity_normalize_selection(cx);
                }
            });
            self.workbench.panes.subscriptions.push(subscription);
            ActivityUi {
                activity: Default::default(),
                activity_search: search,
                activity_query: String::new(),
                selected_node: None,
                selected_span: None,
                trace_filter: TraceFilter::All,
                expanded_tool: None,
                queue_open: false,
                chat_visible_items: crate::app::CHAT_WINDOW,
            }
        });
        let old = ui.exchange(self);
        self.workbench.panes.ui[previous] = Some(old);
        self.workbench.panes.focused = pane;
    }
    pub(crate) fn ensure_pane_composer(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TextareaState> {
        if let Some(input) = self.workbench.panes.composers.get(id) {
            return input.clone();
        }
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Ask…")
                .rows(2)
                .submit_on_enter(true)
        });
        if let Some(draft) = self.workbench.chat_drafts.get(id) {
            input.update(cx, |input, cx| input.set_value(draft.clone(), window, cx));
        }
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this, input, event: &InputEvent, window, cx| {
                if matches!(
                    event,
                    InputEvent::Focus | InputEvent::Blur | InputEvent::Change
                ) {
                    cx.notify();
                }
                if matches!(event, InputEvent::PressEnter { shift: false, .. }) {
                    let Some(session) = this
                        .workbench
                        .panes
                        .composers
                        .iter()
                        .find(|(_, composer)| *composer == input)
                        .map(|(id, _)| id.clone())
                    else {
                        return;
                    };
                    let key = TabKey::Chat(session.clone());
                    let pane = this.workbench.panes.owner(&key);
                    if this.focus_workspace_pane(pane, window, cx) && this.session().id == session {
                        this.send_prompt(window, cx);
                    }
                }
            },
        );
        self.workbench.panes.subscriptions.push(subscription);
        self.workbench
            .panes
            .composers
            .insert(id.into(), input.clone());
        input
    }
    pub(crate) fn sync_workspace_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let project = self.project().id.clone();
        if self.workbench.panes.project != project {
            self.workbench.panes.project = project;
            self.workbench.panes.owners.clear();
            self.workbench.panes.active = [None, None];
            self.workbench.panes.last = None;
            self.workbench.panes.recent.clear();
            self.workbench.panes.restore = None;
        }
        let keys = self.workspace_tab_keys();
        self.workbench.panes.recent.retain(|key| keys.contains(key));
        if let Some(key) = self
            .workbench
            .panes
            .restore
            .take()
            .filter(|key| keys.contains(key))
        {
            self.workbench.panes.restoring = true;
            self.activate_workspace_tab(&key, window, cx);
            self.workbench.panes.restoring = false;
        }
        self.remember_workspace_tab();
        let mut open = keys.clone();
        open.extend(
            self.workbench
                .tabs
                .iter()
                .map(|tab| TabKey::File(tab.id.clone())),
        );
        open.extend(
            self.terminal
                .tabs
                .iter()
                .filter(|tab| tab.workspace)
                .map(|tab| TabKey::Terminal(tab.id.clone())),
        );
        self.workbench.panes.order.retain(|key| open.contains(key));
        for key in &keys {
            self.workbench.panes.opened(key.clone());
        }
        self.workbench
            .panes
            .owners
            .retain(|key, _| keys.contains(key));
        let current = self.current_workspace_tab();
        if current != self.workbench.panes.last
            && !self.has_overlay()
            && matches!(current, Some(TabKey::Chat(_)))
        {
            self.composer
                .update(cx, |input, cx| input.focus(window, cx));
        }
        if current != self.workbench.panes.last || !self.workbench.panes.enabled {
            if let Some(key) = current.clone() {
                let pane = if self.workbench.panes.enabled {
                    self.workbench
                        .panes
                        .owners
                        .get(&key)
                        .copied()
                        .unwrap_or(self.workbench.panes.focused)
                } else {
                    0
                };
                self.exchange_pane_ui(pane, window, cx);
                self.workbench.panes.owners.insert(key.clone(), pane);
                self.workbench.panes.active[pane] = Some(key);
            }
            self.workbench.panes.last = current;
        }
        self.workbench.panes.views[self.workbench.panes.focused] = self.view;
        for pane in 0..2 {
            if self.workbench.panes.active[pane]
                .as_ref()
                .is_some_and(|key| !keys.contains(key) || !self.workbench.panes.belongs(key, pane))
            {
                self.workbench.panes.active[pane] = keys
                    .iter()
                    .find(|key| self.workbench.panes.belongs(key, pane))
                    .cloned();
            }
        }
        let session = self.session().id.clone();
        self.workbench
            .panes
            .composers
            .entry(session)
            .or_insert_with(|| self.composer.clone());
    }
    pub(crate) fn activate_workspace_tab(
        &mut self,
        key: &TabKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match key {
            TabKey::Terminal(id) => self.activate_workspace_terminal(id, window, cx),
            TabKey::File(id) => {
                self.activate_file_tab(id, window, cx);
                self.workbench.active.as_ref() == Some(id) && self.workbench.show_file
            }
            TabKey::Chat(id) => {
                self.activate_chat_tab(id, window, cx);
                if self.session().id != *id {
                    return false;
                }
                self.composer = self.ensure_pane_composer(id, window, cx);
                if let Some(draft) = self.workbench.pending_chat_draft.take() {
                    self.composer
                        .update(cx, |input, cx| input.set_value(draft, window, cx));
                }
                self.set_view(self.workbench.panes.views[self.workbench.panes.focused], cx);
                true
            }
            TabKey::Git(_) => {
                self.open_git_history(window, cx);
                true
            }
            TabKey::Spec(_) => {
                self.open_spec_workspace(window, cx);
                true
            }
        }
    }
    pub(crate) fn focus_workspace_pane(
        &mut self,
        pane: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if pane > 1 || (pane == 1 && !self.workbench.panes.enabled) {
            return false;
        }
        let previous = self.workbench.panes.focused;
        if previous == pane {
            return true;
        }
        let selected = self.workbench.panes.active[pane].clone();
        self.workbench.panes.views[previous] = self.view;
        self.exchange_pane_ui(pane, window, cx);
        if let Some(key) = selected {
            if !self.activate_workspace_tab(&key, window, cx) {
                self.exchange_pane_ui(previous, window, cx);
                return false;
            }
            self.workbench.panes.last = Some(key);
        }
        cx.notify();
        true
    }
    pub(crate) fn split_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_workspace_panes(window, cx);
        if self.workbench.panes.enabled {
            return;
        }
        self.workbench.panes.enabled = true;
        let current = self.current_workspace_tab();
        if let Some(other) = self
            .workspace_tab_keys()
            .into_iter()
            .find(|key| Some(key) != current.as_ref())
        {
            if let TabKey::Chat(id) = &other {
                self.ensure_pane_composer(id, window, cx);
            }
            self.workbench.panes.owners.insert(other.clone(), 1);
            self.workbench.panes.active[1] = Some(other);
        }
        cx.notify();
    }
    pub(crate) fn merge_workspace(&mut self, cx: &mut Context<Self>) {
        self.workbench.panes.enabled = false;
        self.workbench.panes.focused = 0;
        self.workbench.panes.owners.clear();
        self.workbench.panes.active = [self.current_workspace_tab(), None];
        self.workbench.panes.ui = [None, None];
        cx.notify();
    }
    pub(crate) fn move_workspace_tab(
        &mut self,
        pane: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench.panes.enabled {
            self.split_workspace(window, cx);
        }
        let Some(key) = self.workbench.panes.active[pane].clone() else {
            return;
        };
        self.drop_workspace_tab(
            &TabDrag {
                key,
                project: self.project().id.clone(),
            },
            1 - pane,
            window,
            cx,
        );
    }

    pub(crate) fn drop_workspace_tab(
        &mut self,
        drag: &TabDrag,
        target: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drop_workspace_tab_at(drag, target, None, window, cx);
    }

    pub(crate) fn drop_workspace_tab_at(
        &mut self,
        drag: &TabDrag,
        target: usize,
        position: Option<&TabKey>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if target > 1
            || (target == 1 && !self.workbench.panes.enabled)
            || drag.project != self.project().id
            || self.navigation_blocked()
        {
            return;
        }
        let keys = self.workspace_tab_keys();
        let source = self.workbench.panes.owner(&drag.key);
        if !keys.contains(&drag.key)
            || position.is_some_and(|key| {
                !keys.contains(key) || !self.workbench.panes.belongs(key, target)
            })
        {
            return;
        }
        if !self.focus_workspace_pane(source, window, cx)
            || !self.activate_workspace_tab(&drag.key, window, cx)
        {
            return;
        }
        if source != target {
            self.workbench.panes.move_tab(&drag.key, target, &keys);
        }
        self.workbench.panes.reorder(&drag.key, position);
        self.focus_workspace_pane(target, window, cx);
        cx.notify();
    }

    /// Paint another selection without navigation, connections, saves or buffer replacement.
    pub(crate) fn render_inactive_pane(
        &mut self,
        pane: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(key) = self.workbench.panes.active[pane].clone() else {
            return div().flex_1().into_any_element();
        };
        let focused = self.workbench.panes.focused;
        let selected_session = self.selected_session;
        let selected_view = self.view;
        let file = self.workbench.active.clone();
        let show_file = self.workbench.show_file;
        let terminal = self.workbench.terminal.clone();
        self.workbench.terminal = None;
        let inspector = self.inspector.clone();
        let editor = self.editor.clone();
        let dirty = self.editor_dirty;
        let readonly = self.editor_readonly;
        let mtime = self.editor_mtime;
        let conflict = self.editor_conflict.clone();
        let git = self.git_history.active;
        let spec = self.spec.active;
        let composer = self.composer.clone();
        self.exchange_pane_ui(pane, window, cx);
        let mut alternate = None;
        let mut runtime = None;
        let mut scroll = None;
        match &key {
            TabKey::Terminal(id) => {
                self.workbench.terminal = Some(id.clone());
                self.workbench.show_file = false;
                self.git_history.active = false;
                self.spec.active = false;
            }
            TabKey::File(id) => {
                if let Some(tab) = self.workbench.tabs.iter().find(|t| &t.id == id) {
                    self.workbench.active = Some(id.clone());
                    self.workbench.show_file = true;
                    self.inspector = Some(tab.inspector.clone());
                    self.editor = tab.editor.clone();
                    self.editor_dirty = tab.dirty;
                    self.editor_readonly = tab.readonly;
                    self.editor_mtime = tab.mtime;
                    self.editor_conflict = tab.conflict.clone();
                    self.git_history.active = false;
                    self.spec.active = false;
                }
            }
            TabKey::Chat(id) => {
                if let Some(index) = self.sessions.iter().position(|s| &s.id == id) {
                    self.selected_session = index;
                }
                if self.selected_session != selected_session {
                    runtime = self.parked_runtimes.remove(id);
                    let mut state = runtime
                        .as_mut()
                        .map(|r| std::mem::take(&mut r.state))
                        .unwrap_or_default();
                    if runtime.is_none() {
                        state.transcript = self
                            .workbench
                            .panes
                            .transcripts
                            .get(id)
                            .cloned()
                            .unwrap_or_default();
                    }
                    std::mem::swap(&mut self.harness, &mut state);
                    alternate = Some(state);
                    let other_scroll = self.workbench.chat_scrolls.remove(id).unwrap_or_default();
                    scroll = Some(std::mem::replace(&mut self.chat_scroll, other_scroll));
                }
                self.composer = self.ensure_pane_composer(id, window, cx);
                self.view = self.workbench.panes.views[pane];
                self.workbench.show_file = false;
                self.git_history.active = false;
                self.spec.active = false;
            }
            TabKey::Git(_) => {
                self.git_history.active = true;
                self.spec.active = false;
            }
            TabKey::Spec(_) => {
                self.spec.active = true;
                self.git_history.active = false;
            }
        }
        let content = self.workspace_content(window, cx);
        if let TabKey::Chat(id) = &key {
            if let Some(mut state) = alternate {
                std::mem::swap(&mut self.harness, &mut state);
                if let Some(mut parked) = runtime {
                    parked.state = state;
                    self.parked_runtimes.insert(id.clone(), parked);
                }
            }
            if let Some(old) = scroll {
                let other = std::mem::replace(&mut self.chat_scroll, old);
                self.workbench.chat_scrolls.insert(id.clone(), other);
            }
        }
        self.exchange_pane_ui(focused, window, cx);
        self.selected_session = selected_session;
        self.view = selected_view;
        self.workbench.active = file;
        self.workbench.show_file = show_file;
        self.workbench.terminal = terminal;
        self.inspector = inspector;
        self.editor = editor;
        self.editor_dirty = dirty;
        self.editor_readonly = readonly;
        self.editor_mtime = mtime;
        self.editor_conflict = conflict;
        self.git_history.active = git;
        self.spec.active = spec;
        self.composer = composer;
        content
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn close_restores_recent_selection_not_first_open_tab_and_stays_in_its_pane() {
        let a = TabKey::Chat("a".into());
        let b = TabKey::Git("b".into());
        let c = TabKey::Terminal("c".into());
        let d = TabKey::File("d".into());
        let mut state = State::default();
        for key in [&a, &b, &c, &d] {
            state.opened(key.clone());
        }
        for key in [&a, &c, &b, &d] {
            state.selected(key.clone());
        }
        state.closed(&d);
        assert_eq!(state.restore, Some(b.clone()));
        state.closed(&b);
        assert_eq!(state.restore, Some(c.clone()));
        state.restore = None;
        state.closed(&a);
        assert_eq!(
            state.restore, None,
            "closing an inactive tab leaves selection alone"
        );
        state.enabled = true;
        state.opened(d.clone());
        state.owners.insert(d.clone(), 1);
        state.selected(d.clone());
        state.closed(&d);
        assert_eq!(
            state.restore, None,
            "do not steal another pane's selected tab"
        );
    }
    #[test]
    fn drag_reorders_in_both_directions_and_can_append_without_duplication() {
        let a = TabKey::File("a".into());
        let b = TabKey::Chat("b".into());
        let c = TabKey::Git("c".into());
        let mut state = State {
            order: vec![a.clone(), b.clone(), c.clone()],
            ..State::default()
        };
        state.reorder(&a, Some(&c));
        assert_eq!(state.order, [b.clone(), c.clone(), a.clone()]);
        state.reorder(&a, Some(&b));
        assert_eq!(state.order, [a.clone(), b.clone(), c.clone()]);
        state.reorder(&a, Some(&a));
        assert_eq!(state.order, [a.clone(), b.clone(), c.clone()]);
        state.reorder(&b, None);
        assert_eq!(state.order, [a, c, b]);
    }
    #[test]
    fn opening_order_is_shared_by_all_tab_types_and_reopening_appends() {
        let file = TabKey::File("file".into());
        let git = TabKey::Git("project".into());
        let chat = TabKey::Chat("chat".into());
        let mut panes = State::default();
        for key in [&file, &git, &chat, &file] {
            panes.opened(key.clone());
        }
        assert_eq!(panes.order, [file.clone(), git.clone(), chat.clone()]);
        panes.closed(&file);
        panes.opened(file.clone());
        assert_eq!(panes.order, [git, chat, file]);
    }

    #[test]
    fn moving_a_tab_retains_order_and_other_pane_selections() {
        let file = TabKey::File("file".into());
        let chat = TabKey::Chat("chat".into());
        let mut panes = State {
            enabled: true,
            ..Default::default()
        };
        panes.opened(file.clone());
        panes.opened(chat.clone());
        panes.active[0] = Some(file.clone());
        let keys = panes.order.clone();
        panes.move_tab(&file, 1, &keys);
        assert_eq!(panes.active, [Some(chat.clone()), Some(file.clone())]);
        assert_eq!(panes.order, keys);
        panes.move_tab(&file, 0, &keys);
        assert_eq!(panes.active, [Some(file), None]);
        assert_eq!(panes.order, keys);
    }
    #[test]
    fn tab_groups_own_references_not_duplicate_documents_and_merge_without_deletion() {
        let key = TabKey::File("one".into());
        let mut panes = State::default();
        assert!(panes.belongs(&key, 0));
        panes.enabled = true;
        panes.owners.insert(key.clone(), 1);
        assert!(panes.belongs(&key, 1) && !panes.belongs(&key, 0));
        panes.enabled = false;
        assert!(panes.belongs(&key, 0));
    }
    #[test]
    fn session_rekey_keeps_its_pane_selection_and_ownership() {
        let mut panes = State {
            enabled: true,
            ..Default::default()
        };
        let old = TabKey::Chat("draft".into());
        let new = TabKey::Chat("persisted".into());
        panes.owners.insert(old.clone(), 1);
        panes.active[1] = Some(old.clone());
        panes.last = Some(old);
        panes.opened(TabKey::File("first".into()));
        panes.opened(TabKey::Chat("draft".into()));
        panes.rekey_chat("draft", "persisted");
        assert_eq!(panes.active[1], Some(new.clone()));
        assert_eq!(panes.last, Some(new.clone()));
        assert_eq!(panes.owner(&new), 1);
        assert!(!panes.owners.contains_key(&TabKey::Chat("draft".into())));
        assert_eq!(panes.order, [TabKey::File("first".into()), new]);
    }
}
