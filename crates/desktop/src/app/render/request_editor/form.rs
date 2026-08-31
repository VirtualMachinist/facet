use super::*;

impl ProbeApp {
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
}
