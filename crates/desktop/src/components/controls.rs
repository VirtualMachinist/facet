use super::*;

#[derive(Clone, Copy)]
pub(super) struct EditorInsets {
    pub(super) top: f32,
    pub(super) right: f32,
    pub(super) bottom: f32,
    pub(super) left: f32,
}

impl EditorInsets {
    pub(super) fn standard(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: theme.metrics.spacing_2,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_2,
        }
    }

    pub(super) fn response(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: 2.0,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_1,
        }
    }

    pub(super) fn edges(self) -> Edges<Pixels> {
        Edges {
            top: px(self.top),
            right: px(self.right),
            bottom: px(self.bottom),
            left: px(self.left),
        }
    }
}

#[derive(Clone)]
pub(super) struct TextContextMenuExtraAction {
    pub(super) id: &'static str,
    pub(super) label: &'static str,
    pub(super) requires_selection: bool,
    pub(super) is_enabled: TextContextEnableHandler,
    pub(super) on_click: TextContextActionHandler,
}

pub(crate) struct ResponseBodyInputOptions<'a> {
    pub(super) matches: &'a [SearchMatch],
    pub(super) active_match: usize,
    pub(super) inspection_reveal: Option<(Range<usize>, bool)>,
    pub(super) language: SharedString,
    pub(super) soft_wrap: bool,
    pub(super) on_visible_range: VisibleRangeHandler,
    pub(super) on_mouse_down: EditorMouseDownHandler,
    pub(super) inspect_enabled: TextContextEnableHandler,
    pub(super) on_inspect: TextContextActionHandler,
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

#[derive(Clone)]
struct DropdownAction {
    label: String,
    on_activate: DropdownActionHandler,
}

#[derive(Debug)]
struct DropdownState {
    open: bool,
    highlighted: usize,
}

struct DropdownController {
    state: Entity<DropdownState>,
    parent: EntityId,
    trigger_focus: FocusHandle,
    selected_index: usize,
}

impl Clone for DropdownController {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
            parent: self.parent,
            trigger_focus: self.trigger_focus.clone(),
            selected_index: self.selected_index,
        }
    }
}

impl DropdownController {
    fn highlighted(&self, cx: &App) -> usize {
        self.state.read(cx).highlighted
    }

    fn repaint(&self, cx: &mut App) {
        cx.notify(self.parent);
    }

    fn set_open(&self, open: bool, list_focus: &FocusHandle, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.open = open;
            if open {
                state.highlighted = self.selected_index;
            }
            cx.notify();
        });
        if open {
            let list_focus = list_focus.clone();
            let parent = self.parent;
            window.defer(cx, move |window, cx| {
                list_focus.focus(window, cx);
                cx.notify(parent);
            });
        } else {
            self.repaint(cx);
        }
    }

    fn toggle_open(&self, list_focus: &FocusHandle, window: &mut Window, cx: &mut App) {
        let open = !self.state.read(cx).open;
        self.set_open(open, list_focus, window, cx);
    }

    fn close(&self, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.open = false;
            cx.notify();
        });
        self.repaint(cx);
    }

    fn restore_trigger_focus(&self, window: &mut Window, cx: &mut App) {
        let trigger_focus = self.trigger_focus.clone();
        window.defer(cx, move |window, cx| trigger_focus.focus(window, cx));
    }

    fn close_and_restore_trigger(&self, window: &mut Window, cx: &mut App) {
        self.close(cx);
        self.restore_trigger_focus(window, cx);
    }

    fn move_highlight(&self, delta: i32, len: usize, cx: &mut App) {
        if len == 0 {
            return;
        }
        self.state.update(cx, |state, cx| {
            let next = state.highlighted as i32 + delta;
            state.highlighted = next.rem_euclid(len as i32) as usize;
            cx.notify();
        });
        self.repaint(cx);
    }

    fn set_highlight(&self, index: usize, cx: &mut App) {
        self.state.update(cx, |state, cx| {
            state.highlighted = index;
            cx.notify();
        });
        self.repaint(cx);
    }
}

#[derive(IntoElement)]
struct DropdownOption<T: Clone + Eq + 'static> {
    theme: Theme,
    id: &'static str,
    index: usize,
    value: T,
    label: String,
    color: gpui::Rgba,
    selected: bool,
    highlighted: bool,
    controller: DropdownController,
    on_value_change: DropdownChangeHandler<T>,
}

impl<T: Clone + Eq + 'static> RenderOnce for DropdownOption<T> {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let highlight_background = self.theme.colors.selection.inactive_background;
        let theme = self.theme;
        let id = self.id;
        let index = self.index;
        div()
            .id(format!("{id}-item-{index}"))
            .role(Role::ListBoxOption)
            .aria_selected(self.selected)
            .when(self.highlighted, |item| item.aria_active_descendant())
            .w_full()
            .h(px(theme.metrics.control_height))
            .px(px(theme.metrics.spacing_2))
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .overflow_hidden()
            .rounded(px(theme.metrics.radius_small))
            .text_color(self.color)
            .cursor_pointer()
            .debug_selector(move || format!("{id}-item-{index}"))
            .when(self.highlighted, |item| {
                item.bg(highlight_background)
                    .border_1()
                    .border_color(theme.colors.borders.focused)
            })
            .when(!self.highlighted, |item| {
                item.border_1().border_color(transparent_black())
            })
            .hover(move |item| item.bg(theme.colors.surfaces.sidebar))
            .on_hover({
                let controller = self.controller.clone();
                move |hovered, _, cx| {
                    if *hovered {
                        controller.set_highlight(index, cx);
                    }
                }
            })
            .on_click({
                let controller = self.controller.clone();
                let value = self.value;
                let on_value_change = self.on_value_change;
                move |_, window, cx| {
                    on_value_change(Some(&value), window, cx);
                    controller.close_and_restore_trigger(window, cx);
                }
            })
            .child(
                div()
                    .flex_none()
                    .w(px(14.0))
                    .when(!self.selected, |marker| marker.invisible())
                    .child("✓"),
            )
            .child(truncated_label(self.label).min_w(px(0.0)).flex_1())
    }
}

#[derive(IntoElement)]
struct DropdownActionItem {
    theme: Theme,
    id: &'static str,
    index: usize,
    action_index: usize,
    label: String,
    highlighted: bool,
    controller: DropdownController,
    on_activate: DropdownActionHandler,
}

impl RenderOnce for DropdownActionItem {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let highlight_background = self.theme.colors.selection.inactive_background;
        let theme = self.theme;
        let id = self.id;
        let index = self.index;
        let action_index = self.action_index;
        div()
            .id(format!("{id}-action-{action_index}"))
            .role(Role::ListBoxOption)
            .when(self.highlighted, |item| item.aria_active_descendant())
            .w_full()
            .h(px(theme.metrics.control_height))
            .px(px(theme.metrics.spacing_2))
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .overflow_hidden()
            .rounded(px(theme.metrics.radius_small))
            .text_color(theme.colors.text.primary)
            .cursor_pointer()
            .debug_selector(move || format!("{id}-action-{action_index}"))
            .when(self.highlighted, |item| {
                item.bg(highlight_background)
                    .border_1()
                    .border_color(theme.colors.borders.focused)
            })
            .when(!self.highlighted, |item| {
                item.border_1().border_color(transparent_black())
            })
            .hover(move |item| item.bg(theme.colors.surfaces.sidebar))
            .on_hover({
                let controller = self.controller.clone();
                move |hovered, _, cx| {
                    if *hovered {
                        controller.set_highlight(index, cx);
                    }
                }
            })
            .on_click({
                let controller = self.controller.clone();
                let on_activate = self.on_activate;
                move |_, window, cx| {
                    controller.close_and_restore_trigger(window, cx);
                    on_activate(window, cx);
                }
            })
            .child(div().flex_none().w(px(14.0)))
            .child(truncated_label(self.label).min_w(px(0.0)).flex_1())
    }
}

pub(super) type VisibleRangeHandler = Rc<dyn Fn(Range<usize>, &mut App)>;
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
    pub(super) theme: Theme,
    pub(super) id: ElementId,
    pub(super) value: SharedString,
    pub(super) placeholder: SharedString,
    pub(super) variables: VariableContext,
    pub(super) highlight_path_variables: bool,
    pub(super) variable_overlay: bool,
    pub(super) font_family: &'static str,
    pub(super) text_size: f32,
    pub(super) height: f32,
    pub(super) width: Option<f32>,
    pub(super) debug_selector: Option<&'static str>,
    pub(super) on_change: Option<InputChangeHandler>,
    pub(super) on_enter: Option<InputChangeHandler>,
    pub(super) on_focus: Option<FocusChangeHandler>,
    pub(super) autofocus: bool,
    pub(super) readonly: bool,
    pub(super) shared_input: Option<Entity<InputState>>,
    pub(super) flat: bool,
    pub(super) leading_icon: Option<gpui::Div>,
    pub(super) content_gap: f32,
    pub(super) quiet_focus: bool,
    pub(super) focus_on_render: bool,
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

pub(super) fn text_input_base(
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

fn editor_button_base(
    theme: Theme,
    id: impl Into<ElementId>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let id = id.into();
    let debug_id = id.to_string();
    Button::new(id)
        .debug_selector(move || debug_id.clone())
        .selected(selected)
        .h(px(theme.metrics.control_height))
        .px(px(theme.metrics.spacing_3))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .text_size(px(theme.typography.caption_size))
        .text_color(if selected {
            theme.colors.selection.active_foreground
        } else {
            theme.colors.text.secondary
        })
        .bg(if selected {
            theme.colors.selection.active_background.into()
        } else {
            transparent_black()
        })
        .border_1()
        .border_color(if selected {
            theme.colors.selection.active_background.into()
        } else {
            transparent_black()
        })
        .cursor_pointer()
        .hover(move |button| {
            if selected {
                button
            } else {
                button.bg(theme.colors.selection.inactive_background)
            }
        })
        .focus(move |button| button.border_color(theme.colors.borders.focused))
        .on_click(on_click)
}

pub(crate) fn editor_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    editor_button_base(theme, id, selected, on_click).child(label.into())
}

pub(crate) fn editor_subtab(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    selected: bool,
    position: usize,
    size: usize,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    tab_button_base(theme, id, selected, position, size, on_click)
        .h(px(theme.metrics.control_height))
        .px(px(theme.metrics.spacing_2))
        .flex()
        .items_center()
        .border_b_1()
        .border_color(if selected {
            theme.colors.actions.accent.into()
        } else {
            transparent_black()
        })
        .text_size(px(theme.typography.caption_size))
        .text_color(if selected {
            theme.colors.actions.accent
        } else {
            theme.colors.text.secondary
        })
        .when(!selected, |tab| {
            tab.hover(move |tab| tab.bg(theme.colors.surfaces.raised))
        })
        .child(label.into())
}

pub(crate) fn editor_add_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    editor_button_base(theme, id, false, on_click)
        .gap(px(theme.metrics.spacing_1))
        .child(plus_icon(theme))
        .child(label.into())
}

pub(crate) fn editor_key_value_row(theme: Theme) -> gpui::Div {
    div().flex().items_center().gap(px(theme.metrics.spacing_1))
}

pub(crate) fn text_tab(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    selected: bool,
    position: usize,
    size: usize,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    tab_button_base(theme, id, selected, position, size, on_click)
        .h(px(theme.metrics.control_height - 2.0))
        .px(px(theme.metrics.spacing_2))
        .flex()
        .items_center()
        .rounded(px(theme.metrics.radius_medium))
        .border_1()
        .border_color(transparent_black())
        .text_size(px(theme.typography.caption_size))
        .text_color(if selected {
            theme.colors.text.primary
        } else {
            theme.colors.text.secondary
        })
        .when(selected, |tab| {
            tab.bg(theme.colors.selection.inactive_background)
                .font_weight(FontWeight::SEMIBOLD)
        })
        .when(!selected, |tab| {
            tab.hover(move |tab| tab.bg(theme.colors.surfaces.raised))
        })
        .child(label)
}

fn tab_button_base(
    theme: Theme,
    id: impl Into<ElementId>,
    selected: bool,
    position: usize,
    size: usize,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .role(Role::Tab)
        .selected(selected)
        .aria_selected(selected)
        .aria_position_in_set(position)
        .aria_size_of_set(size)
        .focus(move |tab| tab.border_color(theme.colors.borders.focused))
        .cursor_pointer()
        .on_click(on_click)
}

pub(crate) fn remove_row_button(
    theme: Theme,
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    icon_button(
        theme,
        id,
        aria_label,
        trash_icon(theme.colors.text.secondary),
        on_click,
    )
}

pub(crate) fn browse_file_button(
    theme: Theme,
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    icon_button(
        theme,
        id,
        aria_label,
        folder_open_icon(theme.colors.text.secondary),
        on_click,
    )
}

pub(crate) fn icon_button(
    theme: Theme,
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    icon: impl IntoElement,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let id = id.into();
    let debug_id = id.to_string();
    Button::new(id)
        .debug_selector(move || debug_id.clone())
        .accessibility_label(aria_label)
        .w(px(30.0))
        .h(px(theme.metrics.control_height))
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

pub(crate) fn compact_icon_button(
    theme: Theme,
    id: &'static str,
    aria_label: impl Into<SharedString>,
    icon: impl IntoElement,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let size = theme.metrics.control_height - 2.0;
    div()
        .debug_selector(move || id.into())
        .size(px(size))
        .flex_none()
        .child(
            Button::new(id)
                .accessibility_label(aria_label)
                .size_full()
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
                .child(icon),
        )
}

pub(crate) fn dropdown<T: Clone + Eq + 'static>(
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String)>,
    width: f32,
    on_value_change: impl Fn(Option<&T>, &mut Window, &mut App) + 'static,
) -> ProbeDropdown<T> {
    ProbeDropdown {
        theme,
        id,
        aria_label,
        value,
        options: options
            .into_iter()
            .map(|(value, label)| (value, label, None))
            .collect(),
        width,
        actions: Vec::new(),
        disabled: false,
        on_value_change: Rc::new(on_value_change),
    }
}

pub(crate) fn dropdown_with_option_colors<T: Clone + Eq + 'static>(
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String, Option<gpui::Rgba>)>,
    width: f32,
    on_value_change: impl Fn(Option<&T>, &mut Window, &mut App) + 'static,
) -> ProbeDropdown<T> {
    ProbeDropdown {
        theme,
        id,
        aria_label,
        value,
        options,
        width,
        actions: Vec::new(),
        disabled: false,
        on_value_change: Rc::new(on_value_change),
    }
}

#[derive(IntoElement)]
pub(crate) struct ProbeDropdown<T: Clone + Eq + 'static> {
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String, Option<gpui::Rgba>)>,
    width: f32,
    actions: Vec<DropdownAction>,
    disabled: bool,
    on_value_change: DropdownChangeHandler<T>,
}

impl<T: Clone + Eq + 'static> ProbeDropdown<T> {
    pub(crate) fn with_action(
        mut self,
        label: impl Into<String>,
        on_activate: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push(DropdownAction {
            label: label.into(),
            on_activate: Rc::new(on_activate),
        });
        self
    }

    pub(crate) fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl<T: Clone + Eq + 'static> RenderOnce for ProbeDropdown<T> {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let selected_index = self
            .value
            .as_ref()
            .and_then(|selected| {
                self.options
                    .iter()
                    .position(|(value, _, _)| value == selected)
            })
            .unwrap_or(0);
        let state =
            window.use_keyed_state(ElementId::from(format!("{}-state", self.id)), cx, |_, _| {
                DropdownState {
                    open: false,
                    highlighted: selected_index,
                }
            });
        state.update(cx, |state, _| {
            let item_count = self.options.len() + self.actions.len();
            if item_count == 0 {
                state.highlighted = 0;
            } else if state.highlighted >= item_count {
                state.highlighted = selected_index.min(item_count - 1);
            }
        });

        let theme = self.theme;
        let id = self.id;
        let open = state.read(cx).open;
        let highlighted_index = state.read(cx).highlighted;
        let parent = window.current_view();
        let trigger_focus = window
            .use_keyed_state(format!("{id}-trigger-focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let list_focus = window
            .use_keyed_state(format!("{id}-list-focus"), cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        let controller = DropdownController {
            state: state.clone(),
            parent,
            trigger_focus: trigger_focus.clone(),
            selected_index,
        };

        let selected_label = self
            .value
            .as_ref()
            .and_then(|selected| {
                self.options
                    .iter()
                    .find(|(value, _, _)| value == selected)
                    .map(|(_, label, _)| label.clone())
            })
            .unwrap_or_else(|| "None".to_owned());
        let selected_color = self
            .value
            .as_ref()
            .and_then(|selected| {
                self.options
                    .iter()
                    .find(|(value, _, _)| value == selected)
                    .and_then(|(_, _, color)| *color)
            })
            .unwrap_or(theme.colors.text.primary);

        let option_count = self.options.len();
        let item_count = option_count + self.actions.len();
        let on_value_change = self.on_value_change.clone();
        let selected_value = self.value;
        let options = self.options;
        let actions = self.actions;
        let list = div()
            .id(format!("{id}-list"))
            .track_focus(&list_focus)
            .role(Role::ListBox)
            .key_context("Select")
            .on_action({
                let controller = controller.clone();
                move |_: &SelectDown, _, cx| controller.move_highlight(1, item_count, cx)
            })
            .on_action({
                let controller = controller.clone();
                move |_: &SelectUp, _, cx| controller.move_highlight(-1, item_count, cx)
            })
            .on_action({
                let controller = controller.clone();
                let on_value_change = on_value_change.clone();
                let options = options.clone();
                let actions = actions.clone();
                move |_: &Confirm, window, cx| {
                    let index = controller.highlighted(cx);
                    if let Some((value, _, _)) = options.get(index) {
                        on_value_change(Some(value), window, cx);
                        controller.close_and_restore_trigger(window, cx);
                    } else if let Some(action) = actions.get(index.saturating_sub(options.len())) {
                        controller.close_and_restore_trigger(window, cx);
                        (action.on_activate)(window, cx);
                    }
                }
            })
            .flex()
            .flex_col()
            .w(px(self.width.max(160.0)))
            .p(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .children(
                options
                    .into_iter()
                    .enumerate()
                    .map(|(index, (value, label, color))| {
                        let selected = selected_value.as_ref() == Some(&value);
                        DropdownOption {
                            theme,
                            id,
                            index,
                            value,
                            label,
                            color: color.unwrap_or(theme.colors.text.primary),
                            selected,
                            highlighted: index == highlighted_index,
                            controller: controller.clone(),
                            on_value_change: on_value_change.clone(),
                        }
                        .into_any_element()
                    }),
            )
            .when(!actions.is_empty(), |list| {
                list.child(
                    div()
                        .my(px(theme.metrics.spacing_1))
                        .mx(px(theme.metrics.spacing_2))
                        .flex_none()
                        .h(px(1.0))
                        .bg(theme.colors.borders.standard),
                )
            })
            .children(
                actions
                    .into_iter()
                    .enumerate()
                    .map(|(action_index, action)| {
                        let index = option_count + action_index;
                        DropdownActionItem {
                            theme,
                            id,
                            index,
                            action_index,
                            label: action.label,
                            highlighted: index == highlighted_index,
                            controller: controller.clone(),
                            on_activate: action.on_activate,
                        }
                        .into_any_element()
                    }),
            );

        let popup_content = div()
            .occlude()
            .key_context("Select")
            .on_action({
                let controller = controller.clone();
                move |_: &Cancel, window, cx| {
                    cx.stop_propagation();
                    controller.close_and_restore_trigger(window, cx);
                }
            })
            .on_mouse_down_out({
                let controller = controller.clone();
                move |_, _, cx| controller.close(cx)
            })
            .child(list);

        let trigger = Button::new(format!("{id}-trigger"))
            .track_focus(&trigger_focus)
            .accessibility_label(self.aria_label)
            .selected(open)
            .w_full()
            .h(px(theme.metrics.control_height))
            .px(px(theme.metrics.spacing_2))
            .flex()
            .items_center()
            .justify_between()
            .gap(px(theme.metrics.spacing_1))
            .overflow_hidden()
            .rounded(px(theme.metrics.radius_small))
            .bg(theme.colors.surfaces.raised)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .text_color(if self.disabled {
                theme.colors.actions.disabled_foreground
            } else {
                selected_color
            })
            .debug_selector(move || format!("{id}-trigger"))
            .when(!self.disabled, |trigger| {
                trigger.hover(move |trigger| trigger.bg(theme.colors.selection.inactive_background))
            })
            .focus(move |trigger| trigger.border_color(theme.colors.borders.focused))
            .disabled(self.disabled)
            .on_click({
                let controller = controller.clone();
                let list_focus = list_focus.clone();
                let disabled = self.disabled;
                move |_, window, cx| {
                    if disabled {
                        return;
                    }
                    controller.toggle_open(&list_focus, window, cx)
                }
            })
            .child(truncated_label(selected_label).min_w(px(0.0)).flex_1())
            .child(chevron_icon(theme, open));

        let select_root = Select::new(format!("{id}-select"))
            .open(open && !self.disabled)
            .accessibility_label(self.aria_label)
            .focus_handle(&trigger_focus)
            .content_focus_handle(&list_focus)
            .on_open_change({
                let controller = controller.clone();
                let list_focus = list_focus.clone();
                move |next, window, cx| controller.set_open(next, &list_focus, window, cx)
            })
            .child(trigger);

        Popup::new(format!("{id}-popup"), select_root)
            .when(open, |popup| popup.content(popup_content))
    }
}
