use super::*;

impl ProbeApp {
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
