use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BodySyntax {
    Plain,
    Json,
    Xml,
}

pub(crate) fn body_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    syntax: BodySyntax,
    variables: VariableContext,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let value = value.into();
    let ranges = variable_ranges(&value);
    let decorations = body_text_highlights(theme, &ranges);
    ProbeEditor {
        theme,
        id: id.into(),
        value,
        placeholder: SharedString::from("Body content"),
        decorations,
        language: match syntax {
            BodySyntax::Json => "json".into(),
            BodySyntax::Xml => "xml".into(),
            BodySyntax::Plain => SharedString::default(),
        },
        readonly: false,
        min_height: Some(120.0),
        padding: EditorInsets::standard(theme),
        soft_wrap: true,
        text_color: theme.colors.text.primary,
        scroll_to_range: None,
        search_matches: Vec::new(),
        on_change: Some(Rc::new(on_value_change)),
        on_mouse_down: None,
        on_visible_range: None,
        extra_context_menu_actions: Vec::new(),
        debug_selector: None,
        variables: Some(variables),
    }
    .into_any_element()
}

pub(crate) fn response_body_input(
    theme: Theme,
    id: impl Into<ElementId>,
    text: &str,
    options: ResponseBodyInputOptions<'_>,
) -> gpui::AnyElement {
    let inspection_reveal = options.inspection_reveal;
    let search_matches = response_highlights(
        options.matches,
        options.active_match,
        inspection_reveal.as_ref(),
    );
    let (decorations, scroll_to_range) = response_decorations(
        theme,
        options.matches,
        options.active_match,
        inspection_reveal,
    );
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: text.into(),
            decorations,
            language: options.language,
            soft_wrap: options.soft_wrap,
            text_color: theme.colors.syntax.plain,
            scroll_to_range,
            search_matches,
        },
        options.on_visible_range,
        Some(options.on_mouse_down),
        vec![TextContextMenuExtraAction {
            id: "inspect",
            label: "Inspect",
            requires_selection: false,
            is_enabled: options.inspect_enabled,
            on_click: options.on_inspect,
        }],
    )
}

pub(crate) fn response_headers_input(
    theme: Theme,
    id: impl Into<ElementId>,
    headers: &[probe_http::ResponseHeader],
    matches: &[SearchMatch],
    active_match: usize,
    on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
) -> gpui::AnyElement {
    let joined = join_header_lines(headers);
    let mut decorations = Vec::new();
    for (offset, name_len) in joined.line_offsets.iter().zip(&joined.name_lens) {
        decorations.push(text_decoration(
            *offset..*offset + name_len,
            Some(theme.colors.text.secondary.into()),
            None,
        ));
    }
    let (search_decorations, scroll_to_range) =
        response_decorations(theme, matches, active_match, None);
    decorations.extend(search_decorations);
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: joined.text.into(),
            decorations,
            language: SharedString::default(),
            soft_wrap: true,
            text_color: theme.colors.text.primary,
            scroll_to_range,
            search_matches: response_highlights(matches, active_match, None),
        },
        Rc::new(on_visible_range),
        None,
        Vec::new(),
    )
}

pub(crate) fn response_inspector_input(
    theme: Theme,
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
) -> gpui::AnyElement {
    response_editor(
        theme,
        id,
        ResponseEditorPresentation {
            value: text.into(),
            decorations: Vec::new(),
            language: SharedString::default(),
            soft_wrap: true,
            text_color: theme.colors.text.primary,
            scroll_to_range: None,
            search_matches: Vec::new(),
        },
        Rc::new(on_visible_range),
        None,
        Vec::new(),
    )
}

fn response_decorations(
    theme: Theme,
    matches: &[SearchMatch],
    active_match: usize,
    inspection_reveal: Option<(Range<usize>, bool)>,
) -> (Vec<TextDecoration>, Option<Range<usize>>) {
    let mut decorations =
        Vec::with_capacity(matches.len() + usize::from(inspection_reveal.is_some()));
    let mut scroll_to_range = None;
    for (index, found) in matches.iter().enumerate() {
        let active = index == active_match;
        if active {
            scroll_to_range = Some(found.range.start..found.range.start);
        }
        decorations.push(search_match_decoration(theme, found.range.clone(), active));
    }
    if let Some((range, should_scroll)) = inspection_reveal {
        if should_scroll {
            scroll_to_range = Some(range.clone());
        }
        decorations.push(search_match_decoration(theme, range, true));
    }
    (decorations, scroll_to_range)
}

fn response_highlights(
    matches: &[SearchMatch],
    active_match: usize,
    inspection_reveal: Option<&(Range<usize>, bool)>,
) -> Vec<(Range<usize>, bool)> {
    let mut highlights = matches
        .iter()
        .enumerate()
        .map(|(index, found)| (found.range.clone(), index == active_match))
        .collect::<Vec<_>>();
    if let Some((range, _)) = inspection_reveal {
        highlights.push((range.clone(), true));
    }
    highlights
}

struct ResponseEditorPresentation {
    value: SharedString,
    decorations: Vec<TextDecoration>,
    language: SharedString,
    soft_wrap: bool,
    text_color: gpui::Rgba,
    scroll_to_range: Option<Range<usize>>,
    search_matches: Vec<(Range<usize>, bool)>,
}

fn response_editor(
    theme: Theme,
    id: impl Into<ElementId>,
    presentation: ResponseEditorPresentation,
    on_visible_range: VisibleRangeHandler,
    on_mouse_down: Option<EditorMouseDownHandler>,
    extra_context_menu_actions: Vec<TextContextMenuExtraAction>,
) -> gpui::AnyElement {
    ProbeEditor {
        theme,
        id: id.into(),
        value: presentation.value,
        placeholder: SharedString::default(),
        decorations: presentation.decorations,
        language: presentation.language,
        readonly: true,
        min_height: None,
        padding: EditorInsets::response(theme),
        soft_wrap: presentation.soft_wrap,
        text_color: presentation.text_color,
        scroll_to_range: presentation.scroll_to_range,
        search_matches: presentation.search_matches,
        on_change: None,
        on_mouse_down,
        on_visible_range: Some(on_visible_range),
        extra_context_menu_actions,
        debug_selector: None,
        variables: None,
    }
    .into_any_element()
}

struct EditorField {
    state: Entity<EditorState>,
    decorations: TextDecorationCollection,
    last_decorations: Vec<TextDecoration>,
    on_change: Option<InputChangeHandler>,
    last_scroll_range: Option<Range<usize>>,
    language: SharedString,
    soft_wrap: bool,
    _subscription: Subscription,
}

pub(super) fn editor_value_needs_refresh(
    language_changed: bool,
    current_value: &SharedString,
    next_value: &SharedString,
) -> bool {
    language_changed || current_value != next_value
}

impl EditorField {
    fn on_event(
        this: &mut Self,
        input: &Entity<EditorState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change)
            && let Some(on_change) = this.on_change.clone()
        {
            on_change(input.read(cx).value(), window, cx);
        }
    }
}

/// gpui-base paints caret, selection, and gutter from `InputEditorStyle`.
/// Its `Default` is fully transparent, so Probe must supply visible tokens.
pub(super) fn editor_paint_style(theme: Theme) -> InputEditorStyle {
    InputEditorStyle {
        foreground: theme.colors.text.primary.into(),
        muted_foreground: theme.colors.text.muted.into(),
        background: theme.colors.surfaces.raised.into(),
        border: theme.colors.borders.standard.into(),
        selection: theme.editor_selection(),
        caret: theme.colors.text.primary.into(),
        highlight_styles: Arc::new(crate::syntax::ProbeHighlightStyles::new(theme)),
        ..Default::default()
    }
}

#[derive(IntoElement)]
pub(super) struct ProbeEditor {
    pub(super) theme: Theme,
    pub(super) id: ElementId,
    pub(super) value: SharedString,
    pub(super) placeholder: SharedString,
    pub(super) decorations: Vec<TextDecoration>,
    pub(super) language: SharedString,
    pub(super) readonly: bool,
    pub(super) min_height: Option<f32>,
    pub(super) padding: EditorInsets,
    pub(super) soft_wrap: bool,
    pub(super) text_color: gpui::Rgba,
    pub(super) scroll_to_range: Option<Range<usize>>,
    pub(super) search_matches: Vec<(Range<usize>, bool)>,
    pub(super) on_change: Option<InputChangeHandler>,
    pub(super) on_mouse_down: Option<EditorMouseDownHandler>,
    pub(super) on_visible_range: Option<VisibleRangeHandler>,
    pub(super) extra_context_menu_actions: Vec<TextContextMenuExtraAction>,
    pub(super) debug_selector: Option<&'static str>,
    pub(super) variables: Option<VariableContext>,
}

#[derive(Default)]
struct EditorSearchState {
    open: bool,
    query: String,
    active_match: usize,
    focus_input: bool,
}

impl EditorSearchState {
    fn open(&mut self) {
        self.open = true;
        self.focus_input = true;
    }

    fn set_query(&mut self, query: String) {
        if self.query != query {
            self.query = query;
            self.active_match = 0;
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.active_match = 0;
        self.focus_input = false;
    }

    fn take_focus_request(&mut self) -> bool {
        std::mem::take(&mut self.focus_input)
    }

    fn request_input_focus(&mut self) {
        self.focus_input = true;
    }
}

impl RenderOnce for ProbeEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let component_id = self.id.clone();
        let context_menu = window.use_keyed_state(
            text_context_menu_id(&component_id, "context-menu-state"),
            cx,
            |_, _| TextContextMenuState::default(),
        );
        let placeholder = self.placeholder.clone();
        let on_change = self.on_change.clone();
        let soft_wrap = self.soft_wrap;
        let language = self.language.clone();
        let editor_id = self.id.clone();
        let search_state = window.use_keyed_state(
            ElementId::NamedChild(
                Arc::new(editor_id.clone()),
                SharedString::from("search-state"),
            ),
            cx,
            |_, _| EditorSearchState::default(),
        );
        let scroll_to_range = self.scroll_to_range;
        let field = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let state = cx.new(|cx| {
                let mut editor = EditorState::new(window, cx)
                    .placeholder(placeholder.clone())
                    .folding(false)
                    .searchable(true)
                    .soft_wrap(soft_wrap)
                    .language(language.clone());
                editor.set_highlighter_factory(crate::syntax::factory(), cx);
                editor.set_editor_style(editor_paint_style(self.theme));
                editor
            });
            state.update(cx, |editor, cx| {
                editor.set_readonly(self.readonly, cx);
                editor.set_value(self.value.clone(), window, cx);
            });
            let decorations = state.update(cx, |editor, cx| {
                editor.create_decorations_collection(Vec::new(), cx)
            });
            let subscription = cx.subscribe_in(&state, window, EditorField::on_event);
            EditorField {
                state,
                decorations,
                last_decorations: Vec::new(),
                on_change: on_change.clone(),
                last_scroll_range: None,
                language: language.clone(),
                soft_wrap,
                _subscription: subscription,
            }
        });
        field.update(cx, |field, cx| {
            field.on_change = self.on_change.clone();
            let language_changed = field.language != self.language;
            if language_changed {
                field.language = self.language.clone();
            }
            let soft_wrap_changed = field.soft_wrap != self.soft_wrap;
            if soft_wrap_changed {
                field.soft_wrap = self.soft_wrap;
            }
            let context_focus = field.state.read(cx).focus_handle(cx);
            let open_context_menu = context_menu.clone();
            let context_editor = field.state.clone();
            field.state.update(cx, |editor, cx| {
                editor.set_editor_style(editor_paint_style(self.theme));
                editor.set_editor_paddings(self.padding.edges());
                editor.set_readonly(self.readonly, cx);
                if soft_wrap_changed {
                    editor.set_soft_wrap(self.soft_wrap, window, cx);
                }
                editor.on_context_menu(Rc::new(move |_, capabilities, position, window, cx| {
                    context_focus.focus(window, cx);
                    open_context_menu.update(cx, |state, cx| {
                        state.position = Some(position);
                        state.capabilities = capabilities;
                        state.target_focus = Some(context_focus.clone());
                        state.target_editor = Some(context_editor.clone());
                        cx.notify();
                    });
                }));
                if language_changed {
                    editor.set_highlighter(self.language.clone(), cx);
                }
                // gpui-base clears its parser when the language changes and
                // rebuilds it on the next text update. Pretty and Raw XML are
                // often byte-identical, so force that update when only the
                // language changed as well.
                if editor_value_needs_refresh(language_changed, &editor.value(), &self.value) {
                    editor.set_value(self.value.clone(), window, cx);
                }
                if editor.search_session().open {
                    let query = editor.search_session().query.clone();
                    let active_match = editor.search_session().matcher.current_match_index();
                    search_state.update(cx, |search, cx| {
                        search.open();
                        if !query.is_empty() {
                            search.set_query(query);
                        }
                        search.active_match = active_match;
                        cx.notify();
                    });
                    editor.close_search(cx);
                }
            });
            if field.last_decorations != self.decorations {
                field.last_decorations = self.decorations.clone();
                field.decorations.set(self.decorations.clone(), cx);
            }
            if scroll_to_range != field.last_scroll_range {
                if let Some(range) = &scroll_to_range {
                    let laid_out = field.state.read(cx).visible_row_range().is_some();
                    field.state.update(cx, |editor, cx| {
                        editor.set_selected_range(range.clone(), cx);
                        if !range.is_empty() {
                            editor.focus(window, cx);
                        }
                    });
                    if laid_out {
                        field.last_scroll_range = scroll_to_range.clone();
                    }
                } else {
                    field.last_scroll_range = None;
                }
            }
        });
        let state = field.read(cx).state.clone();
        let (matches, active_match) = if search_state.read(cx).open {
            let matches = state.read(cx).search_session().matcher.matched_ranges();
            let active_match = search_state
                .read(cx)
                .active_match
                .min(matches.len().saturating_sub(1));
            if active_match != search_state.read(cx).active_match {
                search_state.update(cx, |search, _| search.active_match = active_match);
            }
            (matches, active_match)
        } else {
            (Rc::new(Vec::new()), 0)
        };
        let local_search_matches = matches
            .iter()
            .enumerate()
            .map(|(index, range)| (range.clone(), index == active_match))
            .collect::<Vec<_>>();
        if let Some(on_visible_range) = self.on_visible_range {
            match state.read(cx).visible_row_range() {
                Some(range) => on_visible_range(range, cx),
                None => window.request_animation_frame(),
            }
        }
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        let theme = self.theme;
        let editor = InputBase::new(editor_id.clone())
            .size_full()
            .when_some(self.min_height, |editor, height| editor.min_h(px(height)))
            .overflow_hidden()
            .rounded(px(theme.metrics.radius_small))
            .font_family(theme.typography.monospace_family)
            .text_size(px(theme.typography.body_size))
            .text_color(self.text_color)
            .bg(theme.colors.surfaces.raised)
            .border_1()
            .border_color(if focused {
                theme.colors.borders.focused
            } else {
                theme.colors.borders.standard
            })
            .focused(focused)
            .styles(move |styles| {
                styles.focused(move |editor| editor.border_color(theme.colors.borders.focused))
            })
            .when_some(self.debug_selector, |editor, selector| {
                editor.debug_selector(move || selector.into())
            })
            .key_context("ProbeEditor")
            .on_action({
                let search_state = search_state.clone();
                move |_: &Search, window, cx| {
                    search_state.update(cx, |search, cx| {
                        search.open();
                        cx.notify();
                    });
                    window.refresh();
                }
            })
            .on_action({
                let search_state = search_state.clone();
                let state = state.clone();
                move |_: &Escape, window, cx| {
                    if search_state.read(cx).open {
                        search_state.update(cx, |search, cx| {
                            search.close();
                            cx.notify();
                        });
                        state.update(cx, |editor, cx| editor.focus(window, cx));
                        window.refresh();
                    }
                }
            })
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                let on_mouse_down = self.on_mouse_down.clone();
                move |_, window, cx| {
                    if let Some(on_mouse_down) = &on_mouse_down {
                        on_mouse_down(window, cx);
                    }
                    state.update(cx, |editor, cx| editor.focus(window, cx));
                }
            })
            .child(div().size_full().child(Editor::new(&state)));
        let editor = response_search_highlight_overlay(
            self.theme,
            state.clone(),
            editor,
            self.value.clone(),
            [self.search_matches, local_search_matches].concat(),
        );
        let editor = editor_search_card_overlay(
            self.theme,
            editor,
            ElementId::NamedChild(
                Arc::new(editor_id.clone()),
                SharedString::from("search-card"),
            ),
            search_state,
            matches,
            active_match,
            state.clone(),
            cx,
        );
        let editor = if let Some(variables) = self.variables {
            variable_editor_overlay(
                theme,
                state,
                ElementId::NamedChild(Arc::new(editor_id), SharedString::from("variable-tooltip")),
                editor,
                self.value,
                variables,
                window,
                cx,
            )
        } else {
            editor.into_any_element()
        };
        with_text_context_menu(
            theme,
            &component_id,
            context_menu,
            editor,
            self.extra_context_menu_actions,
            true,
            window,
            cx,
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn editor_search_card_overlay(
    theme: Theme,
    editor: impl IntoElement,
    card_id: ElementId,
    search_state: Entity<EditorSearchState>,
    matches: Rc<Vec<Range<usize>>>,
    active_match: usize,
    editor_state: Entity<EditorState>,
    cx: &mut App,
) -> gpui::AnyElement {
    if !search_state.read(cx).open {
        return editor.into_any_element();
    }

    let (query, focus_input) = search_state.update(cx, |search, _| {
        (search.query.clone(), search.take_focus_request())
    });
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
        search.active_match = active_match;
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

fn response_search_highlight_overlay(
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

pub(super) fn search_match_bounds(
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

pub(super) fn search_fallback_char_size(
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

pub(super) fn search_match_char_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
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

pub(super) fn normalize_search_char_bounds(
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

pub(super) fn body_text_highlights(
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

fn search_match_decoration(theme: Theme, range: Range<usize>, active: bool) -> TextDecoration {
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

fn text_decoration(
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

#[allow(clippy::too_many_arguments)]
pub(super) fn variable_input_overlay(
    theme: Theme,
    state: Entity<InputState>,
    tooltip_id: ElementId,
    input: impl IntoElement,
    value: SharedString,
    variables: VariableContext,
    highlight_path_variables: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let ranges = input_variable_ranges(&value, highlight_path_variables);
    // Input paints first so it keeps native caret, selection, and scroll.
    // The overlay sits on top, recolors supported variable spans, and covers
    // the native caret while blink is off.
    let mut wrapper = div()
        .id(tooltip_id.clone())
        .relative()
        .debug_selector(|| "variable-input-tooltip-trigger".into())
        .w_full()
        .child(input)
        .child(variable_highlight_layer(
            theme,
            state.clone(),
            ranges.is_empty(),
            theme.typography.monospace_family,
            theme.typography.body_size,
            highlight_path_variables,
        ));
    let tooltip_ranges = variable_ranges(&value);
    if tooltip_ranges.is_empty() {
        return wrapper.into_any_element();
    }

    let hover = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(tooltip_id), SharedString::from("hover")),
        cx,
        VariableHoverState::new,
    );
    let visible_width = hover.read(cx).visible_width;
    let current_scroll = state.read(cx).scroll_offset().x;
    let cursor = state.read(cx).cursor();
    let spans = variable_span_layout(
        window,
        &value,
        &tooltip_ranges,
        theme.typography.monospace_family,
        theme.typography.body_size,
        current_scroll,
        cursor,
        visible_width,
    );
    let mut hits = div().relative().w_full().h_full().on_prepaint({
        let hover = hover.clone();
        move |bounds, window, cx| {
            let width = bounds.size.width;
            let changed = hover.update(cx, |state, _| {
                let changed = state.visible_width != Some(width);
                state.visible_width = Some(width);
                changed
            });
            if changed {
                window.request_animation_frame();
            }
        }
    });
    for (index, (name, left, width)) in spans.into_iter().enumerate() {
        hits = hits.child(
            variable_hover_hit(
                ("variable-hover", index),
                index,
                name,
                hover.clone(),
                left,
                px(0.0),
                width,
                None,
                if index == 0 {
                    "variable-hover-trigger".into()
                } else {
                    format!("variable-hover-trigger-{index}")
                },
            )
            .top(px(0.0))
            .bottom(px(0.0)),
        );
    }
    wrapper = wrapper.child(
        div()
            .absolute()
            .top(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0))
            .right(px(0.0))
            .border_1()
            .border_color(transparent_black())
            .px(px(theme.metrics.spacing_2))
            .overflow_hidden()
            .flex()
            .items_center()
            .child(hits),
    );

    with_variable_tooltip(wrapper, theme, hover, variables, cx)
}

#[allow(clippy::too_many_arguments)]
fn variable_editor_overlay(
    theme: Theme,
    state: Entity<EditorState>,
    tooltip_id: ElementId,
    editor: impl IntoElement,
    value: SharedString,
    variables: VariableContext,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let ranges = variable_ranges(&value);
    let mut wrapper = div()
        .id(tooltip_id.clone())
        .relative()
        .size_full()
        .min_h(px(0.0))
        .child(editor);
    if ranges.is_empty() {
        return wrapper.into_any_element();
    }

    let hover = window.use_keyed_state(
        ElementId::NamedChild(Arc::new(tooltip_id), SharedString::from("hover")),
        cx,
        VariableHoverState::new,
    );
    let overlay_origin = hover.read(cx).overlay_origin;
    let mut hits = div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        .overflow_hidden()
        .on_prepaint({
            let hover = hover.clone();
            move |bounds, window, cx| {
                let origin = bounds.origin;
                let changed = hover.update(cx, |state, _| {
                    let changed = state.overlay_origin != Some(origin);
                    state.overlay_origin = Some(origin);
                    changed
                });
                if changed {
                    window.request_animation_frame();
                }
            }
        });
    if let Some(origin) = overlay_origin {
        let editor = state.read(cx);
        if editor.visible_row_range().is_none() {
            window.request_animation_frame();
        }
        for (index, (range, name)) in ranges.into_iter().enumerate() {
            let Some(bounds) = editor.range_to_bounds(&range) else {
                continue;
            };
            hits = hits.child(variable_hover_hit(
                ("body-variable-hover", index),
                index,
                name,
                hover.clone(),
                bounds.origin.x - origin.x,
                bounds.origin.y - origin.y,
                bounds.size.width.max(px(1.0)),
                Some(bounds.size.height.max(px(1.0))),
                if index == 0 {
                    "body-variable-hover-trigger".into()
                } else {
                    format!("body-variable-hover-trigger-{index}")
                },
            ));
        }
    } else {
        window.request_animation_frame();
    }
    wrapper = wrapper.child(hits);
    with_variable_tooltip(wrapper, theme, hover, variables, cx)
}

#[allow(clippy::too_many_arguments)]
fn variable_hover_hit(
    id: impl Into<ElementId>,
    index: usize,
    name: String,
    hover: Entity<VariableHoverState>,
    left: Pixels,
    top: Pixels,
    width: Pixels,
    height: Option<Pixels>,
    debug_selector: String,
) -> gpui::Stateful<gpui::Div> {
    let hover_trigger = hover.clone();
    div()
        .id(id)
        .absolute()
        .left(left)
        .top(top)
        .w(width)
        .when_some(height, |hit, height| hit.h(height))
        .debug_selector(move || debug_selector.clone())
        .on_hover({
            let hover = hover_trigger.clone();
            move |hovered, _, cx| {
                hover.update(cx, |state, cx| {
                    state.on_trigger_hover(index, name.clone(), *hovered, cx);
                });
            }
        })
        .on_prepaint({
            let hover = hover_trigger;
            move |bounds, window, cx| {
                let changed = hover.update(cx, |state, _| {
                    if state
                        .active
                        .as_ref()
                        .is_none_or(|(active, _)| *active != index)
                    {
                        return false;
                    }
                    let changed = state.trigger_bounds != bounds;
                    state.trigger_bounds = bounds;
                    changed
                });
                if changed {
                    window.request_animation_frame();
                }
            }
        })
}

fn with_variable_tooltip(
    wrapper: gpui::Stateful<gpui::Div>,
    theme: Theme,
    hover: Entity<VariableHoverState>,
    variables: VariableContext,
    cx: &App,
) -> gpui::AnyElement {
    let (open, active, bounds) = {
        let state = hover.read(cx);
        (state.open, state.active.clone(), state.trigger_bounds)
    };
    if !open {
        return wrapper.into_any_element();
    }
    let Some((_, name)) = active else {
        return wrapper.into_any_element();
    };
    if bounds.size.width <= px(0.0) || bounds.size.height <= px(0.0) {
        return wrapper.into_any_element();
    }
    let presentation = variable_tooltip_presentation(&name, &variables);
    let value_input = hover.read(cx).value_input.clone();
    *hover.read(cx).on_value_change.borrow_mut() = variables.on_change.clone();
    wrapper
        .child(
            deferred(
                Positioner::side(bounds)
                    .placement(Placement::Bottom)
                    .align(Align::Start)
                    .offset(px(4.0))
                    .margin(px(4.0))
                    .child(variable_tooltip_popup(
                        theme,
                        name,
                        presentation,
                        hover,
                        value_input,
                        variables.on_manage_environments,
                    )),
            )
            .with_priority(POPUP_PRIORITY + 1),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn variable_span_layout(
    window: &mut Window,
    value: &str,
    ranges: &[(Range<usize>, String)],
    font_family: &'static str,
    font_size: f32,
    current_scroll_x: Pixels,
    cursor: usize,
    visible_width: Option<Pixels>,
) -> Vec<(String, Pixels, Pixels)> {
    let run = TextRun {
        len: value.len(),
        font: font(font_family),
        color: transparent_black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line =
        window
            .text_system()
            .shape_line(SharedString::from(value), px(font_size), &[run], None);
    let scroll_x = visible_width.map_or(current_scroll_x, |width| {
        input_text_scroll_offset(
            line.x_for_index(cursor),
            line.width,
            width,
            current_scroll_x,
        )
    });
    ranges
        .iter()
        .map(|(range, name)| {
            let start = line.x_for_index(range.start) + scroll_x;
            let end = line.x_for_index(range.end) + scroll_x;
            (name.clone(), start, (end - start).max(px(1.0)))
        })
        .collect()
}

fn variable_highlight_layer(
    theme: Theme,
    state: Entity<InputState>,
    ranges_empty: bool,
    font_family: &'static str,
    text_size: f32,
    highlight_path_variables: bool,
) -> impl IntoElement {
    let highlight_color = theme.colors.syntax.string.into();
    let base_color = if ranges_empty {
        transparent_black()
    } else {
        theme.colors.text.primary.into()
    };
    div()
        .absolute()
        .top(px(0.0))
        .bottom(px(0.0))
        .left(px(0.0))
        .right(px(0.0))
        // Match the input's border box so highlight rects share its origin
        // and visible width.
        .border_1()
        .border_color(transparent_black())
        .px(px(theme.metrics.spacing_2))
        .items_center()
        .flex()
        .overflow_hidden()
        .font_family(font_family)
        .text_size(px(text_size))
        .when(!ranges_empty, |layer| {
            layer.debug_selector(|| "variable-highlight-overlay".into())
        })
        .child(VariableHighlightElement {
            state,
            base_color,
            highlight_color,
            highlight_path_variables,
        })
}

pub(super) struct VariableHighlightElement {
    pub(super) state: Entity<InputState>,
    pub(super) base_color: Hsla,
    pub(super) highlight_color: Hsla,
    pub(super) highlight_path_variables: bool,
}

pub(super) struct VariableHighlightPrepaintState {
    line: Option<ShapedLine>,
    pub(super) scroll_offset: Pixels,
    cursor_x: Pixels,
}

impl IntoElement for VariableHighlightElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for VariableHighlightElement {
    type RequestLayoutState = ();
    type PrepaintState = VariableHighlightPrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let state = self.state.clone();
        let value = single_line(state.read(cx).value());
        let ranges = input_variable_ranges(&value, self.highlight_path_variables)
            .into_iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        let cursor = state.read(cx).cursor();
        let style = window.text_style();
        let run = TextRun {
            len: value.len(),
            font: style.font(),
            color: self.base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = variable_highlight_runs(&value, &ranges, &run, self.highlight_color);
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(value, font_size, &runs, None);
        let cursor_x = line.x_for_index(cursor);
        VariableHighlightPrepaintState {
            line: Some(line),
            scroll_offset: px(0.0),
            cursor_x,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        // Input is the previous sibling and paints first. gpui-base commits
        // its finalized cursor-follow scroll offset during that paint, so read
        // it here instead of trying to predict it during prepaint. The fallback
        // keeps isolated element tests meaningful before an Input has laid out.
        let visible_width = bounds.right() - bounds.left();
        let current_scroll_offset = {
            let input = self.state.read(cx);
            input.scroll_offset().x
        };
        let line_width = prepaint
            .line
            .as_ref()
            .map(|line| line.width)
            .unwrap_or(px(0.0));
        let scroll_offset = input_text_scroll_offset(
            prepaint.cursor_x,
            line_width,
            visible_width,
            current_scroll_offset,
        );
        prepaint.scroll_offset = scroll_offset;
        let line = prepaint.line.take();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(line) = line {
                line.paint(
                    bounds.origin + point(scroll_offset, px(0.0)),
                    window.line_height(),
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .expect("variable highlight text should paint");
            }
        });
    }
}

/// Matches Longbridge gpui-base single-line input: shift the painted line left
/// when the caret would otherwise sit past the visible width.
pub(super) fn variable_highlight_runs(
    value: &str,
    ranges: &[Range<usize>],
    base: &TextRun,
    highlight_color: Hsla,
) -> Vec<TextRun> {
    let mut runs = Vec::new();
    let mut ix = 0;
    for range in ranges {
        let start = range.start.min(value.len());
        let end = range.end.min(value.len());
        if ix < start {
            runs.push(TextRun {
                len: start - ix,
                color: base.color,
                ..base.clone()
            });
        }
        if end > start {
            runs.push(TextRun {
                len: end - start,
                color: highlight_color,
                ..base.clone()
            });
        }
        ix = ix.max(end);
    }
    if ix < value.len() {
        runs.push(TextRun {
            len: value.len() - ix,
            color: base.color,
            ..base.clone()
        });
    }
    runs.retain(|run| run.len > 0);
    if runs.is_empty() {
        runs.push(base.clone());
    }
    runs
}

/// Horizontal scroll for single-line input highlights.
///
/// Matches gpui-base `InputBaseState::scroll_to` for left-aligned input without
/// line numbers (`RIGHT_MARGIN` = 10px). The overlay must reuse the input's
/// current scroll offset; recomputing from zero desyncs highlights from text
/// once the caret moves while the field is already scrolled.
pub(super) fn input_text_scroll_offset(
    cursor_x: Pixels,
    line_width: Pixels,
    visible_width: Pixels,
    current_scroll_x: Pixels,
) -> Pixels {
    const RIGHT_MARGIN: Pixels = px(10.0);
    let mut scroll_x = current_scroll_x;
    if cursor_x - RIGHT_MARGIN < -scroll_x {
        scroll_x = -cursor_x + RIGHT_MARGIN;
    } else if cursor_x + RIGHT_MARGIN > -scroll_x + visible_width {
        scroll_x = -(cursor_x - visible_width + RIGHT_MARGIN);
    }
    // gpui-base clamps the offset after shaping. This matters when deleting
    // from the end of a scrolled value: the valid left edge moves right on
    // every keystroke, before the updated offset is observable from state.
    let scroll_width = if line_width + RIGHT_MARGIN > visible_width {
        line_width + RIGHT_MARGIN
    } else {
        line_width
    };
    let minimum_scroll_x = (-scroll_width + visible_width).min(px(0.0));
    scroll_x.clamp(minimum_scroll_x, px(0.0))
}

pub(crate) fn single_line(value: impl Into<SharedString>) -> SharedString {
    let value = value.into();
    if value.find(['\n', '\r']).is_none() {
        value
    } else {
        SharedString::from(value.replace(['\n', '\r'], " "))
    }
}

pub(super) fn variable_tooltip_presentation(
    name: &str,
    variables: &VariableContext,
) -> VariableTooltipPresentation {
    if variables.secrets.contains(name) {
        return unavailable_variable_tooltip(&variables.unavailable_message);
    }
    if let Some(value) = variables.values.get(name) {
        return VariableTooltipPresentation {
            value: value.clone(),
            placeholder: "Variable value",
            editable: variables.on_change.is_some(),
            hint: None,
        };
    }
    if variables.on_change.is_some() {
        return VariableTooltipPresentation {
            value: String::new(),
            placeholder: "Enter a value to create",
            editable: true,
            hint: Some("Not defined in this environment"),
        };
    }
    unavailable_variable_tooltip(&variables.unavailable_message)
}

fn unavailable_variable_tooltip(message: &str) -> VariableTooltipPresentation {
    VariableTooltipPresentation {
        value: message.to_owned(),
        placeholder: "Variable value",
        editable: false,
        hint: None,
    }
}

pub(super) struct VariableTooltipPresentation {
    pub(super) value: String,
    pub(super) placeholder: &'static str,
    pub(super) editable: bool,
    pub(super) hint: Option<&'static str>,
}

pub(super) fn variable_ranges(value: &str) -> Vec<(Range<usize>, String)> {
    let mut ranges = Vec::new();
    let mut offset = 0;
    let mut remaining = value;
    while let Some(start) = remaining.find("{{") {
        let after_start = &remaining[start + 2..];
        let Some(end) = after_start.find("}}") else {
            break;
        };
        let name = after_start[..end].trim();
        if !name.is_empty() && !name.contains("{{") {
            let range_start = offset + start;
            let range_end = range_start + 2 + end + 2;
            ranges.push((range_start..range_end, name.to_owned()));
        }
        let consumed = start + 2 + end + 2;
        offset += consumed;
        remaining = &remaining[consumed..];
    }
    ranges
}

pub(super) fn input_variable_ranges(
    value: &str,
    highlight_path_variables: bool,
) -> Vec<(Range<usize>, String)> {
    let mut ranges = variable_ranges(value);
    if highlight_path_variables {
        ranges.extend(path_variable_ranges(value));
        ranges.sort_by_key(|(range, _)| range.start);
    }
    ranges
}
