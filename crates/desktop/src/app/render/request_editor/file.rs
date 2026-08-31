use super::*;

impl ProbeApp {
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
}
