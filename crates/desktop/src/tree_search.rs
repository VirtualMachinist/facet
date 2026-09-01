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
    let mut search_path = String::new();
    SearchContext {
        workspace,
        atom: &atom,
        matcher: &mut matcher,
        utf32_buf: &mut utf32_buf,
        hits: &mut hits,
    }
    .collect_matches(
        workspace.root_items(),
        &mut ancestors,
        &mut search_path,
        false,
    );
    hits
}

struct SearchContext<'a> {
    workspace: &'a Workspace,
    atom: &'a Atom,
    matcher: &'a mut Matcher,
    utf32_buf: &'a mut Vec<char>,
    hits: &'a mut TreeSearchMatches,
}

impl SearchContext<'_> {
    fn collect_matches(
        &mut self,
        items: &[WorkspaceItemRef],
        ancestors: &mut Vec<FolderKey>,
        search_path: &mut String,
        include_descendants: bool,
    ) {
        for item in items {
            let path_len = search_path.len();
            let matched = !include_descendants && self.item_matches(*item, search_path);
            match *item {
                WorkspaceItemRef::Request(key) => {
                    if matched || include_descendants {
                        self.hits.requests.insert(key);
                        self.hits.folders.extend(ancestors.iter().copied());
                    }
                }
                WorkspaceItemRef::Folder(key) => {
                    let include_folder = matched || include_descendants;
                    if include_folder {
                        self.hits.folders.insert(key);
                        self.hits.folders.extend(ancestors.iter().copied());
                    }
                    if let Some(folder) = self.workspace.folder(key) {
                        ancestors.push(key);
                        self.collect_matches(
                            &folder.children,
                            ancestors,
                            search_path,
                            include_folder,
                        );
                        ancestors.pop();
                    }
                }
            }
            search_path.truncate(path_len);
        }
    }

    fn item_matches(&mut self, item: WorkspaceItemRef, search_path: &mut String) -> bool {
        if !search_path.is_empty() {
            search_path.push_str(" / ");
        }
        search_path.push_str(item_name(self.workspace, item));

        let haystack = Utf32Str::new(search_path, self.utf32_buf);
        self.atom.score(haystack, self.matcher).is_some()
    }
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
    fn fuzzy_path_match_finds_request_by_parent_folder_and_request_name() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "users create"));
        assert_eq!(names, ["Users", "Admin", "Create user"]);
    }

    #[test]
    fn path_matching_does_not_leak_ancestor_names_to_siblings() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "users health"));
        assert!(names.is_empty());
    }

    #[test]
    fn fuzzy_folder_match_includes_descendants_and_ancestors() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "admn"));
        assert_eq!(names, ["Users", "Admin", "Create user"]);
    }

    #[test]
    fn unmatched_root_requests_are_hidden() {
        let workspace = workspace();
        let names = names(&workspace, &matching_tree_items(&workspace, "hlth"));
        assert_eq!(names, ["Health check"]);
    }
}
