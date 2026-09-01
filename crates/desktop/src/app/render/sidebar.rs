use super::*;

impl ProbeApp {
    pub(super) fn render_toasts(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.toasts.is_empty() {
            return div().into_any_element();
        }

        let mut stack = ToastStack::new("probe-toast-stack", self.toasts.stack_state.clone())
            .motion(toast_stack_motion())
            .placement(Anchor::BottomRight)
            .focus_handle(self.toast_focus_handle.clone())
            .absolute()
            .bottom(px(theme.metrics.spacing_3))
            .right(px(theme.metrics.spacing_3))
            .w(px(components::TOAST_STACK_WIDTH))
            .max_h(relative(0.8))
            .occlude();

        for (id, notification, status) in self.toasts.iter() {
            let dismiss_view = cx.weak_entity();
            stack = stack.item(
                ("probe-toast", id),
                components::toast(theme, id, notification, status, move |_, _, cx| {
                    let _ = dismiss_view.update(cx, |view, cx| view.dismiss_toast(id, cx));
                }),
            );
        }

        deferred(stack)
            .with_priority(POPUP_PRIORITY.saturating_sub(1))
            .into_any_element()
    }

    pub(super) fn render_request_tab_tooltip(&self, theme: Theme) -> gpui::AnyElement {
        let Some(tooltip) = self.transient.request_tab_tooltip else {
            return div().into_any_element();
        };
        if !tooltip.open {
            return div().into_any_element();
        }
        if !self.shell.tabs().contains(&tooltip.key) {
            return div().into_any_element();
        }
        let Some(loaded) = &self.loaded_workspace else {
            return div().into_any_element();
        };
        let Some(request) = loaded.workspace().request(tooltip.key) else {
            return div().into_any_element();
        };
        let label = request
            .metadata
            .name
            .as_deref()
            .unwrap_or("Untitled request")
            .to_owned();
        let method = request.method.as_deref().unwrap_or("HTTP").to_uppercase();
        let position = point(
            tooltip.position.x + px(theme.metrics.spacing_1),
            tooltip.position.y + px(theme.metrics.control_height * 0.5),
        );
        let popup = div()
            .id("request-tab-tooltip-popup")
            .debug_selector(|| "request-tab-tooltip-popup".into())
            .max_w(px(320.0))
            .px(px(theme.metrics.spacing_2))
            .py(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_small))
            .bg(theme.colors.surfaces.window)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .shadow_sm()
            .text_size(px(theme.typography.caption_size))
            .text_color(theme.colors.text.primary)
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_2))
            .child(
                div()
                    .id("request-tab-tooltip-method")
                    .debug_selector(|| "request-tab-tooltip-method".into())
                    .flex_none()
                    .font_family(theme.typography.monospace_family)
                    .text_size(px(tree_method_font_size(theme, &method)))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.method_color(&method))
                    .child(method),
            )
            .child(components::truncated_label(label).min_w(px(0.0)).flex_1());

        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(6.0))
                .child(popup),
        )
        .with_priority(POPUP_PRIORITY + 1)
        .into_any_element()
    }

    pub(super) fn render_tab_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(context_menu) = self.transient.tab_context_menu.as_ref() else {
            return div().into_any_element();
        };
        let key = context_menu.target;
        let position = context_menu.position;
        if !self.shell.tabs().contains(&key) {
            return div().into_any_element();
        }

        let close_view = cx.weak_entity();
        let close_other_view = cx.weak_entity();
        let dismiss_view = cx.weak_entity();
        let menu = components::context_menu_surface(theme, "tab-context-menu", 220.0, move |cx| {
            let _ = dismiss_view.update(cx, |view, cx| {
                view.close_tab_context_menu(cx);
            });
        })
        .child(components::menu_button(
            theme,
            "tab-context-close",
            "Close Tab",
            components::shortcut_label_for_action(window, &CloseActiveTab),
            move |window, cx| {
                let _ = close_view.update(cx, |view, cx| {
                    view.request_close_tab(key, window, cx);
                });
            },
        ))
        .child(components::menu_button(
            theme,
            "tab-context-close-other",
            "Close Other Tabs",
            None,
            move |window, cx| {
                let _ = close_other_view.update(cx, |view, cx| {
                    view.request_close_other_tabs(key, window, cx);
                });
            },
        ));
        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(8.0))
                .child(menu),
        )
        .with_priority(POPUP_PRIORITY)
        .into_any_element()
    }

    pub(super) fn render_tree_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(context_menu) = self.transient.tree_context_menu.as_ref() else {
            return div().into_any_element();
        };
        let item = context_menu.target;
        let position = context_menu.position;
        let rename_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-context-rename", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-context-rename", key.slot()),
        };
        let delete_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-context-delete", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-context-delete", key.slot()),
        };
        let duplicate_id = match item {
            WorkspaceItemRef::Request(key) => Some(("tree-context-duplicate", key.slot())),
            WorkspaceItemRef::Folder(_) => None,
        };
        let rename_view = cx.weak_entity();
        let duplicate_view = cx.weak_entity();
        let delete_view = cx.weak_entity();
        let dismiss_view = cx.weak_entity();
        let menu = components::context_menu_surface(theme, "tree-context-menu", 200.0, move |cx| {
            let _ = dismiss_view.update(cx, |view, cx| {
                view.close_tree_context_menu(cx);
            });
        })
        .child(components::menu_button(
            theme,
            rename_id,
            "Rename",
            components::shortcut_label_for_action_in_context(
                window,
                &RenameTreeItem,
                "RequestTree",
            ),
            move |window, cx| {
                let _ = rename_view.update(cx, |view, cx| {
                    view.transient.tree_context_menu = None;
                    view.select_tree_item(item, cx);
                    view.open_rename_dialog(window, cx);
                });
            },
        ))
        .when_some(duplicate_id, |menu, duplicate_id| {
            menu.child(components::menu_button(
                theme,
                duplicate_id,
                "Duplicate",
                components::shortcut_label_for_action_in_context(
                    window,
                    &DuplicateRequest,
                    "RequestTree",
                ),
                move |window, cx| {
                    let _ = duplicate_view.update(cx, |view, cx| {
                        view.transient.tree_context_menu = None;
                        view.select_tree_item(item, cx);
                        view.duplicate_selected_request(window, cx);
                    });
                },
            ))
        })
        .child(components::destructive_menu_button(
            theme,
            delete_id,
            "Delete",
            components::shortcut_label_for_action_in_context(
                window,
                &DeleteTreeItem,
                "RequestTree",
            ),
            move |window, cx| {
                let _ = delete_view.update(cx, |view, cx| {
                    view.transient.tree_context_menu = None;
                    view.select_tree_item(item, cx);
                    view.request_delete_selected(window, cx);
                });
            },
        ));
        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(8.0))
                .child(menu),
        )
        .with_priority(POPUP_PRIORITY)
        .into_any_element()
    }

    pub(super) fn render_tree_row(
        &self,
        row: TreeRow,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(loaded) = &self.loaded_workspace else {
            return div().into_any_element();
        };
        let TreeRow { item, depth } = row;
        let can_edit = self.structure_task.is_none();
        match item {
            WorkspaceItemRef::Request(key) => {
                let Some(request) = loaded.workspace().request(key) else {
                    return div().into_any_element();
                };
                let label = request
                    .metadata
                    .name
                    .as_deref()
                    .unwrap_or("Untitled request");
                let method = request.method.as_deref().unwrap_or("HTTP").to_uppercase();
                let method_label = tree_method_label(&method).to_owned();
                let selected = self.selected_tree_item == Some(WorkspaceItemRef::Request(key));
                let view = cx.weak_entity();
                let context_menu_view = cx.weak_entity();
                let item = WorkspaceItemRef::Request(key);
                let button =
                    tree_row_button(theme, ("request-tree-item", key.slot()), depth, selected)
                        .accessibility_label(format!("Request {label}"))
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |view, cx| view.select_request(key, cx));
                        })
                        .when(can_edit, |row| {
                            row.on_mouse_down(
                                MouseButton::Right,
                                move |event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    let _ = context_menu_view.update(cx, |view, cx| {
                                        view.open_tree_context_menu(item, event.position, cx);
                                    });
                                },
                            )
                        })
                        .child(
                            div()
                                .w(px(26.0))
                                .h_full()
                                .flex_none()
                                .flex()
                                .items_center()
                                .truncate()
                                .font_family(theme.typography.monospace_family)
                                .text_size(px(tree_method_font_size(theme, &method_label)))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if selected {
                                    theme.colors.selection.active_foreground
                                } else {
                                    theme.method_color(&method)
                                })
                                .child(method_label.clone()),
                        )
                        .child(
                            components::truncated_label(label.to_owned())
                                .flex_1()
                                .when(selected, |label| {
                                    label.text_color(theme.colors.selection.active_foreground)
                                })
                                .when(selected, |label| {
                                    label.debug_selector(|| "request-tree-label".into())
                                }),
                        );
                self.wrap_tree_row(
                    TreeRowSpec {
                        item,
                        kind: ItemKind::Request,
                        selector: loaded.request_selector(key).unwrap_or_default().to_owned(),
                        label: label.to_owned(),
                        method: Some(method_label),
                        depth,
                        selected,
                    },
                    can_edit,
                    button,
                    theme,
                    cx,
                )
            }
            WorkspaceItemRef::Folder(key) => {
                let Some(folder) = loaded.workspace().folder(key) else {
                    return div().into_any_element();
                };
                let expanded = self.shell.folder_is_expanded(key);
                let label = folder.metadata.name.as_deref().unwrap_or("Untitled folder");
                let selected = self.selected_tree_item == Some(WorkspaceItemRef::Folder(key));
                let view = cx.weak_entity();
                let context_menu_view = cx.weak_entity();
                let item = WorkspaceItemRef::Folder(key);
                let button =
                    tree_row_button(theme, ("folder-tree-item", key.slot()), depth, selected)
                        .accessibility_label(format!("Folder {label}"))
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.select_tree_item(WorkspaceItemRef::Folder(key), cx);
                                view.shell.toggle_folder(key);
                                view.rebuild_visible_tree_rows_after_visibility_change();
                                view.persist_session(cx);
                                cx.notify();
                            });
                        })
                        .when(can_edit, |row| {
                            row.on_mouse_down(
                                MouseButton::Right,
                                move |event: &MouseDownEvent, _, cx| {
                                    cx.stop_propagation();
                                    let _ = context_menu_view.update(cx, |view, cx| {
                                        view.open_tree_context_menu(item, event.position, cx);
                                    });
                                },
                            )
                        })
                        .child(components::tree_folder_icon(theme, expanded, selected))
                        .child(
                            components::truncated_label(label.to_owned())
                                .flex_1()
                                .when(selected, |label| {
                                    label.text_color(theme.colors.selection.active_foreground)
                                })
                                .font_weight(FontWeight::SEMIBOLD),
                        );
                self.wrap_tree_row(
                    TreeRowSpec {
                        item,
                        kind: ItemKind::Folder,
                        selector: loaded.folder_selector(key).unwrap_or_default().to_owned(),
                        label: label.to_owned(),
                        method: None,
                        depth,
                        selected,
                    },
                    can_edit,
                    button,
                    theme,
                    cx,
                )
            }
        }
    }

    pub(super) fn wrap_tree_row(
        &self,
        spec: TreeRowSpec,
        can_edit: bool,
        button: Button,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let TreeRowSpec {
            item,
            kind,
            selector,
            label,
            method,
            depth,
            selected,
        } = spec;
        let indicator = self.tree_drop_target.map(|intent| intent.indicator);
        let show_before =
            matches!(indicator, Some(DropIndicator::Before(target)) if target == item);
        let show_after = matches!(indicator, Some(DropIndicator::After(target)) if target == item);
        let drop_into = matches!(
            indicator,
            Some(DropIndicator::IntoFolder(folder)) if item == WorkspaceItemRef::Folder(folder)
        );
        let indent = match kind {
            ItemKind::Folder | ItemKind::Request => tree_level_indent(theme, depth),
        };
        let drag_view = cx.weak_entity();
        let row_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-drop-request", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-drop-folder", key.slot()),
        };
        let button = if can_edit {
            button.on_drag(
                TreeDrag {
                    item,
                    kind,
                    label,
                    method,
                },
                move |drag, _, _, cx| {
                    let preview = drag.clone();
                    let item = drag.item;
                    let _ = drag_view.update(cx, |view, cx| {
                        view.tree_drag_source = Some(item);
                        view.select_tree_item(item, cx);
                    });
                    cx.new(|_| preview)
                },
            )
        } else {
            button
        };
        let line = |top: bool| {
            div()
                .absolute()
                .when(top, |line| line.top(px(0.0)))
                .when(!top, |line| line.bottom(px(0.0)))
                .left(px(indent))
                .right(px(theme.metrics.spacing_1))
                .h(px(2.0))
                .rounded(px(1.0))
                .bg(theme.colors.actions.accent)
        };
        div()
            .id(row_id)
            .relative()
            .w_full()
            .h(px(theme.metrics.tree_row_height))
            .debug_selector(move || format!("tree-row-{selector}"))
            .when(drop_into, |row| {
                row.rounded(px(theme.metrics.radius_small))
                    .bg(theme.colors.selection.inactive_background)
            })
            .child(button)
            .when(depth > 0, |row| {
                row.child(tree_hierarchy_guides(theme, depth, selected))
            })
            .when(show_before, |row| row.child(line(true)))
            .when(show_after, |row| row.child(line(false)))
            .into_any_element()
    }

    pub(super) fn render_tree_root_drop_row(&self, theme: Theme) -> gpui::AnyElement {
        let show_line = matches!(
            self.tree_drop_target.map(|intent| intent.indicator),
            Some(DropIndicator::RootEnd)
        );
        div()
            .id("tree-drop-root-end")
            .relative()
            .w_full()
            .h(px(theme.metrics.tree_row_height))
            .when(show_line, |row| {
                row.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(tree_level_indent(theme, 0)))
                        .right(px(theme.metrics.spacing_1))
                        .h(px(2.0))
                        .rounded(px(1.0))
                        .bg(theme.colors.actions.accent),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_sidebar(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let new_request_view = cx.weak_entity();
        let new_folder_view = cx.weak_entity();
        let new_collection_view = cx.weak_entity();
        let open_collection_view = cx.weak_entity();
        let sidebar_import_state_view = cx.weak_entity();
        let sidebar_import_keyboard_view = cx.weak_entity();
        let sidebar_import_postman_view = cx.weak_entity();
        let sidebar_import_yaak_view = cx.weak_entity();
        let sidebar_import_trigger_focus = self.transient.sidebar_import_trigger_focus.clone();
        let sidebar_import_popup_focus = self.transient.sidebar_import_popup_focus.clone();
        let can_edit = self.loaded_workspace.is_some() && self.structure_task.is_none();
        let add_menu_state_view = cx.weak_entity();
        let add_popup = components::popup_surface(theme, "tree-add-menu-popup", 180.0)
            .gap(px(theme.metrics.spacing_1))
            .child(components::menu_button(
                theme,
                "tree-new-request",
                "Add Request",
                None,
                move |window, cx| {
                    let _ = new_request_view.update(cx, |view, cx| {
                        view.transient.structure_add_menu_open = false;
                        view.open_create_request_dialog(window, cx);
                    });
                },
            ))
            .child(components::menu_button(
                theme,
                "tree-new-folder",
                "Add Folder",
                None,
                move |window, cx| {
                    let _ = new_folder_view.update(cx, |view, cx| {
                        view.transient.structure_add_menu_open = false;
                        view.open_create_folder_dialog(window, cx);
                    });
                },
            ));
        let add_trigger =
            components::add_menu_button(theme, self.transient.structure_add_menu_open, can_edit);
        let add_menu = if can_edit {
            Popover::new("tree-add-menu")
                .open(self.transient.structure_add_menu_open)
                .on_open_change(move |open, _, cx| {
                    let _ = add_menu_state_view.update(cx, |view, cx| {
                        view.transient.structure_add_menu_open = *open;
                        cx.notify();
                    });
                })
                .trigger(add_trigger)
                .content(move |_, _, _| add_popup)
                .into_any_element()
        } else {
            add_trigger.into_any_element()
        };
        let sidebar_import_popup =
            components::popup_surface(theme, "sidebar-import-provider-popup", 180.0)
                .aria_label("Import providers")
                .track_focus(&self.transient.sidebar_import_popup_focus)
                .key_context("ImportSubmenu")
                .child(components::menu_button(
                    theme,
                    "sidebar-import-postman",
                    "Postman Export…",
                    None,
                    move |window, cx| {
                        let _ = sidebar_import_postman_view.update(cx, |view, cx| {
                            view.transient.sidebar_import_menu_open = false;
                            if !view.loading {
                                view.request_import(ImportSource::Postman, window, cx);
                            }
                        });
                    },
                ))
                .child(components::menu_button(
                    theme,
                    "sidebar-import-yaak",
                    "Yaak Export…",
                    None,
                    move |window, cx| {
                        let _ = sidebar_import_yaak_view.update(cx, |view, cx| {
                            view.transient.sidebar_import_menu_open = false;
                            if !view.loading {
                                view.request_import(ImportSource::Yaak, window, cx);
                            }
                        });
                    },
                ));
        let sidebar_import_menu = Popover::new("sidebar-import-provider-menu")
            .open(self.transient.sidebar_import_menu_open)
            .track_focus(&self.transient.sidebar_import_popup_focus)
            .on_open_change(move |open, _, cx| {
                let _ = sidebar_import_state_view.update(cx, |view, cx| {
                    view.transient.sidebar_import_menu_open = *open;
                    cx.notify();
                });
            })
            .trigger(components::secondary_menu_trigger(
                theme,
                "sidebar-import-from",
                "Import From…",
                &self.transient.sidebar_import_trigger_focus,
                move |window, cx| {
                    let trigger_focus = sidebar_import_trigger_focus.clone();
                    let popup_focus = sidebar_import_popup_focus.clone();
                    let _ = sidebar_import_keyboard_view.update(cx, |view, cx| {
                        view.transient.sidebar_import_menu_open =
                            !view.transient.sidebar_import_menu_open;
                        if view.transient.sidebar_import_menu_open {
                            popup_focus.focus(window, cx);
                        } else {
                            trigger_focus.focus(window, cx);
                        }
                        cx.notify();
                    });
                },
            ))
            .content(move |_, _, _| sidebar_import_popup);
        let search_view = cx.weak_entity();
        let tree = if self.loaded_workspace.is_some() {
            let row_count = self.visible_tree_rows.len() + 1;
            let drag_view = cx.weak_entity();
            let drop_view = cx.weak_entity();
            let list = uniform_list("request-tree", row_count, {
                cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                    #[cfg(test)]
                    {
                        view.rendered_sidebar_rows = range.len();
                    }
                    range
                        .filter_map(|index| {
                            view.visible_tree_rows
                                .get(index)
                                .copied()
                                .map(|row| view.render_tree_row(row, theme, cx))
                                .or_else(|| {
                                    (index == view.visible_tree_rows.len())
                                        .then(|| view.render_tree_root_drop_row(theme))
                                })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .size_full()
            .track_scroll(&self.tree_scroll)
            .px(px(theme.metrics.spacing_1))
            .py(px(TREE_LIST_PADDING_Y))
            .on_drag_move(move |event: &DragMoveEvent<TreeDrag>, window, cx| {
                let _ = drag_view.update(cx, |view, cx| {
                    view.on_tree_drag_move(event, window, cx);
                });
            })
            .on_drop(move |drag: &TreeDrag, window, cx| {
                let _ = drop_view.update(cx, |view, cx| {
                    view.drop_tree_item(drag, window, cx);
                });
            })
            .can_drop(|value, _, _| value.downcast_ref::<TreeDrag>().is_some());
            div()
                .relative()
                .flex_1()
                .min_h(px(0.0))
                .track_focus(&self.tree_focus_handle)
                .key_context("RequestTree")
                .child(list)
                .child(
                    Scrollbar::vertical(&self.tree_scroll)
                        .id("request-tree-scrollbar")
                        .mode(ScrollbarMode::Scrolling),
                )
                .into_any_element()
        } else {
            let mut tree = div()
                .id("request-tree")
                .flex_1()
                .overflow_y_scroll()
                .p(px(theme.metrics.spacing_2))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(theme.metrics.spacing_2))
                        .pt(px(theme.metrics.spacing_1))
                        .pb(px(theme.metrics.spacing_2))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme.metrics.spacing_2))
                        .child(
                            div()
                                .text_color(theme.colors.text.muted)
                                .child("Create or open a collection to browse its requests."),
                        )
                        .child(components::secondary_button(
                            theme,
                            "sidebar-new-collection",
                            "New Collection…",
                            move |_, window, cx| {
                                let _ = new_collection_view.update(cx, |view, cx| {
                                    if !view.loading {
                                        view.choose_new_workspace(window, cx);
                                    }
                                });
                            },
                        ))
                        .child(components::secondary_button(
                            theme,
                            "sidebar-open-collection",
                            "Open Collection…",
                            move |_, window, cx| {
                                let _ = open_collection_view.update(cx, |view, cx| {
                                    if !view.loading {
                                        view.choose_workspace(window, cx);
                                    }
                                });
                            },
                        ))
                        .child(sidebar_import_menu),
                );
            if !self.session.recent_collections.is_empty() {
                tree = tree.child(
                    div()
                        .px(px(theme.metrics.spacing_2))
                        .pt(px(theme.metrics.spacing_2))
                        .pb(px(theme.metrics.spacing_1))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Recent Collections"),
                );
                for (index, path) in self.session.recent_collections.iter().enumerate() {
                    let open_path = path.clone();
                    let label = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Collection")
                        .to_owned();
                    let detail = path.display().to_string();
                    let view = cx.weak_entity();
                    let row = Button::new(("recent-collection", index))
                        .focusable(false)
                        .tab_stop(false)
                        .py(px(theme.metrics.spacing_2))
                        .px(px(theme.metrics.spacing_2))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme.metrics.spacing_1))
                        .overflow_hidden()
                        .rounded(px(theme.metrics.radius_small))
                        .cursor_pointer()
                        .hover(move |row| row.bg(theme.colors.surfaces.window))
                        .on_click(move |_, window, cx| {
                            let path = open_path.clone();
                            let _ = view.update(cx, |view, cx| {
                                if !view.loading {
                                    view.request_load_workspace(path, None, window, cx);
                                }
                            });
                        })
                        .child(components::truncated_label(label))
                        .child(
                            components::truncated_label(detail)
                                .text_size(px(theme.typography.caption_size))
                                .text_color(theme.colors.text.muted),
                        );
                    #[cfg(test)]
                    let row = row.debug_selector(move || format!("recent-collection-{index}"));
                    tree = tree.child(row);
                }
            }
            tree.into_any_element()
        };

        let sidebar = div()
            .w_full()
            .h_full()
            .flex()
            .flex_col()
            .rounded(px(theme.metrics.radius_medium))
            .overflow_hidden()
            .border_1()
            .border_color(theme.colors.borders.subtle)
            .bg(theme.colors.surfaces.sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(
                        div()
                            .h(px(theme.metrics.tab_bar_height))
                            .px(px(theme.metrics.spacing_1))
                            .flex()
                            .items_center()
                            .gap(px(theme.metrics.spacing_1))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                components::sidebar_search_input(
                                    theme,
                                    self.tree_search.clone(),
                                    "Search requests…",
                                    move |value, _, input_cx| {
                                        let _ = search_view.update(input_cx, |view, cx| {
                                            view.set_tree_search(value.to_string(), cx);
                                        });
                                    },
                                ),
                            ))
                            .child(add_menu),
                    ),
            )
            .child(tree);

        div()
            .w(px(self.shell.sidebar_width))
            .h_full()
            .pl(px(theme.metrics.spacing_1))
            .pb(px(theme.metrics.spacing_1))
            .flex_none()
            .child(sidebar)
    }
}
