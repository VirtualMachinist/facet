use super::*;

impl ProbeApp {
    pub(super) fn render_response_panel(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let active_key = self.shell.active_tab();
        let state = active_key.and_then(|key| self.execution.response(key));
        let (header_leading, header_trailing, content) = match state {
            Some(state @ ResponseState::Running { .. }) => (
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Sending…")
                    .into_any_element(),
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label(format_duration(
                            state.elapsed().unwrap_or_default(),
                        ))
                        .text_color(theme.colors.text.muted),
                    )
                    .into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Waiting for the server…")
                    .into_any_element(),
            ),
            Some(ResponseState::Cancelled) => (
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Response")
                    .into_any_element(),
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label("Cancelled")
                            .text_color(theme.colors.text.muted),
                    )
                    .into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Request cancelled.")
                    .into_any_element(),
            ),
            Some(ResponseState::Failed(error)) => (
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Response")
                    .into_any_element(),
                div()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .justify_end()
                    .child(
                        components::truncated_label("Failed").text_color(theme.colors.status.error),
                    )
                    .into_any_element(),
                div()
                    .id("response-error-scroll")
                    .flex_1()
                    .p(px(theme.metrics.spacing_3))
                    .overflow_y_scroll()
                    .text_color(theme.colors.status.error)
                    .child(error.clone())
                    .into_any_element(),
            ),
            Some(ResponseState::Complete(response)) => {
                let status = format!("{} {}", response.status, response.reason);
                let metadata = format!(
                    "• {} • {}",
                    format_duration(response.duration),
                    format_size(response.size),
                );
                let document = active_key.and_then(|key| self.response_viewer.document(key));
                (
                    document.map_or_else(
                        || {
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("Response")
                                .into_any_element()
                        },
                        |document| self.render_response_tabs(theme, document, cx),
                    ),
                    div()
                        .min_w(px(0.0))
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(px(theme.metrics.spacing_1))
                        .child(
                            components::truncated_label(status.trim_end().to_owned())
                                .id("response-status-code")
                                .debug_selector(|| "response-status-code".into())
                                .flex_none()
                                .max_w(px(220.0))
                                .text_color(response_status_color(theme, response.status)),
                        )
                        .child(
                            div()
                                .id("response-metadata")
                                .debug_selector(|| "response-metadata".into())
                                .flex_none()
                                .text_color(theme.colors.text.muted)
                                .child(metadata),
                        )
                        .into_any_element(),
                    self.render_response_document(theme, document, cx),
                )
            }
            None => (
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Response")
                    .into_any_element(),
                div().into_any_element(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Send a request to see its response.")
                    .into_any_element(),
            ),
        };

        div()
            .when(self.shell.pane_layout == PaneLayout::Vertical, |panel| {
                panel.h(px(self.shell.response_height)).w_full()
            })
            .when(self.shell.pane_layout == PaneLayout::Horizontal, |panel| {
                panel.w(px(self.shell.response_width)).h_full()
            })
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.raised)
            .child(
                div()
                    .pt(px(theme.metrics.spacing_2))
                    .pb(px(theme.metrics.spacing_1))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .child(header_leading),
                    )
                    .child(
                        div()
                            .id("response-status")
                            .debug_selector(|| "response-status".into())
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(theme.typography.caption_size))
                            .child(header_trailing),
                    ),
            )
            .child(content)
    }

    pub(super) fn render_response_tabs(
        &self,
        theme: Theme,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut tabs = Tabs::new("response-view-tabs")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        let available_tabs = if document.truncated {
            &ResponseViewerTab::TRUNCATED[..]
        } else {
            &ResponseViewerTab::ALL[..]
        };
        for (index, tab) in available_tabs.iter().copied().enumerate() {
            let tab_view = cx.weak_entity();
            let selected = self.response_viewer.tab() == tab;
            let label = if tab == ResponseViewerTab::Inspect {
                let count = document.inspection.count();
                if count > 0 {
                    format!("Inspect [{count}]")
                } else {
                    tab.label().to_owned()
                }
            } else {
                tab.label().to_owned()
            };
            tabs = tabs.child(
                components::text_tab(
                    theme,
                    ("response-view-tab", index),
                    label,
                    selected,
                    index + 1,
                    available_tabs.len(),
                    move |_, _, cx| {
                        let _ = tab_view.update(cx, |view, cx| {
                            view.set_response_tab(tab, cx);
                        });
                    },
                )
                .debug_selector(move || {
                    format!("response-tab-{}", tab.label().to_ascii_lowercase())
                }),
            );
        }
        tabs.into_any_element()
    }

    pub(super) fn render_raw_response_tabs(
        &self,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut raw_views = Tabs::new("response-raw-view-tabs")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        for (index, view) in RawBodyView::ALL.iter().copied().enumerate() {
            let view_entity = cx.weak_entity();
            let selected = self.response_viewer.raw_view() == view;
            raw_views = raw_views.child(
                components::editor_subtab(
                    theme,
                    ("response-raw-view", index),
                    view.label(),
                    selected,
                    index + 1,
                    RawBodyView::ALL.len(),
                    move |_, _, cx| {
                        let _ = view_entity.update(cx, |app, cx| {
                            app.set_raw_body_view(view, cx);
                        });
                    },
                )
                .debug_selector(move || {
                    format!("response-raw-view-{}", view.label().to_ascii_lowercase())
                }),
            );
        }
        raw_views.into_any_element()
    }

    pub(super) fn render_response_document(
        &self,
        theme: Theme,
        document: Option<&PreparedDocument>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(key) = self.shell.active_tab() else {
            return div().into_any_element();
        };
        let Some(document) = document else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.colors.text.muted)
                .child("Preparing response…")
                .into_any_element();
        };

        let mut banners = div()
            .px(px(theme.metrics.spacing_2))
            .pt(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1));
        let mut has_banner = false;
        if document.file_backed {
            has_banner = true;
            let previous_view = cx.weak_entity();
            let next_view = cx.weak_entity();
            let can_previous = document.can_load_previous_page();
            let can_next = document.can_load_next_page();
            let first_byte = document.page_offset.saturating_add(1);
            let last_byte = document.page_offset.saturating_add(document.page_len);
            banners = banners.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .text_color(theme.colors.status.warning)
                    .text_size(px(theme.typography.caption_size))
                    .child(
                        div().flex_1().min_w(px(0.0)).child(format!(
                            "The complete response is retained on disk. Showing bytes {first_byte}–{last_byte} of {}; search covers this page only.",
                            document.total_size
                        )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(theme.metrics.spacing_1))
                            .child(
                                response_page_button(
                                    theme,
                                    "response-page-previous",
                                    "Previous",
                                    !can_previous,
                                    move |_, _, cx| {
                                        let _ = previous_view.update(cx, |view, cx| {
                                            view.load_response_page(
                                                key,
                                                PageDirection::Previous,
                                                cx,
                                            );
                                        });
                                    },
                                ),
                            )
                            .child(
                                response_page_button(
                                    theme,
                                    "response-page-next",
                                    "Next",
                                    !can_next,
                                    move |_, _, cx| {
                                        let _ = next_view.update(cx, |view, cx| {
                                            view.load_response_page(
                                                key,
                                                PageDirection::Next,
                                                cx,
                                            );
                                        });
                                    },
                                ),
                            ),
                    ),
            );
        } else if document.truncated {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.status.warning)
                    .text_size(px(theme.typography.caption_size))
                    .child(document.retention_notice.clone().unwrap_or_else(|| {
                        "The response exceeds the in-memory limit and the complete body was not retained."
                            .to_owned()
                    })),
            );
        }
        if let Some(notice) = &document.pretty_notice
            && !matches!(
                self.response_viewer.tab(),
                ResponseViewerTab::Headers | ResponseViewerTab::Inspect
            )
        {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.text.muted)
                    .text_size(px(theme.typography.caption_size))
                    .child(notice.clone()),
            );
        }

        let list = match self.response_viewer.tab() {
            ResponseViewerTab::Headers => self.render_response_headers(theme, document, cx),
            ResponseViewerTab::Inspect => self.render_response_inspector(theme, key, document, cx),
            ResponseViewerTab::Pretty | ResponseViewerTab::Raw => {
                self.render_response_body(theme, key, document, cx)
            }
        };

        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .when(
                self.response_viewer.tab() == ResponseViewerTab::Raw,
                |panel| {
                    panel.child(
                        div()
                            .h(px(theme.metrics.control_height))
                            .px(px(theme.metrics.spacing_2))
                            .flex()
                            .items_center()
                            .child(self.render_raw_response_tabs(theme, cx)),
                    )
                },
            )
            .when(has_banner, |panel| panel.child(banners))
            .child(list)
            .into_any_element()
    }

    pub(super) fn response_tab_content_spacing(theme: Theme) -> gpui::Div {
        div()
            .px(px(theme.metrics.spacing_2))
            .pt(px(theme.metrics.spacing_1))
            .pb(px(theme.metrics.spacing_2))
    }

    pub(super) fn inspect_tab_content_boundary(
        theme: Theme,
        content: impl gpui::IntoElement,
    ) -> gpui::Div {
        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .pt(px(theme.metrics.spacing_1))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(content),
            )
    }

    pub(super) fn render_response_body(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.binary
            && (self.response_viewer.tab() == ResponseViewerTab::Pretty
                || self.response_viewer.raw_view() == RawBodyView::Text)
        {
            return placeholder_message(theme, "Binary response body cannot be displayed as text.");
        }
        if self.response_viewer.tab() == ResponseViewerTab::Raw
            && self.response_viewer.raw_view() == RawBodyView::Base64
            && document.base64_pending
        {
            return placeholder_message(theme, "Encoding Base64…");
        }
        let text = self.response_viewer.visible_text(key);
        if text.is_empty() {
            return placeholder_message(theme, "Empty response body.");
        }
        let view = cx.weak_entity();
        let body_mouse_view = cx.weak_entity();
        let inspect_view = cx.weak_entity();
        let inspect_ranges = document.inspection_ranges.clone();
        let inspect_context_enabled = self.response_viewer.tab() == ResponseViewerTab::Pretty
            && matches!(
                document.syntax,
                ResponseBodySyntax::Json | ResponseBodySyntax::Xml
            )
            && document.pretty_notice.is_none();
        let pretty_reveal = if self.response_viewer.tab() == ResponseViewerTab::Pretty {
            self.pretty_reveal.get()
        } else {
            None
        };
        let inspection_reveal = pretty_reveal
            .and_then(|reveal| {
                self.response_viewer
                    .inspection_range_for_selection(key, reveal.selection)
                    .map(|range| (range, reveal.scroll_pending))
            })
            .and_then(|reveal| {
                (self.response_viewer.tab() == ResponseViewerTab::Pretty).then_some(reveal)
            });
        if let Some(reveal) = pretty_reveal
            && reveal.scroll_pending
            && self.response_viewer.tab() == ResponseViewerTab::Pretty
        {
            self.pretty_reveal.set(Some(PrettyRevealState {
                selection: reveal.selection,
                scroll_pending: false,
            }));
        }
        Self::response_tab_content_spacing(theme)
            .id("response-body")
            .debug_selector(|| "response-body".into())
            .flex_1()
            .min_h(px(0.0))
            .child(components::response_body_input(
                theme,
                "response-body-editor",
                text,
                components::ResponseBodyInputOptions::new(
                    &[],
                    0,
                    if self.response_viewer.tab() == ResponseViewerTab::Pretty {
                        document.syntax.language()
                    } else {
                        ""
                    },
                    move |range, cx| {
                        #[cfg(test)]
                        {
                            let _ = view.update(cx, |this, _| {
                                this.rendered_response_rows = range.len();
                            });
                        }
                        #[cfg(not(test))]
                        {
                            let _ = (&view, range, cx);
                        }
                    },
                    move |_, cx| {
                        let _ = body_mouse_view.update(cx, |view, cx| {
                            if view.response_viewer.tab() == ResponseViewerTab::Pretty
                                && view.pretty_reveal.take().is_some()
                            {
                                cx.notify();
                            }
                        });
                    },
                    move |_, offset| {
                        inspect_context_enabled
                            && inspect_ranges
                                .iter()
                                .any(|entry| entry.range.contains(&offset))
                    },
                    move |_, offset, _, cx| {
                        let _ = inspect_view.update(cx, |view, cx| {
                            if view.response_viewer.tab() != ResponseViewerTab::Pretty {
                                view.show_toast(
                                    ToastIntent::Info,
                                    "Inspect from the Pretty tab to select a response value.",
                                    cx,
                                );
                            } else if view
                                .response_viewer
                                .document(key)
                                .is_some_and(|document| document.inspection_pending)
                            {
                                view.response_viewer.set_tab(ResponseViewerTab::Inspect);
                                view.show_toast(
                                    ToastIntent::Info,
                                    "Inspection is still running.",
                                    cx,
                                );
                            } else if let Some(selection) = view
                                .response_viewer
                                .select_inspection_at_offset(key, offset)
                            {
                                view.pending_inspector_reveal.set(Some(selection));
                                view.pretty_reveal.set(None);
                            } else {
                                view.show_toast(
                                    ToastIntent::Info,
                                    "No inspected JWT or timestamp found at that value.",
                                    cx,
                                );
                            }
                            cx.notify();
                        });
                    },
                )
                .soft_wrap(
                    self.response_viewer.tab() == ResponseViewerTab::Pretty
                        || self.response_viewer.raw_view() == RawBodyView::Base64,
                )
                .inspection_reveal(inspection_reveal),
            ))
            .into_any_element()
    }

    pub(super) fn render_response_headers(
        &self,
        theme: Theme,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.headers.is_empty() {
            return placeholder_message(theme, "No response headers");
        }
        let view = cx.weak_entity();
        Self::response_tab_content_spacing(theme)
            .id("response-headers")
            .debug_selector(|| "response-headers".into())
            .flex_1()
            .min_h(px(0.0))
            .child(components::response_headers_input(
                theme,
                "response-headers-editor",
                &document.headers,
                &[],
                0,
                move |range, cx| {
                    #[cfg(test)]
                    {
                        let _ = view.update(cx, |this, _| {
                            this.rendered_response_rows = range.len();
                        });
                    }
                    #[cfg(not(test))]
                    {
                        let _ = (&view, range, cx);
                    }
                },
            ))
            .into_any_element()
    }

    pub(super) fn render_response_inspector(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let rows = inspect_list_rows(document);
        if let Some(selection) = self.pending_inspector_reveal.take()
            && let Some(index) = inspect_row_index(&rows, selection)
        {
            self.inspector_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
        let selected = self.response_viewer.inspection_selection(key);
        let detail = if document.inspection_pending {
            "Inspecting response…".to_owned()
        } else {
            inspection_detail_text(&document.inspection, selected)
        };
        let revealable = selected.is_some_and(|selection| {
            self.response_viewer
                .inspection_range_for_selection(key, selection)
                .is_some()
        }) && document.pretty_notice.is_none();
        let view = cx.weak_entity();
        let row_count = rows.len();
        let rows_for_list = rows.clone();
        let list = uniform_list("response-inspector-list", row_count, {
            cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                range
                    .filter_map(|index| {
                        let document = view.response_viewer.document(key)?;
                        let selected = view.response_viewer.inspection_selection(key);
                        rows_for_list.get(index).copied().map(|row| {
                            view.render_inspector_list_row(theme, key, row, document, selected, cx)
                        })
                    })
                    .collect::<Vec<_>>()
            })
        })
        .size_full()
        .track_scroll(&self.inspector_scroll);

        if row_count == 0 && !document.inspection_pending {
            return Self::inspect_tab_content_boundary(
                theme,
                div()
                    .id("response-inspector-empty")
                    .debug_selector(|| "response-inspector-empty".into())
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child(document.inspection.skipped.clone().unwrap_or_else(|| {
                        "JWTs and Unix timestamps are detected automatically.".to_owned()
                    })),
            )
            .into_any_element();
        }

        let divider_view = cx.weak_entity();
        let divider =
            components::pane_splitter(theme, "response-inspector-divider", Axis::Horizontal)
                .debug_selector("response-inspector-divider")
                .on_mouse_down(move |event, _, cx| {
                    let _ = divider_view.update(cx, |view, cx| {
                        view.inspector_resize_start =
                            Some((f32::from(event.position.x), view.inspector_list_width));
                        cx.notify();
                    });
                });

        Self::inspect_tab_content_boundary(
            theme,
            div()
                .id("response-inspector")
                .debug_selector(|| "response-inspector".into())
                .flex_1()
                .min_h(px(0.0))
                .flex()
                .child(
                    Self::response_tab_content_spacing(theme)
                        .pr(px(theme.metrics.spacing_2))
                        .w(px(self.inspector_list_width))
                        .flex_none()
                        .min_h(px(0.0))
                        .child(list),
                )
                .child(
                    div()
                        .relative()
                        .flex_1()
                        .min_w(px(0.0))
                        .min_h(px(0.0))
                        .child(
                            Self::response_tab_content_spacing(theme)
                                .pl(px(theme.metrics.spacing_2))
                                .pt(px(theme.metrics.spacing_2))
                                .size_full()
                                .child(
                                    div()
                                        .relative()
                                        .size_full()
                                        .child(components::response_inspector_input(
                                            theme,
                                            "response-inspector-editor",
                                            detail,
                                            move |range, cx| {
                                                #[cfg(test)]
                                                {
                                                    let _ = view.update(cx, |this, _| {
                                                        this.rendered_response_rows = range.len();
                                                    });
                                                }
                                                #[cfg(not(test))]
                                                {
                                                    let _ = (&view, range, cx);
                                                }
                                            },
                                        ))
                                        .when(revealable, |detail| {
                                            let reveal_view = cx.weak_entity();
                                            detail.child(
                                                div()
                                                    .absolute()
                                                    .top(px(theme.metrics.spacing_3))
                                                    .right(px(theme.metrics.spacing_3))
                                                    .child(components::compact_icon_button(
                                                        theme,
                                                        "response-inspector-reveal-pretty",
                                                        "Reveal in Pretty",
                                                        components::locate_icon(theme),
                                                        move |_, _, cx| {
                                                            let _ = reveal_view.update(
                                                                cx,
                                                                |view, cx| {
                                                                    if let Some(selection) = view
                                                                        .response_viewer
                                                                        .reveal_inspection_in_pretty(
                                                                            key,
                                                                        )
                                                                    {
                                                                        view.pretty_reveal.set(
                                                                            Some(
                                                                                PrettyRevealState {
                                                                                    selection,
                                                                                    scroll_pending: true,
                                                                                },
                                                                            ),
                                                                        );
                                                                    } else {
                                                                        view.show_toast(
                                                                            ToastIntent::Warning,
                                                                            "Pretty source is unavailable.",
                                                                            cx,
                                                                        );
                                                                    }
                                                                    cx.notify();
                                                                },
                                                            );
                                                        },
                                                    )),
                                            )
                                        }),
                                ),
                        )
                        .child(divider),
                ),
        )
        .into_any_element()
    }

    pub(super) fn render_inspector_list_row(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        row: InspectListRow,
        document: &PreparedDocument,
        selected: Option<InspectSelection>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        match row {
            InspectListRow::Group { label, count } => div()
                .w_full()
                .h(px(theme.metrics.tree_row_height))
                .px(px(theme.metrics.spacing_1))
                .flex()
                .items_center()
                .gap(px(theme.metrics.spacing_1))
                .text_size(px(theme.typography.caption_size))
                .text_color(theme.colors.text.secondary)
                .font_weight(FontWeight::SEMIBOLD)
                .child(label)
                .child(
                    div()
                        .font_weight(FontWeight::NORMAL)
                        .text_color(theme.colors.text.muted)
                        .child(format!("[{count}]")),
                )
                .into_any_element(),
            InspectListRow::Item { selection } => {
                let label = inspect_row_label(document, selection);
                let row_view = cx.weak_entity();
                let is_selected = selected == Some(selection);
                div()
                    .id("response-inspector-list-row")
                    .debug_selector(|| "response-inspector-list-row".into())
                    .w_full()
                    .h(px(theme.metrics.tree_row_height))
                    .px(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .rounded(px(theme.metrics.radius_small))
                    .cursor(CursorStyle::PointingHand)
                    .text_size(px(theme.typography.caption_size))
                    .text_color(theme.colors.text.primary)
                    .bg(if is_selected {
                        theme.colors.selection.inactive_background
                    } else {
                        theme.colors.surfaces.raised
                    })
                    .hover(move |row| {
                        if is_selected {
                            row
                        } else {
                            row.bg(theme.colors.surfaces.editor)
                        }
                    })
                    .child(components::truncated_label(label).min_w(px(0.0)))
                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                        let _ = row_view.update(cx, |view, cx| {
                            view.response_viewer.select_inspection(key, selection);
                            view.pretty_reveal.set(None);
                            cx.notify();
                        });
                    })
                    .into_any_element()
            }
        }
    }

    pub(super) fn render_editor_response(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let response_view = cx.weak_entity();
        let horizontal = self.shell.pane_layout == PaneLayout::Horizontal;
        let splitter = components::pane_splitter(
            theme,
            "response-resize-handle",
            if horizontal {
                Axis::Horizontal
            } else {
                Axis::Vertical
            },
        )
        .debug_selector("response-resize-handle")
        .on_mouse_down(move |_, _, cx| {
            let _ = response_view.update(cx, |view, cx| {
                view.shell.resizing = Some(ResizePane::Response);
                cx.notify();
            });
        });

        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .when(horizontal, |work_area| work_area.flex_row())
            .when(!horizontal, |work_area| work_area.flex_col())
            .child(self.render_request_editor(theme, cx))
            .child(
                self.render_response_panel(theme, cx)
                    .relative()
                    .child(splitter),
            )
    }

    pub(in crate::app) fn active_request(&self) -> Option<&HttpRequest> {
        let key = self.shell.active_tab()?;
        self.loaded_workspace.as_ref()?.workspace().request(key)
    }

    pub(super) fn variable_context(&self, cx: &mut Context<Self>) -> components::VariableContext {
        let on_manage_environments = self.loaded_workspace.is_some().then(|| {
            let view = cx.weak_entity();
            Rc::new(move |window: &mut Window, cx: &mut gpui::App| {
                let view = view.clone();
                window.defer(cx, move |window, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.open_environment_manager_dialog(window, cx);
                    });
                });
            }) as Rc<dyn Fn(&mut Window, &mut gpui::App)>
        });
        let Some(selected) = self.shell.selected_environment() else {
            return components::VariableContext {
                unavailable_message: "Select an environment to resolve this variable".to_owned(),
                on_manage_environments,
                ..components::VariableContext::default()
            };
        };
        let Some(loaded) = &self.loaded_workspace else {
            return components::VariableContext::default();
        };
        match resolve_environment(loaded.workspace().environments(), selected) {
            Ok(environment) => {
                let view = cx.weak_entity();
                components::VariableContext {
                    values: environment.variables().clone(),
                    secrets: environment.secrets_without_values().clone(),
                    unavailable_message: "Variable value is unavailable".to_owned(),
                    on_change: Some(Rc::new(move |name, value, window, cx| {
                        let name = name.to_owned();
                        let view = view.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.update_environment_variable(&name, value, window, cx);
                            });
                        });
                    })),
                    on_manage_environments,
                }
            }
            Err(error) => components::VariableContext {
                unavailable_message: error.to_string(),
                on_manage_environments,
                ..components::VariableContext::default()
            },
        }
    }
}
