//! Flattened workspace view for the TUI sidebar.
//!
//! Walks `Workspace` in source order and emits a flat list of visible
//! rows. Selection is O(1) by row index and resolves back to a
//! session-only `RequestKey`/`FolderKey`. The runtime keys are session
//! only (per the architecture contract) and never serialized.

use probe_core::{FolderKey, RequestKey, Workspace, WorkspaceItemRef};

/// One row in the flattened sidebar.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Row {
    Folder { key: FolderKey, depth: u8 },
    Request { key: RequestKey, depth: u8 },
}

impl Row {
    pub fn depth(self) -> u8 {
        match self {
            Row::Folder { depth, .. } | Row::Request { depth, .. } => depth,
        }
    }
}

/// Iterator over a workspace's source-order children, recursively.
pub struct Flattened<'w> {
    workspace: &'w Workspace,
    /// Top-level children iterator (consumed before any descent).
    root: Option<std::slice::Iter<'w, WorkspaceItemRef>>,
    /// Folder descent stack; the top frame is the active folder whose
    /// children we are currently walking.
    stack: Vec<StackFrame<'w>>,
}

struct StackFrame<'w> {
    children: std::slice::Iter<'w, WorkspaceItemRef>,
    next_depth: u8,
}

impl<'w> Flattened<'w> {
    pub fn new(workspace: &'w Workspace) -> Self {
        Self {
            workspace,
            root: Some(workspace.root_items().iter()),
            stack: Vec::new(),
        }
    }
}

impl<'w> Iterator for Flattened<'w> {
    type Item = Row;

    fn next(&mut self) -> Option<Row> {
        loop {
            // Inside a folder: keep walking its children, descending into
            // any nested folders we encounter.
            if let Some(frame) = self.stack.last_mut() {
                if let Some(child) = frame.children.next() {
                    return Some(emit(
                        self.workspace,
                        *child,
                        frame.next_depth,
                        &mut self.stack,
                    ));
                }
                self.stack.pop();
                continue;
            }

            // Top level: drain the root children, pushing folders onto
            // the stack so their descendants are walked next.
            let item = match &mut self.root {
                Some(iter) => iter.next().copied(),
                None => return None,
            }?;
            return Some(emit(self.workspace, item, 0, &mut self.stack));
        }
    }
}

fn emit<'w>(
    workspace: &'w Workspace,
    item: WorkspaceItemRef,
    depth: u8,
    stack: &mut Vec<StackFrame<'w>>,
) -> Row {
    match item {
        WorkspaceItemRef::Folder(key) => {
            let folder = workspace
                .folder(key)
                .expect("workspace folder key must resolve");
            let next_depth = depth.saturating_add(1);
            stack.push(StackFrame {
                children: folder.children.iter(),
                next_depth,
            });
            Row::Folder { key, depth }
        }
        WorkspaceItemRef::Request(key) => Row::Request { key, depth },
    }
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

    #[test]
    fn flattens_in_source_order() {
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
        let rows: Vec<_> = Flattened::new(&workspace).collect();
        assert_eq!(rows.len(), 6);
        assert!(matches!(rows[0], Row::Request { depth: 0, .. }));
        assert!(matches!(rows[1], Row::Folder { depth: 0, .. }));
        assert!(matches!(rows[2], Row::Request { depth: 1, .. }));
        assert!(matches!(rows[3], Row::Folder { depth: 1, .. }));
        assert!(matches!(rows[4], Row::Request { depth: 2, .. }));
        assert!(matches!(rows[5], Row::Request { depth: 0, .. }));
    }
}
