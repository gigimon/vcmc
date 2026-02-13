use crate::model::PanelId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    ActivatePanel(PanelId),
    PanelHome(PanelId),
    PanelParent(PanelId),
    PanelCopy(PanelId),
    PanelMove(PanelId),
    PanelDelete(PanelId),
    PanelMkdir(PanelId),
    PanelConnectSftp(PanelId),
    PanelOpenArchiveVfs(PanelId),
    PanelOpenShell(PanelId),
    PanelOpenCommandLine(PanelId),
    PanelFindFd(PanelId),
    ToggleSort,
    Refresh,
    ViewerModesInfo,
    EditorSettingsPlanned,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuItemSpec {
    pub label: &'static str,
    pub action: Option<MenuAction>,
}

impl MenuItemSpec {
    pub const fn action(label: &'static str, action: MenuAction) -> Self {
        Self {
            label,
            action: Some(action),
        }
    }

    pub const fn separator(label: &'static str) -> Self {
        Self {
            label,
            action: None,
        }
    }

    pub fn is_selectable(&self) -> bool {
        self.action.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MenuGroupSpec {
    pub label: &'static str,
    pub hotkey: char,
    pub items: &'static [MenuItemSpec],
}

const LEFT_ITEMS: [MenuItemSpec; 14] = [
    MenuItemSpec::action("Activate Left", MenuAction::ActivatePanel(PanelId::Left)),
    MenuItemSpec::action("Home", MenuAction::PanelHome(PanelId::Left)),
    MenuItemSpec::action("Parent", MenuAction::PanelParent(PanelId::Left)),
    MenuItemSpec::separator("──── Files ────"),
    MenuItemSpec::action("Copy", MenuAction::PanelCopy(PanelId::Left)),
    MenuItemSpec::action("Move", MenuAction::PanelMove(PanelId::Left)),
    MenuItemSpec::action("Delete", MenuAction::PanelDelete(PanelId::Left)),
    MenuItemSpec::action("Mkdir", MenuAction::PanelMkdir(PanelId::Left)),
    MenuItemSpec::separator("─── Command ───"),
    MenuItemSpec::action("Connect SFTP", MenuAction::PanelConnectSftp(PanelId::Left)),
    MenuItemSpec::action(
        "Command Line",
        MenuAction::PanelOpenCommandLine(PanelId::Left),
    ),
    MenuItemSpec::action("Shell", MenuAction::PanelOpenShell(PanelId::Left)),
    MenuItemSpec::action("Find (fd)", MenuAction::PanelFindFd(PanelId::Left)),
    MenuItemSpec::action(
        "Archive VFS",
        MenuAction::PanelOpenArchiveVfs(PanelId::Left),
    ),
];

const OPTIONS_ITEMS: [MenuItemSpec; 4] = [
    MenuItemSpec::action("Sort", MenuAction::ToggleSort),
    MenuItemSpec::action("Refresh", MenuAction::Refresh),
    MenuItemSpec::action("Viewer Modes", MenuAction::ViewerModesInfo),
    MenuItemSpec::action("Editor Settings", MenuAction::EditorSettingsPlanned),
];

const RIGHT_ITEMS: [MenuItemSpec; 14] = [
    MenuItemSpec::action("Activate Right", MenuAction::ActivatePanel(PanelId::Right)),
    MenuItemSpec::action("Home", MenuAction::PanelHome(PanelId::Right)),
    MenuItemSpec::action("Parent", MenuAction::PanelParent(PanelId::Right)),
    MenuItemSpec::separator("──── Files ────"),
    MenuItemSpec::action("Copy", MenuAction::PanelCopy(PanelId::Right)),
    MenuItemSpec::action("Move", MenuAction::PanelMove(PanelId::Right)),
    MenuItemSpec::action("Delete", MenuAction::PanelDelete(PanelId::Right)),
    MenuItemSpec::action("Mkdir", MenuAction::PanelMkdir(PanelId::Right)),
    MenuItemSpec::separator("─── Command ───"),
    MenuItemSpec::action("Connect SFTP", MenuAction::PanelConnectSftp(PanelId::Right)),
    MenuItemSpec::action(
        "Command Line",
        MenuAction::PanelOpenCommandLine(PanelId::Right),
    ),
    MenuItemSpec::action("Shell", MenuAction::PanelOpenShell(PanelId::Right)),
    MenuItemSpec::action("Find (fd)", MenuAction::PanelFindFd(PanelId::Right)),
    MenuItemSpec::action(
        "Archive VFS",
        MenuAction::PanelOpenArchiveVfs(PanelId::Right),
    ),
];

const MENU_GROUPS: [MenuGroupSpec; 3] = [
    MenuGroupSpec {
        label: "Left",
        hotkey: 'l',
        items: &LEFT_ITEMS,
    },
    MenuGroupSpec {
        label: "Options",
        hotkey: 'o',
        items: &OPTIONS_ITEMS,
    },
    MenuGroupSpec {
        label: "Right",
        hotkey: 'r',
        items: &RIGHT_ITEMS,
    },
];

pub fn top_menu_groups() -> &'static [MenuGroupSpec] {
    &MENU_GROUPS
}

pub fn menu_group_index_by_hotkey(input: char) -> Option<usize> {
    let needle = input.to_ascii_lowercase();
    MENU_GROUPS.iter().position(|group| group.hotkey == needle)
}
