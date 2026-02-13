use crate::model::PanelId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAction {
    ActivatePanel(PanelId),
    PanelHome(PanelId),
    PanelParent(PanelId),
    Copy,
    Move,
    Delete,
    Mkdir,
    ConnectSftp,
    OpenShell,
    OpenCommandLine,
    ToggleSort,
    Refresh,
    FindFdPlanned,
    ArchiveVfsPlanned,
    ViewerModesPlanned,
    EditorSettingsPlanned,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuItemSpec {
    pub label: &'static str,
    pub action: MenuAction,
}

#[derive(Debug, Clone, Copy)]
pub struct MenuGroupSpec {
    pub label: &'static str,
    pub hotkey: char,
    pub items: &'static [MenuItemSpec],
}

const LEFT_ITEMS: [MenuItemSpec; 3] = [
    MenuItemSpec {
        label: "Activate Left",
        action: MenuAction::ActivatePanel(PanelId::Left),
    },
    MenuItemSpec {
        label: "Home",
        action: MenuAction::PanelHome(PanelId::Left),
    },
    MenuItemSpec {
        label: "Parent",
        action: MenuAction::PanelParent(PanelId::Left),
    },
];

const FILES_ITEMS: [MenuItemSpec; 4] = [
    MenuItemSpec {
        label: "Copy",
        action: MenuAction::Copy,
    },
    MenuItemSpec {
        label: "Move",
        action: MenuAction::Move,
    },
    MenuItemSpec {
        label: "Delete",
        action: MenuAction::Delete,
    },
    MenuItemSpec {
        label: "Mkdir",
        action: MenuAction::Mkdir,
    },
];

const COMMAND_ITEMS: [MenuItemSpec; 5] = [
    MenuItemSpec {
        label: "Connect SFTP",
        action: MenuAction::ConnectSftp,
    },
    MenuItemSpec {
        label: "Find (fd)",
        action: MenuAction::FindFdPlanned,
    },
    MenuItemSpec {
        label: "Archive VFS",
        action: MenuAction::ArchiveVfsPlanned,
    },
    MenuItemSpec {
        label: "Command Line",
        action: MenuAction::OpenCommandLine,
    },
    MenuItemSpec {
        label: "Shell",
        action: MenuAction::OpenShell,
    },
];

const OPTIONS_ITEMS: [MenuItemSpec; 4] = [
    MenuItemSpec {
        label: "Sort",
        action: MenuAction::ToggleSort,
    },
    MenuItemSpec {
        label: "Refresh",
        action: MenuAction::Refresh,
    },
    MenuItemSpec {
        label: "Viewer Modes",
        action: MenuAction::ViewerModesPlanned,
    },
    MenuItemSpec {
        label: "Editor Settings",
        action: MenuAction::EditorSettingsPlanned,
    },
];

const RIGHT_ITEMS: [MenuItemSpec; 3] = [
    MenuItemSpec {
        label: "Activate Right",
        action: MenuAction::ActivatePanel(PanelId::Right),
    },
    MenuItemSpec {
        label: "Home",
        action: MenuAction::PanelHome(PanelId::Right),
    },
    MenuItemSpec {
        label: "Parent",
        action: MenuAction::PanelParent(PanelId::Right),
    },
];

const MENU_GROUPS: [MenuGroupSpec; 5] = [
    MenuGroupSpec {
        label: "Left",
        hotkey: 'l',
        items: &LEFT_ITEMS,
    },
    MenuGroupSpec {
        label: "Files",
        hotkey: 'f',
        items: &FILES_ITEMS,
    },
    MenuGroupSpec {
        label: "Command",
        hotkey: 'c',
        items: &COMMAND_ITEMS,
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
