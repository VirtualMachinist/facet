use super::*;

#[derive(Clone, Copy)]
pub(in crate::components) struct EditorInsets {
    pub(in crate::components) top: f32,
    pub(in crate::components) right: f32,
    pub(in crate::components) bottom: f32,
    pub(in crate::components) left: f32,
}

impl EditorInsets {
    pub(in crate::components) fn standard(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: theme.metrics.spacing_2,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_2,
        }
    }

    pub(in crate::components) fn response(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: 2.0,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_1,
        }
    }

    pub(in crate::components) fn edges(self) -> Edges<Pixels> {
        Edges {
            top: px(self.top),
            right: px(self.right),
            bottom: px(self.bottom),
            left: px(self.left),
        }
    }
}

#[derive(Clone)]
pub(in crate::components) struct TextContextMenuExtraAction {
    pub(in crate::components) id: &'static str,
    pub(in crate::components) label: &'static str,
    pub(in crate::components) requires_selection: bool,
    pub(in crate::components) is_enabled: TextContextEnableHandler,
    pub(in crate::components) on_click: TextContextActionHandler,
}

pub(crate) struct ResponseBodyInputOptions<'a> {
    pub(in crate::components) matches: &'a [SearchMatch],
    pub(in crate::components) active_match: usize,
    pub(in crate::components) inspection_reveal: Option<(Range<usize>, bool)>,
    pub(in crate::components) language: SharedString,
    pub(in crate::components) soft_wrap: bool,
    pub(in crate::components) on_visible_range: VisibleRangeHandler,
    pub(in crate::components) on_mouse_down: EditorMouseDownHandler,
    pub(in crate::components) inspect_enabled: TextContextEnableHandler,
    pub(in crate::components) on_inspect: TextContextActionHandler,
}

impl<'a> ResponseBodyInputOptions<'a> {
    pub(crate) fn new(
        matches: &'a [SearchMatch],
        active_match: usize,
        language: impl Into<SharedString>,
        on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
        on_mouse_down: impl Fn(&mut Window, &mut App) + 'static,
        inspect_enabled: impl Fn(Option<&str>, usize) -> bool + 'static,
        on_inspect: impl Fn(Option<String>, usize, &mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            matches,
            active_match,
            inspection_reveal: None,
            language: language.into(),
            soft_wrap: true,
            on_visible_range: Rc::new(on_visible_range),
            on_mouse_down: Rc::new(on_mouse_down),
            inspect_enabled: Rc::new(inspect_enabled),
            on_inspect: Rc::new(on_inspect),
        }
    }

    pub(crate) fn inspection_reveal(
        mut self,
        inspection_reveal: Option<(Range<usize>, bool)>,
    ) -> Self {
        self.inspection_reveal = inspection_reveal;
        self
    }

    pub(crate) fn soft_wrap(mut self, soft_wrap: bool) -> Self {
        self.soft_wrap = soft_wrap;
        self
    }
}

pub(in crate::components) type VisibleRangeHandler = Rc<dyn Fn(Range<usize>, &mut App)>;
type FocusChangeHandler = Rc<dyn Fn(bool, &mut App)>;

struct FieldInput {
    state: Entity<InputState>,
    on_change: Option<InputChangeHandler>,
    on_enter: Option<InputChangeHandler>,
    on_focus: Option<FocusChangeHandler>,
    autofocused: bool,
    _subscription: Subscription,
}

impl FieldInput {
    fn on_event(
        this: &mut Self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Some(on_change) = this.on_change.clone() {
                    let value = input.read(cx).value();
                    on_change(value, window, cx);
                }
            }
            InputEvent::PressEnter { .. } => {
                if let Some(on_enter) = this.on_enter.clone() {
                    let value = input.read(cx).value();
                    on_enter(value, window, cx);
                }
            }
            InputEvent::Focus => {
                if let Some(on_focus) = &this.on_focus {
                    on_focus(true, cx);
                }
            }
            InputEvent::Blur => {
                if let Some(on_focus) = &this.on_focus {
                    on_focus(false, cx);
                }
            }
        }
    }
}

#[derive(IntoElement)]
pub(crate) struct ProbeTextInput {
    pub(in crate::components) theme: Theme,
    pub(in crate::components) id: ElementId,
    pub(in crate::components) value: SharedString,
    pub(in crate::components) placeholder: SharedString,
    pub(in crate::components) variables: VariableContext,
    pub(in crate::components) highlight_path_variables: bool,
    pub(in crate::components) variable_overlay: bool,
    pub(in crate::components) font_family: &'static str,
    pub(in crate::components) text_size: f32,
    pub(in crate::components) height: f32,
    pub(in crate::components) width: Option<f32>,
    pub(in crate::components) debug_selector: Option<&'static str>,
    pub(in crate::components) on_change: Option<InputChangeHandler>,
    pub(in crate::components) on_enter: Option<InputChangeHandler>,
    pub(in crate::components) on_focus: Option<FocusChangeHandler>,
    pub(in crate::components) autofocus: bool,
    pub(in crate::components) readonly: bool,
    pub(in crate::components) shared_input: Option<Entity<InputState>>,
    pub(in crate::components) flat: bool,
    pub(in crate::components) leading_icon: Option<gpui::Div>,
    pub(in crate::components) content_gap: f32,
    pub(in crate::components) quiet_focus: bool,
    pub(in crate::components) focus_on_render: bool,
}

impl RenderOnce for ProbeTextInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let component_id = self.id.clone();
        let context_menu = window.use_keyed_state(
            text_context_menu_id(&component_id, "context-menu-state"),
            cx,
            |_, _| TextContextMenuState::default(),
        );
        let overlay_paints_text = self.variable_overlay
            && !input_variable_ranges(&self.value, self.highlight_path_variables).is_empty();
        let placeholder = self.placeholder.clone();
        let on_change = self.on_change.clone();
        let on_enter = self.on_enter.clone();
        let state = if let Some(state) = self.shared_input.clone() {
            state
        } else {
            let field = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
                let state = cx.new(|cx| {
                    let mut state = InputState::new(window, cx).placeholder(placeholder.clone());
                    state.set_editor_style(editor_paint_style(self.theme));
                    state
                });
                let subscription = cx.subscribe_in(&state, window, FieldInput::on_event);
                FieldInput {
                    state,
                    on_change: on_change.clone(),
                    on_enter: on_enter.clone(),
                    on_focus: None,
                    autofocused: false,
                    _subscription: subscription,
                }
            });
            field.update(cx, |field, _| {
                field.on_change = self.on_change.clone();
                field.on_enter = self.on_enter.clone();
                field.on_focus = self.on_focus.clone();
            });
            if self.autofocus && !field.read(cx).autofocused {
                field.update(cx, |field, _| field.autofocused = true);
                let focus_state = field.read(cx).state.clone();
                window.defer(cx, move |window, cx| {
                    focus_state.update(cx, |input, cx| input.focus(window, cx));
                });
            }
            if self.focus_on_render
                && !field
                    .read(cx)
                    .state
                    .read(cx)
                    .focus_handle(cx)
                    .is_focused(window)
            {
                let focus_state = field.read(cx).state.clone();
                window.defer(cx, move |window, cx| {
                    focus_state.update(cx, |input, cx| input.focus(window, cx));
                });
            }
            field.read(cx).state.clone()
        };
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        let context_focus = state.read(cx).focus_handle(cx);
        let open_context_menu = context_menu.clone();
        state.update(cx, |input, cx| {
            input.set_editor_style(editor_paint_style(self.theme));
            input.set_readonly(self.readonly, cx);
            input.set_placeholder(placeholder, window, cx);
            input.on_context_menu(Rc::new(move |_, capabilities, position, window, cx| {
                context_focus.focus(window, cx);
                open_context_menu.update(cx, |state, cx| {
                    state.position = Some(position);
                    state.capabilities = capabilities;
                    state.target_focus = Some(context_focus.clone());
                    cx.notify();
                });
            }));
            if !focused && input.value() != self.value {
                input.set_value(self.value.clone(), window, cx);
            }
        });
        let tooltip_id = ElementId::NamedChild(
            Arc::new(self.id.clone()),
            SharedString::from("variable-tooltip"),
        );
        let theme = self.theme;
        let input = InputBase::new(self.id.clone())
            .h(px(self.height))
            .when_some(self.width, |input, width| input.w(px(width)))
            .when(self.width.is_none(), |input| input.min_w(px(0.0)).w_full())
            .px(px(theme.metrics.spacing_2))
            .flex()
            .items_center()
            .gap(px(self.content_gap))
            .rounded(px(theme.metrics.radius_small))
            .font_family(self.font_family)
            .text_size(px(self.text_size))
            // gpui-base's single-line Input takes glyph color from the
            // enclosing GPUI text style, not InputEditorStyle::foreground.
            // Hide that native glyph copy when the variable layer paints the
            // complete value, otherwise both layers visibly diverge on scroll.
            .text_color(if overlay_paints_text {
                transparent_black()
            } else {
                theme.colors.text.primary.into()
            })
            .bg(if self.flat {
                theme.colors.surfaces.sidebar
            } else {
                theme.colors.surfaces.raised
            })
            .when(!self.flat, |input| {
                input.border_1().border_color(if focused {
                    if self.quiet_focus {
                        theme.colors.borders.strong
                    } else {
                        theme.colors.borders.focused
                    }
                } else {
                    theme.colors.borders.standard
                })
            })
            .focused(focused)
            .styles(move |styles| {
                styles.focused(move |input| {
                    if self.flat {
                        input.bg(hover_fill(theme.colors.surfaces.sidebar))
                    } else if self.quiet_focus {
                        input.border_color(theme.colors.borders.strong)
                    } else {
                        input.border_color(theme.colors.borders.focused)
                    }
                })
            })
            .when_some(self.debug_selector, |input, selector| {
                input.debug_selector(move || selector.into())
            })
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                move |_, window, cx| {
                    state.update(cx, |input, cx| input.focus(window, cx));
                }
            })
            .when_some(self.leading_icon, |input, icon| input.child(icon))
            .child(Input::new(&state));
        let input = if self.variable_overlay {
            variable_input_overlay(
                self.theme,
                state,
                tooltip_id,
                input,
                self.value,
                self.variables,
                self.highlight_path_variables,
                window,
                cx,
            )
        } else {
            input.into_any_element()
        };
        with_text_context_menu(
            self.theme,
            &component_id,
            context_menu,
            input,
            Vec::new(),
            false,
            window,
            cx,
        )
    }
}

pub(in crate::components) fn text_input_base(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
) -> ProbeTextInput {
    ProbeTextInput {
        theme,
        id: id.into(),
        value: single_line(value),
        placeholder: placeholder.into(),
        variables: VariableContext::default(),
        highlight_path_variables: false,
        variable_overlay: false,
        font_family: theme.typography.interface_family,
        text_size: theme.typography.body_size,
        height: theme.metrics.control_height,
        width: None,
        debug_selector: None,
        on_change: None,
        on_enter: None,
        on_focus: None,
        autofocus: false,
        readonly: false,
        shared_input: None,
        flat: false,
        leading_icon: None,
        content_gap: theme.metrics.spacing_1,
        quiet_focus: false,
        focus_on_render: false,
    }
}

pub(crate) fn variable_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    variables: VariableContext,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let mut input = text_input_base(theme, id, value, placeholder);
    input.variables = variables;
    input.variable_overlay = true;
    input.font_family = theme.typography.monospace_family;
    input.on_change = Some(Rc::new(on_value_change));
    input.into_any_element()
}

pub(crate) fn url_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    variables: VariableContext,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    let mut input = text_input_base(theme, id, value, placeholder);
    input.variables = variables;
    input.highlight_path_variables = true;
    input.variable_overlay = true;
    input.font_family = theme.typography.monospace_family;
    input.on_change = Some(Rc::new(on_value_change));
    input.into_any_element()
}

pub(crate) fn dialog_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    autofocus: bool,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    on_enter: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> ProbeTextInput {
    let mut input = text_input_base(theme, id, value, placeholder);
    input.on_change = Some(Rc::new(on_value_change));
    input.on_enter = Some(Rc::new(on_enter));
    input.autofocus = autofocus;
    input
}

impl ProbeTextInput {
    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.readonly = disabled;
        self.autofocus &= !disabled;
        self
    }
}

pub(crate) fn sidebar_search_input(
    theme: Theme,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let mut input = text_input_base(theme, "tree-search-input", value, placeholder);
    input.text_size = theme.typography.caption_size;
    input.debug_selector = Some("tree-search");
    input.on_change = Some(Rc::new(on_value_change));
    input.flat = true;
    input.content_gap = theme.metrics.spacing_2;
    input.leading_icon = Some(
        library_icon("lucide-search", &SEARCH_SVG, theme.metrics.icon_small)
            .text_color(theme.colors.text.muted),
    );
    input
}
