use super::*;

/// Ephemeral menus, popovers, and their focus restoration targets.
///
/// Keeping these together makes dismissal and focus ownership explicit while the
/// application shell continues to own the longer-lived workspace and editor state.
pub(super) struct TransientSurfaces {
    pub(super) desktop_menu_open: Option<DesktopMenu>,
    pub(super) desktop_submenu_open: Option<DesktopSubmenu>,
    pub(super) workspace_switcher_open: bool,
    pub(super) workspace_import_submenu_open: bool,
    pub(super) sidebar_import_menu_open: bool,
    pub(super) workspace_import_trigger_focus: FocusHandle,
    pub(super) workspace_import_popup_focus: FocusHandle,
    pub(super) sidebar_import_trigger_focus: FocusHandle,
    pub(super) sidebar_import_popup_focus: FocusHandle,
    pub(super) structure_add_menu_open: bool,
    pub(super) tree_context_menu: Option<PositionedContextMenu<WorkspaceItemRef>>,
    pub(super) tab_context_menu: Option<PositionedContextMenu<RequestKey>>,
    pub(super) environment_manager_context_menu: Option<PositionedContextMenu<String>>,
    pub(super) request_tab_tooltip: Option<RequestTabTooltip>,
    pub(super) request_tab_tooltip_epoch: usize,
    pub(super) request_tab_tooltip_task: Option<Task<()>>,
}

impl TransientSurfaces {
    pub(super) fn new(cx: &mut Context<ProbeApp>) -> Self {
        Self {
            desktop_menu_open: None,
            desktop_submenu_open: None,
            workspace_switcher_open: false,
            workspace_import_submenu_open: false,
            sidebar_import_menu_open: false,
            workspace_import_trigger_focus: cx.focus_handle(),
            workspace_import_popup_focus: cx.focus_handle(),
            sidebar_import_trigger_focus: cx.focus_handle(),
            sidebar_import_popup_focus: cx.focus_handle(),
            structure_add_menu_open: false,
            tree_context_menu: None,
            tab_context_menu: None,
            environment_manager_context_menu: None,
            request_tab_tooltip: None,
            request_tab_tooltip_epoch: 0,
            request_tab_tooltip_task: None,
        }
    }
}
