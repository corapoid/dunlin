//! History of visible destinations, not edits or transient popovers.
use super::*;

const HISTORY_LIMIT: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Surface {
    Main(MainView),
    File(String),
    Git,
    Spec,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Location {
    Dashboard,
    Security,
    Settings(SettingsTab),
    Workspace {
        project: Option<String>,
        session: Option<String>,
        surface: Surface,
    },
}

#[derive(Default)]
pub(crate) struct History {
    entries: Vec<Location>,
    cursor: usize,
}

impl History {
    fn record(&mut self, location: Location) {
        if self.entries.get(self.cursor) == Some(&location) {
            return;
        }
        self.entries.truncate(self.cursor + 1);
        self.entries.push(location);
        if self.entries.len() > HISTORY_LIMIT {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    fn target(&self, forward: bool, valid: impl Fn(&Location) -> bool) -> Option<usize> {
        if forward {
            (self.cursor + 1..self.entries.len()).find(|&i| valid(&self.entries[i]))
        } else {
            (0..self.cursor).rev().find(|&i| valid(&self.entries[i]))
        }
    }
}

impl DunlinApp {
    #[cfg(debug_assertions)]
    pub(crate) fn smoke_navigation_layout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.navigation_location();
        let section = self.section;
        let collapsed = self.sidebar_collapsed;
        let width = self.sidebar_width;
        self.record_navigation();
        let length = self.navigation.entries.len();
        self.section = SidebarSection::Files;
        self.sidebar_collapsed = !collapsed;
        self.sidebar_width += 30.;
        window.resize(size(px(1300.), px(850.)));
        let resized = window.viewport_size();
        self.record_navigation();
        assert_eq!(self.navigation_location(), before);
        assert_eq!(self.navigation.entries.len(), length);
        self.set_view(MainView::Graph, cx);
        self.record_navigation();
        assert_eq!(self.navigation.entries.len(), length + 1);
        self.navigate_history(false, window, cx);
        assert_eq!(self.navigation_location(), before);
        assert_eq!(window.viewport_size(), resized);
        self.section = section;
        self.sidebar_collapsed = collapsed;
        self.sidebar_width = width;
        window.resize(size(px(1600.), px(950.)));
    }

    fn navigation_location(&self) -> Location {
        if self.settings_open {
            return Location::Settings(self.settings_tab);
        }
        if self.section == SidebarSection::Dashboard {
            return Location::Dashboard;
        }
        if self.section == SidebarSection::Security && self.security_enabled {
            return Location::Security;
        }
        let surface = if self.spec_tools_enabled && self.spec.is_active_for(self.project()) {
            Surface::Spec
        } else if self.git_history.is_active_for(self.project()) {
            Surface::Git
        } else if self.workbench.show_file
            && let Some(tab) = self.active_file_tab()
        {
            Surface::File(tab.id.clone())
        } else {
            Surface::Main(self.view)
        };
        Location::Workspace {
            project: self
                .projects
                .get(self.selected_project)
                .map(|p| p.id.clone()),
            session: self
                .sessions
                .get(self.selected_session)
                .filter(|_| matches!(surface, Surface::Main(_)))
                .map(|s| s.id.clone()),
            surface,
        }
    }

    pub(super) fn record_navigation(&mut self) {
        // Observe once per render so multi-step switches produce one destination.
        self.navigation.record(self.navigation_location());
    }

    fn navigation_target_exists(&self, location: &Location) -> bool {
        match location {
            Location::Security => self.security_enabled,
            Location::Workspace {
                project,
                session,
                surface,
                ..
            } => {
                project.as_ref().map_or(self.projects.is_empty(), |id| {
                    self.projects.iter().any(|p| &p.id == id)
                }) && session.as_ref().is_none_or(|id| {
                    self.all_sessions
                        .iter()
                        .any(|s| &s.id == id && Some(&s.project_id) == project.as_ref())
                }) && match surface {
                    Surface::File(id) => self
                        .workbench
                        .tabs
                        .iter()
                        .any(|t| &t.id == id && Some(&t.project) == project.as_ref()),
                    Surface::Spec => self.spec_tools_enabled,
                    _ => true,
                }
            }
            _ => true,
        }
    }

    pub(crate) fn can_navigate_history(&self, forward: bool) -> bool {
        !self.navigation_blocked()
            && !self.branch_ui.is_open()
            && !self.command_open
            && self
                .navigation
                .target(forward, |l| self.navigation_target_exists(l))
                .is_some()
    }

    pub(crate) fn navigate_history(
        &mut self,
        forward: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.navigation_blocked() || self.branch_ui.is_open() || self.command_open {
            return;
        }
        self.record_navigation();
        let Some(index) = self
            .navigation
            .target(forward, |l| self.navigation_target_exists(l))
        else {
            return;
        };
        let location = self.navigation.entries[index].clone();
        match &location {
            Location::Dashboard => {
                self.settings_open = false;
                self.section = SidebarSection::Dashboard;
            }
            Location::Security => {
                self.settings_open = false;
                self.section = SidebarSection::Security;
            }
            Location::Settings(tab) => {
                self.settings_open = true;
                self.settings_tab = *tab;
            }
            Location::Workspace {
                project,
                session,
                surface,
            } => {
                if let Some(project) = project
                    && self.project().id != *project
                {
                    // Keep the cursor in place rather than discard or implicitly save edits.
                    if self.editor_dirty {
                        self.file_notice(
                            "Save or close the current file before switching projects",
                            cx,
                        );
                        return;
                    }
                    let Some(index) = self.projects.iter().position(|p| &p.id == project) else {
                        return;
                    };
                    self.select_project_internal(index, true, cx);
                    if self.project().id != *project {
                        return;
                    }
                }
                if let Some(session) = session {
                    let Some(index) = self.sessions.iter().position(|s| &s.id == session) else {
                        return;
                    };
                    if self.remote_sessions_loading {
                        // The existing remote refresh will connect the requested session.
                        self.restore_session_id = Some(session.clone());
                        self.selected_session = index;
                        self.load_transcript(session);
                    } else {
                        self.select_session(index, cx);
                    }
                    if self.session().id != *session {
                        return;
                    }
                }
                self.settings_open = false;
                if matches!(self.section, SidebarSection::Dashboard | SidebarSection::Security) {
                    self.section = SidebarSection::Projects;
                }
                match surface {
                    Surface::Main(view) => self.set_view(*view, cx),
                    Surface::File(id) => self.activate_file_tab(id, window, cx),
                    Surface::Git => {
                        self.show_git_history(window, cx);
                    }
                    Surface::Spec => self.open_spec_workspace(window, cx),
                }
            }
        }
        if self.navigation_location() == location {
            self.navigation.cursor = index;
        }
        self.chat_menu_open = false;
        self.context_menu = None;
        self.project_menu_open = false;
        self.remote_picker = None;
        if !matches!(
            location,
            Location::Workspace {
                surface: Surface::File(_),
                ..
            }
        ) {
            self.focus_handle.focus(window, cx);
        }
        self.persist_settings(cx);
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn back_and_forward_do_not_record_duplicates() {
        let mut h = History::default();
        h.record(Location::Dashboard);
        h.record(Location::Security);
        h.record(Location::Settings(SettingsTab::General));
        h.cursor = h.target(false, |_| true).unwrap();
        h.record(Location::Security);
        assert_eq!(h.entries.len(), 3);
        assert_eq!(h.target(false, |_| true), Some(0));
        assert_eq!(h.target(true, |_| true), Some(2));
        h.cursor = 2;
        assert_eq!(h.target(true, |_| true), None);
        h.cursor = 0;
        assert_eq!(h.target(false, |_| true), None);
    }

    #[test]
    fn new_destination_after_back_discards_forward_branch() {
        let mut h = History::default();
        h.record(Location::Dashboard);
        h.record(Location::Security);
        h.cursor = 0;
        h.record(Location::Settings(SettingsTab::Chat));
        assert_eq!(
            h.entries,
            vec![Location::Dashboard, Location::Settings(SettingsTab::Chat)]
        );
        assert_eq!(h.target(true, |_| true), None);
    }

    #[test]
    fn unavailable_destinations_are_skipped_in_both_directions() {
        let mut h = History::default();
        h.record(Location::Dashboard);
        h.record(Location::Security);
        h.record(Location::Settings(SettingsTab::General));
        let valid = |l: &Location| *l != Location::Security;
        assert_eq!(h.target(false, valid), Some(0));
        h.cursor = 0;
        assert_eq!(h.target(true, valid), Some(2));
    }

    #[test]
    fn history_is_bounded() {
        let mut h = History::default();
        for i in 0..HISTORY_LIMIT + 20 {
            h.record(Location::Workspace {
                project: Some(i.to_string()),
                session: None,
                surface: Surface::Main(MainView::Chat),
            });
        }
        assert_eq!(h.entries.len(), HISTORY_LIMIT);
        assert_eq!(h.cursor, HISTORY_LIMIT - 1);
        assert!(
            matches!(&h.entries[0], Location::Workspace { project: Some(id), .. } if id == "20")
        );
    }

    #[test]
    fn git_history_is_a_single_navigation_destination() {
        let mut h = History::default();
        for _ in 0..3 {
            h.record(Location::Workspace {
                project: Some("repo".into()),
                session: None,
                surface: Surface::Git,
            });
        }
        assert_eq!(h.entries.len(), 1);
        assert_eq!(h.target(false, |_| true), None);
    }
}
