use super::*;

impl ProbeApp {
    pub(super) fn close_workspace_now(&mut self, cx: &mut Context<Self>) {
        self.capture_selected_environment();
        self.execution.clear();
        self.response_viewer.clear();
        self.pending_environment_saves.clear();
        self.environment_save_workspace_path = None;
        self.loaded_workspace = None;
        self.workspace_path = None;
        self.shell.reset_for_workspace();
        self.shell.select_environment(None);
        self.reset_collection_ui();
        self.persistence.clear();
        self.filesystem_watch_task = None;
        self.filesystem_watcher = None;
        self.visible_tree_rows.clear();
        self.session.clear_active_collection();
        self.clear_toasts();
        self.persist_session(cx);
        cx.notify();
    }

    pub(super) fn select_environment(
        &mut self,
        environment: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.shell.select_environment(environment);
        self.persist_session(cx);
        cx.notify();
    }

    pub(super) fn show_environment_dialog_error(
        &mut self,
        message: impl Into<String>,
        resolution: EnvironmentDialogErrorResolution,
        cx: &mut Context<Self>,
    ) {
        self.clear_environment_dialog_error(cx);
        let toast_id = self.show_toast(ToastIntent::Error, message, cx);
        self.environment_dialog_error = Some(EnvironmentDialogError::new(toast_id, resolution));
    }

    pub(super) fn environment_manager_draft_has_required_names(&self) -> bool {
        self.environment_manager_dialog
            .as_ref()
            .is_some_and(|dialog| {
                !dialog.draft.name.trim().is_empty()
                    && dialog.draft.variables.iter().all(|variable| {
                        !matches!(
                            variable,
                            EnvironmentVariable::Plain(variable)
                                if variable.name.as_deref().is_none_or(|name| name.trim().is_empty())
                        )
                    })
            })
    }

    pub(super) fn clear_resolved_environment_dialog_error(&mut self, cx: &mut Context<Self>) {
        let Some(resolution) = self
            .environment_dialog_error
            .as_ref()
            .map(|error| error.resolution)
        else {
            return;
        };
        let resolved = match resolution {
            EnvironmentDialogErrorResolution::ManagerDraftValid => {
                self.environment_manager_draft_has_required_names()
            }
            EnvironmentDialogErrorResolution::CreateNameValid => self
                .create_environment_dialog
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty()),
            EnvironmentDialogErrorResolution::ManagerClean => !self.environment_manager_is_dirty(),
            EnvironmentDialogErrorResolution::SavesIdle => {
                self.environment_save_task.is_none()
                    && self.request_save_task.is_none()
                    && self.structure_task.is_none()
                    && self.pending_environment_saves.is_empty()
            }
            EnvironmentDialogErrorResolution::Manual => false,
        };
        if resolved {
            self.clear_environment_dialog_error(cx);
        }
    }

    pub(super) fn open_environment_manager_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .shell
            .selected_environment()
            .and_then(|name| {
                loaded
                    .workspace()
                    .environments()
                    .iter()
                    .find(|environment| environment.name == name)
            })
            .or_else(|| loaded.workspace().environments().first());
        let Some(selected) = selected else {
            self.open_create_environment_dialog(window, cx);
            return;
        };
        self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(selected));
        self.clear_environment_dialog_error(cx);
        self.environment_manager_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn request_close_environment_manager_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_application_dialog(ApplicationDialog::UnsavedEnvironment, window, cx);
            return;
        }
        self.close_environment_manager_dialog(window, cx);
    }

    pub(super) fn close_environment_manager_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        self.discard_environment_manager_dialog();
        self.clear_environment_dialog_error(cx);
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(super) fn discard_environment_manager_dialog(&mut self) {
        self.transient.environment_manager_context_menu = None;
        self.environment_manager_close_after_save = false;
        self.environment_manager_dialog = None;
    }

    pub(super) fn environment_manager_reload_snapshot(
        &self,
        old: &LoadedWorkspace,
    ) -> Option<(EnvironmentManagerDialog, Option<Environment>)> {
        let dialog = self.environment_manager_dialog.as_ref()?;
        let original = old
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
            .cloned();
        Some((dialog.clone(), original))
    }

    pub(super) fn sync_environment_manager_after_reload(
        &mut self,
        previous: Option<(EnvironmentManagerDialog, Option<Environment>)>,
        cx: &mut Context<Self>,
    ) {
        let Some((dialog, previous_original)) = previous else {
            return;
        };
        self.transient.environment_manager_context_menu = None;
        let Some(disk) = self.loaded_workspace.as_ref().and_then(|loaded| {
            loaded
                .workspace()
                .environments()
                .iter()
                .find(|environment| environment.name == dialog.original_name)
                .cloned()
        }) else {
            self.discard_environment_manager_dialog();
            return;
        };
        let dirty = previous_original
            .as_ref()
            .is_none_or(|original| original != &dialog.draft);
        let environment_changed_on_disk = previous_original
            .as_ref()
            .is_none_or(|original| original != &disk);
        if !dirty {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&disk));
            return;
        }
        if environment_changed_on_disk {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&disk));
            self.show_environment_dialog_error(
                "This environment changed on disk. Unsaved environment edits were discarded."
                    .to_owned(),
                EnvironmentDialogErrorResolution::Manual,
                cx,
            );
            return;
        }
        self.environment_manager_dialog = Some(dialog);
    }

    pub(super) fn apply_environment_manager_draft(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut EnvironmentManagerDialog),
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        if let Some(dialog) = self.environment_manager_dialog.as_mut() {
            update(dialog);
            cx.notify();
        }
    }

    pub(super) fn select_environment_manager_environment(
        &mut self,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let Some(original) = loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
        else {
            return;
        };
        if &dialog.draft != original {
            self.show_environment_dialog_error(
                "Save or cancel the current environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
                cx,
            );
            cx.notify();
            return;
        }
        if let Some(environment) = loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == name)
        {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(environment));
            self.clear_environment_dialog_error(cx);
            cx.notify();
        }
    }

    pub(super) fn save_environment_manager_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
            || !self.pending_environment_saves.is_empty()
        {
            self.show_environment_dialog_error(
                "Wait for the current save to finish.",
                EnvironmentDialogErrorResolution::SavesIdle,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return;
        };
        let mut replacement = dialog.draft.clone();
        replacement.name = replacement.name.trim().to_owned();
        for variable in &mut replacement.variables {
            if let EnvironmentVariable::Plain(variable) = variable
                && let Some(name) = variable.name.as_mut()
            {
                *name = name.trim().to_owned();
            }
        }
        let invalid_variable = replacement.variables.iter().any(|variable| {
            matches!(
                variable,
                EnvironmentVariable::Plain(variable)
                    if variable.name.as_deref().is_none_or(str::is_empty)
            )
        });
        if replacement.name.is_empty() || invalid_variable {
            self.show_environment_dialog_error(
                "Environment and variable names are required.",
                EnvironmentDialogErrorResolution::ManagerDraftValid,
                cx,
            );
            cx.notify();
            return;
        }
        let original_name = dialog.original_name.clone();
        let saved_name = replacement.name.clone();
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let prepared = match loaded.prepare_environment_replace(&original_name, replacement) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not save environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                    cx,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.complete_environment_replace(saved);
                            let environment = loaded
                                .workspace()
                                .environments()
                                .iter()
                                .find(|environment| environment.name == saved_name)
                                .cloned();
                            if let Some(environment) = environment {
                                if view.shell.selected_environment() == Some(original_name.as_str())
                                {
                                    view.select_environment(Some(environment.name.clone()), cx);
                                }
                                if view.environment_manager_close_after_save {
                                    view.close_environment_manager_dialog(window, cx);
                                } else {
                                    view.environment_manager_dialog =
                                        Some(EnvironmentManagerDialog::new(&environment));
                                }
                            }
                        }
                        view.environment_manager_close_after_save = false;
                        view.clear_environment_dialog_error(cx);
                        view.show_toast(ToastIntent::Success, "Environment saved.", cx);
                    }
                    Err(error) => {
                        view.environment_manager_close_after_save = false;
                        view.show_environment_dialog_error(
                            format!("Could not save environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                            cx,
                        );
                    }
                }
                view.environment_save_workspace_path = None;
                view.start_next_request_save(window, cx);
                view.start_next_environment_save(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn confirm_delete_environment(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_application_dialog(
            ApplicationDialog::DeleteEnvironment {
                name,
                detail: "The environment and its variables will be removed. This cannot be undone."
                    .to_owned(),
            },
            window,
            cx,
        );
    }

    pub(super) fn delete_selected_environment_from_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        let Some(name) = self
            .environment_manager_dialog
            .as_ref()
            .map(|dialog| dialog.original_name.clone())
        else {
            return;
        };
        self.confirm_delete_environment(name, window, cx);
    }

    pub(super) fn delete_environment(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            self.show_environment_dialog_error(
                "Wait for the current save to finish.",
                EnvironmentDialogErrorResolution::SavesIdle,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let prepared = match loaded.prepare_environment_delete(&name) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not delete environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                    cx,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        let close_manager = view.complete_deleted_environment(saved, &name, cx);
                        view.clear_environment_dialog_error(cx);
                        if close_manager {
                            view.close_environment_manager_dialog(window, cx);
                        }
                    }
                    Err(error) => {
                        view.show_environment_dialog_error(
                            format!("Could not delete environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                            cx,
                        );
                    }
                }
                view.environment_save_workspace_path = None;
                view.start_next_request_save(window, cx);
                view.start_next_environment_save(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    pub(super) fn complete_deleted_environment(
        &mut self,
        saved: CompletedEnvironmentDelete,
        name: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.environment_save_workspace_path != self.workspace_path
            || self.loaded_workspace.is_none()
        {
            return false;
        }
        let deleted_current = self
            .environment_manager_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.original_name == name);
        let deleted_index = self.loaded_workspace.as_ref().and_then(|loaded| {
            loaded
                .workspace()
                .environments()
                .iter()
                .position(|environment| environment.name == name)
        });
        self.loaded_workspace
            .as_mut()
            .expect("workspace was present")
            .complete_environment_delete(saved);
        if self.shell.selected_environment() == Some(name) {
            self.select_environment(None, cx);
        }
        if !deleted_current {
            return false;
        }
        let next = self.loaded_workspace.as_ref().and_then(|loaded| {
            let environments = loaded.workspace().environments();
            deleted_index.and_then(|index| {
                environments
                    .get(index)
                    .or_else(|| environments.get(index.saturating_sub(1)))
                    .cloned()
            })
        });
        match next {
            Some(environment) => {
                self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&environment));
                false
            }
            None => true,
        }
    }

    pub(super) fn environment_manager_is_dirty(&self) -> bool {
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return false;
        };
        let Some(loaded) = self.loaded_workspace.as_ref() else {
            return false;
        };
        loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
            .is_none_or(|environment| environment != &dialog.draft)
    }

    pub(super) fn environment_manager_save_disabled(&self) -> bool {
        self.environment_save_task.is_some() || !self.environment_manager_is_dirty()
    }

    pub(super) fn restore_environment_dialog_focus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_manager_dialog.is_some() {
            self.environment_manager_dialog_focus.focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    pub(super) fn open_create_environment_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.loaded_workspace.is_none()
            || self.structure_task.is_some()
            || self.environment_save_task.is_some()
            || self.request_save_task.is_some()
        {
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_environment_dialog_error(
                "Save or discard unsaved environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
                cx,
            );
            cx.notify();
            return;
        }
        self.structure_dialog = None;
        self.clear_environment_dialog_error(cx);
        self.create_environment_dialog = Some(String::new());
        self.create_environment_dialog_focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_create_environment_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.clear_environment_dialog_error(cx);
        self.restore_environment_dialog_focus(window, cx);
        cx.notify();
    }

    pub(super) fn submit_create_environment_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.create_environment_dialog.as_ref() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.show_environment_dialog_error(
                "Environment name is required.",
                EnvironmentDialogErrorResolution::CreateNameValid,
                cx,
            );
            cx.notify();
            return;
        }
        self.create_named_environment(name, window, cx);
    }

    pub(super) fn create_named_environment(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            self.show_environment_dialog_error(
                "Wait for the current save before creating an environment.",
                EnvironmentDialogErrorResolution::SavesIdle,
                cx,
            );
            cx.notify();
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_environment_dialog_error(
                "Save or discard unsaved environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
                cx,
            );
            cx.notify();
            return;
        }
        let Some(loaded) = self.loaded_workspace.as_mut() else {
            return;
        };
        let prepared = match loaded.prepare_environment_create(name.clone(), None) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not create environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                    cx,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.clear_environment_dialog_error(cx);
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.complete_environment_create(saved);
                            view.environment_manager_dialog = loaded
                                .workspace()
                                .environments()
                                .iter()
                                .find(|environment| environment.name == name)
                                .map(EnvironmentManagerDialog::new);
                            view.select_environment(Some(name), cx);
                        }
                        view.environment_save_workspace_path = None;
                        view.create_environment_dialog = None;
                        view.clear_environment_dialog_error(cx);
                        view.restore_environment_dialog_focus(window, cx);
                        view.start_next_request_save(window, cx);
                        view.start_next_environment_save(window, cx);
                    }
                    Err(error) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.revert_created_environment(&name);
                            if view.shell.selected_environment() == Some(name.as_str()) {
                                view.select_environment(None, cx);
                            }
                        }
                        view.environment_save_workspace_path = None;
                        view.pending_close = None;
                        view.show_environment_dialog_error(
                            format!("Could not create environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                            cx,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}
