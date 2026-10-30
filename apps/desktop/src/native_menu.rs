//! Native menu actions follow the focused window. Repository items are enabled
//! only when a project is available, and retain the ordinary safety guards.
use dunlin_ui::*;
impl crate::app::DunlinApp {
    pub(crate) fn dismiss_history_for_menu(&mut self, cx: &mut Context<Self>) -> bool {
        if self.git_history.running {
            self.file_notice("Wait for branch creation to finish", cx);
            return false;
        }
        self.git_history.view = None;
        true
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
        ToggleSidebar,
        Commands,
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
        KeyBinding::new("cmd-alt-g", GitHistory, None),
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
            MenuItem::action("Open Project Folders…", OpenFolders),
            MenuItem::action("New Session", NewSession),
            MenuItem::separator(),
            MenuItem::action("Save All", SaveAll),
            MenuItem::action("Project Settings…", ProjectSettings),
        ]),
        Menu::new("View").items([
            MenuItem::action("Home", Home),
            MenuItem::action("Projects", Projects),
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::separator(),
            MenuItem::action("Command Palette…", Commands),
        ]),
        Menu::new("Repository").items([
            MenuItem::action("Branch Graph & History…", GitHistory),
            MenuItem::action("Switch Branch…", SwitchBranch),
            MenuItem::separator(),
            MenuItem::action("Refresh Git Status", RefreshGit),
        ]),
    ]);
}
