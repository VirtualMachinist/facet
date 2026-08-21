//! Fuzzy filtering for the collection sidebar tree.

use std::collections::HashSet;

use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{Atom, AtomKind, CaseMatching, Normalization},
};
use probe_core::{FolderKey, RequestKey, Workspace, WorkspaceItemRef};

#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct TreeSearchMatches {
    requests: HashSet<RequestKey>,
    folders: HashSet<FolderKey>,
}

impl TreeSearchMatches {
    pub(crate) fn contains(&self, item: WorkspaceItemRef) -> bool {
        match item {
            WorkspaceItemRef::Request(key) => self.requests.contains(&key),
            WorkspaceItemRef::Folder(key) => self.folders.contains(&key),
        }
    }

    pub(crate) fn folders(&self) -> impl Iterator<Item = FolderKey> + '_ {
        self.folders.iter().copied()
    }
}

pub(crate) fn matching_tree_items(workspace: &Workspace, query: &str) -> TreeSearchMatches {
    let query = query.trim();
    let mut hits = TreeSearchMatches::default();
    if query.is_empty() {
        return hits;
    }

    let mut matcher = Matcher::new(Config::DEFAULT);
    let atom = Atom::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false,
    );
    let mut utf32_buf = Vec::new();
    let mut ancestors = Vec::new();
    collect_matches(
        workspace,
        workspace.root_items(),
        &atom,
        &mut matcher,
        &mut utf32_buf,
        &mut ancestors,
        &mut hits,
    );
    hits
}

fn collect_matches(
    workspace: &Workspace,
    items: &[WorkspaceItemRef],
    atom: &Atom,
    matcher: &mut Matcher,
    utf32_buf: &mut Vec<char>,
    ancestors: &mut Vec<FolderKey>,
    hits: &mut TreeSearchMatches,
) {
    for item in items {
        let matched = item_matches(workspace, *item, atom, matcher, utf32_buf);
        match *item {
            WorkspaceItemRef::Request(key) => {
                if matched {
                    hits.requests.insert(key);
                    hits.folders.extend(ancestors.iter().copied());
                }
            }
            WorkspaceItemRef::Folder(key) => {
                if matched {
                    hits.folders.insert(key);
                    hits.folders.extend(ancestors.iter().copied());
                }
                if let Some(folder) = workspace.folder(key) {
                    ancestors.push(key);
                    collect_matches(
                        workspace,
                        &folder.children,
                        atom,
                        matcher,
                        utf32_buf,
                        ancestors,
                        hits,
                    );
                    ancestors.pop();
                }
            }
        }
    }
}

fn item_matches(
    workspace: &Workspace,
    item: WorkspaceItemRef,
    atom: &Atom,
    matcher: &mut Matcher,
    utf32_buf: &mut Vec<char>,
) -> bool {
    let haystack = Utf32Str::new(item_name(workspace, item), utf32_buf);
    atom.score(haystack, matcher).is_some()
}

fn item_name(workspace: &Workspace, item: WorkspaceItemRef) -> &str {
    match item {
        WorkspaceItemRef::Request(key) => workspace
            .request(key)
            .and_then(|request| request.metadata.name.as_deref())
            .unwrap_or("Untitled request"),
        WorkspaceItemRef::Folder(key) => workspace
            .folder(key)
            .and_then(|folder| folder.metadata.name.as_deref())
            .unwrap_or("Untitled folder"),
    }
}

#[cfg(test)]
mod tests {
    use probe_core::{
        Collection, CollectionItem, Folder, HttpRequest, ItemMetadata, Workspace, WorkspaceItemRef,
    };

    use super::{TreeSearchMatches, matching_tree_items};

    fn named_request(name: &str) -> CollectionItem {
        CollectionItem::HttpRequest(HttpRequest {
            metadata: ItemMetadata {
                name: Some(name.to_owned()),
                ..ItemMetadata::default()
            },
            ..HttpRequest::default()
        })
    }

    fn named_folder(name: &str, items: Vec<CollectionItem>) -> CollectionItem {
        CollectionItem::Folder(Folder {
            metadata: ItemMetadata {
                name: Some(name.to_owned()),
                ..ItemMetadata::default()
            },
            items,
        })
    }

    fn workspace() -> Workspace {
        Workspace::from_collection(Collection {
            items: vec![
                named_folder(
                    "Users",
                    vec![named_folder("Admin", vec![named_request("Create user")])],
                ),
                named_request("Health check"),
            ],
            ..Collection::default()
        })
    }

    fn names(workspace: &Workspace, hits: &TreeSearchMatches) -> Vec<String> {
        let mut names = Vec::new();
        collect_names(workspace, workspace.root_items(), hits, &mut names);
        names
    }

    fn collect_names(
        workspace: &Workspace,
        items: &[WorkspaceItemRef],
        hits: &TreeSearchMatches,
        names: &mut Vec<String>,
    ) {
        for item in items {
            if hits.contains(*item) {
                names.push(super::item_name(workspace, *item).to_owned());
            }
            if let WorkspaceItemRef::Folder(key) = item
                && let Some(folder) = workspace.folder(*key)
            {
                collect_names(workspace, &folder.children, hits, names);
            }
        }
    }

    #[test]
    fn empty_query_matches_nothing_so_the_full_tree_can_be_shown() {
        let workspace = workspace();
        assert_eq!(
            matching_tree_items(&workspace, "   "),
            TreeSearchMatches::default()
        );
    }

    #[test]
    fn fuzzy_request_match_keeps_parent_folders() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "crtusr"));
        assert_eq!(names, ["Users", "Admin", "Create user"]);
    }

    #[test]
    fn fuzzy_folder_match_keeps_ancestors_but_not_unmatched_children() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "admn"));
        assert_eq!(names, ["Users", "Admin"]);
    }

    #[test]
    fn unmatched_root_requests_are_hidden() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "hlth"));
        assert_eq!(names, ["Health check"]);
    }
}
