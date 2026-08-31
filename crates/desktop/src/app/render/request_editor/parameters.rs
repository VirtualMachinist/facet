use super::*;

impl ProbeApp {
    pub(super) fn render_parameter_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        kind: ParameterEditorKind,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, parameter) in kind.parameters(request).iter().enumerate() {
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
                                (kind.name_id(), index),
                                parameter.name.clone(),
                                "Parameter",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                kind.rename(request, index, &value);
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
                                (kind.value_id(), index),
                                parameter.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(parameter) =
                                                    kind.parameter_mut(request, index)
                                                {
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
                            (kind.enabled_id(), index),
                            kind.enable_label(),
                            !parameter.disabled,
                            false,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(parameter) =
                                                kind.parameter_mut(request, index)
                                            {
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
                            (kind.remove_id(), index),
                            kind.remove_label(),
                            move |_, window, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            kind.remove(request, index);
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
            kind.add_id(),
            kind.add_label(),
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            kind.add(request);
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }
}
