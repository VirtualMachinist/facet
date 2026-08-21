use std::collections::HashSet;

use probe_core::{FolderKey, RequestKey};

const MIN_SIDEBAR_WIDTH: f32 = 180.0;
const MAX_SIDEBAR_WIDTH: f32 = 520.0;
const MIN_RESPONSE_HEIGHT: f32 = 120.0;
const MAX_RESPONSE_HEIGHT: f32 = 560.0;
const MIN_RESPONSE_WIDTH: f32 = 240.0;
const MAX_RESPONSE_WIDTH: f32 = 760.0;
pub(crate) const DEFAULT_SIDEBAR_WIDTH: f32 = 260.0;
pub(crate) const DEFAULT_RESPONSE_HEIGHT: f32 = 220.0;
pub(crate) const DEFAULT_RESPONSE_WIDTH: f32 = 440.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum PaneLayout {
    #[default]
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ResizePane {
    Sidebar,
    Response,
}

#[derive(Debug)]
pub(crate) struct ShellState {
    tabs: Vec<RequestKey>,
    active_tab: Option<RequestKey>,
    collapsed_folders: HashSet<FolderKey>,
    selected_environment: Option<String>,
    pub(crate) sidebar_width: f32,
    pub(crate) sidebar_collapsed: bool,
    pub(crate) response_height: f32,
    pub(crate) response_width: f32,
    pub(crate) pane_layout: PaneLayout,
    pub(crate) resizing: Option<ResizePane>,
}

impl Default for ShellState {
    fn default() -> Self {
        Self {
            tabs: Vec::new(),
            active_tab: None,
            collapsed_folders: HashSet::new(),
            selected_environment: None,
            sidebar_width: DEFAULT_SIDEBAR_WIDTH,
            sidebar_collapsed: false,
            response_height: DEFAULT_RESPONSE_HEIGHT,
            response_width: DEFAULT_RESPONSE_WIDTH,
            pane_layout: PaneLayout::Vertical,
            resizing: None,
        }
    }
}

impl ShellState {
    pub(crate) fn tabs(&self) -> &[RequestKey] {
        &self.tabs
    }

    pub(crate) const fn active_tab(&self) -> Option<RequestKey> {
        self.active_tab
    }

    pub(crate) fn open_request(&mut self, key: RequestKey) {
        if !self.tabs.contains(&key) {
            self.tabs.push(key);
        }
        self.active_tab = Some(key);
    }

    pub(crate) fn close_tab(&mut self, key: RequestKey) {
        let Some(index) = self.tabs.iter().position(|tab| *tab == key) else {
            return;
        };
        self.tabs.remove(index);
        if self.active_tab == Some(key) {
            self.active_tab = self
                .tabs
                .get(index)
                .or_else(|| index.checked_sub(1).and_then(|index| self.tabs.get(index)))
                .copied();
        }
    }

    pub(crate) fn toggle_folder(&mut self, key: FolderKey) {
        if !self.collapsed_folders.remove(&key) {
            self.collapsed_folders.insert(key);
        }
    }

    pub(crate) fn folder_is_expanded(&self, key: FolderKey) -> bool {
        !self.collapsed_folders.contains(&key)
    }

    pub(crate) fn collapsed_folders(&self) -> impl Iterator<Item = FolderKey> + '_ {
        self.collapsed_folders.iter().copied()
    }

    pub(crate) fn collapse_folder(&mut self, key: FolderKey) {
        self.collapsed_folders.insert(key);
    }

    pub(crate) fn expand_folder(&mut self, key: FolderKey) {
        self.collapsed_folders.remove(&key);
    }

    pub(crate) fn selected_environment(&self) -> Option<&str> {
        self.selected_environment.as_deref()
    }

    pub(crate) fn select_environment(&mut self, environment: Option<String>) {
        self.selected_environment = environment;
    }

    pub(crate) fn resize_sidebar(&mut self, position: f32) {
        self.sidebar_width = position.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
    }

    pub(crate) fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
    }

    pub(crate) fn resize_response(&mut self, window_height: f32, position: f32) {
        self.response_height =
            (window_height - position).clamp(MIN_RESPONSE_HEIGHT, MAX_RESPONSE_HEIGHT);
    }

    pub(crate) fn resize_response_width(&mut self, window_width: f32, position: f32) {
        self.response_width =
            (window_width - position).clamp(MIN_RESPONSE_WIDTH, MAX_RESPONSE_WIDTH);
    }

    pub(crate) fn set_pane_layout(&mut self, layout: PaneLayout) {
        self.pane_layout = layout;
    }

    pub(crate) fn restore_pane_sizes(
        &mut self,
        sidebar_width: f32,
        response_height: f32,
        response_width: f32,
    ) {
        self.sidebar_width = sidebar_width.clamp(MIN_SIDEBAR_WIDTH, MAX_SIDEBAR_WIDTH);
        self.response_height = response_height.clamp(MIN_RESPONSE_HEIGHT, MAX_RESPONSE_HEIGHT);
        self.response_width = response_width.clamp(MIN_RESPONSE_WIDTH, MAX_RESPONSE_WIDTH);
    }

    pub(crate) fn reset_for_workspace(&mut self) {
        self.tabs.clear();
        self.active_tab = None;
        self.collapsed_folders.clear();
        self.resizing = None;
    }
}

#[cfg(test)]
mod tests {
    use probe_core::{
        Collection, CollectionItem, Folder, HttpRequest, Workspace, WorkspaceItemRef,
    };

    use super::{PaneLayout, ShellState};

    fn keys() -> (
        probe_core::RequestKey,
        probe_core::RequestKey,
        probe_core::FolderKey,
    ) {
        let workspace = Workspace::from_collection(Collection {
            items: vec![
                CollectionItem::HttpRequest(HttpRequest::default()),
                CollectionItem::HttpRequest(HttpRequest::default()),
                CollectionItem::Folder(Folder::default()),
            ],
            ..Collection::default()
        });
        let [
            WorkspaceItemRef::Request(first),
            WorkspaceItemRef::Request(second),
            WorkspaceItemRef::Folder(folder),
        ] = workspace.root_items()
        else {
            panic!("fixture must retain its item kinds");
        };
        (*first, *second, *folder)
    }

    #[test]
    fn opening_requests_deduplicates_tabs_and_selects_them() {
        let (first, second, _) = keys();
        let mut state = ShellState::default();
        state.open_request(first);
        state.open_request(second);
        state.open_request(first);

        assert_eq!(state.tabs(), &[first, second]);
        assert_eq!(state.active_tab(), Some(first));
    }

    #[test]
    fn closing_active_tab_selects_a_neighbor() {
        let (first, second, _) = keys();
        let mut state = ShellState::default();
        state.open_request(first);
        state.open_request(second);
        state.close_tab(second);

        assert_eq!(state.tabs(), &[first]);
        assert_eq!(state.active_tab(), Some(first));
    }

    #[test]
    fn folders_and_pane_sizes_are_constrained() {
        let (_, _, folder) = keys();
        let mut state = ShellState::default();
        assert!(state.folder_is_expanded(folder));
        state.toggle_folder(folder);
        assert!(!state.folder_is_expanded(folder));
        state.expand_folder(folder);
        assert!(state.folder_is_expanded(folder));
        state.collapse_folder(folder);
        assert!(!state.folder_is_expanded(folder));

        state.resize_sidebar(20.0);
        state.resize_response(800.0, 790.0);
        assert_eq!(state.sidebar_width, 180.0);
        assert_eq!(state.response_height, 120.0);

        state.resize_response_width(1000.0, 990.0);
        state.set_pane_layout(PaneLayout::Horizontal);
        assert_eq!(state.response_width, 240.0);
        assert_eq!(state.pane_layout, PaneLayout::Horizontal);

        state.toggle_sidebar();
        assert!(state.sidebar_collapsed);
        state.toggle_sidebar();
        assert!(!state.sidebar_collapsed);

        state.select_environment(Some("development".to_owned()));
        assert_eq!(state.selected_environment(), Some("development"));
        state.reset_for_workspace();
        assert_eq!(state.selected_environment(), Some("development"));
    }
}
