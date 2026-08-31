use super::*;

impl ProbeApp {
    pub(super) fn restore_shell_state(&mut self, cx: &mut Context<Self>) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let tabs: Vec<_> = self
            .session
            .open_tabs
            .iter()
            .filter_map(|selector| loaded.request_key(selector))
            .collect();
        let active_tab = self
            .session
            .active_tab
            .as_deref()
            .and_then(|selector| loaded.request_key(selector));
        let collapsed_folders: Vec<_> = self
            .session
            .collapsed_folders
            .iter()
            .filter_map(|selector| loaded.folder_key(selector))
            .collect();

        self.shell.restore_pane_sizes(
            self.session.sidebar_width,
            self.session.response_height,
            self.session.response_width,
        );
        self.shell.sidebar_collapsed = self.session.sidebar_collapsed;
        self.shell
            .set_pane_layout(if self.session.horizontal_panes {
                PaneLayout::Horizontal
            } else {
                PaneLayout::Vertical
            });
        self.refresh_system_menu(cx);
        for key in tabs {
            self.shell.open_request(key);
        }
        if let Some(key) = active_tab {
            self.shell.open_request(key);
            self.selected_tree_item = Some(WorkspaceItemRef::Request(key));
        }
        for key in collapsed_folders {
            self.shell.collapse_folder(key);
        }
        self.rebuild_visible_tree_rows();
        self.reveal_active_tab();
    }

    pub(super) fn capture_session(&mut self) {
        self.session.sidebar_width = self.shell.sidebar_width;
        self.session.sidebar_collapsed = self.shell.sidebar_collapsed;
        self.session.response_height = self.shell.response_height;
        self.session.response_width = self.shell.response_width;
        self.session.horizontal_panes = self.shell.pane_layout == PaneLayout::Horizontal;
        let (Some(path), Some(loaded)) = (&self.workspace_path, &self.loaded_workspace) else {
            self.session.clear_active_collection();
            return;
        };
        self.session.activate_collection(path.clone());
        self.session.open_tabs = self
            .shell
            .tabs()
            .iter()
            .filter_map(|key| loaded.request_selector(*key).map(str::to_owned))
            .collect();
        self.session.active_tab = self
            .shell
            .active_tab()
            .and_then(|key| loaded.request_selector(key))
            .map(str::to_owned);
        self.session.collapsed_folders = self
            .shell
            .collapsed_folders()
            .filter_map(|key| loaded.folder_selector(key).map(str::to_owned))
            .collect();
        self.session.collapsed_folders.sort();
        self.session.remember_selected_environment(
            path.clone(),
            self.shell.selected_environment().map(str::to_owned),
        );
    }

    pub(super) fn capture_selected_environment(&mut self) {
        let Some(path) = self.workspace_path.clone() else {
            return;
        };
        self.session.remember_selected_environment(
            path,
            self.shell.selected_environment().map(str::to_owned),
        );
    }

    pub(super) fn restore_selected_environment(&mut self) {
        let (Some(path), Some(loaded)) = (&self.workspace_path, &self.loaded_workspace) else {
            self.shell.select_environment(None);
            return;
        };
        let name = self
            .session
            .selected_environment_for(path)
            .filter(|name| {
                loaded
                    .workspace()
                    .environments()
                    .iter()
                    .any(|environment| environment.name == *name)
            })
            .map(str::to_owned);
        self.shell.select_environment(name);
    }

    pub(super) fn persist_session(&mut self, cx: &mut Context<Self>) {
        self.capture_session();
        let Some(store) = self.session_store.clone() else {
            return;
        };
        let state = self.session.clone();
        self.session_save_task = Some(cx.spawn(async move |view, cx| {
            let result = cx.background_spawn(async move { store.save(&state) }).await;
            if let Err(error) = result {
                let _ = view.update(cx, |view, cx| {
                    view.show_toast(
                        ToastIntent::Error,
                        format!("Could not save desktop session state: {error}"),
                        cx,
                    );
                });
            }
        }));
    }
}
