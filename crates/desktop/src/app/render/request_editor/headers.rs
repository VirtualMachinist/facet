use super::*;

impl ProbeApp {
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
}
