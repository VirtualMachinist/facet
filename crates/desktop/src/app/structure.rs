use super::*;

impl ProbeApp {
    pub(super) fn select_request(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        if self
            .loaded_workspace
            .as_ref()
            .is_some_and(|loaded| loaded.workspace().request(key).is_some())
        {
            self.selected_tree_item = Some(WorkspaceItemRef::Request(key));
            self.shell.open_request(key);
            self.response_viewer.ensure_available_tab(key);
            self.start_base64_encoding(key, cx);
            self.reveal_active_tab();
            self.reveal_request_in_sidebar(key);
            if self
                .loaded_workspace
                .as_mut()
                .and_then(|loaded| loaded.request_mut(key))
                .is_some_and(ensure_path_parameters_from_url)
            {
                self.persistence.edited(key);
            }
            self.persist_session(cx);
            cx.notify();
        }
    }

    pub(super) fn select_tree_item(&mut self, item: WorkspaceItemRef, cx: &mut Context<Self>) {
        self.selected_tree_item = Some(item);
        cx.notify();
    }

    pub(super) fn selected_parent_selector(&self) -> Option<String> {
        let loaded = self.loaded_workspace.as_ref()?;
        let selected = self.selected_tree_item?;
        if let WorkspaceItemRef::Folder(key) = selected {
            return loaded.folder_selector(key).map(str::to_owned);
        }
        let (parent, _) = item_position(loaded.workspace(), selected)?;
        parent.and_then(|key| loaded.folder_selector(key).map(str::to_owned))
    }

    pub(super) fn open_create_request_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loaded_workspace.is_none() || self.structure_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.structure_dialog = Some(StructureDialog::create_request(
            self.selected_parent_selector(),
        ));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn open_create_folder_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loaded_workspace.is_none() || self.structure_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.structure_dialog = Some(StructureDialog::create_folder(
            self.selected_parent_selector(),
        ));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn selected_item_details(&self) -> Option<(ItemKind, String, String)> {
        let loaded = self.loaded_workspace.as_ref()?;
        match self.selected_tree_item? {
            WorkspaceItemRef::Request(key) => Some((
                ItemKind::Request,
                loaded.request_selector(key)?.to_owned(),
                loaded
                    .workspace()
                    .request(key)?
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "Untitled request".to_owned()),
            )),
            WorkspaceItemRef::Folder(key) => Some((
                ItemKind::Folder,
                loaded.folder_selector(key)?.to_owned(),
                loaded
                    .workspace()
                    .folder(key)?
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "Untitled folder".to_owned()),
            )),
        }
    }

    pub(super) fn open_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, name)) = self.selected_item_details() else {
            return;
        };
        self.structure_dialog = Some(StructureDialog::rename(kind, selector, name));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn open_move_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, _)) = self.selected_item_details() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .selected_tree_item
            .expect("details require a selection");
        let Some((parent, _)) = item_position(loaded.workspace(), selected) else {
            return;
        };
        let parent = parent.and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        self.structure_dialog = Some(StructureDialog::move_item(kind, selector, parent));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn reorder_selected(
        &mut self,
        offset: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, _)) = self.selected_item_details() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .selected_tree_item
            .expect("details require a selection");
        let Some((_, index)) = item_position(loaded.workspace(), selected) else {
            return;
        };
        let Some(index) = index.checked_add_signed(offset) else {
            return;
        };
        let operation = match kind {
            ItemKind::Request => StructureOperation::ReorderRequest { selector, index },
            ItemKind::Folder => StructureOperation::ReorderFolder { selector, index },
        };
        self.apply_structure(operation, window, cx);
    }

    pub(super) fn duplicate_selected_request(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((ItemKind::Request, selector, _)) = self.selected_item_details() else {
            return;
        };
        self.apply_structure(
            StructureOperation::DuplicateRequest { selector },
            window,
            cx,
        );
    }

    pub(super) fn request_delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, name)) = self.selected_item_details() else {
            return;
        };
        let dirty_count = match self.selected_tree_item {
            Some(WorkspaceItemRef::Request(key)) => self
                .loaded_workspace
                .as_ref()
                .and_then(|loaded| {
                    loaded
                        .workspace()
                        .request(key)
                        .map(|request| (key, request))
                })
                .is_some_and(|(key, request)| self.persistence.is_dirty(key, request))
                as usize,
            Some(WorkspaceItemRef::Folder(key)) => {
                let mut requests = Vec::new();
                if let Some(loaded) = &self.loaded_workspace {
                    descendant_requests(loaded.workspace(), key, &mut requests);
                }
                requests
                    .into_iter()
                    .filter(|key| {
                        self.loaded_workspace
                            .as_ref()
                            .and_then(|loaded| loaded.workspace().request(*key))
                            .is_some_and(|request| self.persistence.is_dirty(*key, request))
                    })
                    .count()
            }
            None => 0,
        };
        let detail = if dirty_count == 0 {
            "This cannot be undone.".to_owned()
        } else {
            format!(
                "This will discard unsaved changes in {dirty_count} request(s) and cannot be undone."
            )
        };
        self.show_application_dialog(
            ApplicationDialog::Delete {
                kind,
                selector,
                name,
                detail,
            },
            window,
            cx,
        );
    }

    pub(super) fn submit_structure_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.structure_dialog.as_ref() else {
            return;
        };
        match dialog.operation() {
            Ok(operation) => {
                self.structure_dialog = None;
                self.focus_handle.focus(window, cx);
                self.apply_structure(operation, window, cx);
            }
            Err(message) => {
                self.show_toast(ToastIntent::Error, message, cx);
            }
        }
    }

    pub(super) fn submit_application_dialog_primary(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self
            .application_dialog
            .as_ref()
            .and_then(ApplicationDialog::primary_action)
        else {
            return;
        };
        self.handle_application_dialog_action(action, window, cx);
    }

    pub(super) fn submit_application_dialog_destructive(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self
            .application_dialog
            .as_ref()
            .and_then(ApplicationDialog::destructive_action)
        else {
            return;
        };
        self.handle_application_dialog_action(action, window, cx);
    }

    pub(super) fn apply_structure(
        &mut self,
        operation: StructureOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        if self.request_save_task.is_some() || self.environment_save_task.is_some() {
            self.show_toast(
                ToastIntent::Warning,
                "Wait for the current save before changing collection structure.",
                cx,
            );
            return;
        }
        let (Some(mut workspace), Some(path)) =
            (self.loaded_workspace.clone(), self.workspace_path.clone())
        else {
            return;
        };
        self.loading = true;
        let operation_for_task = operation.clone();
        self.structure_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move {
                    let structure_result = workspace
                        .apply_structure(operation_for_task)
                        .map_err(|error| error.to_string())?;
                    let disk_workspace =
                        load_workspace(&path).map_err(|error| error.to_string())?;
                    Ok::<_, String>((workspace, disk_workspace, structure_result))
                })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.structure_task = None;
                view.loading = false;
                match result {
                    Ok((workspace, disk_workspace, result)) => {
                        view.apply_structure_result(
                            workspace,
                            disk_workspace,
                            result,
                            &operation,
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        view.show_toast(
                            ToastIntent::Error,
                            format!("Could not edit collection structure: {error}"),
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn apply_structure_result(
        &mut self,
        mut workspace: LoadedWorkspace,
        disk_workspace: LoadedWorkspace,
        result: StructureResult,
        operation: &StructureOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.loaded_workspace.as_ref() else {
            return;
        };
        let selectors = self.snapshot_shell_selectors(old);
        let key_remaps = request_key_remaps(old, &workspace, &result.selector_remaps);
        let current_requests = old
            .requests()
            .iter()
            .filter_map(|located| {
                old.workspace()
                    .request(located.key())
                    .cloned()
                    .map(|request| (located.selector().to_owned(), request))
            })
            .collect::<Vec<_>>();

        for (old_selector, mut request) in current_requests {
            let Some(new_selector) = result.selector_remaps.get(&old_selector) else {
                continue;
            };
            let Some(new_key) = workspace.request_key(new_selector) else {
                continue;
            };
            let persisted = disk_workspace
                .request_key(new_selector)
                .and_then(|key| disk_workspace.workspace().request(key));
            if let Some(persisted) = persisted {
                request.metadata.sequence = persisted.metadata.sequence;
                if matches!(
                    operation,
                    StructureOperation::RenameRequest { selector, .. }
                        if selector == &old_selector
                ) {
                    request.metadata.name.clone_from(&persisted.metadata.name);
                }
            }
            if let Some(target) = workspace.request_mut(new_key) {
                *target = request;
            }
        }

        let baselines = workspace
            .requests()
            .iter()
            .filter_map(|located| {
                let disk_key = disk_workspace.request_key(located.selector())?;
                let baseline = disk_workspace.workspace().request(disk_key)?.clone();
                Some((located.key(), baseline))
            })
            .collect::<Vec<_>>();
        self.install_reloaded_workspace(workspace, baselines, &key_remaps);
        self.restore_shell_selectors(&result.selector_remaps, selectors);
        let should_select_result = matches!(
            operation,
            StructureOperation::CreateRequest { .. }
                | StructureOperation::CreateFolder { .. }
                | StructureOperation::DuplicateRequest { .. }
        );
        if matches!(
            operation,
            StructureOperation::CreateRequest { .. } | StructureOperation::CreateFolder { .. }
        ) && let Some(parent) = result.parent.as_deref()
        {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            if let Some(key) = loaded.folder_key(parent) {
                self.shell.expand_folder(key);
            }
        }
        if should_select_result && let Some(selector) = result.selector.as_deref() {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            self.selected_tree_item = match result.kind {
                ItemKind::Request => loaded.request_key(selector).map(WorkspaceItemRef::Request),
                ItemKind::Folder => loaded.folder_key(selector).map(WorkspaceItemRef::Folder),
            };
            if matches!(operation, StructureOperation::DuplicateRequest { .. })
                && self.structure_dialog.is_none()
            {
                self.tree_focus_handle.focus(window, cx);
            }
        }
        if self.selected_tree_item.is_none()
            && let Some(selector) = result.selector.as_deref()
        {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            self.selected_tree_item = match result.kind {
                ItemKind::Request => loaded.request_key(selector).map(WorkspaceItemRef::Request),
                ItemKind::Folder => loaded.folder_key(selector).map(WorkspaceItemRef::Folder),
            };
        }
        if let Some(WorkspaceItemRef::Request(key)) = self.selected_tree_item
            && result.previous_selector.is_none()
        {
            self.shell.open_request(key);
        }
        self.rebuild_visible_tree_rows();
        if should_select_result {
            match self.selected_tree_item {
                Some(WorkspaceItemRef::Request(key)) => self.reveal_request_in_sidebar(key),
                Some(WorkspaceItemRef::Folder(_)) | None => {
                    self.scroll_selected_tree_item_into_view();
                }
            }
        }
        self.reveal_active_tab();
        self.persist_session(cx);
    }

    pub(super) fn scroll_selected_tree_item_into_view(&self) {
        let Some(selected) = self.selected_tree_item else {
            return;
        };
        if let Some(index) = self
            .visible_tree_rows
            .iter()
            .position(|row| row.item == selected)
        {
            self.tree_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    pub(super) fn reveal_request_in_sidebar(&mut self, key: RequestKey) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let workspace = loaded.workspace();
        let item = WorkspaceItemRef::Request(key);
        let query = self.tree_search.trim();
        if !query.is_empty()
            && !self
                .tree_search_matches
                .as_ref()
                .is_some_and(|matches| matches.contains(item))
        {
            return;
        }
        let Some(ancestors) = workspace.request_ancestor_folders(key) else {
            return;
        };
        let ancestors = ancestors.to_vec();
        let mut expanded = false;
        for folder in ancestors {
            if !self.shell.folder_is_expanded(folder) {
                self.shell.expand_folder(folder);
                expanded = true;
            }
        }
        if expanded {
            self.rebuild_visible_tree_rows_after_visibility_change();
        }
        self.scroll_selected_tree_item_into_view();
    }

    pub(super) fn select_tree_offset(&mut self, offset: isize, cx: &mut Context<Self>) {
        if self.visible_tree_rows.is_empty() {
            return;
        }
        let current = self
            .selected_tree_item
            .and_then(|item| {
                self.visible_tree_rows
                    .iter()
                    .position(|row| row.item == item)
            })
            .unwrap_or(if offset < 0 {
                self.visible_tree_rows.len()
            } else {
                0
            });
        let next = current
            .checked_add_signed(offset)
            .unwrap_or(0)
            .min(self.visible_tree_rows.len() - 1);
        self.selected_tree_item = Some(self.visible_tree_rows[next].item);
        self.tree_scroll
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(super) fn activate_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        match self.selected_tree_item {
            Some(WorkspaceItemRef::Request(key)) => self.select_request(key, cx),
            Some(WorkspaceItemRef::Folder(key)) => {
                self.shell.toggle_folder(key);
                self.rebuild_visible_tree_rows_after_visibility_change();
                self.persist_session(cx);
                cx.notify();
            }
            None => self.select_tree_offset(0, cx),
        }
    }

    pub(super) fn collapse_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected_tree_item else {
            return;
        };
        match selected {
            WorkspaceItemRef::Folder(key) if self.shell.folder_is_expanded(key) => {
                self.shell.collapse_folder(key);
                self.rebuild_visible_tree_rows_after_visibility_change();
                self.persist_session(cx);
                cx.notify();
            }
            _ => {
                let Some(loaded) = &self.loaded_workspace else {
                    return;
                };
                if let Some((Some(parent), _)) = item_position(loaded.workspace(), selected) {
                    self.selected_tree_item = Some(WorkspaceItemRef::Folder(parent));
                    cx.notify();
                }
            }
        }
    }

    pub(super) fn expand_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        let Some(WorkspaceItemRef::Folder(key)) = self.selected_tree_item else {
            return;
        };
        if !self.shell.folder_is_expanded(key) {
            self.shell.toggle_folder(key);
            self.rebuild_visible_tree_rows_after_visibility_change();
            self.persist_session(cx);
            cx.notify();
        }
    }
}
