use super::*;

pub(super) fn mutate_bundled(
    document: &mut Value,
    operation: StructureOperation,
) -> Result<StructureResult, StructureError> {
    match operation {
        StructureOperation::CreateRequest {
            parent,
            index,
            name,
            method,
            url,
        } => {
            validate_name(&name)?;
            let parent_path = destination_path(document, parent.as_deref())?;
            let items = items_mut(document, &parent_path)?;
            let index = checked_index(index, items.len())?;
            items.insert(index, request_value(&name, method, url));
            Ok(result(ItemKind::Request, None, parent, index, &parent_path))
        }
        StructureOperation::CreateFolder {
            parent,
            index,
            name,
        } => {
            validate_name(&name)?;
            let parent_path = destination_path(document, parent.as_deref())?;
            let items = items_mut(document, &parent_path)?;
            let index = checked_index(index, items.len())?;
            items.insert(index, folder_value(&name));
            Ok(result(ItemKind::Folder, None, parent, index, &parent_path))
        }
        StructureOperation::RenameRequest { selector, name } => {
            validate_name(&name)?;
            rename_bundled(document, &selector, ItemKind::Request, &name)?;
            let (parent, index) = selector_parent(&selector)?;
            Ok(StructureResult {
                kind: ItemKind::Request,
                previous_selector: Some(selector.clone()),
                selector: Some(selector),
                parent,
                index: Some(index),
                selector_remaps: BTreeMap::new(),
            })
        }
        StructureOperation::RenameFolder { selector, name } => {
            validate_name(&name)?;
            rename_bundled(document, &selector, ItemKind::Folder, &name)?;
            let (parent, index) = selector_parent(&selector)?;
            Ok(StructureResult {
                kind: ItemKind::Folder,
                previous_selector: Some(selector.clone()),
                selector: Some(selector),
                parent,
                index: Some(index),
                selector_remaps: BTreeMap::new(),
            })
        }
        StructureOperation::DeleteRequest { selector } => {
            delete_bundled(document, &selector, ItemKind::Request)
        }
        StructureOperation::DuplicateRequest { selector } => duplicate_bundled(document, selector),
        StructureOperation::DeleteFolder { selector } => {
            delete_bundled(document, &selector, ItemKind::Folder)
        }
        StructureOperation::MoveRequest {
            selector,
            parent,
            index,
        } => move_bundled(document, selector, ItemKind::Request, parent, index),
        StructureOperation::MoveFolder {
            selector,
            parent,
            index,
        } => move_bundled(document, selector, ItemKind::Folder, parent, index),
        StructureOperation::ReorderRequest { selector, index } => {
            let (parent, _) = selector_parent(&selector)?;
            move_bundled(document, selector, ItemKind::Request, parent, Some(index))
        }
        StructureOperation::ReorderFolder { selector, index } => {
            let (parent, _) = selector_parent(&selector)?;
            move_bundled(document, selector, ItemKind::Folder, parent, Some(index))
        }
    }
}

pub(super) fn move_bundled(
    document: &mut Value,
    selector: String,
    kind: ItemKind,
    parent: Option<String>,
    requested_index: Option<usize>,
) -> Result<StructureResult, StructureError> {
    if kind == ItemKind::Folder
        && parent.as_deref().is_some_and(|destination| {
            destination == selector || destination.starts_with(&(selector.clone() + "/items/"))
        })
    {
        return Err(StructureError::InvalidDestination(
            "folder cannot be moved into itself or its descendant".to_owned(),
        ));
    }
    let source_path = parse_selector(&selector)?;
    let source_index = *source_path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let source_parent = &source_path[..source_path.len() - 1];
    let mut destination_path = destination_path(document, parent.as_deref())?;
    let source_items = items_mut(document, source_parent)?;
    let item = source_items
        .get(source_index)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind,
            selector: selector.clone(),
        })?;
    ensure_kind(item, kind, &selector)?;
    let item = source_items.remove(source_index);
    adjust_path_after_removal(&mut destination_path, source_parent, source_index);
    let destination_items = items_mut(document, &destination_path)?;
    let index = checked_index(requested_index, destination_items.len())?;
    destination_items.insert(index, item);
    let actual_parent = selector_from_path(&destination_path);
    Ok(result(
        kind,
        Some(selector),
        actual_parent,
        index,
        &destination_path,
    ))
}

pub(super) fn adjust_path_after_removal(
    destination_path: &mut [usize],
    source_parent: &[usize],
    source_index: usize,
) {
    if destination_path.len() > source_parent.len()
        && destination_path[..source_parent.len()] == *source_parent
        && destination_path[source_parent.len()] > source_index
    {
        destination_path[source_parent.len()] -= 1;
    }
}

pub(super) fn delete_bundled(
    document: &mut Value,
    selector: &str,
    kind: ItemKind,
) -> Result<StructureResult, StructureError> {
    let path = parse_selector(selector)?;
    let index = *path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let parent_path = &path[..path.len() - 1];
    let items = items_mut(document, parent_path)?;
    let item = items
        .get(index)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind,
            selector: selector.to_owned(),
        })?;
    ensure_kind(item, kind, selector)?;
    items.remove(index);
    let (parent, _) = selector_parent(selector)?;
    Ok(StructureResult {
        kind,
        previous_selector: Some(selector.to_owned()),
        selector: None,
        parent,
        index: None,
        selector_remaps: BTreeMap::new(),
    })
}

pub(super) fn duplicate_bundled(
    document: &mut Value,
    selector: String,
) -> Result<StructureResult, StructureError> {
    let path = parse_selector(&selector)?;
    let index = *path
        .last()
        .ok_or_else(|| StructureError::InvalidDocument("empty selector".to_owned()))?;
    let parent_path = &path[..path.len() - 1];
    let mut duplicate = item(document, &path)
        .ok_or_else(|| StructureError::ItemNotFound {
            kind: ItemKind::Request,
            selector: selector.clone(),
        })?
        .clone();
    ensure_kind(&duplicate, ItemKind::Request, &selector)?;
    let name = copied_request_name(&duplicate)?;
    set_info_field(&mut duplicate, "name", Value::String(name))?;
    let items = items_mut(document, parent_path)?;
    let insertion_index = index + 1;
    items.insert(insertion_index, duplicate);
    Ok(result(
        ItemKind::Request,
        None,
        selector_from_path(parent_path),
        insertion_index,
        parent_path,
    ))
}

pub(super) fn rename_bundled(
    document: &mut Value,
    selector: &str,
    kind: ItemKind,
    name: &str,
) -> Result<(), StructureError> {
    let path = parse_selector(selector)?;
    let item = item_mut(document, &path).ok_or_else(|| StructureError::ItemNotFound {
        kind,
        selector: selector.to_owned(),
    })?;
    ensure_kind(item, kind, selector)?;
    set_info_field(item, "name", Value::String(name.to_owned()))
}

pub(super) fn copied_request_name(value: &Value) -> Result<String, StructureError> {
    let original = value
        .get("info")
        .and_then(|info| info.get("name"))
        .and_then(Value::as_str)
        .unwrap_or("Untitled request");
    let name = format!("{original} Copied");
    validate_name(&name)?;
    Ok(name)
}

pub(super) fn destination_path(
    document: &Value,
    selector: Option<&str>,
) -> Result<Vec<usize>, StructureError> {
    let Some(selector) = selector else {
        return Ok(Vec::new());
    };
    let path = parse_selector(selector)?;
    let item = item(document, &path)
        .ok_or_else(|| StructureError::DestinationNotFound(selector.to_owned()))?;
    ensure_kind(item, ItemKind::Folder, selector)
        .map_err(|_| StructureError::DestinationNotFound(selector.to_owned()))?;
    Ok(path)
}

pub(super) fn result(
    kind: ItemKind,
    previous_selector: Option<String>,
    parent: Option<String>,
    index: usize,
    parent_path: &[usize],
) -> StructureResult {
    StructureResult {
        kind,
        previous_selector,
        selector: Some(selector_for(parent_path, index)),
        parent,
        index: Some(index),
        selector_remaps: BTreeMap::new(),
    }
}
