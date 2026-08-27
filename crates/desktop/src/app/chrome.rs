use super::*;

fn environment_variable_text(variable: &Variable) -> (String, bool) {
    match variable.value.as_ref() {
        Some(VariableValueSet::Single(VariableValue::String(value))) => (value.clone(), true),
        Some(VariableValueSet::Single(VariableValue::Typed { data, .. })) => (data.clone(), true),
        Some(VariableValueSet::Variants(variants)) => (
            variants
                .iter()
                .find(|variant| variant.selected)
                .map(|variant| match &variant.value {
                    VariableValue::String(value) => value.clone(),
                    VariableValue::Typed { data, .. } => data.clone(),
                })
                .unwrap_or_default(),
            false,
        ),
        None => (String::new(), true),
    }
}

fn set_environment_variable_text(variable: &mut Variable, value: String) {
    match variable.value.as_mut() {
        Some(VariableValueSet::Single(VariableValue::String(current))) => *current = value,
        Some(VariableValueSet::Single(VariableValue::Typed { data, .. })) => *data = value,
        Some(VariableValueSet::Variants(_)) => {}
        None => {
            variable.value = Some(VariableValueSet::Single(VariableValue::String(value)));
        }
    }
}

fn environment_variant_value(
    theme: Theme,
    name: &str,
    row_index: usize,
    value: String,
    inherited: bool,
) -> gpui::AnyElement {
    let selector = if name.is_empty() {
        format!("environment-variable-variant-{row_index}")
    } else {
        format!("environment-variable-variant-{name}")
    };
    div()
        .id(selector.clone())
        .debug_selector(move || selector.clone())
        .flex_1()
        .min_w(px(120.0))
        .flex()
        .flex_col()
        .justify_center()
        .gap(px(2.0))
        .child(
            components::truncated_label(value)
                .font_family(theme.typography.monospace_family)
                .when(inherited, |label| label.text_color(theme.colors.text.muted)),
        )
        .child(
            div()
                .text_size(px(theme.typography.caption_size))
                .text_color(theme.colors.text.muted)
                .child("Multiple values"),
        )
        .into_any_element()
}

impl ProbeApp {
    fn handle_titlebar_mouse_down(event: &MouseDownEvent, window: &mut Window) {
        if event.click_count > 1 {
            #[cfg(target_os = "macos")]
            window.titlebar_double_click();

            #[cfg(any(target_os = "linux", target_os = "windows"))]
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    }

    fn render_desktop_action_menu_item(
        theme: Theme,
        id: ElementId,
        label: &'static str,
        action: Box<dyn Action>,
        checked: Option<bool>,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(checked) = checked {
            let view = cx.weak_entity();
            return components::checked_menu_button(
                theme,
                id.clone(),
                label,
                checked,
                move |window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.close_desktop_menu(cx);
                    });
                    window.dispatch_action(action.boxed_clone(), cx);
                },
            )
            .into_any_element();
        }

        let shortcut = components::shortcut_label_for_action(window, action.as_ref());
        let view = cx.weak_entity();
        components::menu_button(theme, id, label, shortcut, move |window, cx| {
            let _ = view.update(cx, |view, cx| {
                view.close_desktop_menu(cx);
            });
            window.dispatch_action(action.boxed_clone(), cx);
        })
        .into_any_element()
    }

    fn render_desktop_top_level_menu(
        &self,
        theme: Theme,
        menu: DesktopMenu,
        id: &'static str,
        label: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.desktop_menu_open == Some(menu);
        let hover_view = cx.weak_entity();
        let change_view = cx.weak_entity();
        let popup = self.render_desktop_menu_popup(theme, menu, window, cx);

        Popover::new(id)
            .open(open)
            .on_open_change(move |open, _, cx| {
                let _ = change_view.update(cx, |view, cx| {
                    view.desktop_menu_open = open.then_some(menu);
                    view.desktop_submenu_open = None;
                    cx.notify();
                });
            })
            .trigger(components::app_menu_trigger(
                theme,
                (id, 0_usize),
                label,
                open,
                move |cx| {
                    let _ = hover_view.update(cx, |view, cx| {
                        if view.desktop_menu_open.is_some() && view.desktop_menu_open != Some(menu)
                        {
                            view.open_desktop_menu(menu, cx);
                        }
                    });
                },
            ))
            .content(move |_, _, _| popup)
            .into_any_element()
    }

    fn render_desktop_menu_bar(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if cfg!(target_os = "macos") {
            return div().into_any_element();
        }

        let mut bar = div()
            .id("desktop-menu-bar")
            .debug_selector(|| "desktop-menu-bar".into())
            .h_full()
            .flex_none()
            .flex()
            .items_center();
        for (menu, id, label) in [
            (DesktopMenu::File, "desktop-file-menu", "File"),
            (DesktopMenu::Edit, "desktop-edit-menu", "Edit"),
            (DesktopMenu::View, "desktop-view-menu", "View"),
            (DesktopMenu::Help, "desktop-help-menu", "Help"),
        ] {
            bar = bar.child(self.render_desktop_top_level_menu(theme, menu, id, label, window, cx));
        }
        bar.into_any_element()
    }

    fn render_desktop_menu_popup(
        &self,
        theme: Theme,
        menu: DesktopMenu,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let definition = self.desktop_menu_definition(menu);
        self.render_desktop_menu_definition(theme, definition, window, cx)
    }

    fn render_desktop_menu_definition(
        &self,
        theme: Theme,
        definition: DesktopMenuDefinition,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut popup = components::popup_surface(theme, definition.id, definition.width)
            .aria_label(format!("{} menu", definition.label));

        for (index, item) in definition.items.into_iter().enumerate() {
            let id = (definition.id, index).into();
            popup = match item {
                DesktopMenuItem::Action(label, action, checked) => {
                    popup.child(Self::render_desktop_action_menu_item(
                        theme, id, label, action, checked, window, cx,
                    ))
                }
                DesktopMenuItem::Separator => popup.child(components::menu_separator(theme)),
                DesktopMenuItem::Submenu(label, state, submenu) => {
                    let open = self.desktop_submenu_open == Some(state);
                    let submenu_view = cx.weak_entity();
                    let submenu = self.render_desktop_menu_definition(theme, submenu, window, cx);
                    popup.child(components::cascading_menu(
                        theme,
                        id,
                        label,
                        open,
                        definition.width,
                        submenu,
                        move |cx| {
                            let _ = submenu_view.update(cx, |view, cx| {
                                if view.desktop_submenu_open != Some(state) {
                                    view.desktop_submenu_open = Some(state);
                                    cx.notify();
                                }
                            });
                        },
                    ))
                }
            };
        }

        popup.into_any_element()
    }

    fn desktop_menu_definition(&self, menu: DesktopMenu) -> DesktopMenuDefinition {
        match menu {
            DesktopMenu::File => DesktopMenuDefinition {
                id: "desktop-file-menu-popup",
                label: "File",
                width: 240.0,
                items: vec![
                    DesktopMenuItem::action("New Collection…", NewCollection),
                    DesktopMenuItem::action("Open Collection…", OpenWorkspace),
                    DesktopMenuItem::submenu(
                        "Import From…",
                        DesktopSubmenu::Import,
                        DesktopMenuDefinition {
                            id: "desktop-import-menu-popup",
                            label: "Import",
                            width: 210.0,
                            items: vec![DesktopMenuItem::action("Yaak Export…", ImportYaakExport)],
                        },
                    ),
                    DesktopMenuItem::Separator,
                    DesktopMenuItem::action("Save Request", SaveRequest),
                    DesktopMenuItem::Separator,
                    DesktopMenuItem::action("Close Tab", CloseActiveTab),
                    DesktopMenuItem::action("Close Window", CloseWindow),
                    DesktopMenuItem::Separator,
                    DesktopMenuItem::action(
                        if cfg!(target_os = "windows") {
                            "Exit"
                        } else {
                            "Quit"
                        },
                        QuitApplication,
                    ),
                ],
            },
            DesktopMenu::Edit => DesktopMenuDefinition {
                id: "desktop-edit-menu-popup",
                label: "Edit",
                width: 220.0,
                items: vec![
                    DesktopMenuItem::action("Undo", Undo),
                    DesktopMenuItem::action("Redo", Redo),
                    DesktopMenuItem::Separator,
                    DesktopMenuItem::action("Cut", Cut),
                    DesktopMenuItem::action("Copy", Copy),
                    DesktopMenuItem::action("Paste", Paste),
                    DesktopMenuItem::Separator,
                    DesktopMenuItem::action("Select All", SelectAll),
                ],
            },
            DesktopMenu::View => DesktopMenuDefinition {
                id: "desktop-view-menu-popup",
                label: "View",
                width: 230.0,
                items: vec![
                    DesktopMenuItem::action(
                        if self.shell.sidebar_collapsed {
                            "Show Sidebar"
                        } else {
                            "Hide Sidebar"
                        },
                        ToggleSidebar,
                    ),
                    DesktopMenuItem::submenu(
                        "Editor Layout",
                        DesktopSubmenu::EditorLayout,
                        DesktopMenuDefinition {
                            id: "desktop-editor-layout-menu-popup",
                            label: "Editor Layout",
                            width: 190.0,
                            items: vec![
                                DesktopMenuItem::checked_action(
                                    "Vertical",
                                    self.shell.pane_layout == PaneLayout::Vertical,
                                    UseVerticalEditorLayout,
                                ),
                                DesktopMenuItem::checked_action(
                                    "Horizontal",
                                    self.shell.pane_layout == PaneLayout::Horizontal,
                                    UseHorizontalEditorLayout,
                                ),
                            ],
                        },
                    ),
                ],
            },
            DesktopMenu::Help => DesktopMenuDefinition {
                id: "desktop-help-menu-popup",
                label: "Help",
                width: 190.0,
                items: vec![DesktopMenuItem::action("About Probe", AboutProbe)],
            },
        }
    }

    pub(super) fn render_titlebar(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let switcher_view = cx.weak_entity();
        let sidebar_toggle_view = cx.weak_entity();
        let home_view = cx.weak_entity();
        let new_view = cx.weak_entity();
        let open_view = cx.weak_entity();
        let import_menu_view = cx.weak_entity();
        let import_postman_view = cx.weak_entity();
        let import_yaak_view = cx.weak_entity();
        let import_trigger_focus = self.workspace_import_trigger_focus.clone();
        let import_popup_focus = self.workspace_import_popup_focus.clone();
        let layout_view = cx.weak_entity();
        let collection_open = self.loaded_workspace.is_some();
        let mut popup = components::popup_surface(
            theme,
            "workspace-switcher-popup",
            WORKSPACE_SWITCHER_MENU_WIDTH,
        )
        .aria_label("Workspaces");

        if !self.session.recent_collections.is_empty() {
            popup = popup.child(
                div()
                    .px(px(theme.metrics.spacing_2))
                    .py(px(theme.metrics.spacing_1))
                    .text_size(px(theme.typography.caption_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.text.muted)
                    .child("RECENT COLLECTIONS"),
            );
            for (index, path) in self.session.recent_collections.iter().enumerate() {
                let open_path = path.clone();
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Collection")
                    .to_owned();
                let view = cx.weak_entity();
                popup = popup.child(components::menu_button(
                    theme,
                    ("workspace-switcher-recent", index),
                    label,
                    None,
                    move |window, cx| {
                        let path = open_path.clone();
                        let _ = view.update(cx, |view, cx| {
                            view.workspace_switcher_open = false;
                            if !view.loading {
                                view.request_load_workspace(path, None, window, cx);
                            }
                        });
                    },
                ));
            }
            popup = popup.child(
                div()
                    .my(px(theme.metrics.spacing_1))
                    .mx(px(theme.metrics.spacing_2))
                    .flex_none()
                    .h(px(1.0))
                    .bg(theme.colors.borders.standard),
            );
        }

        let import_popup =
            components::popup_surface(theme, "workspace-switcher-import-popup", 180.0)
                .debug_selector(|| "workspace-switcher-import-popup".into())
                .aria_label("Import providers")
                .track_focus(&self.workspace_import_popup_focus)
                .key_context("ImportSubmenu")
                .child(components::menu_button(
                    theme,
                    "workspace-switcher-import-postman",
                    "Postman Export…",
                    None,
                    move |window, cx| {
                        let _ = import_postman_view.update(cx, |view, cx| {
                            view.workspace_import_submenu_open = false;
                            view.workspace_switcher_open = false;
                            if !view.loading {
                                view.request_import(ImportSource::Postman, window, cx);
                            }
                        });
                    },
                ))
                .child(components::menu_button(
                    theme,
                    "workspace-switcher-import-yaak",
                    "Yaak Export…",
                    None,
                    move |window, cx| {
                        let _ = import_yaak_view.update(cx, |view, cx| {
                            view.workspace_import_submenu_open = false;
                            view.workspace_switcher_open = false;
                            if !view.loading {
                                view.request_import(ImportSource::Yaak, window, cx);
                            }
                        });
                    },
                ));
        let import_trigger = components::import_submenu_menu_button(
            theme,
            "workspace-switcher-import-from",
            "Import From…",
            self.workspace_import_submenu_open,
            &self.workspace_import_trigger_focus,
            move |window, cx| {
                let trigger_focus = import_trigger_focus.clone();
                let popup_focus = import_popup_focus.clone();
                let _ = import_menu_view.update(cx, |view, cx| {
                    view.workspace_import_submenu_open = !view.workspace_import_submenu_open;
                    if view.workspace_import_submenu_open {
                        popup_focus.focus(window, cx);
                    } else {
                        trigger_focus.focus(window, cx);
                    }
                    cx.notify();
                });
            },
        );
        let import_menu = components::positioned_cascading_menu(
            theme,
            self.workspace_import_submenu_open,
            WORKSPACE_SWITCHER_MENU_WIDTH,
            import_trigger,
            import_popup.into_any_element(),
        );

        popup = popup
            .child(components::menu_button(
                theme,
                "workspace-switcher-new",
                "New Collection…",
                components::shortcut_label_for_action(window, &NewCollection),
                move |window, cx| {
                    let _ = new_view.update(cx, |view, cx| {
                        view.workspace_switcher_open = false;
                        if !view.loading {
                            view.choose_new_workspace(window, cx);
                        }
                    });
                },
            ))
            .child(components::menu_button(
                theme,
                "workspace-switcher-open",
                "Open Collection…",
                components::shortcut_label_for_action(window, &OpenWorkspace),
                move |window, cx| {
                    let _ = open_view.update(cx, |view, cx| {
                        view.workspace_switcher_open = false;
                        if !view.loading {
                            view.choose_workspace(window, cx);
                        }
                    });
                },
            ))
            .child(import_menu);

        let switcher = Popover::new("workspace-switcher")
            .open(self.workspace_switcher_open)
            .on_open_change(move |open, _, cx| {
                let _ = switcher_view.update(cx, |view, cx| {
                    view.workspace_switcher_open = *open;
                    if !*open {
                        view.workspace_import_submenu_open = false;
                    }
                    cx.notify();
                });
            })
            .trigger(
                Button::new("workspace-switcher-trigger")
                    .accessibility_label("Switch workspace")
                    .selected(self.workspace_switcher_open)
                    .h(px(theme.metrics.control_height))
                    .max_w(px(260.0))
                    .px(px(theme.metrics.spacing_3))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_2))
                    .overflow_hidden()
                    .rounded(px(theme.metrics.radius_small))
                    .border_1()
                    .border_color(theme.colors.borders.subtle)
                    .debug_selector(|| "workspace-switcher-trigger".into())
                    .hover(move |trigger| trigger.bg(theme.colors.surfaces.sidebar))
                    .focus(move |trigger| trigger.border_color(theme.colors.borders.focused))
                    .styles(move |styles| {
                        styles.selected(move |trigger| trigger.bg(theme.colors.surfaces.sidebar))
                    })
                    .child(components::truncated_label(self.workspace_name()).flex_1())
                    .child(components::chevron_icon(
                        theme,
                        self.workspace_switcher_open,
                    )),
            )
            .content(move |_, _, _| popup);

        div()
            .h(px(theme.metrics.tab_bar_height))
            .w_full()
            .pl(px(if cfg!(target_os = "macos") {
                80.0
            } else {
                theme.metrics.spacing_3
            }))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .bg(theme.colors.surfaces.raised)
            .child(components::sidebar_toggle(
                theme,
                self.shell.sidebar_collapsed,
                move |_, cx| {
                    let _ = sidebar_toggle_view.update(cx, |view, cx| {
                        view.toggle_sidebar(cx);
                    });
                },
            ))
            .child(components::home_button(
                theme,
                collection_open,
                move |window, cx| {
                    let _ = home_view.update(cx, |view, cx| {
                        if view.loaded_workspace.is_some() {
                            view.request_close_workspace(window, cx);
                        }
                    });
                },
            ))
            .child(switcher)
            .child(self.render_desktop_menu_bar(theme, window, cx))
            .child(
                div()
                    .h_full()
                    .flex_1()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |event, window, _| {
                        Self::handle_titlebar_mouse_down(event, window);
                    }),
            )
            .child(components::pane_layout_toggle(
                theme,
                self.shell.pane_layout,
                move |layout, _, cx| {
                    let _ = layout_view.update(cx, |view, cx| {
                        view.set_pane_layout(layout, cx);
                    });
                },
            ))
            .child(render_windows_controls(theme))
    }

    fn render_application_dialog_actions(
        theme: Theme,
        specs: &[DialogActionSpec],
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let mut actions = components::dialog_actions(theme);
        for spec in specs.iter().copied() {
            let view = cx.weak_entity();
            let shortcut_hint = match spec.style {
                components::DialogActionStyle::Primary => {
                    components::shortcut_label_for_action_in_context(
                        window,
                        &SubmitApplicationDialog,
                        "ApplicationDialog",
                    )
                }
                components::DialogActionStyle::Secondary => None,
                components::DialogActionStyle::Destructive => {
                    components::shortcut_label_for_action_in_context(
                        window,
                        &SubmitApplicationDialogDestructive,
                        "ApplicationDialog",
                    )
                }
            };
            actions = actions.child(components::dialog_action_button(
                theme,
                spec.id,
                spec.label,
                spec.style,
                shortcut_hint,
                false,
                move |_, window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.handle_application_dialog_action(spec.action, window, cx);
                    });
                },
            ));
        }
        actions
    }

    pub(super) fn render_application_dialog(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(dialog) = self.application_dialog.as_ref() else {
            return div().into_any_element();
        };
        let mut content = components::dialog_surface(theme, "application-dialog", dialog.width())
            .child(components::dialog_title(theme, dialog.title()))
            .child(
                components::dialog_description(theme, dialog.description())
                    .id("application-dialog-description")
                    .mt(px(theme.metrics.spacing_2))
                    .max_h(px(280.0))
                    .overflow_y_scroll()
                    .line_height(relative(theme.typography.body_line_height)),
            );

        if let Some(specs) = dialog.action_specs() {
            content = content.child(Self::render_application_dialog_actions(
                theme, specs, window, cx,
            ));
        } else if let ApplicationDialog::SelectYaakWorkspace { workspaces, .. } = dialog {
            let mut choices = div()
                .id("application-dialog-workspaces")
                .mt(px(theme.metrics.spacing_3))
                .max_h(px(320.0))
                .overflow_y_scroll()
                .flex()
                .flex_col()
                .gap(px(theme.metrics.spacing_2));
            for (index, workspace) in workspaces.iter().enumerate() {
                let choice_view = cx.weak_entity();
                choices = choices.child(components::dialog_choice_button(
                    theme,
                    format!("application-dialog-workspace-{index}"),
                    format!("{} — {}", workspace.name, workspace.id),
                    move |_, window, cx| {
                        let _ = choice_view.update(cx, |view, cx| {
                            view.handle_application_dialog_action(
                                ApplicationDialogAction::SelectWorkspace(index),
                                window,
                                cx,
                            );
                        });
                    },
                ));
            }
            content = content
                .child(choices)
                .child(Self::render_application_dialog_actions(
                    theme,
                    &[CANCEL_DIALOG_ACTION],
                    window,
                    cx,
                ));
        }

        components::dialog_layer(
            theme,
            &self.application_dialog_focus,
            "ApplicationDialog",
            content,
        )
        .into_any_element()
    }

    pub(super) fn render_structure_dialog(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(dialog) = self.structure_dialog.as_ref() else {
            return div().into_any_element();
        };
        let name_view = cx.weak_entity();
        let name_enter_view = cx.weak_entity();
        let parent_view = cx.weak_entity();
        let index_view = cx.weak_entity();
        let index_enter_view = cx.weak_entity();
        let cancel_view = cx.weak_entity();
        let submit_view = cx.weak_entity();
        let mut form = div()
            .mt(px(theme.metrics.spacing_4))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_3));
        if dialog.edits_name() {
            form = form.child(components::dialog_field(
                theme,
                "Name",
                components::dialog_text_input(
                    theme,
                    "structure-name",
                    dialog.name.clone(),
                    "",
                    true,
                    move |value, _, cx| {
                        let _ = name_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.name = value.to_string();
                            }
                            cx.notify();
                        });
                    },
                    move |value, window, cx| {
                        let _ = name_enter_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.name = value.to_string();
                            }
                            view.submit_structure_dialog(window, cx);
                        });
                    },
                ),
            ));
        }
        if dialog.edits_destination() {
            let mut options = vec![(ROOT_PARENT.to_owned(), "Collection root".to_owned())];
            if let Some(loaded) = &self.loaded_workspace {
                options.extend(loaded.folders().iter().filter_map(|located| {
                    let name = loaded
                        .workspace()
                        .folder(located.key())?
                        .metadata
                        .name
                        .as_deref()
                        .unwrap_or("Untitled folder");
                    Some((
                        located.selector().to_owned(),
                        format!("{name} — {}", located.selector()),
                    ))
                }));
            }
            form = form
                .child(components::dialog_field(
                    theme,
                    "Destination",
                    components::dropdown(
                        theme,
                        "structure-parent",
                        "Destination folder",
                        Some(dialog.parent.clone()),
                        options,
                        388.0,
                        move |value, _, cx| {
                            let Some(value) = value else {
                                return;
                            };
                            let value = value.clone();
                            let _ = parent_view.update(cx, |view, cx| {
                                if let Some(dialog) = view.structure_dialog.as_mut() {
                                    dialog.parent = value;
                                    dialog.index.clear();
                                }
                                cx.notify();
                            });
                        },
                    ),
                ))
                .child(components::dialog_field(
                    theme,
                    "Position",
                    components::dialog_text_input(
                        theme,
                        "structure-index",
                        dialog.index.clone(),
                        "Append",
                        false,
                        move |value, _, cx| {
                            let _ = index_view.update(cx, |view, cx| {
                                if let Some(dialog) = view.structure_dialog.as_mut() {
                                    dialog.index = value.to_string();
                                }
                                cx.notify();
                            });
                        },
                        move |value, window, cx| {
                            let _ = index_enter_view.update(cx, |view, cx| {
                                if let Some(dialog) = view.structure_dialog.as_mut() {
                                    dialog.index = value.to_string();
                                }
                                view.submit_structure_dialog(window, cx);
                            });
                        },
                    ),
                ));
        }
        let submit_label = dialog.submit_label();
        let content =
            components::dialog_surface(theme, "structure-dialog", components::COMPACT_DIALOG_WIDTH)
                .child(components::dialog_title(theme, dialog.title()))
                .child(form)
                .child(
                    components::dialog_actions(theme)
                        .child(components::dialog_action_button(
                            theme,
                            "structure-cancel",
                            "Cancel",
                            components::DialogActionStyle::Secondary,
                            None,
                            false,
                            move |_, window, cx| {
                                let _ = cancel_view.update(cx, |view, cx| {
                                    view.structure_dialog = None;
                                    view.focus_handle.focus(window, cx);
                                    cx.notify();
                                });
                            },
                        ))
                        .child(components::dialog_action_button(
                            theme,
                            "structure-submit",
                            submit_label,
                            components::DialogActionStyle::Primary,
                            components::shortcut_label_for_action_in_context(
                                window,
                                &SubmitStructureDialog,
                                "StructureDialog",
                            ),
                            false,
                            move |_, window, cx| {
                                let _ = submit_view.update(cx, |view, cx| {
                                    view.submit_structure_dialog(window, cx);
                                });
                            },
                        )),
                );

        components::dialog_layer(
            theme,
            &self.structure_dialog_focus,
            "StructureDialog",
            content,
        )
        .into_any_element()
    }

    pub(super) fn render_create_environment_dialog(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(name) = self.create_environment_dialog.as_ref() else {
            return div().into_any_element();
        };
        let name_view = cx.weak_entity();
        let name_enter_view = cx.weak_entity();
        let cancel_view = cx.weak_entity();
        let submit_view = cx.weak_entity();
        let content = components::dialog_surface(
            theme,
            "create-environment-dialog",
            components::COMPACT_DIALOG_WIDTH,
        )
        .child(components::dialog_title(theme, "New Environment"))
        .child(
            div()
                .mt(px(theme.metrics.spacing_4))
                .flex()
                .flex_col()
                .gap(px(theme.metrics.spacing_3))
                .child(components::dialog_field(
                    theme,
                    "Name",
                    components::dialog_text_input(
                        theme,
                        "create-environment-name",
                        name.clone(),
                        "",
                        true,
                        move |value, _, cx| {
                            let _ = name_view.update(cx, |view, cx| {
                                if let Some(name) = view.create_environment_dialog.as_mut() {
                                    *name = value.to_string();
                                }
                                cx.notify();
                            });
                        },
                        move |value, window, cx| {
                            let _ = name_enter_view.update(cx, |view, cx| {
                                if let Some(name) = view.create_environment_dialog.as_mut() {
                                    *name = value.to_string();
                                }
                                view.submit_create_environment_dialog(window, cx);
                            });
                        },
                    ),
                )),
        )
        .child(
            components::dialog_actions(theme)
                .child(components::dialog_action_button(
                    theme,
                    "create-environment-cancel",
                    "Cancel",
                    components::DialogActionStyle::Secondary,
                    None,
                    false,
                    move |_, window, cx| {
                        let _ = cancel_view.update(cx, |view, cx| {
                            view.close_create_environment_dialog(window, cx);
                        });
                    },
                ))
                .child(components::dialog_action_button(
                    theme,
                    "create-environment-submit",
                    "Create",
                    components::DialogActionStyle::Primary,
                    components::shortcut_label_for_action_in_context(
                        window,
                        &SubmitCreateEnvironmentDialog,
                        "CreateEnvironmentDialog",
                    ),
                    false,
                    move |_, window, cx| {
                        let _ = submit_view.update(cx, |view, cx| {
                            view.submit_create_environment_dialog(window, cx);
                        });
                    },
                )),
        );

        components::dialog_layer(
            theme,
            &self.create_environment_dialog_focus,
            "CreateEnvironmentDialog",
            content,
        )
        .into_any_element()
    }

    pub(super) fn render_environment_manager_dialog(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return div().into_any_element();
        };
        let Some(loaded) = self.loaded_workspace.as_ref() else {
            return div().into_any_element();
        };
        let environments = loaded.workspace().environments();
        let rows = loaded
            .workspace()
            .effective_environment_variables(&dialog.draft);
        let rows_empty = rows.is_empty();
        let selected_name = dialog.original_name.clone();
        let busy = self.environment_save_task.is_some();
        let dirty = environments
            .iter()
            .find(|environment| environment.name == dialog.original_name)
            .is_none_or(|environment| environment != &dialog.draft);
        let active_environment = self.shell.selected_environment();
        let mut selected_background: Hsla = theme.colors.actions.accent.into();
        selected_background.a = 0.12;

        let mut environment_list = div()
            .id("environment-manager-list")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1));
        for (index, environment) in environments.iter().enumerate() {
            let name = environment.name.clone();
            let selected = name == selected_name;
            let active = active_environment == Some(name.as_str());
            let item_dirty = selected && dirty;
            let select_name = name.clone();
            let menu_name = name;
            let select_view = cx.weak_entity();
            let context_menu_view = cx.weak_entity();
            environment_list = environment_list.child(
                Button::new(("environment-manager-environment", index))
                    .selected(selected)
                    .w_full()
                    .h(px(theme.metrics.control_height))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .rounded(px(theme.metrics.radius_small))
                    .text_color(theme.colors.text.secondary)
                    .when(selected, |button| {
                        button
                            .bg(selected_background)
                            .text_color(theme.colors.actions.accent)
                    })
                    .when(!selected && !busy, |button| {
                        button.hover(move |button| {
                            button.bg(theme.colors.selection.inactive_background)
                        })
                    })
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = select_view.update(cx, |view, cx| {
                            view.select_environment_manager_environment(&select_name, cx);
                        });
                    })
                    .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let _ = context_menu_view.update(cx, |view, cx| {
                            view.open_environment_manager_context_menu(
                                menu_name.clone(),
                                event.position,
                                cx,
                            );
                        });
                    })
                    .child(
                        div()
                            .min_w(px(0.0))
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap(px(theme.metrics.spacing_2))
                            .child(components::truncated_label(environment.name.clone()).flex_1())
                            .when(item_dirty, |row| {
                                row.child(
                                    div()
                                        .id(("environment-manager-dirty", index))
                                        .debug_selector(|| "environment-manager-dirty".into())
                                        .flex_none()
                                        .w(px(6.0))
                                        .h(px(6.0))
                                        .rounded(px(3.0))
                                        .bg(theme.colors.actions.accent),
                                )
                            })
                            .when(active, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .px(px(theme.metrics.spacing_1))
                                        .rounded(px(theme.metrics.radius_small))
                                        .border_1()
                                        .border_color(theme.colors.status.success)
                                        .text_size(px(theme.typography.caption_size))
                                        .text_color(theme.colors.status.success)
                                        .child("Active"),
                                )
                            }),
                    ),
            );
        }

        let add_view = cx.weak_entity();
        let add_disabled = busy || dirty;
        let add_label = if dirty {
            "Add environment. Save or discard unsaved changes first."
        } else {
            "Add environment"
        };
        let sidebar = div()
            .w(px(210.0))
            .h_full()
            .pr(px(theme.metrics.spacing_1))
            .border_r_1()
            .border_color(theme.colors.borders.subtle)
            .flex()
            .flex_col()
            .child(
                div()
                    .mb(px(theme.metrics.spacing_2))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .child(
                        div()
                            .text_size(px(theme.typography.caption_size))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.colors.text.muted)
                            .child("ENVIRONMENTS"),
                    )
                    .child(
                        components::icon_button(
                            theme,
                            "environment-manager-add",
                            add_label,
                            components::plus_icon(theme).text_color(if add_disabled {
                                theme.colors.actions.disabled_foreground
                            } else {
                                theme.colors.text.secondary
                            }),
                            move |_, window, cx| {
                                let _ = add_view.update(cx, |view, cx| {
                                    view.open_create_environment_dialog(window, cx);
                                });
                            },
                        )
                        .flex_none()
                        .disabled(add_disabled),
                    ),
            )
            .child(environment_list);

        let name_view = cx.weak_entity();
        let name_enter_view = cx.weak_entity();
        let parent_view = cx.weak_entity();
        let parent_options = std::iter::once((String::new(), "None".to_owned()))
            .chain(
                environments
                    .iter()
                    .filter(|environment| environment.name != dialog.original_name)
                    .map(|environment| (environment.name.clone(), environment.name.clone())),
            )
            .collect::<Vec<_>>();
        let selected_parent = dialog.draft.extends.clone().unwrap_or_default();
        let table_header = div()
            .h(px(30.0))
            .px(px(theme.metrics.spacing_2))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_2))
            .bg(theme.colors.surfaces.raised)
            .border_b_1()
            .border_color(theme.colors.borders.subtle)
            .text_size(px(theme.typography.caption_size))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(theme.colors.text.muted)
            .child(div().w(px(68.0)).child("ENABLED"))
            .child(div().w(px(155.0)).child("NAME"))
            .child(div().flex_1().child("VALUE"))
            .child(div().w(px(140.0)).child("DEFINED IN"))
            .child(div().w(px(32.0)));
        let mut table_body = div()
            .id("environment-manager-variables")
            .debug_selector(|| "environment-manager-variables".into())
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .flex()
            .flex_col();
        if rows_empty {
            table_body = table_body.child(
                div()
                    .min_h(px(theme.metrics.control_height + theme.metrics.spacing_2))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .text_color(theme.colors.text.muted)
                    .child("No variables in this environment."),
            );
        }
        for (row_index, row) in rows.into_iter().enumerate() {
            let (value, editable) = environment_variable_text(&row.variable);
            let direct_index = row.direct_index;
            let inherited = direct_index.is_none();
            let toggle_variable = row.variable.clone();
            let value_variable = row.variable.clone();
            let toggle_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            let variable_name_view = cx.weak_entity();
            let name = row.variable.name.clone().unwrap_or_default();
            let value_selector = if name.is_empty() {
                format!("environment-variable-value-{row_index}")
            } else {
                format!("environment-variable-value-{name}")
            };
            let mut row_element = div()
                .id(("environment-manager-variable-row", row_index))
                .min_h(px(theme.metrics.control_height + theme.metrics.spacing_2))
                .px(px(theme.metrics.spacing_2))
                .flex()
                .items_center()
                .gap(px(theme.metrics.spacing_2))
                .border_b_1()
                .border_color(theme.colors.borders.subtle)
                .when(inherited, |row| row.text_color(theme.colors.text.muted))
                .child(div().w(px(68.0)).child(components::switch(
                    theme,
                    ("environment-variable-enabled", row_index),
                    format!("Enable {name}"),
                    !row.variable.disabled,
                    busy,
                    move |enabled, _, cx| {
                        let mut variable = toggle_variable.clone();
                        variable.disabled = !enabled;
                        let _ = toggle_view.update(cx, |view, cx| {
                            view.apply_environment_manager_draft(cx, |dialog| {
                                if let Some(index) = direct_index {
                                    dialog.draft.variables[index] =
                                        EnvironmentVariable::Plain(variable);
                                } else {
                                    dialog
                                        .draft
                                        .variables
                                        .push(EnvironmentVariable::Plain(variable));
                                }
                            });
                        });
                    },
                )))
                .child(if inherited {
                    components::truncated_label(name.clone())
                        .w(px(155.0))
                        .font_family(theme.typography.monospace_family)
                        .text_color(theme.colors.text.muted)
                        .into_any_element()
                } else {
                    let name_selector = if name.is_empty() {
                        format!("environment-variable-name-{row_index}")
                    } else {
                        format!("environment-variable-name-{name}")
                    };
                    div()
                        .id(("environment-variable-name", row_index))
                        .debug_selector({
                            let selector = name_selector.clone();
                            move || selector
                        })
                        .w(px(155.0))
                        .child(
                            components::dialog_text_input(
                                theme,
                                format!("environment-variable-name-input-{row_index}"),
                                name.clone(),
                                "Name",
                                name.is_empty() && !busy,
                                move |value, _, cx| {
                                    let _ = variable_name_view.update(cx, |view, cx| {
                                        view.apply_environment_manager_draft(cx, |dialog| {
                                            if let Some(index) = direct_index
                                                && let Some(EnvironmentVariable::Plain(variable)) =
                                                    dialog.draft.variables.get_mut(index)
                                            {
                                                variable.name = Some(value.to_string());
                                            }
                                        });
                                    });
                                },
                                |_, _, _| {},
                            )
                            .disabled(busy),
                        )
                        .into_any_element()
                })
                .child(if editable {
                    let input_id = format!("{value_selector}-input");
                    div()
                        .id(value_selector.clone())
                        .debug_selector({
                            let selector = value_selector.clone();
                            move || selector
                        })
                        .flex_1()
                        .min_w(px(120.0))
                        .child(
                            components::dialog_text_input(
                                theme,
                                input_id,
                                value,
                                "Value",
                                false,
                                move |value, _, cx| {
                                    let mut variable = value_variable.clone();
                                    set_environment_variable_text(&mut variable, value.to_string());
                                    let _ = value_view.update(cx, |view, cx| {
                                        view.apply_environment_manager_draft(cx, |dialog| {
                                            if let Some(index) = direct_index {
                                                dialog.draft.variables[index] =
                                                    EnvironmentVariable::Plain(variable);
                                            } else {
                                                dialog
                                                    .draft
                                                    .variables
                                                    .push(EnvironmentVariable::Plain(variable));
                                            }
                                        });
                                    });
                                },
                                |_, _, _| {},
                            )
                            .disabled(busy),
                        )
                        .into_any_element()
                } else {
                    environment_variant_value(theme, &name, row_index, value, inherited)
                })
                .child(
                    components::truncated_label(row.defined_in)
                        .w(px(140.0))
                        .text_size(px(theme.typography.caption_size))
                        .text_color(theme.colors.text.muted),
                );
            row_element = if let Some(index) = direct_index {
                row_element.child(
                    components::remove_row_button(
                        theme,
                        ("environment-variable-delete", row_index),
                        format!("Delete {name}"),
                        move |_, _, cx| {
                            let _ = remove_view.update(cx, |view, cx| {
                                view.apply_environment_manager_draft(cx, |dialog| {
                                    if index < dialog.draft.variables.len() {
                                        dialog.draft.variables.remove(index);
                                    }
                                });
                            });
                        },
                    )
                    .disabled(busy),
                )
            } else {
                row_element.child(div().w(px(32.0)))
            };
            table_body = table_body.child(row_element);
        }

        let add_variable_view = cx.weak_entity();
        table_body = table_body.child(
            div().p(px(theme.metrics.spacing_2)).child(
                components::editor_add_button(
                    theme,
                    "environment-manager-add-variable",
                    "Add variable",
                    move |_, _, cx| {
                        let _ = add_variable_view.update(cx, |view, cx| {
                            view.apply_environment_manager_draft(cx, |dialog| {
                                dialog
                                    .draft
                                    .variables
                                    .push(EnvironmentVariable::Plain(Variable {
                                        name: Some(String::new()),
                                        value: Some(VariableValueSet::Single(
                                            VariableValue::String(String::new()),
                                        )),
                                        disabled: false,
                                    }));
                            });
                        });
                    },
                )
                .disabled(busy),
            ),
        );
        let table = div()
            .mt(px(theme.metrics.spacing_2))
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(px(theme.metrics.radius_small))
            .border_1()
            .border_color(theme.colors.borders.standard)
            .child(table_header)
            .child(table_body);
        let form = div()
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .pl(px(theme.metrics.spacing_2))
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_3))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(
                            components::dialog_text_input(
                                theme,
                                "environment-manager-name",
                                dialog.draft.name.clone(),
                                "Environment name",
                                false,
                                move |value, _, cx| {
                                    let _ = name_view.update(cx, |view, cx| {
                                        view.apply_environment_manager_draft(cx, |dialog| {
                                            dialog.draft.name = value.to_string();
                                        });
                                    });
                                },
                                move |value, _, cx| {
                                    let _ = name_enter_view.update(cx, |view, cx| {
                                        view.apply_environment_manager_draft(cx, |dialog| {
                                            dialog.draft.name = value.to_string();
                                        });
                                    });
                                },
                            )
                            .disabled(busy),
                        ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(theme.metrics.spacing_2))
                            .child(components::dialog_field_label(theme, "Extends"))
                            .child(
                                components::dropdown(
                                    theme,
                                    "environment-manager-parent",
                                    "Parent environment",
                                    Some(selected_parent),
                                    parent_options,
                                    180.0,
                                    move |value, _, cx| {
                                        let value = value.cloned().unwrap_or_default();
                                        let _ = parent_view.update(cx, |view, cx| {
                                            view.apply_environment_manager_draft(cx, |dialog| {
                                                dialog.draft.extends =
                                                    (!value.is_empty()).then_some(value);
                                            });
                                        });
                                    },
                                )
                                .disabled(busy),
                            ),
                    ),
            )
            .child(table);

        let close_view = cx.weak_entity();
        let save_view = cx.weak_entity();
        let save_disabled = busy || !dirty;
        let content = components::dialog_surface(theme, "environment-manager-dialog", 900.0)
            .debug_selector(|| "environment-manager-dialog".into())
            .h(px(600.0))
            .max_h(relative(0.9))
            .child(components::dialog_title(
                theme,
                format!("Environments — {}", self.workspace_name()),
            ))
            .child(
                div()
                    .mt(px(theme.metrics.spacing_4))
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .child(sidebar)
                    .child(form),
            )
            .child(
                components::dialog_actions(theme)
                    .child(components::dialog_action_button(
                        theme,
                        "environment-manager-close",
                        "Close",
                        components::DialogActionStyle::Secondary,
                        None,
                        false,
                        move |_, window, cx| {
                            let _ = close_view.update(cx, |view, cx| {
                                view.request_close_environment_manager_dialog(window, cx);
                            });
                        },
                    ))
                    .child(components::dialog_action_button(
                        theme,
                        "environment-manager-save",
                        "Save Changes",
                        components::DialogActionStyle::Primary,
                        components::shortcut_label_for_action_in_context(
                            window,
                            &SubmitEnvironmentManagerDialog,
                            "EnvironmentManagerDialog",
                        ),
                        save_disabled,
                        move |_, window, cx| {
                            let _ = save_view.update(cx, |view, cx| {
                                view.save_environment_manager_dialog(window, cx);
                            });
                        },
                    )),
            );

        components::dialog_layer(
            theme,
            &self.environment_manager_dialog_focus,
            "EnvironmentManagerDialog",
            content,
        )
        .into_any_element()
    }

    pub(super) fn render_environment_manager_context_menu(
        &self,
        theme: Theme,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(name) = self.environment_manager_context_menu.as_ref() else {
            return div().into_any_element();
        };
        let Some(position) = self.environment_manager_context_menu_position else {
            return div().into_any_element();
        };
        let busy = self.environment_save_task.is_some();
        let delete_name = name.clone();
        let delete_view = cx.weak_entity();
        let dismiss_view = cx.weak_entity();
        let menu = components::context_menu_surface(
            theme,
            "environment-manager-context-menu",
            180.0,
            move |cx| {
                let _ = dismiss_view.update(cx, |view, cx| {
                    view.close_environment_manager_context_menu(cx);
                });
            },
        )
        .child(components::destructive_menu_button(
            theme,
            "environment-manager-delete",
            "Delete",
            components::shortcut_label_for_action_in_context(
                window,
                &DeleteSelectedEnvironment,
                "EnvironmentManagerDialog",
            ),
            move |window, cx| {
                let _ = delete_view.update(cx, |view, cx| {
                    if busy {
                        return;
                    }
                    view.close_environment_manager_context_menu(cx);
                    view.confirm_delete_environment(delete_name.clone(), window, cx);
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
}
