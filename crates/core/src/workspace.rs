use crate::{
    Collection, CollectionItem, CollectionMetadata, Environment, HttpRequest, ItemMetadata,
};
use std::collections::BTreeMap;
use std::{error::Error, fmt};

/// Session-only generational key for an HTTP request in a loaded workspace.
///
/// Keys are rebuilt whenever a workspace is loaded and are never serialized. If a
/// deleted request's slot is reused, its replacement receives a different generation.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct RequestKey {
    slot: usize,
    generation: u64,
}

impl RequestKey {
    /// Returns the workspace-local storage slot.
    #[must_use]
    pub const fn slot(self) -> usize {
        self.slot
    }

    /// Returns the slot generation used to reject stale keys.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// Session-only generational key for a folder in a loaded workspace.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FolderKey {
    slot: usize,
    generation: u64,
}

impl FolderKey {
    /// Returns the workspace-local storage slot.
    #[must_use]
    pub const fn slot(self) -> usize {
        self.slot
    }

    /// Returns the slot generation used to reject stale keys.
    #[must_use]
    pub const fn generation(self) -> u64 {
        self.generation
    }
}

/// A request or folder reference used to retain collection ordering.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceItemRef {
    /// A folder reference.
    Folder(FolderKey),
    /// An HTTP request reference.
    Request(RequestKey),
}

/// Indexed folder metadata and ordered children.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceFolder {
    /// Session-only folder key.
    pub key: FolderKey,
    /// Folder metadata.
    pub metadata: ItemMetadata,
    /// Ordered direct children.
    pub children: Vec<WorkspaceItemRef>,
}

/// An active, fully in-memory workspace.
///
/// Requests and folders are stored once in generational arenas. Selecting an item by
/// key requires no filesystem access, parsing, database query, or network operation.
#[derive(Clone, Debug, PartialEq)]
pub struct Workspace {
    metadata: CollectionMetadata,
    root_items: Vec<WorkspaceItemRef>,
    requests: Arena<HttpRequest>,
    folders: Arena<WorkspaceFolder>,
    request_ancestors: BTreeMap<RequestKey, Vec<FolderKey>>,
    environments: Vec<Environment>,
}

/// A parent location for a structural workspace edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceParent {
    /// The collection root.
    Root,
    /// A folder in the loaded workspace.
    Folder(FolderKey),
}

/// Errors produced by in-memory hierarchy operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkspaceEditError {
    /// A runtime key does not resolve in this workspace.
    ItemNotFound,
    /// The destination folder does not resolve.
    DestinationNotFound,
    /// A folder cannot be moved into itself or one of its descendants.
    InvalidDestination,
    /// The requested insertion index is greater than the child count.
    InvalidIndex,
}

impl fmt::Display for WorkspaceEditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ItemNotFound => "workspace item not found",
            Self::DestinationNotFound => "destination folder not found",
            Self::InvalidDestination => "folder cannot be moved into itself or its descendant",
            Self::InvalidIndex => "insertion index is out of bounds",
        })
    }
}

impl Error for WorkspaceEditError {}

impl Workspace {
    /// Builds an indexed workspace from a domain collection.
    #[must_use]
    pub fn from_collection(collection: Collection) -> Self {
        let mut requests = Arena::default();
        let mut folders = Arena::default();
        let mut request_ancestors = BTreeMap::new();
        let root_items = index_items(
            collection.items,
            &mut requests,
            &mut folders,
            &mut request_ancestors,
            &[],
        );

        Self {
            metadata: collection.metadata,
            root_items,
            requests,
            folders,
            request_ancestors,
            environments: collection.environments,
        }
    }

    /// Returns collection metadata.
    #[must_use]
    pub const fn metadata(&self) -> &CollectionMetadata {
        &self.metadata
    }

    /// Returns the ordered items at the workspace root.
    #[must_use]
    pub fn root_items(&self) -> &[WorkspaceItemRef] {
        &self.root_items
    }

    /// Looks up a request in constant time, rejecting stale generations.
    #[must_use]
    pub fn request(&self, key: RequestKey) -> Option<&HttpRequest> {
        self.requests.get(key.into())
    }

    /// Mutably looks up a request in constant time, rejecting stale generations.
    pub fn request_mut(&mut self, key: RequestKey) -> Option<&mut HttpRequest> {
        self.requests.get_mut(key.into())
    }

    /// Returns the number of live requests.
    #[must_use]
    pub const fn request_count(&self) -> usize {
        self.requests.len()
    }

    /// Adds a request at the workspace root and returns its new runtime key.
    pub fn add_root_request(&mut self, request: HttpRequest) -> RequestKey {
        let key = RequestKey::from(self.requests.insert(request));
        self.request_ancestors.insert(key, Vec::new());
        self.root_items.push(WorkspaceItemRef::Request(key));
        key
    }

    /// Inserts a request at an exact position under a parent.
    pub fn insert_request(
        &mut self,
        parent: WorkspaceParent,
        index: usize,
        request: HttpRequest,
    ) -> Result<RequestKey, WorkspaceEditError> {
        self.validate_insertion(parent, index)?;
        let key = RequestKey::from(self.requests.insert(request));
        self.insert_reference(parent, index, WorkspaceItemRef::Request(key))?;
        self.rebuild_request_ancestors();
        Ok(key)
    }

    /// Inserts an empty folder at an exact position under a parent.
    pub fn insert_folder(
        &mut self,
        parent: WorkspaceParent,
        index: usize,
        metadata: ItemMetadata,
    ) -> Result<FolderKey, WorkspaceEditError> {
        self.validate_insertion(parent, index)?;
        let arena_key = self.folders.insert(WorkspaceFolder {
            key: FolderKey {
                slot: 0,
                generation: 0,
            },
            metadata,
            children: Vec::new(),
        });
        let key = FolderKey::from(arena_key);
        self.folders
            .get_mut(arena_key)
            .expect("new folder must resolve")
            .key = key;
        self.insert_reference(parent, index, WorkspaceItemRef::Folder(key))?;
        Ok(key)
    }

    /// Renames a request in memory.
    pub fn rename_request(
        &mut self,
        key: RequestKey,
        name: String,
    ) -> Result<(), WorkspaceEditError> {
        self.request_mut(key)
            .ok_or(WorkspaceEditError::ItemNotFound)?
            .metadata
            .name = Some(name);
        Ok(())
    }

    /// Renames a folder in memory.
    pub fn rename_folder(
        &mut self,
        key: FolderKey,
        name: String,
    ) -> Result<(), WorkspaceEditError> {
        self.folders
            .get_mut(key.into())
            .ok_or(WorkspaceEditError::ItemNotFound)?
            .metadata
            .name = Some(name);
        Ok(())
    }

    /// Moves or reorders a request under a parent.
    pub fn move_request(
        &mut self,
        key: RequestKey,
        parent: WorkspaceParent,
        index: usize,
    ) -> Result<(), WorkspaceEditError> {
        if self.request(key).is_none() {
            return Err(WorkspaceEditError::ItemNotFound);
        }
        self.move_reference(WorkspaceItemRef::Request(key), parent, index)?;
        self.rebuild_request_ancestors();
        Ok(())
    }

    /// Moves or reorders a folder under a parent.
    pub fn move_folder(
        &mut self,
        key: FolderKey,
        parent: WorkspaceParent,
        index: usize,
    ) -> Result<(), WorkspaceEditError> {
        if self.folder(key).is_none() {
            return Err(WorkspaceEditError::ItemNotFound);
        }
        if parent == WorkspaceParent::Folder(key)
            || matches!(parent, WorkspaceParent::Folder(destination) if self.folder_contains(key, destination))
        {
            return Err(WorkspaceEditError::InvalidDestination);
        }
        self.move_reference(WorkspaceItemRef::Folder(key), parent, index)?;
        self.rebuild_request_ancestors();
        Ok(())
    }

    /// Removes a folder and all descendant folders and requests.
    pub fn remove_folder(&mut self, key: FolderKey) -> Result<WorkspaceFolder, WorkspaceEditError> {
        let folder = self
            .folders
            .get(key.into())
            .cloned()
            .ok_or(WorkspaceEditError::ItemNotFound)?;
        self.remove_reference(WorkspaceItemRef::Folder(key))
            .ok_or(WorkspaceEditError::ItemNotFound)?;
        self.remove_descendants(&folder.children);
        let removed = self
            .folders
            .remove(key.into())
            .expect("validated folder key must remain live");
        self.rebuild_request_ancestors();
        Ok(removed)
    }

    /// Removes a request and every hierarchy reference to it.
    ///
    /// A later request may reuse the storage slot, but receives a new generation so
    /// the removed key can never resolve to the replacement.
    pub fn remove_request(&mut self, key: RequestKey) -> Option<HttpRequest> {
        let request = self.requests.remove(key.into())?;
        self.request_ancestors.remove(&key);
        self.root_items
            .retain(|item| *item != WorkspaceItemRef::Request(key));
        for folder in self.folders.values_mut() {
            folder
                .children
                .retain(|item| *item != WorkspaceItemRef::Request(key));
        }
        Some(request)
    }

    /// Looks up a folder in constant time, rejecting stale generations.
    #[must_use]
    pub fn folder(&self, key: FolderKey) -> Option<&WorkspaceFolder> {
        self.folders.get(key.into())
    }

    /// Returns the number of live folders.
    #[must_use]
    pub const fn folder_count(&self) -> usize {
        self.folders.len()
    }

    /// Returns a request's ancestor folder keys from the collection root inward.
    ///
    /// The path is indexed when the workspace is built, so request selection and
    /// presentation do not need to scan the collection tree.
    #[must_use]
    pub fn request_ancestor_folders(&self, key: RequestKey) -> Option<&[FolderKey]> {
        self.request_ancestors.get(&key).map(Vec::as_slice)
    }

    /// Returns collection environments in source order.
    #[must_use]
    pub fn environments(&self) -> &[Environment] {
        &self.environments
    }

    /// Creates a new environment with an optional parent.
    pub fn create_environment(
        &mut self,
        name: String,
        extends: Option<String>,
    ) -> Result<(), crate::EnvironmentResolutionError> {
        crate::create_environment(&mut self.environments, name, extends)
    }

    /// Removes a newly created environment that has not been persisted.
    pub fn revert_created_environment(&mut self, name: &str) {
        crate::revert_created_environment(&mut self.environments, name);
    }

    /// Updates a plain variable on the named environment, or adds an override.
    pub fn set_environment_variable(
        &mut self,
        environment_name: &str,
        variable_name: &str,
        value: String,
    ) -> Result<(), crate::EnvironmentResolutionError> {
        crate::set_environment_variable(
            &mut self.environments,
            environment_name,
            variable_name,
            value,
        )
    }

    /// Removes a plain variable from the named environment only.
    pub fn unset_environment_variable(
        &mut self,
        environment_name: &str,
        variable_name: &str,
    ) -> Result<(), crate::EnvironmentResolutionError> {
        crate::unset_environment_variable(&mut self.environments, environment_name, variable_name)
    }

    fn children(&self, parent: WorkspaceParent) -> Result<&[WorkspaceItemRef], WorkspaceEditError> {
        match parent {
            WorkspaceParent::Root => Ok(&self.root_items),
            WorkspaceParent::Folder(key) => self
                .folder(key)
                .map(|folder| folder.children.as_slice())
                .ok_or(WorkspaceEditError::DestinationNotFound),
        }
    }

    fn children_mut(
        &mut self,
        parent: WorkspaceParent,
    ) -> Result<&mut Vec<WorkspaceItemRef>, WorkspaceEditError> {
        match parent {
            WorkspaceParent::Root => Ok(&mut self.root_items),
            WorkspaceParent::Folder(key) => self
                .folders
                .get_mut(key.into())
                .map(|folder| &mut folder.children)
                .ok_or(WorkspaceEditError::DestinationNotFound),
        }
    }

    fn validate_insertion(
        &self,
        parent: WorkspaceParent,
        index: usize,
    ) -> Result<(), WorkspaceEditError> {
        if index > self.children(parent)?.len() {
            return Err(WorkspaceEditError::InvalidIndex);
        }
        Ok(())
    }

    fn insert_reference(
        &mut self,
        parent: WorkspaceParent,
        index: usize,
        item: WorkspaceItemRef,
    ) -> Result<(), WorkspaceEditError> {
        let children = self.children_mut(parent)?;
        if index > children.len() {
            return Err(WorkspaceEditError::InvalidIndex);
        }
        children.insert(index, item);
        Ok(())
    }

    fn remove_reference(&mut self, item: WorkspaceItemRef) -> Option<(WorkspaceParent, usize)> {
        if let Some(index) = self
            .root_items
            .iter()
            .position(|candidate| *candidate == item)
        {
            self.root_items.remove(index);
            return Some((WorkspaceParent::Root, index));
        }
        for folder in self.folders.values_mut() {
            if let Some(index) = folder
                .children
                .iter()
                .position(|candidate| *candidate == item)
            {
                folder.children.remove(index);
                return Some((WorkspaceParent::Folder(folder.key), index));
            }
        }
        None
    }

    fn move_reference(
        &mut self,
        item: WorkspaceItemRef,
        parent: WorkspaceParent,
        index: usize,
    ) -> Result<(), WorkspaceEditError> {
        // Validate the destination before detaching the item so an invalid runtime
        // key cannot mutate the existing hierarchy.
        self.children(parent)?;
        let (old_parent, old_index) = self
            .remove_reference(item)
            .ok_or(WorkspaceEditError::ItemNotFound)?;
        if index > self.children(parent)?.len() {
            self.insert_reference(old_parent, old_index, item)
                .expect("original position must remain valid");
            return Err(WorkspaceEditError::InvalidIndex);
        }
        self.insert_reference(parent, index, item)
    }

    fn folder_contains(&self, ancestor: FolderKey, candidate: FolderKey) -> bool {
        self.folder(ancestor).is_some_and(|folder| {
            folder.children.iter().any(|item| match item {
                WorkspaceItemRef::Folder(child) => {
                    *child == candidate || self.folder_contains(*child, candidate)
                }
                WorkspaceItemRef::Request(_) => false,
            })
        })
    }

    fn remove_descendants(&mut self, items: &[WorkspaceItemRef]) {
        for item in items {
            match *item {
                WorkspaceItemRef::Request(key) => {
                    let _ = self.requests.remove(key.into());
                    self.request_ancestors.remove(&key);
                }
                WorkspaceItemRef::Folder(key) => {
                    if let Some(folder) = self.folders.get(key.into()).cloned() {
                        self.remove_descendants(&folder.children);
                    }
                    let _ = self.folders.remove(key.into());
                }
            }
        }
    }

    fn rebuild_request_ancestors(&mut self) {
        let mut ancestors = BTreeMap::new();
        collect_request_ancestors(
            &self.folders,
            &self.root_items,
            &mut ancestors,
            &mut Vec::new(),
        );
        self.request_ancestors = ancestors;
    }
}

fn collect_request_ancestors(
    folders: &Arena<WorkspaceFolder>,
    items: &[WorkspaceItemRef],
    output: &mut BTreeMap<RequestKey, Vec<FolderKey>>,
    path: &mut Vec<FolderKey>,
) {
    for item in items {
        match *item {
            WorkspaceItemRef::Request(key) => {
                output.insert(key, path.clone());
            }
            WorkspaceItemRef::Folder(key) => {
                if let Some(folder) = folders.get(key.into()) {
                    path.push(key);
                    collect_request_ancestors(folders, &folder.children, output, path);
                    path.pop();
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ArenaKey {
    slot: usize,
    generation: u64,
}

impl From<ArenaKey> for RequestKey {
    fn from(key: ArenaKey) -> Self {
        Self {
            slot: key.slot,
            generation: key.generation,
        }
    }
}

impl From<RequestKey> for ArenaKey {
    fn from(key: RequestKey) -> Self {
        Self {
            slot: key.slot,
            generation: key.generation,
        }
    }
}

impl From<ArenaKey> for FolderKey {
    fn from(key: ArenaKey) -> Self {
        Self {
            slot: key.slot,
            generation: key.generation,
        }
    }
}

impl From<FolderKey> for ArenaKey {
    fn from(key: FolderKey) -> Self {
        Self {
            slot: key.slot,
            generation: key.generation,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Arena<T> {
    slots: Vec<ArenaSlot<T>>,
    free_head: Option<usize>,
    len: usize,
}

impl<T> Default for Arena<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            free_head: None,
            len: 0,
        }
    }
}

impl<T> Arena<T> {
    const fn len(&self) -> usize {
        self.len
    }

    fn insert(&mut self, value: T) -> ArenaKey {
        self.len += 1;

        if let Some(slot_index) = self.free_head {
            let slot = &mut self.slots[slot_index];
            self.free_head = slot.next_free.take();
            slot.value = Some(value);
            ArenaKey {
                slot: slot_index,
                generation: slot.generation,
            }
        } else {
            let key = ArenaKey {
                slot: self.slots.len(),
                generation: 0,
            };
            self.slots.push(ArenaSlot {
                generation: key.generation,
                value: Some(value),
                next_free: None,
            });
            key
        }
    }

    fn get(&self, key: ArenaKey) -> Option<&T> {
        let slot = self.slots.get(key.slot)?;
        if slot.generation != key.generation {
            return None;
        }
        slot.value.as_ref()
    }

    fn get_mut(&mut self, key: ArenaKey) -> Option<&mut T> {
        let slot = self.slots.get_mut(key.slot)?;
        if slot.generation != key.generation {
            return None;
        }
        slot.value.as_mut()
    }

    fn remove(&mut self, key: ArenaKey) -> Option<T> {
        let slot = self.slots.get_mut(key.slot)?;
        if slot.generation != key.generation {
            return None;
        }

        let value = slot.value.take()?;
        if let Some(next_generation) = slot.generation.checked_add(1) {
            slot.generation = next_generation;
            slot.next_free = self.free_head;
            self.free_head = Some(key.slot);
        } else {
            // Retire an exhausted slot rather than allowing a generation to repeat.
            slot.next_free = None;
        }
        self.len -= 1;
        Some(value)
    }

    fn values_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.slots.iter_mut().filter_map(|slot| slot.value.as_mut())
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ArenaSlot<T> {
    generation: u64,
    value: Option<T>,
    next_free: Option<usize>,
}

fn index_items(
    items: Vec<CollectionItem>,
    requests: &mut Arena<HttpRequest>,
    folders: &mut Arena<WorkspaceFolder>,
    request_ancestors: &mut BTreeMap<RequestKey, Vec<FolderKey>>,
    ancestors: &[FolderKey],
) -> Vec<WorkspaceItemRef> {
    items
        .into_iter()
        .map(|item| index_item(item, requests, folders, request_ancestors, ancestors))
        .collect()
}

fn index_item(
    item: CollectionItem,
    requests: &mut Arena<HttpRequest>,
    folders: &mut Arena<WorkspaceFolder>,
    request_ancestors: &mut BTreeMap<RequestKey, Vec<FolderKey>>,
    ancestors: &[FolderKey],
) -> WorkspaceItemRef {
    match item {
        CollectionItem::HttpRequest(request) => {
            let key = RequestKey::from(requests.insert(request));
            request_ancestors.insert(key, ancestors.to_vec());
            WorkspaceItemRef::Request(key)
        }
        CollectionItem::Folder(folder) => {
            let placeholder = WorkspaceFolder {
                key: FolderKey {
                    slot: 0,
                    generation: 0,
                },
                metadata: folder.metadata,
                children: Vec::new(),
            };
            let arena_key = folders.insert(placeholder);
            let key = FolderKey::from(arena_key);
            let mut child_ancestors = ancestors.to_vec();
            child_ancestors.push(key);
            let children = index_items(
                folder.items,
                requests,
                folders,
                request_ancestors,
                &child_ancestors,
            );
            let indexed_folder = folders
                .get_mut(arena_key)
                .expect("newly inserted folder key must remain valid");
            indexed_folder.key = key;
            indexed_folder.children = children;
            WorkspaceItemRef::Folder(key)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        Collection, CollectionItem, CollectionMetadata, Folder, HttpRequest, ItemMetadata,
    };

    use super::{Workspace, WorkspaceEditError, WorkspaceItemRef, WorkspaceParent};

    fn request(name: &str) -> CollectionItem {
        CollectionItem::HttpRequest(HttpRequest {
            metadata: ItemMetadata {
                name: Some(name.to_owned()),
                sequence: None,
            },
            ..HttpRequest::default()
        })
    }

    fn http_request(name: &str) -> HttpRequest {
        let CollectionItem::HttpRequest(request) = request(name) else {
            unreachable!();
        };
        request
    }

    #[test]
    fn indexes_nested_items_and_preserves_order() {
        let collection = Collection {
            metadata: CollectionMetadata {
                name: Some("Example".to_owned()),
                ..CollectionMetadata::default()
            },
            items: vec![
                CollectionItem::Folder(Folder {
                    metadata: ItemMetadata {
                        name: Some("Users".to_owned()),
                        sequence: Some(1.0),
                    },
                    items: vec![request("List users")],
                }),
                request("Health"),
            ],
            environments: Vec::new(),
        };

        let workspace = Workspace::from_collection(collection);
        let WorkspaceItemRef::Folder(folder_key) = workspace.root_items()[0] else {
            panic!("first root item should be a folder");
        };
        let WorkspaceItemRef::Request(health_key) = workspace.root_items()[1] else {
            panic!("second root item should be a request");
        };
        let folder = workspace
            .folder(folder_key)
            .expect("folder key should resolve");
        let WorkspaceItemRef::Request(list_users_key) = folder.children[0] else {
            panic!("folder child should be a request");
        };

        assert_eq!(workspace.request_count(), 2);
        assert_eq!(workspace.folder_count(), 1);
        assert_eq!(
            workspace
                .request(list_users_key)
                .and_then(|request| request.metadata.name.as_deref()),
            Some("List users")
        );
        assert_eq!(
            workspace
                .request(health_key)
                .and_then(|request| request.metadata.name.as_deref()),
            Some("Health")
        );
    }

    #[test]
    fn indexes_request_ancestor_folders_from_root_inward() {
        let collection = Collection {
            items: vec![CollectionItem::Folder(Folder {
                metadata: ItemMetadata {
                    name: Some("Accounts".to_owned()),
                    sequence: None,
                },
                items: vec![CollectionItem::Folder(Folder {
                    metadata: ItemMetadata {
                        name: Some("Users".to_owned()),
                        sequence: None,
                    },
                    items: vec![request("List users")],
                })],
            })],
            ..Collection::default()
        };

        let workspace = Workspace::from_collection(collection);
        let request_key = workspace
            .root_items()
            .iter()
            .find_map(|item| match item {
                WorkspaceItemRef::Folder(folder_key) => workspace.folder(*folder_key),
                WorkspaceItemRef::Request(_) => None,
            })
            .and_then(|folder| match folder.children[0] {
                WorkspaceItemRef::Folder(folder_key) => workspace.folder(folder_key),
                WorkspaceItemRef::Request(_) => None,
            })
            .and_then(|folder| match folder.children[0] {
                WorkspaceItemRef::Request(request_key) => Some(request_key),
                WorkspaceItemRef::Folder(_) => None,
            })
            .expect("nested request should resolve");

        let ancestor_names = workspace
            .request_ancestor_folders(request_key)
            .expect("request ancestry should be indexed")
            .iter()
            .map(|key| {
                workspace
                    .folder(*key)
                    .and_then(|folder| folder.metadata.name.as_deref())
                    .expect("ancestor folder should resolve")
            })
            .collect::<Vec<_>>();

        assert_eq!(ancestor_names, ["Accounts", "Users"]);
    }

    #[test]
    fn stale_request_key_cannot_resolve_reused_slot() {
        let mut workspace = Workspace::from_collection(Collection {
            items: vec![request("Request X")],
            ..Collection::default()
        });
        let WorkspaceItemRef::Request(request_x_key) = workspace.root_items()[0] else {
            panic!("root item should be a request");
        };

        let removed = workspace
            .remove_request(request_x_key)
            .expect("request X should be removed");
        assert_eq!(removed.metadata.name.as_deref(), Some("Request X"));
        let request_y_key = workspace.add_root_request(http_request("Request Y"));

        assert_eq!(request_x_key.slot(), request_y_key.slot());
        assert_ne!(request_x_key.generation(), request_y_key.generation());
        assert!(workspace.request(request_x_key).is_none());
        assert_eq!(
            workspace
                .request(request_y_key)
                .and_then(|request| request.metadata.name.as_deref()),
            Some("Request Y")
        );
        assert_eq!(
            workspace.root_items(),
            [WorkspaceItemRef::Request(request_y_key)]
        );
    }

    #[test]
    fn structural_operations_preserve_hierarchy_and_reject_folder_cycles() {
        let mut workspace = Workspace::from_collection(Collection {
            items: vec![request("Root")],
            ..Collection::default()
        });
        let request_key = match workspace.root_items()[0] {
            WorkspaceItemRef::Request(key) => key,
            WorkspaceItemRef::Folder(_) => unreachable!(),
        };
        let folder_key = workspace
            .insert_folder(
                WorkspaceParent::Root,
                0,
                ItemMetadata {
                    name: Some("Folder".to_owned()),
                    sequence: None,
                },
            )
            .unwrap();
        let child_key = workspace
            .insert_folder(
                WorkspaceParent::Folder(folder_key),
                0,
                ItemMetadata {
                    name: Some("Child".to_owned()),
                    sequence: None,
                },
            )
            .unwrap();
        workspace
            .move_request(request_key, WorkspaceParent::Folder(child_key), 0)
            .unwrap();
        assert_eq!(
            workspace.request_ancestor_folders(request_key),
            Some([folder_key, child_key].as_slice())
        );
        workspace
            .move_folder(child_key, WorkspaceParent::Root, 0)
            .unwrap();
        assert_eq!(
            workspace.request_ancestor_folders(request_key),
            Some([child_key].as_slice())
        );
        workspace
            .move_folder(child_key, WorkspaceParent::Folder(folder_key), 0)
            .unwrap();
        assert_eq!(
            workspace.move_folder(folder_key, WorkspaceParent::Folder(child_key), 0),
            Err(WorkspaceEditError::InvalidDestination)
        );
        workspace
            .rename_folder(child_key, "Renamed".to_owned())
            .unwrap();
        workspace.remove_folder(folder_key).unwrap();
        assert!(workspace.request(request_key).is_none());
        assert_eq!(workspace.folder_count(), 0);
        assert_eq!(workspace.request_count(), 0);
    }

    #[test]
    fn invalid_move_destination_does_not_detach_the_item() {
        let mut workspace = Workspace::from_collection(Collection {
            items: vec![request("Root"), CollectionItem::Folder(Folder::default())],
            ..Collection::default()
        });
        let WorkspaceItemRef::Request(request_key) = workspace.root_items()[0] else {
            panic!("first item should be a request");
        };
        let WorkspaceItemRef::Folder(folder_key) = workspace.root_items()[1] else {
            panic!("second item should be a folder");
        };
        workspace.remove_folder(folder_key).unwrap();

        assert_eq!(
            workspace.move_request(request_key, WorkspaceParent::Folder(folder_key), 0),
            Err(WorkspaceEditError::DestinationNotFound)
        );
        assert_eq!(
            workspace.root_items(),
            [WorkspaceItemRef::Request(request_key)]
        );
    }
}
