use super::*;

impl ProbeApp {
    pub(super) fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((start_x, start_width)) = self.inspector_resize_start {
            let delta = f32::from(event.position.x) - start_x;
            self.inspector_list_width =
                (start_width + delta).clamp(MIN_INSPECT_LIST_WIDTH, MAX_INSPECT_LIST_WIDTH);
            cx.notify();
            return;
        }
        match self.shell.resizing {
            Some(ResizePane::Sidebar) => self.shell.resize_sidebar(event.position.x.into()),
            Some(ResizePane::Response) => match self.shell.pane_layout {
                PaneLayout::Vertical => self.shell.resize_response(
                    window.window_bounds().get_bounds().size.height.into(),
                    event.position.y.into(),
                ),
                PaneLayout::Horizontal => self.shell.resize_response_width(
                    window.window_bounds().get_bounds().size.width.into(),
                    event.position.x.into(),
                ),
            },
            None => return,
        }
        cx.notify();
    }

    pub(super) fn finish_resize(&mut self, cx: &mut Context<Self>) {
        let was_inspector_resizing = self.inspector_resize_start.take().is_some();
        if self.shell.resizing.take().is_none() && !was_inspector_resizing {
            return;
        }
        self.persist_session(cx);
        cx.notify();
    }

    pub(super) fn set_tree_search(&mut self, query: String, cx: &mut Context<Self>) {
        if self.tree_search == query {
            return;
        }
        self.tree_search = query;
        let expanded = self.expand_folders_for_tree_search();
        self.rebuild_visible_tree_rows();
        if expanded {
            self.persist_session(cx);
        }
        cx.notify();
    }

    pub(super) fn expand_folders_for_tree_search(&mut self) -> bool {
        let query = self.tree_search.trim();
        if query.is_empty() {
            return false;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return false;
        };
        let hits = matching_tree_items(loaded.workspace(), query);
        let mut expanded = false;
        for folder in hits.folders() {
            if !self.shell.folder_is_expanded(folder) {
                self.shell.expand_folder(folder);
                expanded = true;
            }
        }
        expanded
    }

    pub(super) fn rebuild_visible_tree_rows(&mut self) {
        let Some(loaded) = &self.loaded_workspace else {
            self.visible_tree_rows.clear();
            return;
        };
        let workspace = loaded.workspace();
        let query = self.tree_search.trim();
        let filter = if query.is_empty() {
            None
        } else {
            Some(matching_tree_items(workspace, query))
        };
        let mut rows = Vec::with_capacity(workspace.request_count());
        flatten_visible_tree_rows(
            workspace,
            workspace.root_items(),
            0,
            &self.shell,
            filter.as_ref(),
            &mut rows,
        );
        self.visible_tree_rows = rows;
    }

    pub(super) fn clear_tree_drag(&mut self) {
        self.tree_drag_source = None;
        self.tree_drop_target = None;
        self.tree_list_bounds = None;
        self.tree_auto_scroll.stop();
    }

    pub(super) fn scroll_tree_by(&mut self, delta: Pixels) {
        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let mut offset = handle.offset();
        let max = handle.max_offset();
        offset.y = (offset.y - delta).max(-max.y).min(px(0.0));
        handle.set_offset(offset);
    }

    pub(super) fn on_tree_drag_move(
        &mut self,
        event: &DragMoveEvent<TreeDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = event.drag(cx).item;
        self.tree_drag_source = Some(source);
        self.tree_list_bounds = Some(event.bounds);
        self.tree_auto_scroll.last_drag_position = Some(event.event.position);
        self.tree_row_height = Theme::for_window_appearance(window.appearance())
            .metrics
            .tree_row_height;
        let in_x = event.event.position.x >= event.bounds.left()
            && event.event.position.x <= event.bounds.right();
        let delta = in_x
            .then(|| AutoScroll::compute_delta(event.event.position.y, event.bounds))
            .flatten();
        self.tree_auto_scroll.set(delta, cx, |delta, view, cx| {
            view.scroll_tree_by(delta);
            view.recompute_tree_drop_from_stored_pointer(None, cx);
            cx.notify();
        });
        if in_x || event.bounds.contains(&event.event.position) {
            self.recompute_tree_drop(source, event.event.position, event.bounds, Some(window), cx);
        } else if delta.is_none() {
            self.tree_drop_target = None;
            cx.set_active_drag_cursor_style(CursorStyle::OperationNotAllowed, window);
            cx.notify();
        }
    }

    pub(super) fn recompute_tree_drop_from_stored_pointer(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let (Some(source), Some(pointer), Some(bounds)) = (
            self.tree_drag_source,
            self.tree_auto_scroll.last_drag_position,
            self.tree_list_bounds,
        ) else {
            return;
        };
        self.recompute_tree_drop(source, pointer, bounds, window, cx);
    }

    pub(super) fn recompute_tree_drop(
        &mut self,
        source: WorkspaceItemRef,
        pointer: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let pointer_y = pointer.y.into();
        let list_top = bounds.top().into();
        let scroll_y = self.tree_scroll.0.borrow().base_handle.offset().y.into();
        let Some((hovered_index, relative_y)) = hovered_row_index(
            pointer_y,
            list_top,
            TREE_LIST_PADDING_Y,
            scroll_y,
            self.tree_row_height,
            self.visible_tree_rows.len() + 1,
        ) else {
            self.tree_drop_target = None;
            return;
        };
        let root_end_drop = hovered_index == self.visible_tree_rows.len();
        let hovered = if root_end_drop {
            self.visible_tree_rows.last().copied()
        } else {
            self.visible_tree_rows.get(hovered_index).copied()
        };
        let Some(hovered) = hovered else {
            self.tree_drop_target = None;
            return;
        };
        let folder_expanded = match hovered.item {
            WorkspaceItemRef::Folder(key) => self.shell.folder_is_expanded(key),
            WorkspaceItemRef::Request(_) => false,
        };
        let zone = drop_zone(
            matches!(hovered.item, WorkspaceItemRef::Folder(_)),
            relative_y,
        );
        let intent = if root_end_drop {
            TreeDropIntent {
                parent: None,
                index: loaded.workspace().root_items().len(),
                indicator: DropIndicator::RootEnd,
            }
        } else {
            let Some(intent) = drop_intent(loaded.workspace(), hovered.item, zone, folder_expanded)
            else {
                self.tree_drop_target = None;
                return;
            };
            intent
        };
        let Some((source_parent, source_index)) = item_position(loaded.workspace(), source) else {
            self.tree_drop_target = None;
            return;
        };
        let source_selector = match source {
            WorkspaceItemRef::Request(key) => loaded.request_selector(key).map(str::to_owned),
            WorkspaceItemRef::Folder(key) => loaded.folder_selector(key).map(str::to_owned),
        };
        let Some(source_selector) = source_selector else {
            self.tree_drop_target = None;
            return;
        };
        let dest_parent_selector = intent
            .parent
            .and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        let duplicate_path = would_duplicate_path(
            loaded.uses_path_locators(),
            &source_selector,
            dest_parent_selector.as_deref(),
            |selector| {
                loaded.request_key(selector).is_some() || loaded.folder_key(selector).is_some()
            },
        );
        let cursor = match validate_tree_drop(
            loaded.workspace(),
            source,
            source_parent,
            source_index,
            intent,
            duplicate_path,
        ) {
            Ok(intent) => {
                self.tree_drop_target = Some(intent);
                CursorStyle::ClosedHand
            }
            Err(DropReject::NoOp) => {
                self.tree_drop_target = None;
                CursorStyle::ClosedHand
            }
            Err(_) => {
                self.tree_drop_target = None;
                CursorStyle::OperationNotAllowed
            }
        };
        if let Some(window) = window {
            cx.set_active_drag_cursor_style(cursor, window);
        }
        cx.notify();
    }

    pub(super) fn drop_tree_item(
        &mut self,
        drag: &TreeDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let intent = self.tree_drop_target.take();
        self.clear_tree_drag();
        let Some(intent) = intent else {
            return;
        };
        if self.structure_task.is_some() {
            return;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let Some(selector) = (match drag.item {
            WorkspaceItemRef::Request(key) => loaded.request_selector(key),
            WorkspaceItemRef::Folder(key) => loaded.folder_selector(key),
        })
        .map(str::to_owned) else {
            return;
        };
        let Some((source_parent, source_index)) = item_position(loaded.workspace(), drag.item)
        else {
            return;
        };
        let dest_parent_selector = intent
            .parent
            .and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        let Some(operation) = structure_operation_for_drop(
            drag.kind,
            selector,
            source_parent,
            source_index,
            intent.parent,
            dest_parent_selector,
            intent.index,
        ) else {
            return;
        };
        self.apply_structure(operation, window, cx);
    }

    pub(super) fn open_tree_context_menu(
        &mut self,
        item: WorkspaceItemRef,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        self.transient.tree_context_menu = Some(PositionedContextMenu {
            target: item,
            position,
        });
        self.select_tree_item(item, cx);
    }

    pub(super) fn close_tree_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.transient.tree_context_menu.is_none() {
            return;
        }
        self.transient.tree_context_menu = None;
        cx.notify();
    }

    pub(super) fn open_tab_context_menu(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.tabs().contains(&key) {
            return;
        }
        self.transient.tab_context_menu = Some(PositionedContextMenu {
            target: key,
            position,
        });
        self.transient.request_tab_tooltip = None;
        cx.notify();
    }

    pub(super) fn close_tab_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.transient.tab_context_menu.is_none() {
            return;
        }
        self.transient.tab_context_menu = None;
        cx.notify();
    }

    pub(super) fn open_environment_manager_context_menu(
        &mut self,
        name: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.environment_manager_dialog.is_none() || self.environment_save_task.is_some() {
            return;
        }
        self.transient.environment_manager_context_menu = Some(PositionedContextMenu {
            target: name,
            position,
        });
        cx.notify();
    }

    pub(super) fn close_environment_manager_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.transient.environment_manager_context_menu.is_none() {
            return;
        }
        self.transient.environment_manager_context_menu = None;
        cx.notify();
    }

    pub(super) fn open_request_tab_tooltip(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.tabs().contains(&key) {
            return;
        }
        self.transient.request_tab_tooltip_epoch =
            self.transient.request_tab_tooltip_epoch.wrapping_add(1);
        let epoch = self.transient.request_tab_tooltip_epoch;
        self.transient.request_tab_tooltip = Some(RequestTabTooltip {
            key,
            position,
            open: false,
        });
        self.transient.request_tab_tooltip_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(REQUEST_TAB_TOOLTIP_DELAY)
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.transient.request_tab_tooltip_epoch == epoch
                    && let Some(tooltip) = view.transient.request_tab_tooltip.as_mut()
                {
                    tooltip.open = true;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    pub(super) fn update_request_tab_tooltip_position(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(tooltip) = self.transient.request_tab_tooltip.as_mut() else {
            return;
        };
        if tooltip.key != key {
            return;
        }
        tooltip.position = position;
        if tooltip.open {
            cx.notify();
        }
    }

    pub(super) fn close_request_tab_tooltip(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        if self
            .transient
            .request_tab_tooltip
            .is_none_or(|tooltip| tooltip.key != key)
        {
            return;
        }
        self.transient.request_tab_tooltip_epoch =
            self.transient.request_tab_tooltip_epoch.wrapping_add(1);
        self.transient.request_tab_tooltip_task = None;
        self.transient.request_tab_tooltip = None;
        cx.notify();
    }

    pub(super) fn update_environment_variable(
        &mut self,
        name: &str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(environment) = self.shell.selected_environment().map(str::to_owned) else {
            return;
        };
        let Some(loaded) = self.loaded_workspace.as_mut() else {
            return;
        };
        if loaded
            .set_environment_variable(&environment, name, value)
            .is_ok()
        {
            self.pending_environment_saves
                .insert((environment, name.to_owned()));
            self.start_next_environment_save(window, cx);
            cx.notify();
        }
    }

    pub(super) fn start_next_environment_save(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            return;
        }
        let Some((environment, name)) = self.pending_environment_saves.pop_first() else {
            self.finish_pending_close_if_idle(window, cx);
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            self.pending_environment_saves.insert((environment, name));
            self.finish_pending_close_if_idle(window, cx);
            return;
        };
        let prepared = match loaded.prepare_environment_variable_save(&environment, &name) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.pending_environment_saves.insert((environment, name));
                self.pending_close = None;
                self.show_toast(
                    ToastIntent::Error,
                    format!("Could not save environment variable: {error}"),
                    cx,
                );
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
                            loaded.complete_environment_save(saved);
                        }
                        view.environment_save_workspace_path = None;
                        view.start_next_request_save(window, cx);
                        view.start_next_environment_save(window, cx);
                    }
                    Err(error) => {
                        view.environment_save_workspace_path = None;
                        view.pending_close = None;
                        view.show_toast(
                            ToastIntent::Error,
                            format!("Could not save environment variable: {error}"),
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
