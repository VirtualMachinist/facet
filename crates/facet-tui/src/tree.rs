//! Sidebar tree with collapse + search state.
//!
//! `TreeView` holds the session-only collapse set, the search query, and
//! produces the visible rows in source order. Workspace keys remain
//! session-only per the architecture contract.

use std::collections::BTreeSet;

use probe_core::{FolderKey, RequestKey, Workspace, WorkspaceFolder, WorkspaceItemRef};

/// One row in the visible tree.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Row {
    Folder {
        key: FolderKey,
        depth: u8,
        /// Recursive request count, used for the `▸ Folder (3)` badge.
        request_count: u32,
    },
    Request {
        key: RequestKey,
        depth: u8,
    },
}

impl Row {
    pub fn depth(self) -> u8 {
        match self {
            Row::Folder { depth, .. } | Row::Request { depth, .. } => depth,
        }
    }
}

/// Sidebar state. Visible rows rebuild whenever the workspace, collapse
/// set, or search query changes.
#[derive(Default)]
pub struct TreeView {
    workspace: Option<Workspace>,
    collapsed: BTreeSet<FolderKey>,
    search: String,
    visible: Vec<Row>,
    selection: usize,
}

impl TreeView {
    /// Resets state to a fresh workspace, fully expanded.
    pub fn reset(&mut self, workspace: Workspace) {
        self.workspace = Some(workspace);
        self.collapsed.clear();
        self.search.clear();
        self.rebuild();
        self.selection = self.first_request_index().unwrap_or(0);
    }

    /// Updates the workspace in place (after persistence writes).
    pub fn set_workspace(&mut self, workspace: Workspace) {
        self.workspace = Some(workspace);
        self.rebuild();
    }

    /// Returns the workspace if one is loaded.
    #[cfg(test)]
    pub fn workspace(&self) -> Option<&Workspace> {
        self.workspace.as_ref()
    }

    /// True if the search query is non-empty.
    pub fn has_search(&self) -> bool {
        !self.search.is_empty()
    }

    /// Current search query.
    pub fn search(&self) -> &str {
        &self.search
    }

    /// Replaces the search query, rebuilding visible rows.
    #[cfg(test)]
    pub fn set_search(&mut self, query: impl Into<String>) {
        self.search = query.into();
        self.rebuild();
    }

    /// Pushes a character to the search query (no-op when no workspace).
    pub fn push_search(&mut self, ch: char) {
        if self.workspace.is_some() {
            self.search.push(ch);
            self.rebuild();
        }
    }

    /// Pops the last search character.
    pub fn pop_search(&mut self) {
        if self.search.pop().is_some() {
            self.rebuild();
        }
    }

    /// Clears the search query and restores the unfiltered tree.
    pub fn clear_search(&mut self) {
        if !self.search.is_empty() {
            let current = self.selected_request_key();
            self.search.clear();
            self.rebuild();
            if let Some(key) = current
                && let Some(index) = self.index_of_request(key)
            {
                self.selection = index;
            }
        }
    }

    /// Toggles the collapsed state of `key` and keeps that folder selected.
    pub fn toggle_collapsed(&mut self, key: FolderKey) {
        if !self.collapsed.remove(&key) {
            self.collapsed.insert(key);
        }
        self.rebuild();
        if let Some(index) = self.visible.iter().position(|row| match row {
            Row::Folder { key: candidate, .. } => *candidate == key,
            Row::Request { .. } => false,
        }) {
            self.selection = index;
        }
    }

    /// True when `key` is collapsed.
    pub fn is_collapsed(&self, key: FolderKey) -> bool {
        self.collapsed.contains(&key)
    }

    /// Currently visible rows in source order.
    pub fn visible_rows(&self) -> &[Row] {
        &self.visible
    }

    /// Current selection index into `visible_rows`.
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Currently selected row, if any.
    #[cfg(test)]
    pub fn selected_row(&self) -> Option<Row> {
        self.visible.get(self.selection).copied()
    }

    /// Moves the selection by `delta`, clamped to the visible range.
    pub fn move_selection(&mut self, delta: isize) {
        if self.visible.is_empty() {
            return;
        }
        let len = self.visible.len() as isize;
        let current = self.selection as isize;
        let next = (current + delta).clamp(0, len - 1);
        self.selection = next as usize;
    }

    /// `gg` — first visible row.
    pub fn select_first(&mut self) {
        if !self.visible.is_empty() {
            self.selection = 0;
        }
    }

    /// `G` — last visible row.
    pub fn select_last(&mut self) {
        if !self.visible.is_empty() {
            self.selection = self.visible.len() - 1;
        }
    }

    /// Returns the request at the current selection, if any.
    pub fn selected_request_key(&self) -> Option<RequestKey> {
        self.visible.get(self.selection).and_then(|row| match row {
            Row::Request { key, .. } => Some(*key),
            Row::Folder { .. } => None,
        })
    }

    /// Returns the folder at the current selection, if any.
    pub fn selected_folder_key(&self) -> Option<FolderKey> {
        self.visible.get(self.selection).and_then(|row| match row {
            Row::Folder { key, .. } => Some(*key),
            Row::Request { .. } => None,
        })
    }

    fn first_request_index(&self) -> Option<usize> {
        self.visible
            .iter()
            .position(|row| matches!(row, Row::Request { .. }))
    }

    fn index_of_request(&self, key: RequestKey) -> Option<usize> {
        self.visible.iter().position(|row| match row {
            Row::Request { key: candidate, .. } => *candidate == key,
            Row::Folder { .. } => false,
        })
    }

    fn rebuild(&mut self) {
        let Some(workspace) = self.workspace.as_ref() else {
            self.visible.clear();
            self.selection = 0;
            return;
        };
        let query = self.search.trim().to_ascii_lowercase();
        let filter_active = !query.is_empty();
        let mut out: Vec<Row> = Vec::new();
        for item in workspace.root_items() {
            walk(
                *item,
                0,
                workspace,
                &self.collapsed,
                &query,
                filter_active,
                &mut out,
            );
        }
        self.visible = out;
        if self.visible.is_empty() {
            self.selection = 0;
        } else if self.selection >= self.visible.len() {
            self.selection = self.visible.len() - 1;
        }
    }
}

fn walk(
    item: WorkspaceItemRef,
    depth: u8,
    workspace: &Workspace,
    collapsed: &BTreeSet<FolderKey>,
    query: &str,
    filter_active: bool,
    out: &mut Vec<Row>,
) {
    match item {
        WorkspaceItemRef::Folder(key) => {
            let Some(folder) = workspace.folder(key) else {
                return;
            };
            let request_count = count_requests(folder, workspace);
            let name_match = !filter_active || folder_name_matches(folder, query);
            let start = out.len();
            // Search auto-expands so matches under a collapsed folder still appear.
            let descend = filter_active || !collapsed.contains(&key);
            if descend {
                for child in &folder.children {
                    walk(
                        *child,
                        depth.saturating_add(1),
                        workspace,
                        collapsed,
                        query,
                        filter_active,
                        out,
                    );
                }
            }
            let child_emitted = out.len() > start;
            if !filter_active || name_match || child_emitted {
                out.insert(
                    start,
                    Row::Folder {
                        key,
                        depth,
                        request_count,
                    },
                );
            }
        }
        WorkspaceItemRef::Request(key) => {
            let Some(request) = workspace.request(key) else {
                return;
            };
            let name = request
                .metadata
                .name
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase();
            let url = request.url.as_deref().unwrap_or("").to_ascii_lowercase();
            if !filter_active || name.contains(query) || url.contains(query) {
                out.push(Row::Request { key, depth });
            }
        }
    }
}

fn folder_name_matches(folder: &WorkspaceFolder, query: &str) -> bool {
    folder
        .metadata
        .name
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase()
        .contains(query)
}

fn count_requests(folder: &WorkspaceFolder, workspace: &Workspace) -> u32 {
    let mut total = 0u32;
    let mut stack: Vec<&WorkspaceFolder> = vec![folder];
    while let Some(active) = stack.pop() {
        for child in &active.children {
            match *child {
                WorkspaceItemRef::Folder(key) => {
                    if let Some(nested) = workspace.folder(key) {
                        stack.push(nested);
                    }
                }
                WorkspaceItemRef::Request(_) => total += 1,
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use probe_core::{Collection, CollectionItem, Folder, HttpRequest, ItemMetadata};

    fn collection(items: Vec<CollectionItem>) -> Collection {
        Collection {
            metadata: Default::default(),
            items,
            environments: Vec::new(),
        }
    }

    fn folder(name: &str, children: Vec<CollectionItem>) -> CollectionItem {
        CollectionItem::Folder(Folder {
            metadata: ItemMetadata {
                name: Some(name.to_string()),
                ..Default::default()
            },
            items: children,
        })
    }

    fn request(name: &str) -> CollectionItem {
        CollectionItem::HttpRequest(HttpRequest {
            metadata: ItemMetadata {
                name: Some(name.to_string()),
                ..Default::default()
            },
            ..Default::default()
        })
    }

    fn request_with_url(name: &str, url: &str) -> CollectionItem {
        CollectionItem::HttpRequest(HttpRequest {
            metadata: ItemMetadata {
                name: Some(name.to_string()),
                ..Default::default()
            },
            url: Some(url.to_string()),
            ..Default::default()
        })
    }

    #[test]
    fn expand_all_lists_every_request() {
        let workspace = Workspace::from_collection(collection(vec![
            request("alpha"),
            folder(
                "beta",
                vec![
                    request("beta-1"),
                    folder("nested", vec![request("nested-1")]),
                ],
            ),
            request("gamma"),
        ]));
        let mut view = TreeView::default();
        view.reset(workspace);
        let visible: Vec<_> = view
            .visible_rows()
            .iter()
            .map(|row| match row {
                Row::Folder { depth, .. } => format!("F d={depth}"),
                Row::Request { depth, .. } => format!("R d={depth}"),
            })
            .collect();
        assert_eq!(
            visible,
            vec![
                "R d=0".to_string(),
                "F d=0".to_string(),
                "R d=1".to_string(),
                "F d=1".to_string(),
                "R d=2".to_string(),
                "R d=0".to_string(),
            ]
        );
    }

    #[test]
    fn collapse_hides_descendants() {
        let workspace = Workspace::from_collection(collection(vec![folder(
            "beta",
            vec![
                request("beta-1"),
                folder("nested", vec![request("nested-1")]),
            ],
        )]));
        let mut view = TreeView::default();
        view.reset(workspace);
        let folder_key = match view.visible_rows()[0] {
            Row::Folder { key, .. } => key,
            _ => panic!("first row should be the folder"),
        };
        view.toggle_collapsed(folder_key);
        let visible = view.visible_rows();
        assert_eq!(visible.len(), 1, "collapsed folder hides its descendants");
        assert!(view.is_collapsed(folder_key));
        assert!(matches!(view.selected_row(), Some(Row::Folder { .. })));
    }

    #[test]
    fn search_filters_visible_rows() {
        let workspace = Workspace::from_collection(collection(vec![
            request_with_url("List pets", "https://api.example.com/pets"),
            request_with_url("Health", "https://api.example.com/health"),
        ]));
        let mut view = TreeView::default();
        view.reset(workspace);
        view.set_search("pet");
        let names: Vec<_> = view
            .visible_rows()
            .iter()
            .filter_map(|row| match row {
                Row::Request { key, .. } => {
                    let request = view.workspace().unwrap().request(*key).unwrap();
                    Some(request.metadata.name.clone().unwrap_or_default())
                }
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["List pets".to_string()]);
    }

    #[test]
    fn search_finds_under_folder_with_match() {
        let workspace = Workspace::from_collection(collection(vec![folder(
            "Pets",
            vec![request_with_url("List", "https://api.example.com/pets")],
        )]));
        let mut view = TreeView::default();
        view.reset(workspace);
        view.set_search("PET");
        assert_eq!(
            view.visible_rows().len(),
            2,
            "folder and its child stay visible"
        );
    }

    #[test]
    fn search_opens_collapsed_folder() {
        let workspace = Workspace::from_collection(collection(vec![folder(
            "Pets",
            vec![request_with_url("List", "https://api.example.com/pets")],
        )]));
        let mut view = TreeView::default();
        view.reset(workspace);
        let folder_key = match view.visible_rows()[0] {
            Row::Folder { key, .. } => key,
            _ => panic!("folder"),
        };
        view.toggle_collapsed(folder_key);
        assert_eq!(view.visible_rows().len(), 1);
        view.set_search("list");
        assert_eq!(view.visible_rows().len(), 2);
    }

    #[test]
    fn folder_request_count_is_recursive() {
        let workspace = Workspace::from_collection(collection(vec![folder(
            "beta",
            vec![
                request("a"),
                request("b"),
                folder("nested", vec![request("c"), request("d"), request("e")]),
            ],
        )]));
        let mut view = TreeView::default();
        view.reset(workspace);
        let counts: Vec<_> = view
            .visible_rows()
            .iter()
            .filter_map(|row| match row {
                Row::Folder { request_count, .. } => Some(*request_count),
                _ => None,
            })
            .collect();
        assert_eq!(counts, vec![5, 3]);
    }

    #[test]
    fn select_first_and_last_jump_the_visible_rows() {
        let workspace = Workspace::from_collection(collection(vec![
            request("alpha"),
            request("beta"),
            request("gamma"),
        ]));
        let mut view = TreeView::default();
        view.reset(workspace);
        view.move_selection(1);
        assert_eq!(view.selection(), 1);
        view.select_last();
        assert_eq!(view.selection(), 2);
        view.select_first();
        assert_eq!(view.selection(), 0);
    }
}
