//! Native menu actions follow the focused window. Repository items are enabled
//! only when a project is available, and retain the ordinary safety guards.
use dunlin_ui::*;
impl crate::app::DunlinApp {
    pub(crate) fn shortcut_label(&self, label: &str, keys: &str) -> String {
        shortcut_label(self.show_shortcut_hints, label, keys)
    }
    pub(crate) fn dismiss_history_for_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.onboarding.setup.is_some() {
            self.set_notice(
                "Wait for adapter setup to finish, or use its recovery actions",
                cx,
            );
            return false;
        }
        // History is a document tab, not a blocking overlay. Menus may open over
        // it without destroying its state or releasing an in-flight Git lock.
        if self.settings_open && self.settings_have_draft() {
            self.set_notice(
                "Save or cancel the settings form before navigating elsewhere",
                cx,
            );
            return false;
        }
        true
    }
}

fn shortcut_label(visible: bool, label: &str, keys: &str) -> String {
    if visible {
        format!("{label} · {keys}")
    } else {
        label.into()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shortcut_hints_follow_the_preference_without_changing_labels() {
        assert_eq!(
            super::shortcut_label(true, "New session", "⌘N"),
            "New session · ⌘N"
        );
        assert_eq!(
            super::shortcut_label(false, "New session", "⌘N"),
            "New session"
        );
    }
}
actions!(
    dunlin_menu,
    [
        Settings,
        OpenFolders,
        NewSession,
        SaveAll,
        Home,
        Projects,
        Security,
        Back,
        Forward,
        ToggleSidebar,
        Commands,
        ProjectSearch,
        IncreaseText,
        DecreaseText,
        ResetText,
        GitHistory,
        SwitchBranch,
        RefreshGit,
        ProjectSettings
    ]
);

pub(crate) fn install(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-,", Settings, None),
        KeyBinding::new("cmd-o", OpenFolders, None),
        KeyBinding::new("cmd-n", NewSession, None),
        KeyBinding::new("cmd-shift-s", SaveAll, None),
        KeyBinding::new("cmd-b", ToggleSidebar, None),
        KeyBinding::new("cmd-shift-p", Commands, None),
        KeyBinding::new("cmd-alt-p", ProjectSearch, None),
        KeyBinding::new("cmd-=", IncreaseText, None),
        KeyBinding::new("cmd-+", IncreaseText, None),
        KeyBinding::new("cmd--", DecreaseText, None),
        KeyBinding::new("cmd-0", ResetText, None),
        KeyBinding::new("cmd-alt-g", GitHistory, None),
        KeyBinding::new("cmd-[", Back, None),
        KeyBinding::new("cmd-]", Forward, None),
    ]);
    cx.set_menus([
        Menu::new("Dunlin").items([
            MenuItem::action("Settings…", Settings),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Quit Dunlin", crate::app::Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("Add Projects…", OpenFolders),
            MenuItem::action("New Session", NewSession),
            MenuItem::separator(),
            MenuItem::action("Save All", SaveAll),
            MenuItem::action("Project Settings…", ProjectSettings),
        ]),
        Menu::new("View").items([
            MenuItem::action("Back", Back),
            MenuItem::action("Forward", Forward),
            MenuItem::separator(),
            MenuItem::action("Home", Home),
            MenuItem::action("Projects", Projects),
            MenuItem::action("Security", Security),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Command Palette…", Commands),
            MenuItem::action("Search Projects…", ProjectSearch),
            MenuItem::separator(),
            MenuItem::action("Larger Workspace Text", IncreaseText),
            MenuItem::action("Smaller Workspace Text", DecreaseText),
            MenuItem::action("Reset Workspace Text", ResetText),
        ]),
        Menu::new("Repository").items([
            MenuItem::action("Branch Graph & History…", GitHistory),
            MenuItem::action("Switch Branch…", SwitchBranch),
            MenuItem::separator(),
            MenuItem::action("Refresh Git Status", RefreshGit),
        ]),
    ]);
}
