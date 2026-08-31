use super::*;

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

pub(in crate::components) fn editor_value_needs_refresh(
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
pub(in crate::components) fn editor_paint_style(theme: Theme) -> InputEditorStyle {
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
pub(in crate::components) struct ProbeEditor {
    pub(in crate::components) theme: Theme,
    pub(in crate::components) id: ElementId,
    pub(in crate::components) value: SharedString,
    pub(in crate::components) placeholder: SharedString,
    pub(in crate::components) decorations: Vec<TextDecoration>,
    pub(in crate::components) language: SharedString,
    pub(in crate::components) readonly: bool,
    pub(in crate::components) min_height: Option<f32>,
    pub(in crate::components) padding: EditorInsets,
    pub(in crate::components) soft_wrap: bool,
    pub(in crate::components) text_color: gpui::Rgba,
    pub(in crate::components) scroll_to_range: Option<Range<usize>>,
    pub(in crate::components) search_matches: Vec<(Range<usize>, bool)>,
    pub(in crate::components) on_change: Option<InputChangeHandler>,
    pub(in crate::components) on_mouse_down: Option<EditorMouseDownHandler>,
    pub(in crate::components) on_visible_range: Option<VisibleRangeHandler>,
    pub(in crate::components) extra_context_menu_actions: Vec<TextContextMenuExtraAction>,
    pub(in crate::components) debug_selector: Option<&'static str>,
    pub(in crate::components) variables: Option<VariableContext>,
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
                        search.set_active_match(active_match);
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
        let (matches, active_match) = if search_state.read(cx).is_open() {
            let matches = state.read(cx).search_session().matcher.matched_ranges();
            let active_match = search_state
                .read(cx)
                .active_match()
                .min(matches.len().saturating_sub(1));
            if active_match != search_state.read(cx).active_match() {
                search_state.update(cx, |search, _| search.set_active_match(active_match));
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
                    if search_state.read(cx).is_open() {
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
