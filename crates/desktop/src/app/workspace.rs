use super::*;

impl ProbeApp {
    pub(super) fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.session_store.clone() else {
            return;
        };
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx.background_spawn(async move { store.load() }).await;
                let _ = view.update_in(cx, |view, window, cx| match result {
                    Ok(state) => {
                        let active_path = state.active_collection.clone();
                        view.session = state.clone();
                        if let Some(path) = active_path {
                            view.load_workspace_path(path, Some(state), window, cx);
                        }
                    }
                    Err(error) => {
                        view.show_toast(
                            ToastIntent::Error,
                            format!("Could not restore the previous desktop session: {error}"),
                            cx,
                        );
                    }
                });
            })
            .detach();
    }

    pub(super) fn load_workspace_path(
        &mut self,
        path: PathBuf,
        restored_state: Option<SessionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_selected_environment();
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let load_path = path.clone();
                let result = cx
                    .background_spawn(async move {
                        let canonical_path = fs::canonicalize(&load_path).map_err(|error| {
                            format!("failed to locate {}: {error}", load_path.display())
                        })?;
                        let workspace =
                            load_workspace(&canonical_path).map_err(|error| error.to_string())?;
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((canonical_path, workspace)) => {
                            view.set_workspace(canonical_path, workspace);
                            if let Some(state) = restored_state {
                                view.session = state;
                                view.restore_shell_state(cx);
                            }
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                        }
                        Err(error) => {
                            if let Some(state) = restored_state {
                                view.session = state;
                                view.session.clear_active_collection();
                                view.persist_session(cx);
                                view.show_toast(
                                    ToastIntent::Error,
                                    format!("Could not restore the previous collection. {error}"),
                                    cx,
                                );
                            } else {
                                view.show_toast(ToastIntent::Error, error, cx);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub(super) fn request_load_workspace(
        &mut self,
        path: PathBuf,
        restored_state: Option<SessionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(
                dirty,
                PendingClose::Open {
                    path,
                    restored_state,
                },
                window,
                cx,
            );
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Open {
                path,
                restored_state,
            });
            self.start_next_environment_save(window, cx);
            return;
        }
        self.load_workspace_path(path, restored_state, window, cx);
    }

    pub(super) fn request_create_workspace(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Create { path }, window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Create { path });
            self.start_next_environment_save(window, cx);
            return;
        }
        self.create_workspace_path(path, window, cx);
    }

    pub(super) fn create_workspace_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_selected_environment();
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx
                    .background_spawn(async move {
                        let workspace = create_bundled_workspace(&path, None, true)
                            .map_err(|error| error.to_string())?;
                        let canonical_path = workspace
                            .source_path()
                            .ok_or_else(|| {
                                format!(
                                    "created collection at {} has no filesystem path",
                                    path.display()
                                )
                            })?
                            .to_owned();
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((canonical_path, workspace)) => {
                            view.set_workspace(canonical_path, workspace);
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                        }
                        Err(error) => {
                            view.show_toast(ToastIntent::Error, error, cx);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub(super) fn has_pending_environment_work(&self) -> bool {
        self.environment_save_task.is_some() || !self.pending_environment_saves.is_empty()
    }

    pub(super) fn finish_pending_close_if_idle(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.request_save_task.is_some()
            || self.environment_save_task.is_some()
            || self.persistence.has_outstanding_saves()
        {
            return;
        }
        if !self.pending_environment_saves.is_empty() {
            self.start_next_environment_save(window, cx);
            return;
        }
        if let Some(pending) = self.pending_close.take() {
            let dirty = match &pending {
                PendingClose::Tab(key) => self
                    .request_is_dirty(*key)
                    .then_some(vec![*key])
                    .unwrap_or_default(),
                PendingClose::OtherTabs { keep } => self.other_dirty_tab_keys(*keep),
                PendingClose::Workspace
                | PendingClose::Window
                | PendingClose::Quit
                | PendingClose::Open { .. }
                | PendingClose::Create { .. }
                | PendingClose::Import(_) => self.dirty_keys(),
            };
            if dirty.is_empty() {
                self.finish_pending_close(pending, window, cx);
            } else {
                self.prompt_unsaved(dirty, pending, window, cx);
            }
        }
    }

    pub(super) fn set_workspace(&mut self, path: PathBuf, workspace: LoadedWorkspace) {
        self.persistence
            .reset(workspace.requests().iter().filter_map(|located| {
                workspace
                    .workspace()
                    .request(located.key())
                    .cloned()
                    .map(|request| (located.key(), request))
            }));
        self.execution.clear();
        self.response_viewer.clear();
        self.loaded_workspace = Some(workspace);
        self.workspace_path = Some(path);
        self.shell.reset_for_workspace();
        self.reset_collection_ui();
        self.pending_environment_saves.clear();
        self.environment_save_workspace_path = None;
        self.restore_selected_environment();
        self.rebuild_visible_tree_rows();
        self.clear_toasts();
    }

    pub(super) fn start_workspace_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filesystem_watch_task = None;
        self.filesystem_watcher = None;
        let Some(path) = self.workspace_path.clone() else {
            return;
        };
        let watcher = match WorkspaceWatcher::start(&path) {
            Ok(watcher) => watcher,
            Err(error) => {
                self.show_toast(
                    ToastIntent::Error,
                    format!("Could not watch this collection: {error}"),
                    cx,
                );
                return;
            }
        };
        let WorkspaceWatcher {
            watcher,
            receiver,
            workspace_path,
            ..
        } = watcher;
        self.filesystem_watcher = Some(watcher);
        #[cfg(not(test))]
        let mut receiver = receiver;
        #[cfg(test)]
        let receiver = receiver;
        let view = cx.weak_entity();
        self.filesystem_watch_task = Some(window.spawn(cx, async move |cx| {
            loop {
                let (events, watch_error, disconnected) = {
                    #[cfg(not(test))]
                    {
                        let Some(first) = receiver.recv().await else {
                            return;
                        };
                        cx.background_executor().timer(WATCH_DEBOUNCE).await;
                        let mut events = Vec::new();
                        let mut watch_error = None;
                        match first {
                            Ok(event) => events.push(event),
                            Err(error) => watch_error = Some(error.to_string()),
                        }
                        while let Ok(event) = receiver.try_recv() {
                            match event {
                                Ok(event) => events.push(event),
                                Err(error) => watch_error = Some(error.to_string()),
                            }
                        }
                        (events, watch_error, false)
                    }

                    #[cfg(test)]
                    {
                        loop {
                            let mut events = Vec::new();
                            let mut watch_error = None;
                            let mut disconnected =
                                drain_watch_events(&receiver, &mut events, &mut watch_error);
                            if events.is_empty() && watch_error.is_none() {
                                if disconnected {
                                    return;
                                }
                                cx.background_executor().timer(WATCH_POLL).await;
                                continue;
                            }
                            cx.background_executor().timer(WATCH_DEBOUNCE).await;
                            disconnected |=
                                drain_watch_events(&receiver, &mut events, &mut watch_error);
                            break (events, watch_error, disconnected);
                        }
                    }
                };
                if let Some(error) = watch_error {
                    let _ = view.update_in(cx, |view, _, cx| {
                        view.show_toast(
                            ToastIntent::Error,
                            format!("Collection watcher error: {error}"),
                            cx,
                        );
                    });
                }
                if !events
                    .iter()
                    .any(|event| event_affects_workspace(event, &workspace_path))
                {
                    if disconnected {
                        return;
                    }
                    continue;
                }
                let hints = rename_hints(&events, &workspace_path);
                let reload_path = workspace_path.clone();
                let result = cx
                    .background_spawn(async move { load_workspace(&reload_path) })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    if view.workspace_path.as_ref() != Some(&workspace_path) {
                        return;
                    }
                    if view.structure_task.is_some() {
                        return;
                    }
                    match result {
                        Ok(fresh) => view.reconcile_filesystem_workspace(fresh, hints, window, cx),
                        Err(error) => {
                            view.show_toast(
                                ToastIntent::Warning,
                                format!(
                                    "The collection changed on disk but is not yet valid: {error}. The last valid version is still open."
                                ),
                                cx,
                            );
                        }
                    }
                });
                if disconnected {
                    return;
                }
            }
        }));
    }

    pub(super) fn local_request_states(&self) -> Vec<LocalRequestState> {
        let Some(loaded) = &self.loaded_workspace else {
            return Vec::new();
        };
        loaded
            .requests()
            .iter()
            .filter_map(|located| {
                Some(LocalRequestState {
                    selector: located.selector().to_owned(),
                    baseline: self.persistence.saved_request(located.key())?.clone(),
                    local: loaded.workspace().request(located.key())?.clone(),
                })
            })
            .collect()
    }

    pub(super) fn reconcile_filesystem_workspace(
        &mut self,
        fresh: LoadedWorkspace,
        rename_hints: BTreeMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match reconcile(self.local_request_states(), fresh, &rename_hints) {
            ReconcileResult::Applied(reconciled) => {
                self.apply_reconciled_workspace(*reconciled, cx);
            }
            ReconcileResult::Conflicted(conflicts) => {
                self.prompt_filesystem_conflict(conflicts, window, cx);
            }
        }
    }

    pub(super) fn show_application_dialog(
        &mut self,
        dialog: ApplicationDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.application_dialog.is_some() {
            self.enqueue_application_dialog(dialog);
            return;
        }
        self.structure_dialog = None;
        self.create_environment_dialog = None;
        self.dismiss_transient_surfaces();
        self.application_dialog = Some(dialog);
        self.application_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn enqueue_application_dialog(&mut self, dialog: ApplicationDialog) {
        if let ApplicationDialog::FilesystemConflict { path, .. } = &dialog
            && (matches!(
                &self.application_dialog,
                Some(ApplicationDialog::FilesystemConflict {
                    path: current_path,
                    ..
                }) if current_path == path
            ) || self.pending_application_dialogs.iter().any(|pending| {
                matches!(
                    pending,
                    ApplicationDialog::FilesystemConflict {
                        path: pending_path,
                        ..
                    } if pending_path == path
                )
            }))
        {
            return;
        }
        self.pending_application_dialogs.push_back(dialog);
    }

    pub(super) fn show_next_application_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.application_dialog.is_none()
            && let Some(dialog) = self.pending_application_dialogs.pop_front()
        {
            self.show_application_dialog(dialog, window, cx);
        }
    }

    pub(super) fn dismiss_transient_surfaces(&mut self) {
        self.transient.desktop_menu_open = None;
        self.transient.desktop_submenu_open = None;
        self.transient.workspace_switcher_open = false;
        self.transient.workspace_import_submenu_open = false;
        self.transient.sidebar_import_menu_open = false;
        self.transient.structure_add_menu_open = false;
        self.transient.tree_context_menu = None;
        self.transient.tab_context_menu = None;
        self.transient.environment_manager_context_menu = None;
    }

    pub(super) fn open_desktop_menu(&mut self, menu: DesktopMenu, cx: &mut Context<Self>) {
        self.transient.desktop_menu_open = Some(menu);
        self.transient.desktop_submenu_open = None;
        cx.notify();
    }

    pub(super) fn close_desktop_menu(&mut self, cx: &mut Context<Self>) {
        self.transient.desktop_menu_open = None;
        self.transient.desktop_submenu_open = None;
        cx.notify();
    }

    pub(super) fn handle_application_dialog_action(
        &mut self,
        action: ApplicationDialogAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.application_dialog.take() else {
            return;
        };
        match (dialog, action) {
            (ApplicationDialog::Unsaved { keys, pending }, ApplicationDialogAction::Save) => {
                self.pending_close = Some(pending);
                self.persistence.enqueue(keys);
                self.start_next_request_save(window, cx);
            }
            (ApplicationDialog::Unsaved { keys, pending }, ApplicationDialogAction::Discard) => {
                self.discard_dirty_requests(&keys);
                self.finish_pending_close(pending, window, cx);
            }
            (ApplicationDialog::UnsavedEnvironment, ApplicationDialogAction::Save) => {
                self.environment_manager_close_after_save = true;
                self.save_environment_manager_dialog(window, cx);
                if self.environment_save_task.is_none() {
                    self.environment_manager_close_after_save = false;
                    self.restore_environment_dialog_focus(window, cx);
                }
            }
            (ApplicationDialog::UnsavedEnvironment, ApplicationDialogAction::Discard) => {
                self.close_environment_manager_dialog(window, cx);
            }
            (ApplicationDialog::Delete { kind, selector, .. }, ApplicationDialogAction::Delete) => {
                let operation = match kind {
                    ItemKind::Request => StructureOperation::DeleteRequest { selector },
                    ItemKind::Folder => StructureOperation::DeleteFolder { selector },
                };
                self.apply_structure(operation, window, cx);
            }
            (
                ApplicationDialog::DeleteEnvironment { name, .. },
                ApplicationDialogAction::Delete,
            ) => self.delete_environment(name, window, cx),
            (
                ApplicationDialog::FilesystemConflict { path, .. },
                ApplicationDialogAction::UseDisk,
            ) => self.reload_conflicted_workspace(path, window, cx),
            (ApplicationDialog::FilesystemConflict { .. }, ApplicationDialogAction::KeepLocal) => {
                self.show_toast(
                    ToastIntent::Warning,
                    "Kept local edits. Probe will not overwrite the changed disk files; resolve the conflict before saving.",
                    cx,
                );
            }
            (
                ApplicationDialog::SelectYaakWorkspace {
                    preview,
                    workspaces,
                },
                ApplicationDialogAction::SelectWorkspace(index),
            ) => {
                if let Some(workspace) = workspaces.get(index) {
                    self.convert_yaak_import(preview, workspace.id.clone(), false, window, cx);
                } else {
                    self.loading = false;
                    cx.notify();
                }
            }
            (
                ApplicationDialog::ConfirmPartialYaakImport {
                    preview,
                    workspace_id,
                    ..
                },
                ApplicationDialogAction::ImportSupportedData,
            ) => self.convert_yaak_import(preview, workspace_id, true, window, cx),
            (
                ApplicationDialog::ConfirmPartialPostmanImport { preview, .. },
                ApplicationDialogAction::ImportSupportedData,
            ) => self.convert_postman_import(*preview, true, window, cx),
            (
                ApplicationDialog::SelectYaakWorkspace { .. }
                | ApplicationDialog::ConfirmPartialYaakImport { .. }
                | ApplicationDialog::ConfirmPartialPostmanImport { .. },
                ApplicationDialogAction::Cancel,
            ) => {
                self.loading = false;
                self.focus_handle.focus(window, cx);
                cx.notify();
            }
            (_, ApplicationDialogAction::Cancel) => {
                self.restore_environment_dialog_focus(window, cx);
                cx.notify();
            }
            (dialog, _) => {
                self.application_dialog = Some(dialog);
                cx.notify();
            }
        }
        self.show_next_application_dialog(window, cx);
    }

    pub(super) fn reload_conflicted_workspace(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_path != path {
            return;
        }
        let Some(path) = path else {
            return;
        };
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx.background_spawn(async move { load_workspace(path) }).await;
                let _ = view.update_in(cx, |view, _, cx| {
                    view.loading = false;
                    match result {
                        Ok(workspace) => {
                            let clean_local = view
                                .local_request_states()
                                .into_iter()
                                .map(|mut state| {
                                    state.local.clone_from(&state.baseline);
                                    state
                                })
                                .collect();
                            if let ReconcileResult::Applied(reconciled) =
                                reconcile(clean_local, workspace, &BTreeMap::new())
                            {
                                view.apply_reconciled_workspace(*reconciled, cx);
                                view.show_toast(
                                    ToastIntent::Warning,
                                    "Reloaded the collection from disk; conflicting local edits were discarded.",
                                    cx,
                                );
                            }
                        }
                        Err(error) => {
                            view.show_toast(
                                ToastIntent::Error,
                                format!("Could not reload the collection from disk: {error}"),
                                cx,
                            );
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    pub(super) fn prompt_filesystem_conflict(
        &mut self,
        conflicts: Vec<SynchronizationConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = conflicts
            .iter()
            .take(3)
            .map(SynchronizationConflict::description)
            .collect::<Vec<_>>()
            .join("; ");
        self.show_application_dialog(
            ApplicationDialog::FilesystemConflict {
                path: self.workspace_path.clone(),
                detail: format!(
                    "{detail}. Choose Use Disk to discard the conflicting local edits, or Keep Local to retain them without overwriting disk."
                ),
            },
            window,
            cx,
        );
    }

    pub(super) fn reset_collection_ui(&mut self) {
        self.selected_tree_item = None;
        self.structure_dialog = None;
        self.create_environment_dialog = None;
        self.discard_environment_manager_dialog();
        self.application_dialog = None;
        self.pending_application_dialogs.clear();
        self.dismiss_transient_surfaces();
        self.clear_tree_drag();
        self.tree_search.clear();
        self.tree_search_matches = None;
        self.request_editor.clear();
    }

    pub(super) fn snapshot_shell_selectors(&self, old: &LoadedWorkspace) -> ShellSelectors {
        ShellSelectors {
            tab_selectors: self
                .shell
                .tabs()
                .iter()
                .filter_map(|key| old.request_selector(*key).map(str::to_owned))
                .collect(),
            active_selector: self
                .shell
                .active_tab()
                .and_then(|key| old.request_selector(key))
                .map(str::to_owned),
            folder_selectors: self
                .shell
                .collapsed_folders()
                .filter_map(|key| old.folder_selector(key).map(str::to_owned))
                .collect(),
            selected: self.selected_tree_item.and_then(|item| match item {
                WorkspaceItemRef::Request(key) => old
                    .request_selector(key)
                    .map(|selector| (ItemKind::Request, selector.to_owned())),
                WorkspaceItemRef::Folder(key) => old
                    .folder_selector(key)
                    .map(|selector| (ItemKind::Folder, selector.to_owned())),
            }),
        }
    }

    pub(super) fn install_reloaded_workspace(
        &mut self,
        workspace: LoadedWorkspace,
        baselines: Vec<(RequestKey, HttpRequest)>,
        key_remaps: &BTreeMap<RequestKey, RequestKey>,
    ) {
        self.persistence.reset(baselines);
        self.loaded_workspace = Some(workspace);
        self.shell.reset_for_workspace();
        self.execution.remap_requests(key_remaps);
        self.response_viewer.remap_requests(key_remaps);
        self.request_editor.remap_requests(key_remaps);
    }

    pub(super) fn remap_structure_dialog(&mut self, remaps: &BTreeMap<String, String>) {
        let Some(dialog) = self.structure_dialog.as_mut() else {
            return;
        };
        let Some(loaded) = self.loaded_workspace.as_ref() else {
            self.structure_dialog = None;
            return;
        };

        let mut target_exists = true;
        match &mut dialog.mode {
            StructureDialogMode::CreateRequest | StructureDialogMode::CreateFolder => {}
            StructureDialogMode::Rename { kind, selector }
            | StructureDialogMode::Move { kind, selector } => {
                if let Some(mapped) = remaps.get(selector) {
                    selector.clone_from(mapped);
                }
                target_exists = match kind {
                    ItemKind::Request => loaded.request_key(selector).is_some(),
                    ItemKind::Folder => loaded.folder_key(selector).is_some(),
                };
            }
        }

        if !target_exists {
            self.structure_dialog = None;
            return;
        }

        if !dialog.parent.is_empty() {
            if let Some(mapped) = remaps.get(&dialog.parent) {
                dialog.parent.clone_from(mapped);
            }
            if loaded.folder_key(&dialog.parent).is_none() {
                self.structure_dialog = None;
            }
        }
    }

    pub(super) fn restore_shell_selectors(
        &mut self,
        remaps: &BTreeMap<String, String>,
        selectors: ShellSelectors,
    ) {
        let loaded = self
            .loaded_workspace
            .as_ref()
            .expect("workspace was replaced");
        for selector in selectors.tab_selectors {
            if let Some(key) = remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
            {
                self.shell.open_request(key);
            }
        }
        if let Some(selector) = selectors.active_selector
            && let Some(key) = remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
        {
            self.shell.open_request(key);
        }
        for selector in selectors.folder_selectors {
            let selector = remaps
                .get(&selector)
                .map_or(selector.as_str(), String::as_str);
            if let Some(key) = loaded.folder_key(selector) {
                self.shell.collapse_folder(key);
            }
        }
        self.selected_tree_item = selectors.selected.and_then(|(kind, selector)| match kind {
            ItemKind::Request => remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
                .map(WorkspaceItemRef::Request),
            ItemKind::Folder => {
                let selector = remaps
                    .get(&selector)
                    .map_or(selector.as_str(), String::as_str);
                loaded.folder_key(selector).map(WorkspaceItemRef::Folder)
            }
        });
    }

    pub(super) fn apply_reconciled_workspace(
        &mut self,
        mut reconciled: ReconciledWorkspace,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.loaded_workspace.as_ref() else {
            return;
        };
        let selectors = self.snapshot_shell_selectors(old);
        let key_remaps =
            request_key_remaps(old, &reconciled.workspace, &reconciled.selector_remaps);
        let baselines = reconciled
            .workspace
            .requests()
            .iter()
            .filter_map(|located| {
                reconciled
                    .disk_baselines
                    .remove(located.selector())
                    .map(|request| (located.key(), request))
            })
            .collect::<Vec<_>>();
        let environment_manager_reload = self.environment_manager_reload_snapshot(old);
        self.install_reloaded_workspace(reconciled.workspace, baselines, &key_remaps);
        self.restore_shell_selectors(&reconciled.selector_remaps, selectors);
        self.remap_structure_dialog(&reconciled.selector_remaps);
        self.create_environment_dialog = None;
        self.sync_environment_manager_after_reload(environment_manager_reload, cx);
        if self.shell.selected_environment().is_some_and(|name| {
            !self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced")
                .workspace()
                .environments()
                .iter()
                .any(|environment| environment.name == name)
        }) {
            self.shell.select_environment(None);
        }
        self.rebuild_visible_tree_rows();
        self.persist_session(cx);
        cx.notify();
    }
}
