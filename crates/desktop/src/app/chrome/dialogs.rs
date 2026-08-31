use super::*;

impl ProbeApp {
    pub(super) fn render_application_dialog_actions(
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

    pub(in crate::app) fn render_application_dialog(
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

    pub(in crate::app) fn render_structure_dialog(
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

    pub(in crate::app) fn render_create_environment_dialog(
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
        let busy = self.environment_save_task.is_some();
        let mut content = components::dialog_surface(
            theme,
            "create-environment-dialog",
            components::COMPACT_DIALOG_WIDTH,
        )
        .child(components::dialog_title(theme, "New Environment"));
        content = content
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
                        )
                        .disabled(busy),
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
                        busy,
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
                        busy,
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
}
