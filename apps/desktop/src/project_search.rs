//! Explorer-style name search. Typing selects a match; only Enter opens a project.
use crate::app::DunlinApp;
use dunlin_domain::{Project, ProjectGroup};
use dunlin_ui::*;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct State {
    pub query: String,
    pub matches: Vec<String>,
    pub index: usize,
    pub focus: Option<FocusHandle>,
    pub scroll: ScrollHandle,
    pub anchor: ScrollAnchor,
    collapsed_before: Option<Vec<String>>,
}

impl Default for State {
    fn default() -> Self {
        let scroll = ScrollHandle::new();
        Self {
            query: String::new(),
            matches: Vec::new(),
            index: 0,
            focus: None,
            anchor: ScrollAnchor::for_handle(scroll.clone()),
            scroll,
            collapsed_before: None,
        }
    }
}

impl State {
    pub fn current(&self) -> Option<&str> {
        self.matches.get(self.index).map(String::as_str)
    }
    pub fn saved_groups<'a>(&'a self, current: &'a [String]) -> &'a [String] {
        self.collapsed_before.as_deref().unwrap_or(current)
    }

    pub fn set_query(
        &mut self,
        query: String,
        projects: &[Project],
        groups: &[ProjectGroup],
        collapsed: &mut Vec<String>,
    ) {
        if query.is_empty() {
            if let Some(before) = self.collapsed_before.take() {
                *collapsed = before;
            }
            self.query.clear();
            self.matches.clear();
            self.index = 0;
            return;
        }
        let before = self
            .collapsed_before
            .get_or_insert_with(|| collapsed.clone());
        *collapsed = before.clone();
        self.query = query;
        let selected = self.current().map(str::to_owned);
        let mut matches: Vec<_> = projects
            .iter()
            .filter(|project| {
                !project.is_chat_space()
                    && !crate::explorer::search::match_ranges(&project.name, &self.query).is_empty()
            })
            .collect();
        matches.sort_by_key(|project| {
            (
                !project.pinned,
                project.name.to_lowercase(),
                project.id.clone(),
            )
        });
        for project in &matches {
            let mut next = project.group_id.as_deref();
            let mut seen = std::collections::HashSet::new();
            while let Some(id) = next {
                if !seen.insert(id) {
                    break;
                }
                collapsed.retain(|collapsed| collapsed != id);
                next = groups
                    .iter()
                    .find(|group| group.id == id)
                    .and_then(|group| group.parent_id.as_deref());
            }
        }
        self.matches = matches
            .into_iter()
            .map(|project| project.id.clone())
            .collect();
        self.index = selected
            .and_then(|id| self.matches.iter().position(|entry| *entry == id))
            .unwrap_or(0);
    }

    fn move_selection(&mut self, direction: isize) {
        if !self.matches.is_empty() {
            self.index =
                (self.index as isize + direction).rem_euclid(self.matches.len() as isize) as usize;
        }
    }
}

impl DunlinApp {
    pub(crate) fn focus_projects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.project_search
            .focus
            .get_or_insert_with(|| cx.focus_handle())
            .focus(window, cx);
    }

    pub(crate) fn clear_project_search(&mut self, cx: &mut Context<Self>) {
        self.project_search.set_query(
            String::new(),
            &self.projects,
            &self.project_groups,
            &mut self.collapsed_project_groups,
        );
        cx.notify();
    }

    pub(crate) fn project_search_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.sidebar_section() != dunlin_domain::SidebarSection::Projects
            || self.sidebar_collapsed
            || self.has_overlay()
            || !self
                .project_search
                .focus
                .as_ref()
                .is_some_and(|focus| focus.is_focused(window))
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control
        {
            return false;
        }
        let query = match event.keystroke.key.as_str() {
            "escape" => Some(String::new()),
            "backspace" if !self.project_search.query.is_empty() => {
                let end = self
                    .project_search
                    .query
                    .grapheme_indices(true)
                    .next_back()
                    .map_or(0, |(index, _)| index);
                Some(self.project_search.query[..end].to_string())
            }
            "up" | "down" if !self.project_search.query.is_empty() => {
                self.project_search
                    .move_selection(if event.keystroke.key == "up" { -1 } else { 1 });
                None
            }
            "enter" if !self.project_search.query.is_empty() => {
                if let Some(index) = self
                    .project_search
                    .current()
                    .and_then(|id| self.projects.iter().position(|project| project.id == id))
                {
                    self.select_project(index, cx);
                    self.focus_projects(window, cx);
                }
                None
            }
            _ => {
                let Some(text) = crate::explorer::search::typed_text(&event.keystroke) else {
                    return false;
                };
                Some(format!("{}{text}", self.project_search.query))
            }
        };
        if let Some(query) = query {
            self.project_search.set_query(
                query,
                &self.projects,
                &self.project_groups,
                &mut self.collapsed_project_groups,
            );
        }
        if self.project_search.current().is_some() {
            self.project_search.anchor.scroll_to(window, cx);
        }
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn name_search_is_case_insensitive_and_restores_collapsed_groups() {
        let projects: Vec<Project> = serde_json::from_value(serde_json::json!([
            {"id":"one","name":"Dunlin","path":"/one","group_id":"child","tone":"Blue","pinned":false,"trusted":false,"running_sessions":0},
            {"id":"two","name":"DUNLIN tools","path":"/two","tone":"Blue","pinned":false,"trusted":false,"running_sessions":0}
        ])).unwrap();
        let groups: Vec<ProjectGroup> = serde_json::from_value(serde_json::json!([
            {"id":"parent","name":"Parent","tone":"Blue"}, {"id":"child","name":"Child","parent_id":"parent","tone":"Blue"}
        ])).unwrap();
        let mut state = State::default();
        let mut collapsed = vec!["parent".into(), "child".into()];
        state.set_query("dunlin".into(), &projects, &groups, &mut collapsed);
        assert_eq!(state.matches.len(), 2);
        assert!(collapsed.is_empty());
        state.move_selection(-1);
        assert_eq!(state.current(), Some("two"));
        state.set_query("missing".into(), &projects, &groups, &mut collapsed);
        assert!(state.current().is_none());
        state.set_query(String::new(), &projects, &groups, &mut collapsed);
        assert_eq!(collapsed, ["parent", "child"]);
    }
}
