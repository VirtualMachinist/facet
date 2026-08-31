use super::*;

impl ProbeApp {
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
}
