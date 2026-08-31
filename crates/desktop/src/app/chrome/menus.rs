use super::*;

impl ProbeApp {
    pub(super) fn handle_titlebar_mouse_down(event: &MouseDownEvent, window: &mut Window) {
        if event.click_count > 1 {
            #[cfg(target_os = "macos")]
            window.titlebar_double_click();

            #[cfg(any(target_os = "linux", target_os = "windows"))]
            window.zoom_window();
        } else {
            window.start_window_move();
        }
    }

    pub(super) fn render_desktop_action_menu_item(
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

    pub(super) fn render_desktop_top_level_menu(
        &self,
        theme: Theme,
        menu: DesktopMenu,
        id: &'static str,
        label: &'static str,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let open = self.transient.desktop_menu_open == Some(menu);
        let hover_view = cx.weak_entity();
        let change_view = cx.weak_entity();
        let popup = self.render_desktop_menu_popup(theme, menu, window, cx);

        Popover::new(id)
            .open(open)
            .on_open_change(move |open, _, cx| {
                let _ = change_view.update(cx, |view, cx| {
                    view.transient.desktop_menu_open = open.then_some(menu);
                    view.transient.desktop_submenu_open = None;
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
                        if view.transient.desktop_menu_open.is_some()
                            && view.transient.desktop_menu_open != Some(menu)
                        {
                            view.open_desktop_menu(menu, cx);
                        }
                    });
                },
            ))
            .content(move |_, _, _| popup)
            .into_any_element()
    }

    pub(super) fn render_desktop_menu_bar(
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

    pub(super) fn render_desktop_menu_popup(
        &self,
        theme: Theme,
        menu: DesktopMenu,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let definition = self.desktop_menu_definition(menu);
        self.render_desktop_menu_definition(theme, definition, window, cx)
    }

    pub(super) fn render_desktop_menu_definition(
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
                    let open = self.transient.desktop_submenu_open == Some(state);
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
                                if view.transient.desktop_submenu_open != Some(state) {
                                    view.transient.desktop_submenu_open = Some(state);
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

    pub(super) fn desktop_menu_definition(&self, menu: DesktopMenu) -> DesktopMenuDefinition {
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

    pub(in crate::app) fn render_titlebar(
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
        let import_trigger_focus = self.transient.workspace_import_trigger_focus.clone();
        let import_popup_focus = self.transient.workspace_import_popup_focus.clone();
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
                            view.transient.workspace_switcher_open = false;
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
                .track_focus(&self.transient.workspace_import_popup_focus)
                .key_context("ImportSubmenu")
                .child(components::menu_button(
                    theme,
                    "workspace-switcher-import-postman",
                    "Postman Export…",
                    None,
                    move |window, cx| {
                        let _ = import_postman_view.update(cx, |view, cx| {
                            view.transient.workspace_import_submenu_open = false;
                            view.transient.workspace_switcher_open = false;
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
                            view.transient.workspace_import_submenu_open = false;
                            view.transient.workspace_switcher_open = false;
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
            self.transient.workspace_import_submenu_open,
            &self.transient.workspace_import_trigger_focus,
            move |window, cx| {
                let trigger_focus = import_trigger_focus.clone();
                let popup_focus = import_popup_focus.clone();
                let _ = import_menu_view.update(cx, |view, cx| {
                    view.transient.workspace_import_submenu_open =
                        !view.transient.workspace_import_submenu_open;
                    if view.transient.workspace_import_submenu_open {
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
            self.transient.workspace_import_submenu_open,
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
                        view.transient.workspace_switcher_open = false;
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
                        view.transient.workspace_switcher_open = false;
                        if !view.loading {
                            view.choose_workspace(window, cx);
                        }
                    });
                },
            ))
            .child(import_menu);

        let switcher = Popover::new("workspace-switcher")
            .open(self.transient.workspace_switcher_open)
            .on_open_change(move |open, _, cx| {
                let _ = switcher_view.update(cx, |view, cx| {
                    view.transient.workspace_switcher_open = *open;
                    if !*open {
                        view.transient.workspace_import_submenu_open = false;
                    }
                    cx.notify();
                });
            })
            .trigger(
                Button::new("workspace-switcher-trigger")
                    .accessibility_label("Switch workspace")
                    .selected(self.transient.workspace_switcher_open)
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
                        self.transient.workspace_switcher_open,
                    )),
            )
            .content(move |_, _, _| popup);

        let titlebar_padding = theme.metrics.spacing_1;
        div()
            .h(px(theme.metrics.control_height + titlebar_padding * 2.0))
            .w_full()
            .pl(px(if cfg!(target_os = "macos") {
                80.0
            } else {
                theme.metrics.spacing_3
            }))
            .py(px(titlebar_padding))
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
}
