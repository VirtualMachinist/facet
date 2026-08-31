use super::*;

#[derive(Default)]
pub(super) struct EditorSearchState {
    session: Option<EditorSearchSession>,
}

#[derive(Default)]
struct EditorSearchSession {
    query: String,
    active_match: usize,
    focus_input: bool,
}

impl EditorSearchState {
    pub(super) fn open(&mut self) {
        self.session
            .get_or_insert_with(EditorSearchSession::default)
            .focus_input = true;
    }

    pub(super) fn is_open(&self) -> bool {
        self.session.is_some()
    }

    pub(super) fn set_query(&mut self, query: String) {
        let session = self
            .session
            .get_or_insert_with(EditorSearchSession::default);
        if session.query != query {
            session.query = query;
            session.active_match = 0;
        }
    }

    pub(super) fn close(&mut self) {
        self.session = None;
    }

    fn query_and_take_focus_request(&mut self) -> Option<(String, bool)> {
        self.session.as_mut().map(|session| {
            (
                session.query.clone(),
                std::mem::take(&mut session.focus_input),
            )
        })
    }

    pub(super) fn active_match(&self) -> usize {
        self.session
            .as_ref()
            .map_or(0, |session| session.active_match)
    }

    pub(super) fn set_active_match(&mut self, active_match: usize) {
        if let Some(session) = self.session.as_mut() {
            session.active_match = active_match;
        }
    }

    fn request_input_focus(&mut self) {
        if let Some(session) = self.session.as_mut() {
            session.focus_input = true;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn editor_search_card_overlay(
    theme: Theme,
    editor: impl IntoElement,
    card_id: ElementId,
    search_state: Entity<EditorSearchState>,
    matches: Rc<Vec<Range<usize>>>,
    active_match: usize,
    editor_state: Entity<EditorState>,
    cx: &mut App,
) -> gpui::AnyElement {
    if !search_state.read(cx).is_open() {
        return editor.into_any_element();
    }

    let Some((query, focus_input)) =
        search_state.update(cx, |search, _| search.query_and_take_focus_request())
    else {
        return editor.into_any_element();
    };
    let match_count = matches.len();
    let count_label = if query.is_empty() || match_count == 0 {
        "0/0".to_owned()
    } else {
        format!("{}/{}", active_match + 1, match_count)
    };
    let input_id = ElementId::NamedChild(Arc::new(card_id.clone()), SharedString::from("input"));
    let previous_id =
        ElementId::NamedChild(Arc::new(card_id.clone()), SharedString::from("previous"));
    let next_id = ElementId::NamedChild(Arc::new(card_id.clone()), SharedString::from("next"));
    let close_id = ElementId::NamedChild(Arc::new(card_id.clone()), SharedString::from("close"));
    let query_state = search_state.clone();
    let query_editor = editor_state.clone();
    let enter_state = search_state.clone();
    let enter_editor = editor_state.clone();
    let enter_matches = matches.clone();
    let previous_state = search_state.clone();
    let previous_editor = editor_state.clone();
    let previous_matches = matches.clone();
    let next_state = search_state.clone();
    let next_editor = editor_state.clone();
    let next_matches = matches;
    let close_state = search_state.clone();
    let close_editor = editor_state.clone();
    let escape_state = search_state.clone();
    let escape_editor = editor_state.clone();

    let mut input = text_input_base(theme, input_id, query, "Search");
    input.width = Some(150.0);
    input.height = theme.metrics.control_height - 2.0;
    input.text_size = theme.typography.caption_size;
    input.debug_selector = Some("editor-search-input");
    input.content_gap = theme.metrics.spacing_1;
    input.leading_icon = Some(
        library_icon("lucide-search", &SEARCH_SVG, theme.metrics.icon_small)
            .text_color(theme.colors.text.muted),
    );
    input.focus_on_render = focus_input;
    input.on_change = Some(Rc::new(move |value, _, cx| {
        let query = value.to_string();
        query_state.update(cx, |search, cx| {
            search.set_query(query.clone());
            cx.notify();
        });
        query_editor.update(cx, |editor, cx| {
            reveal_editor_search_match(editor, &query, cx);
        });
    }));
    input.on_enter = Some(Rc::new(move |_, _, cx| {
        let range = enter_editor.update(cx, |editor, cx| {
            navigate_editor_search_match(editor, SearchDirection::Next, cx)
        });
        sync_active_search_match(&enter_state, &enter_matches, range, false, cx);
    }));

    div()
        .relative()
        .size_full()
        .min_h(px(0.0))
        .child(editor)
        .child(
            div()
                .id(card_id)
                .debug_selector(|| "editor-search-card".into())
                .key_context("ProbeEditorSearch")
                .on_action(move |_: &Escape, window, cx| {
                    escape_state.update(cx, |search, cx| {
                        search.close();
                        cx.notify();
                    });
                    escape_editor.update(cx, |editor, cx| editor.focus(window, cx));
                })
                .absolute()
                .top(px(theme.metrics.spacing_2))
                .right(px(theme.metrics.spacing_2))
                .h(px(theme.metrics.control_height + 8.0))
                .flex()
                .items_center()
                .gap(px(theme.metrics.spacing_1))
                .pl(px(theme.metrics.spacing_1))
                .pr(px(theme.metrics.spacing_1))
                .rounded(px(theme.metrics.radius_medium))
                .bg(theme.colors.surfaces.overlay)
                .border_1()
                .border_color(theme.colors.borders.standard)
                .shadow(temporary_surface_shadow(theme, 8.0))
                .child(input)
                .child(
                    div()
                        .w(px(46.0))
                        .text_align(TextAlign::Center)
                        .text_size(px(theme.typography.caption_size))
                        .text_color(theme.colors.text.secondary)
                        .child(count_label),
                )
                .child(search_card_button(
                    theme,
                    previous_id,
                    "Previous search result",
                    chevron_icon(theme, true),
                    match_count == 0,
                    move |event, _, cx| {
                        let range = previous_editor.update(cx, |editor, cx| {
                            navigate_editor_search_match(editor, SearchDirection::Previous, cx)
                        });
                        sync_active_search_match(
                            &previous_state,
                            &previous_matches,
                            range,
                            matches!(event, ClickEvent::Mouse(_)),
                            cx,
                        );
                    },
                ))
                .child(search_card_button(
                    theme,
                    next_id,
                    "Next search result",
                    chevron_icon(theme, false),
                    match_count == 0,
                    move |event, _, cx| {
                        let range = next_editor.update(cx, |editor, cx| {
                            navigate_editor_search_match(editor, SearchDirection::Next, cx)
                        });
                        sync_active_search_match(
                            &next_state,
                            &next_matches,
                            range,
                            matches!(event, ClickEvent::Mouse(_)),
                            cx,
                        );
                    },
                ))
                .child(search_card_button(
                    theme,
                    close_id,
                    "Close search",
                    close_icon(theme),
                    false,
                    move |_, window, cx| {
                        close_state.update(cx, |search, cx| {
                            search.close();
                            cx.notify();
                        });
                        close_editor.update(cx, |editor, cx| editor.focus(window, cx));
                    },
                )),
        )
        .into_any_element()
}

fn reveal_editor_search_match(
    editor: &mut EditorState,
    query: &str,
    cx: &mut Context<EditorState>,
) {
    editor.set_search_query(query, true, cx);
    if editor.search_session().query.is_empty() || editor.search_session().matcher.is_empty() {
        editor.close_search(cx);
        return;
    }
    editor.previous_search_match(cx);
    editor.next_search_match(cx);
    editor.close_search(cx);
}

enum SearchDirection {
    Previous,
    Next,
}

fn navigate_editor_search_match(
    editor: &mut EditorState,
    direction: SearchDirection,
    cx: &mut Context<EditorState>,
) -> Option<Range<usize>> {
    let range = match direction {
        SearchDirection::Previous => editor.previous_search_match(cx),
        SearchDirection::Next => editor.next_search_match(cx),
    };
    editor.close_search(cx);
    range
}

fn sync_active_search_match(
    search_state: &Entity<EditorSearchState>,
    matches: &[Range<usize>],
    range: Option<Range<usize>>,
    refocus_input: bool,
    cx: &mut App,
) {
    let Some(active_match) = range.and_then(|range| matches.iter().position(|item| *item == range))
    else {
        return;
    };
    search_state.update(cx, |search, cx| {
        search.set_active_match(active_match);
        if refocus_input {
            search.request_input_focus();
        }
        cx.notify();
    });
}

fn search_card_button(
    theme: Theme,
    id: ElementId,
    aria_label: impl Into<SharedString>,
    icon: impl IntoElement,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let size = theme.metrics.control_height - 2.0;
    Button::new(id)
        .accessibility_label(aria_label)
        .disabled(disabled)
        .size(px(size))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .bg(transparent_black())
        .border_1()
        .border_color(transparent_black())
        .cursor_pointer()
        .hover(move |button| button.bg(theme.colors.selection.inactive_background))
        .focus(move |button| button.border_color(theme.colors.borders.focused))
        .on_click(on_click)
        .child(icon)
}

pub(super) fn response_search_highlight_overlay(
    theme: Theme,
    state: Entity<EditorState>,
    editor: impl IntoElement,
    text: SharedString,
    matches: Vec<(Range<usize>, bool)>,
) -> gpui::AnyElement {
    if matches.is_empty() {
        return editor.into_any_element();
    }

    let mut active_color: Hsla = theme.colors.selection.active_background.into();
    active_color.a = 0.42;
    let mut inactive_color: Hsla = theme.colors.selection.active_background.into();
    inactive_color.a = 0.22;
    div()
        .relative()
        .size_full()
        .min_h(px(0.0))
        .child(editor)
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let editor = state.read(cx);
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        let fallback_char_size = search_fallback_char_size(theme, editor, window);
                        for (range, active) in &matches {
                            let color = if *active {
                                active_color
                            } else {
                                inactive_color
                            };
                            for match_bounds in
                                search_match_bounds(editor, &text, range, fallback_char_size)
                            {
                                window.paint_quad(fill(match_bounds, color));
                            }
                        }
                    });
                },
            )
            .absolute()
            .top(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0)),
        )
        .into_any_element()
}

pub(in crate::components) fn search_match_bounds(
    editor: &EditorState,
    text: &str,
    range: &Range<usize>,
    fallback_char_size: gpui::Size<Pixels>,
) -> Vec<Bounds<Pixels>> {
    let mut bounds = Vec::new();
    let start = range.start.min(text.len());
    let end = range.end.min(text.len());
    if start >= end || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
        return bounds;
    }

    let measured_bounds = search_match_char_ranges(text, start..end)
        .into_iter()
        .filter_map(|char_range| editor.range_to_bounds(&char_range))
        .collect::<Vec<_>>();
    let repair_char_size = measured_bounds
        .iter()
        .find_map(usable_search_char_size)
        .unwrap_or(fallback_char_size);

    let mut last_char_size = None;
    for mut char_bounds in measured_bounds {
        normalize_search_char_bounds(
            &mut char_bounds,
            Some(last_char_size.unwrap_or(repair_char_size)),
        );
        if char_bounds.size.width <= px(0.0) || char_bounds.size.height <= px(0.0) {
            continue;
        }
        last_char_size = Some(char_bounds.size);
        push_merged_highlight_bounds(&mut bounds, char_bounds);
    }
    for bound in &mut bounds {
        bound.size.width += px(1.0);
    }
    bounds
}

pub(in crate::components) fn search_fallback_char_size(
    theme: Theme,
    editor: &EditorState,
    window: &Window,
) -> gpui::Size<Pixels> {
    let font_size = px(theme.typography.body_size);
    let font_id = window
        .text_system()
        .resolve_font(&font(theme.typography.monospace_family));
    size(
        window.text_system().em_layout_width(font_id, font_size),
        editor.line_height().unwrap_or(px(
            theme.typography.body_size * theme.typography.body_line_height
        )),
    )
}

fn usable_search_char_size(bounds: &Bounds<Pixels>) -> Option<gpui::Size<Pixels>> {
    (bounds.size.width > px(0.0) && bounds.size.height > px(0.0)).then_some(bounds.size)
}

pub(in crate::components) fn search_match_char_ranges(
    text: &str,
    range: Range<usize>,
) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut char_starts = text[range.clone()]
        .char_indices()
        .map(|(index, _)| range.start + index)
        .peekable();
    while let Some(start) = char_starts.next() {
        let end = char_starts.peek().copied().unwrap_or(range.end);
        let slice = &text[start..end];
        if slice != "\n" && slice != "\r" {
            ranges.push(start..end);
        }
    }
    ranges
}

pub(in crate::components) fn normalize_search_char_bounds(
    bounds: &mut Bounds<Pixels>,
    last_char_size: Option<gpui::Size<Pixels>>,
) {
    let Some(last_char_size) = last_char_size else {
        return;
    };
    if bounds.size.width <= px(0.0) {
        bounds.size.width = last_char_size.width;
    }
    if bounds.size.height > last_char_size.height {
        bounds.size.height = last_char_size.height;
    }
}

fn push_merged_highlight_bounds(bounds: &mut Vec<Bounds<Pixels>>, next: Bounds<Pixels>) {
    let Some(current) = bounds.last_mut() else {
        bounds.push(next);
        return;
    };
    if current.origin.y != next.origin.y || current.size.height != next.size.height {
        bounds.push(next);
        return;
    }

    let current_right = current.origin.x + current.size.width;
    let next_right = next.origin.x + next.size.width;
    if next.origin.x > current_right {
        bounds.push(next);
    } else if next_right > current_right {
        current.size.width = next_right - current.origin.x;
    }
}

pub(in crate::components) fn body_text_highlights(
    theme: Theme,
    variables: &[(Range<usize>, String)],
) -> Vec<TextDecoration> {
    variables
        .iter()
        .map(|(range, _)| {
            text_decoration(range.clone(), Some(theme.colors.syntax.string.into()), None)
        })
        .collect()
}

pub(super) fn search_match_decoration(
    theme: Theme,
    range: Range<usize>,
    active: bool,
) -> TextDecoration {
    text_decoration(
        range,
        None,
        Some(if active {
            theme.colors.selection.active_background.into()
        } else {
            theme.colors.selection.inactive_background.into()
        }),
    )
}

pub(super) fn text_decoration(
    range: Range<usize>,
    color: Option<Hsla>,
    background: Option<Hsla>,
) -> TextDecoration {
    TextDecoration::new(
        range,
        HighlightStyle {
            color,
            background_color: background,
            ..Default::default()
        },
    )
}
