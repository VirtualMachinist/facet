use super::*;

impl ProbeApp {
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
}
