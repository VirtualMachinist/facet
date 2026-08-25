use super::*;

impl ProbeApp {
    fn render_tab_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(key) = self.tab_context_menu else {
            return div().into_any_element();
        };
        let Some(position) = self.tab_context_menu_position else {
            return div().into_any_element();
        };
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

    fn render_tree_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(item) = self.tree_context_menu else {
            return div().into_any_element();
        };
        let Some(position) = self.tree_context_menu_position else {
            return div().into_any_element();
        };
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
                    view.tree_context_menu = None;
                    view.tree_context_menu_position = None;
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
                        view.tree_context_menu = None;
                        view.tree_context_menu_position = None;
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
                    view.tree_context_menu = None;
                    view.tree_context_menu_position = None;
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

    fn render_tree_row(
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
                                view.rebuild_visible_tree_rows();
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

    fn wrap_tree_row(
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

    fn render_tree_root_drop_row(&self, theme: Theme) -> gpui::AnyElement {
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

    fn render_sidebar(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let new_request_view = cx.weak_entity();
        let new_folder_view = cx.weak_entity();
        let new_collection_view = cx.weak_entity();
        let open_collection_view = cx.weak_entity();
        let sidebar_import_state_view = cx.weak_entity();
        let sidebar_import_keyboard_view = cx.weak_entity();
        let sidebar_import_postman_view = cx.weak_entity();
        let sidebar_import_yaak_view = cx.weak_entity();
        let sidebar_import_trigger_focus = self.sidebar_import_trigger_focus.clone();
        let sidebar_import_popup_focus = self.sidebar_import_popup_focus.clone();
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
                        view.structure_add_menu_open = false;
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
                        view.structure_add_menu_open = false;
                        view.open_create_folder_dialog(window, cx);
                    });
                },
            ));
        let add_trigger =
            components::add_menu_button(theme, self.structure_add_menu_open, can_edit);
        let add_menu = if can_edit {
            Popover::new("tree-add-menu")
                .open(self.structure_add_menu_open)
                .on_open_change(move |open, _, cx| {
                    let _ = add_menu_state_view.update(cx, |view, cx| {
                        view.structure_add_menu_open = *open;
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
                .track_focus(&self.sidebar_import_popup_focus)
                .key_context("ImportSubmenu")
                .child(components::menu_button(
                    theme,
                    "sidebar-import-postman",
                    "Postman Export…",
                    None,
                    move |window, cx| {
                        let _ = sidebar_import_postman_view.update(cx, |view, cx| {
                            view.sidebar_import_menu_open = false;
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
                            view.sidebar_import_menu_open = false;
                            if !view.loading {
                                view.request_import(ImportSource::Yaak, window, cx);
                            }
                        });
                    },
                ));
        let sidebar_import_menu = Popover::new("sidebar-import-provider-menu")
            .open(self.sidebar_import_menu_open)
            .track_focus(&self.sidebar_import_popup_focus)
            .on_open_change(move |open, _, cx| {
                let _ = sidebar_import_state_view.update(cx, |view, cx| {
                    view.sidebar_import_menu_open = *open;
                    cx.notify();
                });
            })
            .trigger(components::secondary_menu_trigger(
                theme,
                "sidebar-import-from",
                "Import From…",
                &self.sidebar_import_trigger_focus,
                move |window, cx| {
                    let trigger_focus = sidebar_import_trigger_focus.clone();
                    let popup_focus = sidebar_import_popup_focus.clone();
                    let _ = sidebar_import_keyboard_view.update(cx, |view, cx| {
                        view.sidebar_import_menu_open = !view.sidebar_import_menu_open;
                        if view.sidebar_import_menu_open {
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

    fn render_tabs(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let tab_count = self.shell.tabs().len();
        let mut active_tab_background: Hsla = theme.colors.actions.accent.into();
        active_tab_background.a = 0.12;
        let mut active_tab_close_hover: Hsla = theme.colors.actions.accent.into();
        active_tab_close_hover.a = 0.18;
        let request_tab_bar_height = theme.metrics.tab_bar_height + 2.0;
        let mut tab_strip = Tabs::new("request-tabs-scroll")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px(px(theme.metrics.spacing_1))
            .flex()
            .items_center()
            .overflow_x_scroll()
            .track_scroll(&self.tab_bar_scroll);
        let Some(loaded) = &self.loaded_workspace else {
            return div()
                .id("request-tabs")
                .h(px(request_tab_bar_height))
                .w_full()
                .bg(theme.colors.surfaces.raised)
                .border_b_1()
                .border_color(theme.colors.borders.subtle);
        };
        for key in self.shell.tabs() {
            let Some(request) = loaded.workspace().request(*key) else {
                continue;
            };
            let active = self.shell.active_tab() == Some(*key);
            let dirty = self.persistence.is_dirty(*key, request);
            let label = request
                .metadata
                .name
                .as_deref()
                .unwrap_or("Untitled request");
            let select_view = cx.weak_entity();
            let close_view = cx.weak_entity();
            let context_menu_view = cx.weak_entity();
            let middle_close_view = close_view.clone();
            let close_hover = if active {
                active_tab_close_hover
            } else {
                theme.colors.actions.disabled.into()
            };
            let tab_key = *key;
            let tab_index = self
                .shell
                .tabs()
                .iter()
                .position(|open| *open == *key)
                .unwrap_or(0);
            tab_strip = tab_strip.child(
                Tab::new(("request-tab", key.slot()))
                    .selected(active)
                    .set_position(tab_index + 1, tab_count)
                    .h(px(request_tab_bar_height))
                    .min_w(px(80.0))
                    .max_w(px(176.0))
                    .pl(px(theme.metrics.spacing_3))
                    .pr(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_1))
                    .overflow_hidden()
                    .rounded_tl(px(theme.metrics.radius_medium))
                    .rounded_tr(px(theme.metrics.radius_medium))
                    .when(active, |tab| {
                        tab.bg(active_tab_background)
                            .border_b_1()
                            .border_color(theme.colors.actions.accent)
                            .text_color(theme.colors.actions.accent)
                    })
                    .when(!active, |tab| {
                        tab.text_color(theme.colors.text.secondary)
                            .hover(move |tab| tab.bg(theme.colors.surfaces.sidebar))
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = select_view.update(cx, |view, cx| view.select_request(tab_key, cx));
                    })
                    .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let _ = context_menu_view.update(cx, |view, cx| {
                            view.open_tab_context_menu(tab_key, event.position, cx);
                        });
                    })
                    .on_aux_click(move |event, window, cx| {
                        if event.is_middle_click() {
                            cx.stop_propagation();
                            let _ = middle_close_view
                                .update(cx, |view, cx| view.request_close_tab(tab_key, window, cx));
                        }
                    })
                    .child(
                        components::truncated_label(label.to_owned())
                            .flex_1()
                            .when(active, |label| {
                                label.debug_selector(|| "request-tab-label".into())
                            }),
                    )
                    .when(dirty, |tab| {
                        tab.child(
                            div()
                                .id(("request-dirty", key.slot()))
                                .flex_none()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(theme.colors.actions.accent),
                        )
                    })
                    .child(
                        Button::new(("close-tab", key.slot()))
                            .focusable(false)
                            .tab_stop(false)
                            .flex_none()
                            .w(px(theme.metrics.icon_standard + 4.0))
                            .h(px(theme.metrics.icon_standard + 4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(theme.metrics.radius_small))
                            .hover(move |close| close.bg(close_hover))
                            .child(components::close_icon(theme))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                let _ = close_view.update(cx, |view, cx| {
                                    view.request_close_tab(tab_key, window, cx)
                                });
                            }),
                    ),
            );
        }

        let mut tabs = div()
            .id("request-tabs")
            .h(px(request_tab_bar_height))
            .w_full()
            .flex()
            .items_center()
            .bg(theme.colors.surfaces.raised)
            .border_b_1()
            .border_color(theme.colors.borders.subtle)
            .child(tab_strip);
        let selected = self.shell.selected_environment().unwrap_or("").to_owned();
        let mut options = vec![(String::new(), "No environment".to_owned())];
        options.extend(
            loaded
                .workspace()
                .environments()
                .iter()
                .map(|environment| (environment.name.clone(), environment.name.clone())),
        );
        let environment_view = cx.weak_entity();
        let create_environment_view = cx.weak_entity();
        tabs = tabs.child(
            div().flex_none().px(px(theme.metrics.spacing_2)).child(
                components::dropdown(
                    theme,
                    "request-environment",
                    "Request environment",
                    Some(selected),
                    options,
                    190.0,
                    move |value, _, cx| {
                        let value = value.cloned().unwrap_or_default();
                        let _ = environment_view.update(cx, |view, cx| {
                            view.select_environment((!value.is_empty()).then_some(value), cx);
                        });
                    },
                )
                .with_action("Create environment…", move |window, cx| {
                    let _ = create_environment_view.update(cx, |view, cx| {
                        view.open_create_environment_dialog(window, cx);
                    });
                }),
            ),
        );
        tabs
    }

    pub(super) fn edit_request(
        &mut self,
        key: RequestKey,
        edit: impl FnOnce(&mut HttpRequest),
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        edit(request);
        self.persistence.edited(key);
        cx.notify();
    }

    pub(super) fn change_body_kind(
        &mut self,
        key: RequestKey,
        kind: BodyEditorKind,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        self.request_editor.switch_body_kind(key, request, kind);
        self.persistence.edited(key);
        cx.notify();
    }

    fn render_request_editor(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let Some(key) = self.shell.active_tab() else {
            return div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.colors.surfaces.editor)
                .text_color(theme.colors.text.muted)
                .child("Select a request from the collection sidebar.");
        };
        let Some(request) = self.active_request().cloned() else {
            return div().flex_1();
        };
        let method = request.method.as_deref().unwrap_or("GET").to_uppercase();
        let url = url_bar_value(&request);
        let request_dirty = self.persistence.is_dirty(key, &request);
        let mut breadcrumb_labels = self
            .loaded_workspace
            .as_ref()
            .and_then(|loaded| {
                loaded
                    .workspace()
                    .request_ancestor_folders(key)
                    .map(|folders| {
                        folders
                            .iter()
                            .filter_map(|folder_key| loaded.workspace().folder(*folder_key))
                            .map(|folder| {
                                folder
                                    .metadata
                                    .name
                                    .as_deref()
                                    .unwrap_or("Untitled folder")
                                    .to_owned()
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        let request_breadcrumb_index = breadcrumb_labels.len();
        breadcrumb_labels.push(
            request
                .metadata
                .name
                .as_deref()
                .unwrap_or("Untitled request")
                .to_owned(),
        );
        let save_view = cx.weak_entity();
        let mut breadcrumb_path = div()
            .id("request-breadcrumb-path")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .overflow_x_scroll()
            .text_size(px(theme.typography.caption_size))
            .text_color(theme.colors.text.muted);
        for (index, label) in breadcrumb_labels.into_iter().enumerate() {
            if index > 0 {
                breadcrumb_path = breadcrumb_path.child(div().flex_none().child("›"));
            }
            let segment = components::truncated_label(label)
                .max_w(px(220.0))
                .flex_none();
            let segment = if index == request_breadcrumb_index {
                segment
                    .debug_selector(|| "request-breadcrumb-request".into())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.text.primary)
            } else {
                segment.debug_selector(move || format!("request-breadcrumb-folder-{index}"))
            };
            breadcrumb_path = breadcrumb_path.child(segment);
        }
        let breadcrumb = div()
            .id("request-breadcrumb")
            .debug_selector(|| "request-breadcrumb".into())
            .h(px(theme.metrics.control_height))
            .w_full()
            .flex()
            .items_center()
            .child(breadcrumb_path)
            .child(
                Button::new("request-save")
                    .accessibility_label("Save request")
                    .debug_selector(|| "request-save".into())
                    .disabled(!request_dirty)
                    .ml(px(theme.metrics.spacing_2))
                    .flex_none()
                    .w(px(theme.metrics.control_height))
                    .h(px(theme.metrics.control_height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.metrics.radius_small))
                    .border_1()
                    .border_color(theme.colors.borders.standard)
                    .bg(theme.colors.surfaces.raised)
                    .hover(move |button| button.bg(theme.colors.selection.inactive_background))
                    .focus(move |button| button.border_color(theme.colors.borders.focused))
                    .styles(move |styles| {
                        styles.disabled(move |button| {
                            button
                                .bg(theme.colors.selection.inactive_background)
                                .border_color(theme.colors.selection.inactive_background)
                                .text_color(theme.colors.actions.disabled_foreground)
                        })
                    })
                    .child(components::save_icon(theme).when(!request_dirty, |icon| {
                        icon.text_color(theme.colors.actions.disabled_foreground)
                    }))
                    .on_click(move |_, window, cx| {
                        let _ = save_view.update(cx, |view, cx| {
                            view.save_active_request(window, cx);
                        });
                    }),
            );
        let url_view = cx.weak_entity();
        let execution_view = cx.weak_entity();
        let request_running = self
            .execution
            .response(key)
            .is_some_and(ResponseState::is_running);
        let mut section_tabs = Tabs::new("request-editor-sections")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        for (index, section) in EditorSection::ALL.into_iter().enumerate() {
            let section_view = cx.weak_entity();
            section_tabs = section_tabs.child(components::text_tab(
                theme,
                ("request-editor-section", index),
                format!(
                    "{}{}",
                    section.label(),
                    match section {
                        EditorSection::Query => format!("  {}", request.query_parameters.len()),
                        EditorSection::Path => format!("  {}", request.path_parameters.len()),
                        EditorSection::Headers => format!("  {}", request.headers.len()),
                        EditorSection::Body | EditorSection::Authentication => String::new(),
                    }
                ),
                self.request_editor.section == section,
                index + 1,
                EditorSection::ALL.len(),
                move |_, _, cx| {
                    let _ = section_view.update(cx, |view, cx| {
                        view.request_editor.section = section;
                        cx.notify();
                    });
                },
            ));
        }

        let section = match self.request_editor.section {
            EditorSection::Query => self.render_query_editor(key, &request, theme, cx),
            EditorSection::Path => self.render_parameter_editor(key, &request, true, theme, cx),
            EditorSection::Headers => self.render_header_editor(key, &request, theme, cx),
            EditorSection::Body => self.render_body_editor(key, &request, theme, cx),
            EditorSection::Authentication => {
                self.render_authentication_editor(key, &request, theme, cx)
            }
        };

        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(120.0))
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.editor)
            .child(
                div()
                    .p(px(theme.metrics.spacing_2))
                    .pb(px(theme.metrics.spacing_2))
                    .flex()
                    .flex_col()
                    .gap(px(theme.metrics.spacing_2))
                    .child(breadcrumb)
                    .child(
                        div()
                            .id("request-url-bar")
                            .debug_selector(|| "request-url-bar".into())
                            .h(px(theme.metrics.control_height))
                            .w_full()
                            .flex()
                            .items_center()
                            .child(div().w(px(108.0)).mr(px(theme.metrics.spacing_1)).child(
                                components::dropdown_with_option_colors(
                                    theme,
                                    "request-method",
                                    "HTTP method",
                                    Some(method.clone()),
                                    request_method_options(theme, &method),
                                    108.0,
                                    {
                                        let method_view = cx.weak_entity();
                                        move |value, _, cx| {
                                            let Some(value) = value.cloned() else {
                                                return;
                                            };
                                            let _ = method_view.update(cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| request.method = Some(value),
                                                    cx,
                                                );
                                            });
                                        }
                                    },
                                ),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(components::url_text_input(
                                        theme,
                                        ("request-url", key.slot()),
                                        url.clone(),
                                        "https://api.example.com/users/:userId",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = url_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| apply_url_bar_value(request, &value),
                                                    cx,
                                                );
                                            });
                                        },
                                    )),
                            )
                            .child(div().ml(px(theme.metrics.spacing_1)).flex_none().child(
                                components::primary_button(
                                    theme,
                                    "request-execution",
                                    if request_running { "Cancel" } else { "Send" },
                                    move |_, _, cx| {
                                        let _ = execution_view.update(cx, |view, cx| {
                                            if view
                                                .execution
                                                .response(key)
                                                .is_some_and(ResponseState::is_running)
                                            {
                                                view.cancel_request(key, cx);
                                            } else {
                                                view.send_request(key, cx);
                                            }
                                        });
                                    },
                                ),
                            )),
                    )
                    .child(section_tabs),
            )
            .child(
                div()
                    .id("request-editor-section-content")
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(theme.metrics.spacing_2))
                    .pb(px(theme.metrics.spacing_2))
                    .when(
                        self.request_editor.section != EditorSection::Body,
                        |content| content.overflow_y_scroll(),
                    )
                    .child(section),
            )
    }

    fn render_query_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_parameter_editor(key, request, false, theme, cx)
    }

    fn render_parameter_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        path: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        let parameters = if path {
            &request.path_parameters
        } else {
            &request.query_parameters
        };
        for (index, parameter) in parameters.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                (if path { "path-name" } else { "query-name" }, index),
                                parameter.name.clone(),
                                "Parameter",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if path {
                                                    rename_path_parameter_at(
                                                        request, index, &value,
                                                    );
                                                } else if let Some(parameter) =
                                                    request.query_parameters.get_mut(index)
                                                {
                                                    parameter.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                (if path { "path-value" } else { "query-value" }, index),
                                parameter.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(parameter) = if path {
                                                    request.path_parameters.get_mut(index)
                                                } else {
                                                    request.query_parameters.get_mut(index)
                                                } {
                                                    parameter.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            (
                                if path {
                                    "path-enabled"
                                } else {
                                    "query-enabled"
                                },
                                index,
                            ),
                            if path {
                                "Enable path parameter"
                            } else {
                                "Enable query parameter"
                            },
                            !parameter.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(parameter) = if path {
                                                request.path_parameters.get_mut(index)
                                            } else {
                                                request.query_parameters.get_mut(index)
                                            } {
                                                parameter.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            (if path { "remove-path" } else { "remove-query" }, index),
                            if path {
                                "Remove path parameter"
                            } else {
                                "Remove query parameter"
                            },
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if path {
                                                remove_path_parameter_at(request, index);
                                            } else if index < request.query_parameters.len() {
                                                request.query_parameters.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            if path {
                "add-path-parameter"
            } else {
                "add-query-parameter"
            },
            if path {
                "Add path parameter"
            } else {
                "Add query parameter"
            },
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if path {
                                add_path_parameter(request);
                            } else {
                                request.query_parameters.push(QueryParameter {
                                    name: String::new(),
                                    value: String::new(),
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_header_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, header) in request.headers.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("header-name", index),
                                header.name.clone(),
                                "Header",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(header) = request.headers.get_mut(index)
                                                {
                                                    header.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("header-value", index),
                                header.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(header) = request.headers.get_mut(index)
                                                {
                                                    header.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("header-enabled", index),
                            "Enable header",
                            !header.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(header) = request.headers.get_mut(index) {
                                                header.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-header", index),
                            "Remove header",
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if index < request.headers.len() {
                                                request.headers.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-header",
            "Add header",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            request.headers.push(Header {
                                name: String::new(),
                                value: String::new(),
                                disabled: false,
                            })
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_body_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active_kind = body_kind(request);
        let choices = [
            ("None", BodyEditorKind::None),
            ("JSON", BodyEditorKind::Json),
            ("Text", BodyEditorKind::Text),
            ("XML", BodyEditorKind::Xml),
            ("SPARQL", BodyEditorKind::Sparql),
            ("Form", BodyEditorKind::Form),
            ("Multipart", BodyEditorKind::Multipart),
            ("File", BodyEditorKind::File),
        ];
        let choice_count = choices.len();
        let mut kind_buttons = div().flex().flex_wrap().gap(px(theme.metrics.spacing_1));
        for (index, (label, kind)) in choices.into_iter().enumerate() {
            let kind_view = cx.weak_entity();
            kind_buttons = kind_buttons.child(components::editor_subtab(
                theme,
                ("body-kind", index),
                label,
                active_kind == label,
                index + 1,
                choice_count,
                move |_, _, cx| {
                    let _ = kind_view.update(cx, |view, cx| {
                        view.change_body_kind(key, kind, cx);
                    });
                },
            ));
        }

        let mut editor = div()
            .size_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_2))
            .child(kind_buttons);
        match request.body.as_ref() {
            Some(RequestBody::Single(Body::Raw(raw))) => {
                let body_view = cx.weak_entity();
                editor = editor.child(
                    div()
                        .id("request-body-editor")
                        .debug_selector(|| "request-body-editor".into())
                        .flex_1()
                        .min_h(px(0.0))
                        .child(components::body_text_input(
                            theme,
                            ("request-body", key.slot()),
                            raw.data.clone(),
                            match raw.kind {
                                RawBodyKind::Json => components::BodySyntax::Json,
                                RawBodyKind::Xml => components::BodySyntax::Xml,
                                _ => components::BodySyntax::Plain,
                            },
                            self.variable_context(cx),
                            move |value, _, input_cx| {
                                let _ = body_view.update(input_cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(data) = raw_body_mut(request) {
                                                *data = value.to_string();
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
            }
            Some(RequestBody::Single(Body::FormUrlEncoded(fields))) => {
                editor = editor.child(self.render_form_body_editor(key, fields, theme, cx));
            }
            Some(RequestBody::Single(Body::Multipart(parts))) => {
                editor = editor.child(self.render_multipart_body_editor(key, parts, theme, cx));
            }
            Some(RequestBody::Single(Body::File(files))) => {
                editor = editor.child(self.render_file_body_editor(key, files, theme, cx));
            }
            Some(_) => {
                editor = editor.child(
                    div()
                        .p(px(theme.metrics.spacing_3))
                        .rounded(px(theme.metrics.radius_small))
                        .bg(theme.colors.surfaces.window)
                        .text_color(theme.colors.text.secondary)
                        .child(format!(
                            "This request uses a {active_kind} body. Choose a raw body type to replace it."
                        )),
                );
            }
            None => {
                editor = editor.child(
                    div()
                        .text_color(theme.colors.text.muted)
                        .child("This request has no body."),
                );
            }
        }
        editor.into_any_element()
    }

    fn render_form_body_editor(
        &self,
        key: RequestKey,
        fields: &[FormField],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, field) in fields.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("form-field-name", index),
                                field.name.clone(),
                                "Field",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(
                                                    Body::FormUrlEncoded(fields),
                                                )) = request.body.as_mut()
                                                    && let Some(field) = fields.get_mut(index)
                                                {
                                                    field.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("form-field-value", index),
                                field.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(
                                                    Body::FormUrlEncoded(fields),
                                                )) = request.body.as_mut()
                                                    && let Some(field) = fields.get_mut(index)
                                                {
                                                    field.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("form-field-enabled", index),
                            "Enable form field",
                            !field.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::FormUrlEncoded(
                                                fields,
                                            ))) = request.body.as_mut()
                                                && let Some(field) = fields.get_mut(index)
                                            {
                                                field.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-form-field", index),
                            "Remove form field",
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::FormUrlEncoded(
                                                fields,
                                            ))) = request.body.as_mut()
                                                && index < fields.len()
                                            {
                                                fields.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-form-field",
            "Add field",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::FormUrlEncoded(fields))) =
                                request.body.as_mut()
                            {
                                fields.push(FormField {
                                    name: String::new(),
                                    value: String::new(),
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_multipart_body_editor(
        &self,
        key: RequestKey,
        parts: &[MultipartPart],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, part) in parts.iter().enumerate() {
            let value = match &part.value {
                MultipartValue::Single(value) => value.clone(),
                MultipartValue::Multiple(values) => values.join(", "),
            };
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let kind_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            let browse_view = cx.weak_entity();
            let is_file = part.kind == MultipartPartKind::File;
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(components::editor_button(
                            theme,
                            ("multipart-kind", index),
                            if is_file { "File" } else { "Text" },
                            is_file,
                            move |_, _, cx| {
                                let _ = kind_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && let Some(part) = parts.get_mut(index)
                                        {
                                            part.kind = if part.kind == MultipartPartKind::Text {
                                                MultipartPartKind::File
                                            } else {
                                                MultipartPartKind::Text
                                            };
                                        }
                                    },
                                    cx,
                                );
                                });
                            },
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("multipart-name", index),
                                part.name.clone(),
                                "Part",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(Body::Multipart(
                                                    parts,
                                                ))) = request.body.as_mut()
                                                    && let Some(part) = parts.get_mut(index)
                                                {
                                                    part.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(if is_file {
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap(px(theme.metrics.spacing_1))
                                .child(div().flex_1().min_w(px(0.0)).child(
                                    components::variable_text_input(
                                        theme,
                                        ("multipart-value", index),
                                        value,
                                        "File path",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = value_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| {
                                                        if let Some(RequestBody::Single(
                                                            Body::Multipart(parts),
                                                        )) = request.body.as_mut()
                                                            && let Some(part) = parts.get_mut(index)
                                                        {
                                                            part.value = MultipartValue::Single(
                                                                value.to_string(),
                                                            );
                                                        }
                                                    },
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                ))
                                .child(components::browse_file_button(
                                    theme,
                                    ("multipart-file-browse", index),
                                    "Browse for file",
                                    move |_, window, cx| {
                                        let _ = browse_view.update(cx, |view, cx| {
                                            view.choose_multipart_file(key, index, window, cx);
                                        });
                                    },
                                ))
                                .into_any_element()
                        } else {
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(components::variable_text_input(
                                    theme,
                                    ("multipart-value", index),
                                    value,
                                    "Value",
                                    self.variable_context(cx),
                                    move |value, _, input_cx| {
                                        let _ = value_view.update(input_cx, |view, cx| {
                                            view.edit_request(
                                                key,
                                                |request| {
                                                    if let Some(RequestBody::Single(
                                                        Body::Multipart(parts),
                                                    )) = request.body.as_mut()
                                                        && let Some(part) = parts.get_mut(index)
                                                    {
                                                        part.value = MultipartValue::Single(
                                                            value.to_string(),
                                                        );
                                                    }
                                                },
                                                cx,
                                            );
                                        });
                                    },
                                ))
                                .into_any_element()
                        })
                        .child(components::switch(
                            theme,
                            ("multipart-enabled", index),
                            "Enable multipart part",
                            !part.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && let Some(part) = parts.get_mut(index)
                                        {
                                            part.disabled = !enabled;
                                        }
                                    },
                                    cx,
                                );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-multipart-part", index),
                            "Remove multipart part",
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && index < parts.len()
                                        {
                                            parts.remove(index);
                                        }
                                    },
                                    cx,
                                );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-multipart-part",
            "Add part",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                request.body.as_mut()
                            {
                                parts.push(MultipartPart {
                                    name: String::new(),
                                    kind: MultipartPartKind::Text,
                                    value: MultipartValue::Single(String::new()),
                                    content_type: None,
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_file_body_editor(
        &self,
        key: RequestKey,
        files: &[FileReference],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, file) in files.iter().enumerate() {
            let path_view = cx.weak_entity();
            let type_view = cx.weak_entity();
            let selected_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            let browse_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap(px(theme.metrics.spacing_1))
                                .child(div().flex_1().min_w(px(0.0)).child(
                                    components::variable_text_input(
                                        theme,
                                        ("body-file-path", index),
                                        file.file_path.clone(),
                                        "File path",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = path_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| {
                                                        if let Some(RequestBody::Single(
                                                            Body::File(files),
                                                        )) = request.body.as_mut()
                                                            && let Some(file) = files.get_mut(index)
                                                        {
                                                            file.file_path = value.to_string();
                                                        }
                                                    },
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                ))
                                .child(components::browse_file_button(
                                    theme,
                                    ("body-file-browse", index),
                                    "Browse for file",
                                    move |_, window, cx| {
                                        let _ = browse_view.update(cx, |view, cx| {
                                            view.choose_body_file(key, index, window, cx);
                                        });
                                    },
                                )),
                        )
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("body-file-content-type", index),
                                file.content_type.clone(),
                                "Content type",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = type_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::File(files))) =
                                            request.body.as_mut()
                                            && let Some(file) = files.get_mut(index)
                                        {
                                            file.content_type = value.to_string();
                                        }
                                    },
                                    cx,
                                );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("body-file-selected", index),
                            "Select body file",
                            file.selected,
                            move |selected, _, cx| {
                                let _ = selected_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::File(files))) =
                                                request.body.as_mut()
                                                && let Some(file) = files.get_mut(index)
                                            {
                                                file.selected = selected;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-body-file", index),
                            "Remove file",
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::File(files))) =
                                                request.body.as_mut()
                                                && index < files.len()
                                            {
                                                files.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-body-file",
            "Add file",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::File(files))) =
                                request.body.as_mut()
                            {
                                files.push(FileReference {
                                    file_path: String::new(),
                                    content_type: "application/octet-stream".to_owned(),
                                    selected: files.is_empty(),
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_authentication_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = request
            .authentication
            .as_ref()
            .map(|auth| auth_label(&auth.kind));
        let choices = [
            ("None", None),
            ("Inherit", Some(AuthenticationKind::Inherit)),
            ("Basic", Some(AuthenticationKind::Basic)),
            ("Bearer", Some(AuthenticationKind::Bearer)),
            ("API Key", Some(AuthenticationKind::ApiKey)),
            ("OAuth 1", Some(AuthenticationKind::OAuth1)),
            ("OAuth 2", Some(AuthenticationKind::OAuth2)),
            ("AWS v4", Some(AuthenticationKind::AwsV4)),
            ("WSSE", Some(AuthenticationKind::Wsse)),
            ("Digest", Some(AuthenticationKind::Digest)),
            ("NTLM", Some(AuthenticationKind::Ntlm)),
        ];
        let choice_count = choices.len();
        let mut kind_buttons = div().flex().flex_wrap().gap(px(theme.metrics.spacing_1));
        for (index, (label, kind)) in choices.into_iter().enumerate() {
            let kind_view = cx.weak_entity();
            kind_buttons = kind_buttons.child(components::editor_subtab(
                theme,
                ("authentication-kind", index),
                label,
                active == Some(label) || (active.is_none() && label == "None"),
                index + 1,
                choice_count,
                move |_, _, cx| {
                    let kind = kind.clone();
                    let _ = kind_view.update(cx, |view, cx| {
                        view.edit_request(key, |request| set_authentication(request, kind), cx);
                    });
                },
            ));
        }

        let mut editor = div()
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_2))
            .child(kind_buttons);
        if let Some(authentication) = &request.authentication {
            for (index, (property_name, value)) in authentication.properties.iter().enumerate() {
                let old_name = property_name.clone();
                let name_view = cx.weak_entity();
                let value_name = property_name.clone();
                let value_view = cx.weak_entity();
                let remove_name = property_name.clone();
                let remove_view = cx.weak_entity();
                editor = editor.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_1))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("authentication-property-name", index),
                                property_name.clone(),
                                "Property",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let old_name = old_name.clone();
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                let Some(authentication) =
                                                    request.authentication.as_mut()
                                                else {
                                                    return;
                                                };
                                                if let Some(old_value) =
                                                    authentication.properties.remove(&old_name)
                                                {
                                                    authentication
                                                        .properties
                                                        .insert(value.to_string(), old_value);
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("authentication-property-value", index),
                                auth_value(value),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let value_name = value_name.clone();
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                set_auth_property(
                                                    request,
                                                    value_name,
                                                    value.to_string(),
                                                )
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-authentication-property", index),
                            "Remove authentication property",
                            move |_, window, cx| {
                                let remove_name = remove_name.clone();
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(authentication) =
                                                request.authentication.as_mut()
                                            {
                                                authentication.properties.remove(&remove_name);
                                            }
                                        },
                                        cx,
                                    );
                                    view.focus_handle.focus(window, cx);
                                });
                            },
                        )),
                );
            }
            let add_view = cx.weak_entity();
            editor = editor.child(components::editor_add_button(
                theme,
                "add-authentication-property",
                "Add property",
                move |_, _, cx| {
                    let _ = add_view.update(cx, |view, cx| {
                        view.edit_request(
                            key,
                            |request| {
                                let Some(authentication) = request.authentication.as_mut() else {
                                    return;
                                };
                                let mut index = authentication.properties.len() + 1;
                                let mut name = "property".to_owned();
                                while authentication.properties.contains_key(&name) {
                                    name = format!("property{index}");
                                    index += 1;
                                }
                                authentication
                                    .properties
                                    .insert(name, AuthenticationValue::String(String::new()));
                            },
                            cx,
                        );
                    });
                },
            ));
        } else {
            editor = editor.child(
                div()
                    .text_color(theme.colors.text.muted)
                    .child("This request does not use authentication."),
            );
        }
        editor.into_any_element()
    }

    fn render_response_panel(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let active_key = self.shell.active_tab();
        let state = active_key.and_then(|key| self.execution.response(key));
        let (summary, content) = match state {
            Some(state @ ResponseState::Running { .. }) => (
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label(format!(
                            "Sending… • {}",
                            format_duration(state.elapsed().unwrap_or_default())
                        ))
                        .text_color(theme.colors.text.muted),
                    )
                    .into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Waiting for the server…")
                    .into_any_element(),
            ),
            Some(ResponseState::Cancelled) => (
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label("Cancelled")
                            .text_color(theme.colors.text.muted),
                    )
                    .into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Request cancelled.")
                    .into_any_element(),
            ),
            Some(ResponseState::Failed(error)) => (
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label("Failed").text_color(theme.colors.status.error),
                    )
                    .into_any_element(),
                div()
                    .id("response-error-scroll")
                    .flex_1()
                    .p(px(theme.metrics.spacing_3))
                    .overflow_y_scroll()
                    .text_color(theme.colors.status.error)
                    .child(error.clone())
                    .into_any_element(),
            ),
            Some(ResponseState::Complete(response)) => {
                let status = format!("{} {}", response.status, response.reason);
                let metadata = format!(
                    "• {} • {}",
                    format_duration(response.duration),
                    format_size(response.size),
                );
                let document = active_key.and_then(|key| self.response_viewer.document(key));
                (
                    div()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(theme.metrics.spacing_1))
                        .child(
                            components::truncated_label(status.trim_end().to_owned())
                                .id("response-status-code")
                                .debug_selector(|| "response-status-code".into())
                                .flex_none()
                                .max_w(px(220.0))
                                .text_color(response_status_color(theme, response.status)),
                        )
                        .child(
                            div()
                                .id("response-metadata")
                                .debug_selector(|| "response-metadata".into())
                                .flex_none()
                                .text_color(theme.colors.text.muted)
                                .child(metadata),
                        )
                        .into_any_element(),
                    self.render_response_document(theme, document, cx),
                )
            }
            None => (
                div().into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Send a request to see its response.")
                    .into_any_element(),
            ),
        };

        div()
            .when(self.shell.pane_layout == PaneLayout::Vertical, |panel| {
                panel.h(px(self.shell.response_height)).w_full()
            })
            .when(self.shell.pane_layout == PaneLayout::Horizontal, |panel| {
                panel.w(px(self.shell.response_width)).h_full()
            })
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.raised)
            .child(
                div()
                    .h(px(theme.metrics.tab_bar_height))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Response"))
                    .child(
                        div()
                            .id("response-status")
                            .debug_selector(|| "response-status".into())
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(theme.typography.caption_size))
                            .child(summary),
                    ),
            )
            .child(content)
    }

    fn render_response_document(
        &self,
        theme: Theme,
        document: Option<&PreparedDocument>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(key) = self.shell.active_tab() else {
            return div().into_any_element();
        };
        let Some(document) = document else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.colors.text.muted)
                .child("Preparing response…")
                .into_any_element();
        };

        let mut tabs = Tabs::new("response-view-tabs")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        for (index, tab) in ResponseViewerTab::ALL.into_iter().enumerate() {
            let tab_view = cx.weak_entity();
            let selected = self.response_viewer.tab() == tab;
            let label = if tab == ResponseViewerTab::Inspect {
                let count = document.inspection.count();
                if count > 0 {
                    format!("Inspect [{count}]")
                } else {
                    tab.label().to_owned()
                }
            } else {
                tab.label().to_owned()
            };
            tabs = tabs.child(
                components::text_tab(
                    theme,
                    ("response-view-tab", index),
                    label,
                    selected,
                    index + 1,
                    ResponseViewerTab::ALL.len(),
                    move |_, _, cx| {
                        let _ = tab_view.update(cx, |view, cx| {
                            view.response_viewer.set_tab(tab);
                            cx.notify();
                        });
                    },
                )
                .debug_selector(move || {
                    format!("response-tab-{}", tab.label().to_ascii_lowercase())
                }),
            );
        }

        let inspect_selected = self.response_viewer.tab() == ResponseViewerTab::Inspect;
        let matches = self.response_viewer.matches(key);
        let match_count = matches.len();
        let search_label = if self.response_viewer.search().is_empty() {
            String::new()
        } else if match_count == 0 {
            "No matches".to_owned()
        } else {
            format!(
                "{} of {match_count}",
                self.response_viewer.active_match() + 1
            )
        };
        let search_view = cx.weak_entity();
        let enter_view = cx.weak_entity();
        let previous_view = cx.weak_entity();
        let next_view = cx.weak_entity();
        let search = div()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .child(
                div()
                    .id("response-search-count")
                    .debug_selector(|| "response-search-count".into())
                    .text_size(px(theme.typography.caption_size))
                    .text_color(theme.colors.text.muted)
                    .mr(px(theme.metrics.spacing_1))
                    .child(search_label),
            )
            .child(components::search_input(
                theme,
                "response-search-input",
                self.response_viewer.search().to_owned(),
                "Search",
                move |value, _, input_cx| {
                    let _ = search_view.update(input_cx, |view, cx| {
                        view.response_viewer.set_search(value.to_string());
                        cx.notify();
                    });
                },
                move |_, _, input_cx| {
                    let _ = enter_view.update(input_cx, |view, cx| {
                        view.step_response_match(key, 1);
                        cx.notify();
                    });
                },
            ))
            .child(components::compact_icon_button(
                theme,
                "response-search-previous",
                "Previous search result",
                components::chevron_icon(theme, true),
                move |_, _, cx| {
                    let _ = previous_view.update(cx, |view, cx| {
                        view.step_response_match(key, -1);
                        cx.notify();
                    });
                },
            ))
            .child(components::compact_icon_button(
                theme,
                "response-search-next",
                "Next search result",
                components::chevron_icon(theme, false),
                move |_, _, cx| {
                    let _ = next_view.update(cx, |view, cx| {
                        view.step_response_match(key, 1);
                        cx.notify();
                    });
                },
            ));

        let mut banners = div()
            .px(px(theme.metrics.spacing_2))
            .pt(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1));
        let mut has_banner = false;
        if document.truncated {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.status.warning)
                    .text_size(px(theme.typography.caption_size))
                    .child("Response body is truncated at the in-memory limit."),
            );
        }
        if let Some(notice) = &document.pretty_notice
            && !matches!(
                self.response_viewer.tab(),
                ResponseViewerTab::Headers | ResponseViewerTab::Inspect
            )
        {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.text.muted)
                    .text_size(px(theme.typography.caption_size))
                    .child(notice.clone()),
            );
        }

        let list = match self.response_viewer.tab() {
            ResponseViewerTab::Headers => self.render_response_headers(theme, key, document, cx),
            ResponseViewerTab::Inspect => self.render_response_inspector(theme, key, document, cx),
            ResponseViewerTab::Pretty | ResponseViewerTab::Raw => {
                self.render_response_body(theme, key, document, cx)
            }
        };

        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(theme.metrics.spacing_2))
                    .py(px(theme.metrics.spacing_1))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(tabs)
                    .when(!inspect_selected, |bar| bar.child(search)),
            )
            .when(has_banner, |panel| panel.child(banners))
            .child(list)
            .into_any_element()
    }

    fn step_response_match(&mut self, key: probe_core::RequestKey, delta: isize) {
        self.response_viewer.step_match(key, delta);
    }

    fn render_response_body(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.binary {
            return placeholder_message(theme, "Binary response body cannot be displayed as text.");
        }
        let text = self.response_viewer.visible_text(key);
        if text.is_empty() {
            return placeholder_message(theme, "Empty response body.");
        }
        let matches = self.response_viewer.matches(key);
        let active_match = self.response_viewer.active_match();
        let view = cx.weak_entity();
        let body_mouse_view = cx.weak_entity();
        let inspect_view = cx.weak_entity();
        let inspect_ranges = document.inspection_ranges.clone();
        let inspect_context_enabled = self.response_viewer.tab() == ResponseViewerTab::Pretty
            && document.pretty_notice.is_none();
        let pretty_reveal = if self.response_viewer.tab() == ResponseViewerTab::Pretty {
            self.pretty_reveal.get()
        } else {
            None
        };
        let inspection_reveal = pretty_reveal
            .and_then(|reveal| {
                self.response_viewer
                    .inspection_range_for_selection(key, reveal.selection)
                    .map(|range| (range, reveal.scroll_pending))
            })
            .and_then(|reveal| {
                (self.response_viewer.tab() == ResponseViewerTab::Pretty).then_some(reveal)
            });
        if let Some(reveal) = pretty_reveal
            && reveal.scroll_pending
            && self.response_viewer.tab() == ResponseViewerTab::Pretty
        {
            self.pretty_reveal.set(Some(PrettyRevealState {
                selection: reveal.selection,
                scroll_pending: false,
            }));
        }
        div()
            .id("response-body")
            .debug_selector(|| "response-body".into())
            .flex_1()
            .min_h(px(0.0))
            .p(px(theme.metrics.spacing_2))
            .child(components::response_body_input(
                theme,
                "response-body-editor",
                text,
                components::ResponseBodyInputOptions::new(
                    &matches,
                    active_match,
                    if self.response_viewer.tab() == ResponseViewerTab::Pretty
                        && document.pretty_notice.is_none()
                    {
                        "json"
                    } else {
                        ""
                    },
                    move |range, cx| {
                        #[cfg(test)]
                        {
                            let _ = view.update(cx, |this, _| {
                                this.rendered_response_rows = range.len();
                            });
                        }
                        #[cfg(not(test))]
                        {
                            let _ = (&view, range, cx);
                        }
                    },
                    move |_, cx| {
                        let _ = body_mouse_view.update(cx, |view, cx| {
                            if view.response_viewer.tab() == ResponseViewerTab::Pretty
                                && view.pretty_reveal.take().is_some()
                            {
                                cx.notify();
                            }
                        });
                    },
                    move |_, offset| {
                        inspect_context_enabled
                            && inspect_ranges
                                .iter()
                                .any(|entry| entry.range.contains(&offset))
                    },
                    move |_, offset, _, cx| {
                        let _ = inspect_view.update(cx, |view, cx| {
                            if view.response_viewer.tab() != ResponseViewerTab::Pretty {
                                view.message = Some(
                                    "Inspect from the Pretty tab to select a response value."
                                        .to_owned(),
                                );
                            } else if view
                                .response_viewer
                                .document(key)
                                .is_some_and(|document| document.inspection_pending)
                            {
                                view.response_viewer.set_tab(ResponseViewerTab::Inspect);
                                view.message = Some("Inspection is still running.".to_owned());
                            } else if let Some(selection) = view
                                .response_viewer
                                .select_inspection_at_offset(key, offset)
                            {
                                view.pending_inspector_reveal.set(Some(selection));
                                view.pretty_reveal.set(None);
                            } else {
                                view.message = Some(
                                    "No inspected JWT or timestamp found at that value.".to_owned(),
                                );
                            }
                            cx.notify();
                        });
                    },
                )
                .inspection_reveal(inspection_reveal),
            ))
            .into_any_element()
    }

    fn render_response_headers(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.headers.is_empty() {
            return placeholder_message(theme, "No response headers");
        }
        let matches = self.response_viewer.matches(key);
        let active_match = self.response_viewer.active_match();
        let view = cx.weak_entity();
        div()
            .id("response-headers")
            .debug_selector(|| "response-headers".into())
            .flex_1()
            .min_h(px(0.0))
            .p(px(theme.metrics.spacing_2))
            .child(components::response_headers_input(
                theme,
                "response-headers-editor",
                &document.headers,
                &matches,
                active_match,
                move |range, cx| {
                    #[cfg(test)]
                    {
                        let _ = view.update(cx, |this, _| {
                            this.rendered_response_rows = range.len();
                        });
                    }
                    #[cfg(not(test))]
                    {
                        let _ = (&view, range, cx);
                    }
                },
            ))
            .into_any_element()
    }

    fn render_response_inspector(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = inspect_list_rows(document);
        if let Some(selection) = self.pending_inspector_reveal.take()
            && let Some(index) = inspect_row_index(&rows, selection)
        {
            self.inspector_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
        let selected = self.response_viewer.inspection_selection(key);
        let detail = if document.inspection_pending {
            "Inspecting response…".to_owned()
        } else {
            inspection_detail_text(&document.inspection, selected)
        };
        let revealable = selected.is_some_and(|selection| {
            self.response_viewer
                .inspection_range_for_selection(key, selection)
                .is_some()
        }) && document.pretty_notice.is_none();
        let view = cx.weak_entity();
        let row_count = rows.len();
        let rows_for_list = rows.clone();
        let list = uniform_list("response-inspector-list", row_count, {
            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        let document = view.response_viewer.document(key)?;
                        let selected = view.response_viewer.inspection_selection(key);
                        rows_for_list.get(index).copied().map(|row| {
                            view.render_inspector_list_row(theme, key, row, document, selected, cx)
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .size_full()
        .track_scroll(&self.inspector_scroll);

        if row_count == 0 && !document.inspection_pending {
            return div()
                .id("response-inspector-empty")
                .debug_selector(|| "response-inspector-empty".into())
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.colors.text.muted)
                .child(document.inspection.skipped.clone().unwrap_or_else(|| {
                    "JWTs and Unix timestamps are detected automatically.".to_owned()
                }))
                .into_any_element();
        }

        let divider_view = cx.weak_entity();
        let divider = div()
            .id("response-inspector-divider")
            .debug_selector(|| "response-inspector-divider".into())
            .w(px(5.0))
            .h_full()
            .flex_none()
            .border_l_1()
            .border_color(theme.colors.borders.subtle)
            .cursor(CursorStyle::ResizeLeftRight)
            .hover(move |handle| handle.bg(theme.colors.borders.subtle))
            .on_mouse_down(MouseButton::Left, move |event, _, cx| {
                let _ = divider_view.update(cx, |view, cx| {
                    view.inspector_resize_start =
                        Some((f32::from(event.position.x), view.inspector_list_width));
                    cx.notify();
                });
            });

        div()
            .id("response-inspector")
            .debug_selector(|| "response-inspector".into())
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .child(
                div()
                    .w(px(self.inspector_list_width))
                    .flex_none()
                    .min_h(px(0.0))
                    .p(px(theme.metrics.spacing_2))
                    .child(list),
            )
            .child(divider)
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .p(px(theme.metrics.spacing_2))
                    .child(
                        div()
                            .relative()
                            .size_full()
                            .child(components::response_inspector_input(
                                theme,
                                "response-inspector-editor",
                                detail,
                                move |range, cx| {
                                    #[cfg(test)]
                                    {
                                        let _ = view.update(cx, |this, _| {
                                            this.rendered_response_rows = range.len();
                                        });
                                    }
                                    #[cfg(not(test))]
                                    {
                                        let _ = (&view, range, cx);
                                    }
                                },
                            ))
                            .when(revealable, |detail| {
                                let reveal_view = cx.weak_entity();
                                detail.child(
                                    div()
                                        .absolute()
                                        .top(px(theme.metrics.spacing_3))
                                        .right(px(theme.metrics.spacing_3))
                                        .child(components::compact_icon_button(
                                            theme,
                                            "response-inspector-reveal-pretty",
                                            "Reveal in Pretty",
                                            components::locate_icon(theme),
                                            move |_, _, cx| {
                                                let _ = reveal_view.update(cx, |view, cx| {
                                                    if let Some(selection) = view
                                                        .response_viewer
                                                        .reveal_inspection_in_pretty(key)
                                                    {
                                                        view.pretty_reveal.set(Some(
                                                            PrettyRevealState {
                                                                selection,
                                                                scroll_pending: true,
                                                            },
                                                        ));
                                                    } else {
                                                        view.message = Some(
                                                            "Pretty source is unavailable."
                                                                .to_owned(),
                                                        );
                                                    }
                                                    cx.notify();
                                                });
                                            },
                                        )),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_inspector_list_row(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        row: InspectListRow,
        document: &PreparedDocument,
        selected: Option<InspectSelection>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match row {
            InspectListRow::Group { label, count } => div()
                .w_full()
                .h(px(theme.metrics.tree_row_height))
                .px(px(theme.metrics.spacing_1))
                .flex()
                .items_center()
                .gap(px(theme.metrics.spacing_1))
                .text_size(px(theme.typography.caption_size))
                .text_color(theme.colors.text.secondary)
                .font_weight(FontWeight::SEMIBOLD)
                .child(label)
                .child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme.colors.text.muted)
                        .child(format!("[{count}]")),
                )
                .into_any_element(),
            InspectListRow::Item { selection } => {
                let label = inspect_row_label(document, selection);
                let row_view = cx.weak_entity();
                let is_selected = selected == Some(selection);
                div()
                    .id("response-inspector-list-row")
                    .debug_selector(|| "response-inspector-list-row".into())
                    .w_full()
                    .h(px(theme.metrics.tree_row_height))
                    .px(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .rounded(px(theme.metrics.radius_small))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(theme.typography.caption_size))
                    .text_color(theme.colors.text.primary)
                    .bg(if is_selected {
                        theme.colors.selection.inactive_background
                    } else {
                        theme.colors.surfaces.raised
                    })
                    .hover(move |row| {
                        if is_selected {
                            row
                        } else {
                            row.bg(theme.colors.surfaces.editor)
                        }
                    })
                    .child(components::truncated_label(label).min_w(px(0.0)))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let _ = row_view.update(cx, |view, cx| {
                            view.response_viewer.select_inspection(key, selection);
                            view.pretty_reveal.set(None);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            }
        }
    }

    fn render_editor_response(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let response_view = cx.weak_entity();
        let horizontal = self.shell.pane_layout == PaneLayout::Horizontal;
        let handle = div()
            .id("response-resize-handle")
            .flex_none()
            .when(horizontal, |handle| {
                handle
                    .w(px(5.0))
                    .h_full()
                    .border_l_1()
                    .border_color(theme.colors.borders.subtle)
                    .cursor(CursorStyle::ResizeLeftRight)
            })
            .when(!horizontal, |handle| {
                handle
                    .h(px(5.0))
                    .w_full()
                    .border_t_1()
                    .border_color(theme.colors.borders.subtle)
                    .cursor(CursorStyle::ResizeUpDown)
            })
            .hover(move |handle| handle.bg(theme.colors.borders.subtle))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = response_view.update(cx, |view, cx| {
                    view.shell.resizing = Some(ResizePane::Response);
                    cx.notify();
                });
            });

        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .when(horizontal, |work_area| work_area.flex_row())
            .when(!horizontal, |work_area| work_area.flex_col())
            .child(self.render_request_editor(theme, cx))
            .child(handle)
            .child(self.render_response_panel(theme, cx))
    }

    pub(super) fn active_request(&self) -> Option<&HttpRequest> {
        let key = self.shell.active_tab()?;
        self.loaded_workspace.as_ref()?.workspace().request(key)
    }

    fn variable_context(&self, cx: &mut Context<Self>) -> components::VariableContext {
        let Some(selected) = self.shell.selected_environment() else {
            return components::VariableContext {
                values: Default::default(),
                unavailable_message: "Select an environment to resolve this variable".to_owned(),
                on_change: None,
                ..components::VariableContext::default()
            };
        };
        let Some(loaded) = &self.loaded_workspace else {
            return components::VariableContext::default();
        };
        match resolve_environment(loaded.workspace().environments(), selected) {
            Ok(environment) => {
                let view = cx.weak_entity();
                components::VariableContext {
                    values: environment.variables().clone(),
                    secrets: environment.secrets_without_values().clone(),
                    unavailable_message: "Variable value is unavailable".to_owned(),
                    on_change: Some(Rc::new(move |name, value, window, cx| {
                        let name = name.to_owned();
                        let view = view.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.update_environment_variable(&name, value, window, cx);
                            });
                        });
                    })),
                }
            }
            Err(error) => components::VariableContext {
                values: Default::default(),
                unavailable_message: error.to_string(),
                on_change: None,
                ..components::VariableContext::default()
            },
        }
    }
}

impl Render for ProbeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag()
            && (self.tree_drop_target.is_some() || self.tree_drag_source.is_some())
        {
            self.clear_tree_drag();
        }
        if self.pending_tab_reveal {
            self.pending_tab_reveal = false;
            cx.on_next_frame(window, |this, _, cx| {
                this.scroll_active_tab_into_view();
                cx.notify();
            });
        }
        let theme = Theme::for_window_appearance(window.appearance());
        let sidebar_view = cx.weak_entity();
        let status_message = self.message.clone();

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .bg(theme.colors.surfaces.window)
            .text_color(theme.colors.text.primary)
            .font_family(theme.typography.interface_family)
            .text_size(px(theme.typography.body_size))
            .line_height(relative(theme.typography.body_line_height))
            .flex()
            .flex_col()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.reset_caret_blink(cx)),
            )
            .on_action(cx.listener(|view, _: &SaveRequest, window, cx| {
                if view.application_dialog.is_none() {
                    view.save_active_request(window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &OpenFileMenu, _, cx| {
                view.open_desktop_menu(DesktopMenu::File, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenEditMenu, _, cx| {
                view.open_desktop_menu(DesktopMenu::Edit, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenViewMenu, _, cx| {
                view.open_desktop_menu(DesktopMenu::View, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenHelpMenu, _, cx| {
                view.open_desktop_menu(DesktopMenu::Help, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenWorkspace, window, cx| {
                if view.application_dialog.is_none() {
                    view.choose_workspace(window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &NewCollection, window, cx| {
                if !view.loading && view.application_dialog.is_none() {
                    view.choose_new_workspace(window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &ImportPostmanExport, window, cx| {
                if !view.loading && view.application_dialog.is_none() {
                    view.request_import(ImportSource::Postman, window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &ImportYaakExport, window, cx| {
                if !view.loading && view.application_dialog.is_none() {
                    view.request_import(ImportSource::Yaak, window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &OpenImportSubmenu, window, cx| {
                if view.workspace_switcher_open {
                    view.workspace_import_submenu_open = true;
                    view.workspace_import_popup_focus.focus(window, cx);
                } else {
                    view.sidebar_import_menu_open = true;
                    view.sidebar_import_popup_focus.focus(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CloseImportSubmenu, window, cx| {
                if view.workspace_import_submenu_open {
                    view.workspace_import_submenu_open = false;
                    view.workspace_import_trigger_focus.focus(window, cx);
                } else if view.sidebar_import_menu_open {
                    view.sidebar_import_menu_open = false;
                    view.sidebar_import_trigger_focus.focus(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CloseActiveTab, window, cx| {
                if view.application_dialog.is_none()
                    && let Some(key) = view.shell.active_tab()
                {
                    view.request_close_tab(key, window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &AboutProbe, window, cx| {
                view.show_application_dialog(ApplicationDialog::About, window, cx);
            }))
            .on_action(cx.listener(|view, _: &CloseWindow, window, cx| {
                if view.request_close_window(window, cx) {
                    window.remove_window();
                }
            }))
            .on_action(cx.listener(|_, _: &MinimizeWindow, window, _| {
                window.minimize_window();
            }))
            .on_action(cx.listener(|_, _: &ZoomWindow, window, _| {
                window.zoom_window();
            }))
            .on_action(cx.listener(|view, _: &ToggleSidebar, _, cx| {
                view.toggle_sidebar(cx);
            }))
            .on_action(cx.listener(|view, _: &UseVerticalEditorLayout, _, cx| {
                view.set_pane_layout(PaneLayout::Vertical, cx);
            }))
            .on_action(cx.listener(|view, _: &UseHorizontalEditorLayout, _, cx| {
                view.set_pane_layout(PaneLayout::Horizontal, cx);
            }))
            .on_action(cx.listener(|view, _: &QuitApplication, window, cx| {
                view.quit_application(window, cx);
            }))
            .on_action(cx.listener(|_, _: &FocusNextControl, window, cx| {
                window.focus_next(cx);
            }))
            .on_action(cx.listener(|_, _: &FocusPreviousControl, window, cx| {
                window.focus_prev(cx);
            }))
            .on_action(cx.listener(|view, _: &NewRequest, window, cx| {
                view.open_create_request_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &NewFolder, window, cx| {
                view.open_create_folder_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &DuplicateRequest, window, cx| {
                view.duplicate_selected_request(window, cx);
            }))
            .on_action(cx.listener(|view, _: &RenameTreeItem, window, cx| {
                view.open_rename_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &DeleteTreeItem, window, cx| {
                view.request_delete_selected(window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItem, window, cx| {
                view.open_move_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItemUp, window, cx| {
                view.reorder_selected(-1, window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItemDown, window, cx| {
                view.reorder_selected(1, window, cx);
            }))
            .on_action(cx.listener(|view, _: &SelectPreviousTreeItem, _, cx| {
                view.select_tree_offset(-1, cx);
            }))
            .on_action(cx.listener(|view, _: &SelectNextTreeItem, _, cx| {
                view.select_tree_offset(1, cx);
            }))
            .on_action(cx.listener(|view, _: &CollapseTreeItem, _, cx| {
                view.collapse_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &ExpandTreeItem, _, cx| {
                view.expand_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &ActivateTreeItem, _, cx| {
                view.activate_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &SubmitStructureDialog, window, cx| {
                view.submit_structure_dialog(window, cx);
            }))
            .on_action(
                cx.listener(|view, _: &SubmitCreateEnvironmentDialog, window, cx| {
                    view.submit_create_environment_dialog(window, cx);
                }),
            )
            .on_action(
                cx.listener(|view, _: &SubmitApplicationDialog, window, cx| {
                    view.submit_application_dialog_primary(window, cx);
                }),
            )
            .on_action(
                cx.listener(|view, _: &SubmitApplicationDialogDestructive, window, cx| {
                    view.submit_application_dialog_destructive(window, cx);
                }),
            )
            .on_action(cx.listener(|view, _: &CancelStructureDialog, window, cx| {
                view.structure_dialog = None;
                view.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .on_action(
                cx.listener(|view, _: &CancelCreateEnvironmentDialog, window, cx| {
                    view.close_create_environment_dialog(window, cx);
                }),
            )
            .on_action(
                cx.listener(|view, _: &CancelApplicationDialog, window, cx| {
                    view.handle_application_dialog_action(
                        ApplicationDialogAction::Cancel,
                        window,
                        cx,
                    );
                }),
            )
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.finish_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.finish_resize(cx)),
            )
            .child(self.render_titlebar(theme, window, cx))
            .when_some(status_message, |root, message| {
                root.child(
                    div()
                        .px(px(theme.metrics.spacing_3))
                        .py(px(theme.metrics.spacing_2))
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(px(theme.metrics.spacing_2))
                        .bg(theme.colors.status.error)
                        .text_color(theme.colors.text.inverse)
                        .child(div().flex_1().min_w(px(0.0)).child(message))
                        .child(
                            Button::new("status-message-dismiss")
                                .focusable(true)
                                .tab_stop(true)
                                .flex_none()
                                .w(px(theme.metrics.control_height - 4.0))
                                .h(px(theme.metrics.control_height - 4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(theme.metrics.radius_small))
                                .text_color(theme.colors.text.inverse)
                                .hover(move |button| {
                                    button.bg(components::hover_fill(theme.colors.status.error))
                                })
                                .on_click({
                                    let dismiss_view = cx.weak_entity();
                                    move |_, _, cx| {
                                        let _ = dismiss_view.update(cx, |view, cx| {
                                            view.message = None;
                                            cx.notify();
                                        });
                                    }
                                })
                                .child(
                                    components::close_icon(theme)
                                        .text_color(theme.colors.text.inverse),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .bg(theme.colors.surfaces.raised)
                    .when(!self.shell.sidebar_collapsed, |row| {
                        row.child(self.render_sidebar(theme, cx)).child(
                            div()
                                .id("sidebar-resize-handle")
                                .w(px(5.0))
                                .h_full()
                                .ml(px(-5.0))
                                .flex_none()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    let _ = sidebar_view.update(cx, |view, cx| {
                                        view.shell.resizing = Some(ResizePane::Sidebar);
                                        cx.notify();
                                    });
                                }),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(self.render_tabs(theme, cx))
                            .child(self.render_editor_response(theme, cx)),
                    ),
            )
            .child(self.render_structure_dialog(theme, window, cx))
            .child(self.render_create_environment_dialog(theme, window, cx))
            .child(self.render_application_dialog(theme, window, cx))
            .child(self.render_tab_context_menu(theme, window, cx))
            .child(self.render_tree_context_menu(theme, window, cx))
    }
}
