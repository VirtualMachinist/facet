use super::*;

mod request_editor;
mod request_tabs;
mod response;
mod sidebar;

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ParameterEditorKind {
    Path,
    Query,
}

impl ParameterEditorKind {
    pub(super) const fn is_path(self) -> bool {
        matches!(self, Self::Path)
    }
}

pub(super) fn response_page_button(
    theme: Theme,
    id: &'static str,
    label: &'static str,
    disabled: bool,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .disabled(disabled)
        .px(px(theme.metrics.spacing_2))
        .h(px(theme.metrics.control_height - 8.0))
        .rounded(px(theme.metrics.radius_small))
        .border_1()
        .border_color(theme.colors.borders.standard)
        .styles(move |styles| {
            styles.disabled(move |button| {
                button
                    .bg(theme.colors.selection.inactive_background)
                    .border_color(theme.colors.borders.subtle)
                    .text_color(theme.colors.actions.disabled_foreground)
            })
        })
        .hover(move |button| button.bg(theme.colors.selection.inactive_background))
        .on_click(on_click)
        .child(label)
}

impl Render for ProbeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.clear_resolved_environment_dialog_error(cx);
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
        if self.toasts.stack_state.is_expanded() != self.toast_paused {
            self.schedule_toast_lifecycle(cx);
        }

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
                if view.application_dialog.is_some() {
                    return;
                }
                if view.environment_manager_dialog.is_some() {
                    if !view.environment_manager_save_disabled() {
                        view.save_environment_manager_dialog(window, cx);
                    }
                    return;
                }
                view.save_active_request(window, cx);
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
                if view.transient.workspace_switcher_open {
                    view.transient.workspace_import_submenu_open = true;
                    view.transient
                        .workspace_import_popup_focus
                        .focus(window, cx);
                } else {
                    view.transient.sidebar_import_menu_open = true;
                    view.transient.sidebar_import_popup_focus.focus(window, cx);
                }
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &CloseImportSubmenu, window, cx| {
                if view.transient.workspace_import_submenu_open {
                    view.transient.workspace_import_submenu_open = false;
                    view.transient
                        .workspace_import_trigger_focus
                        .focus(window, cx);
                } else if view.transient.sidebar_import_menu_open {
                    view.transient.sidebar_import_menu_open = false;
                    view.transient
                        .sidebar_import_trigger_focus
                        .focus(window, cx);
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
                cx.listener(|view, _: &SubmitEnvironmentManagerDialog, window, cx| {
                    if view.environment_manager_save_disabled() {
                        return;
                    }
                    view.save_environment_manager_dialog(window, cx);
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
                cx.listener(|view, _: &CancelEnvironmentManagerDialog, window, cx| {
                    view.request_close_environment_manager_dialog(window, cx);
                }),
            )
            .on_action(
                cx.listener(|view, _: &DeleteSelectedEnvironment, window, cx| {
                    view.delete_selected_environment_from_manager(window, cx);
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
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .bg(theme.colors.surfaces.raised)
                    .when(!self.shell.sidebar_collapsed, |row| {
                        row.child(self.render_sidebar(theme, cx))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .relative()
                            .flex()
                            .flex_col()
                            .child(self.render_tabs(theme, cx))
                            .child(self.render_editor_response(theme, cx))
                            .when(!self.shell.sidebar_collapsed, |column| {
                                column.child(
                                    components::pane_splitter(
                                        theme,
                                        "sidebar-resize-handle",
                                        Axis::Horizontal,
                                    )
                                    .show_line(false)
                                    .debug_selector("sidebar-resize-handle")
                                    .on_mouse_down(
                                        move |_, _, cx| {
                                            let _ = sidebar_view.update(cx, |view, cx| {
                                                view.shell.resizing = Some(ResizePane::Sidebar);
                                                cx.notify();
                                            });
                                        },
                                    ),
                                )
                            }),
                    ),
            )
            .child(self.render_structure_dialog(theme, window, cx))
            .child(self.render_environment_manager_dialog(theme, window, cx))
            .child(self.render_environment_manager_context_menu(theme, window, cx))
            .child(self.render_create_environment_dialog(theme, window, cx))
            .child(self.render_application_dialog(theme, window, cx))
            .child(self.render_request_tab_tooltip(theme))
            .child(self.render_tab_context_menu(theme, window, cx))
            .child(self.render_tree_context_menu(theme, window, cx))
            .child(self.render_toasts(theme, cx))
    }
}
