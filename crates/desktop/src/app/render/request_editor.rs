use super::*;

impl ProbeApp {
    pub(super) fn render_request_editor(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
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
            EditorSection::Query => {
                self.render_parameter_editor(key, &request, ParameterEditorKind::Query, theme, cx)
            }
            EditorSection::Path => {
                self.render_parameter_editor(key, &request, ParameterEditorKind::Path, theme, cx)
            }
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

    pub(super) fn render_parameter_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        kind: ParameterEditorKind,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let path = kind.is_path();
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
                    components::editor_key_value_row(theme)
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
                            false,
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

    pub(super) fn render_header_editor(
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
                    components::editor_key_value_row(theme)
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
                            false,
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

    pub(super) fn render_body_editor(
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

    pub(super) fn render_form_body_editor(
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
                    components::editor_key_value_row(theme)
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
                            false,
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

    pub(super) fn render_multipart_body_editor(
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
                    components::editor_key_value_row(theme)
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
                            false,
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

    pub(super) fn render_file_body_editor(
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
                    components::editor_key_value_row(theme)
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
                            false,
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

    pub(super) fn render_authentication_editor(
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
                    components::editor_key_value_row(theme)
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
}
