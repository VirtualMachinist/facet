use super::*;

mod authentication;
mod body;
mod file;
mod form;
mod headers;
mod multipart;
mod parameters;

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
}
