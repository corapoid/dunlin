//! Inline project creation. Drafts are UI-only; Enter creates one app-owned
//! folder with an opaque ID, never interprets the display name as a path.
use crate::app::DunlinApp;
use dunlin_domain::{Project, ProjectTone};
use dunlin_ui::{
    component::input::{InputEvent, InputState},
    *,
};

#[derive(Default)]
pub(crate) struct State {
    pub input: Option<Entity<InputState>>,
    subscription: Option<Subscription>,
    pub editing: bool,
    pub recent: Option<String>,
    pub error: Option<String>,
}

fn name(value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err("Enter a project name");
    }
    if value.chars().count() > 100 || value.chars().any(char::is_control) {
        return Err("Use up to 100 characters without control characters");
    }
    Ok(value.into())
}

pub(crate) fn preview(value: &str) -> Project {
    Project {
        id: "project-draft".into(),
        name: value.trim().into(),
        path: String::new(),
        branch: None,
        tone: ProjectTone::Teal,
        pinned: false,
        trusted: true,
        running_sessions: 0,
        kind: Default::default(),
        remote: None,
        group_id: None,
        settings: Default::default(),
    }
}

impl DunlinApp {
    pub(crate) fn begin_inline_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.navigation_blocked() {
            return;
        }
        if !self.project_creation.editing {
            let input = cx.new(|cx| InputState::new(window, cx).placeholder("Project name"));
            let subscription = cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::PressEnter { .. } => this.confirm_inline_project(cx),
                    InputEvent::Change => {
                        this.project_creation.error = None;
                        cx.notify();
                    }
                    _ => {}
                },
            );
            self.project_creation.input = Some(input);
            self.project_creation.subscription = Some(subscription);
            self.project_creation.editing = true;
            self.project_creation.error = None;
        }
        if let Some(input) = &self.project_creation.input {
            input.read(cx).focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn confirm_inline_project(&mut self, cx: &mut Context<Self>) {
        if !self.project_creation.editing || !self.project_menu_open || self.navigation_blocked() {
            return;
        }
        let Some(input) = &self.project_creation.input else {
            return;
        };
        let result = (|| -> Result<Project, String> {
            let name = name(&input.read(cx).value()).map_err(str::to_string)?;
            let id = format!("project-{}", uuid::Uuid::now_v7());
            let path = crate::app::data_dir()
                .ok_or("Local data directory is unavailable")?
                .join("projects")
                .join(&id);
            std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
            let project = Project {
                id,
                name,
                path: path.to_string_lossy().into_owned(),
                ..preview("")
            };
            if let Some(store) = &self.store
                && let Err(error) = store.upsert_project(&project)
            {
                // Only remove the empty directory just created by this operation.
                let _ = std::fs::remove_dir(&path);
                return Err(error.to_string());
            }
            Ok(project)
        })();
        match result {
            Ok(project) => {
                self.project_creation.recent = Some(project.id.clone());
                self.projects.push(project);
                self.project_creation.editing = false;
                self.project_creation.input = None;
                self.project_creation.subscription = None;
                self.repo_summaries.invalidate();
                self.persist_settings(cx);
            }
            Err(error) => self.project_creation.error = Some(error),
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_names_are_trimmed_bounded_and_never_paths() {
        assert_eq!(name("  My project  ").unwrap(), "My project");
        assert!(name("  ").is_err());
        assert!(name("bad\nname").is_err());
        assert!(name(&"x".repeat(101)).is_err());
        assert_eq!(preview("../../display only").path, "");
        assert!(!preview("New").is_chat_space());
    }
}
