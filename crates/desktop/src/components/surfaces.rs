use super::*;

pub(crate) fn truncated_label(text: impl Into<String>) -> gpui::Div {
    div().min_w(px(0.0)).truncate().child(text.into())
}

pub(crate) fn popup_surface(
    theme: Theme,
    id: impl Into<ElementId>,
    width: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(width))
        .p(px(theme.metrics.spacing_1))
        .flex()
        .flex_col()
        .rounded(px(theme.metrics.radius_medium))
        .bg(theme.colors.surfaces.overlay)
        .border_1()
        .border_color(theme.colors.borders.standard)
}

pub(super) fn temporary_surface_shadow(theme: Theme, y_offset: f32) -> Vec<BoxShadow> {
    let (alpha, vertical_offset) = match theme.appearance {
        crate::theme::ThemeAppearance::Light => (0.14, y_offset / 4.0),
        crate::theme::ThemeAppearance::Dark => (0.06, 0.0),
    };

    vec![
        BoxShadow::new(px(0.0), px(vertical_offset), theme.elevation_shadow(alpha))
            .blur_radius(px(20.0)),
    ]
}

/// A restrained temporary surface for application dialogs.
pub(crate) fn dialog_surface(
    theme: Theme,
    id: impl Into<ElementId>,
    width: f32,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .w(px(width))
        .p(px(theme.metrics.spacing_4))
        .flex()
        .flex_col()
        .rounded(px(theme.metrics.radius_medium))
        .bg(theme.colors.surfaces.overlay)
        .border_1()
        .border_color(theme.colors.borders.standard)
        .shadow(temporary_surface_shadow(theme, 6.0))
}

pub(crate) fn context_menu_surface(
    theme: Theme,
    id: impl Into<ElementId>,
    width: f32,
    on_dismiss: impl Fn(&mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    popup_surface(theme, id, width)
        .gap(px(theme.metrics.spacing_1))
        .occlude()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
        .on_mouse_down_out(move |_, _, cx| on_dismiss(cx))
}

#[derive(Default)]
pub(super) struct TextContextMenuState {
    pub(super) position: Option<Point<Pixels>>,
    pub(super) capabilities: InputContextMenuCapabilities,
    pub(super) target_focus: Option<FocusHandle>,
    pub(super) target_editor: Option<Entity<EditorState>>,
}

pub(super) fn text_context_menu_id(id: &ElementId, child: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(child))
}

fn text_context_menu_action(
    theme: Theme,
    id: ElementId,
    label: &'static str,
    enabled: bool,
    state: Entity<TextContextMenuState>,
    window: &Window,
    action: fn() -> Box<dyn gpui::Action>,
) -> impl IntoElement {
    let shortcut = shortcut_label_for_action(window, action().as_ref());
    menu_button_with_style(
        theme,
        id,
        label,
        shortcut,
        enabled,
        MenuButtonStyle::standard(theme),
        move |window, cx| {
            let target_focus = state.read(cx).target_focus.clone();
            state.update(cx, |state, cx| {
                state.position = None;
                cx.notify();
            });
            if let Some(target_focus) = target_focus {
                target_focus.focus(window, cx);
            }
            window.dispatch_action(action(), cx);
        },
    )
}

pub(super) fn clipboard_has_pasteable_text(cx: &App) -> bool {
    cx.read_from_clipboard()
        .and_then(|item| item.text())
        .is_some()
}

fn text_context_target(
    state: &Entity<TextContextMenuState>,
    cx: &App,
) -> Option<(Option<String>, usize)> {
    state.read(cx).target_editor.as_ref().map(|editor| {
        let editor = editor.read(cx);
        let range = editor.selected_range();
        let selected = (!range.is_empty())
            .then(|| editor.value().get(range.clone()).map(str::to_owned))
            .flatten();
        (selected, range.start)
    })
}

#[allow(clippy::too_many_arguments)]
pub(super) fn with_text_context_menu(
    theme: Theme,
    id: &ElementId,
    state: Entity<TextContextMenuState>,
    content: impl IntoElement,
    extra_actions: Vec<TextContextMenuExtraAction>,
    fill_parent: bool,
    window: &Window,
    cx: &App,
) -> gpui::AnyElement {
    let mut wrapper = div()
        .relative()
        .min_w(px(0.0))
        .when(fill_parent, |wrapper| wrapper.size_full().min_h(px(0.0)))
        .when(!fill_parent, |wrapper| wrapper.w_full())
        .child(content);
    let Some(position) = state.read(cx).position else {
        return wrapper.into_any_element();
    };

    let capabilities = state.read(cx).capabilities;
    let has_selection = capabilities.has_selection();
    let editable = capabilities.is_editable();
    let can_paste = editable && clipboard_has_pasteable_text(cx);
    let menu_id = text_context_menu_id(id, "context-menu");
    let dismiss_state = state.clone();
    let mut menu = context_menu_surface(theme, menu_id.clone(), 180.0, move |cx| {
        dismiss_state.update(cx, |state, cx| {
            state.position = None;
            cx.notify();
        });
    })
    .debug_selector(|| "text-context-menu".into());

    if !capabilities.is_readonly() {
        menu = menu
            .child(text_context_menu_action(
                theme,
                "text-context-cut".into(),
                "Cut",
                editable && has_selection,
                state.clone(),
                window,
                || Box::new(Cut),
            ))
            .child(text_context_menu_action(
                theme,
                "text-context-copy".into(),
                "Copy",
                has_selection,
                state.clone(),
                window,
                || Box::new(Copy),
            ))
            .child(text_context_menu_action(
                theme,
                "text-context-paste".into(),
                "Paste",
                can_paste,
                state.clone(),
                window,
                || Box::new(Paste),
            ))
            .child(context_menu_separator(theme));
    } else {
        menu = menu.child(text_context_menu_action(
            theme,
            "text-context-copy".into(),
            "Copy",
            has_selection,
            state.clone(),
            window,
            || Box::new(Copy),
        ));
    }

    if !extra_actions.is_empty() {
        menu = menu.child(context_menu_separator(theme));
        for action in extra_actions {
            let action_state = state.clone();
            let action_is_enabled = action.is_enabled.clone();
            let action_handler = action.on_click.clone();
            let target = text_context_target(&action_state, cx);
            let enabled = target.as_ref().is_some_and(|(selected, cursor_offset)| {
                (!action.requires_selection || selected.is_some())
                    && action_is_enabled(selected.as_deref(), *cursor_offset)
            });
            menu = menu.child(menu_button_with_style(
                theme,
                text_context_menu_id(id, action.id),
                action.label,
                None,
                enabled,
                MenuButtonStyle::standard(theme),
                move |window, cx| {
                    let target_focus = action_state.read(cx).target_focus.clone();
                    let target = text_context_target(&action_state, cx);
                    action_state.update(cx, |state, cx| {
                        state.position = None;
                        cx.notify();
                    });
                    if let Some(target_focus) = target_focus {
                        target_focus.focus(window, cx);
                    }
                    if let Some((selected, cursor_offset)) = target {
                        action_handler(selected, cursor_offset, window, cx);
                    }
                },
            ));
        }
    }

    menu = menu.child(text_context_menu_action(
        theme,
        "text-context-select-all".into(),
        "Select All",
        true,
        state,
        window,
        || Box::new(SelectAll),
    ));

    wrapper = wrapper.child(
        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(8.0))
                .child(menu),
        )
        .with_priority(POPUP_PRIORITY),
    );
    wrapper.into_any_element()
}

pub(crate) fn dialog_field_label(theme: Theme, label: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(theme.typography.caption_size))
        .font_weight(FontWeight::SEMIBOLD)
        .child(label.into())
}

pub(crate) fn dialog_title(theme: Theme, title: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(theme.typography.dialog_title_size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme.colors.text.primary)
        .child(title.into())
}

pub(crate) fn dialog_description(theme: Theme, description: impl Into<String>) -> gpui::Div {
    div()
        .text_size(px(theme.typography.body_size))
        .text_color(theme.colors.text.secondary)
        .child(description.into())
}

pub(crate) fn dialog_field(
    theme: Theme,
    label: impl Into<String>,
    control: impl IntoElement,
) -> gpui::Div {
    div()
        .flex()
        .flex_col()
        .gap(px(theme.metrics.spacing_1))
        .child(dialog_field_label(theme, label))
        .child(control)
}

pub(crate) fn dialog_actions(theme: Theme) -> gpui::Div {
    div()
        .mt(px(theme.metrics.spacing_4))
        .flex()
        .justify_end()
        .gap(px(theme.metrics.spacing_2))
}

pub(crate) fn dialog_layer(
    theme: Theme,
    focus: &FocusHandle,
    key_context: &'static str,
    content: impl IntoElement,
) -> impl IntoElement {
    div()
        .absolute()
        .top(px(0.0))
        .right(px(0.0))
        .bottom(px(0.0))
        .left(px(0.0))
        .occlude()
        .tab_stop(true)
        .key_context(key_context)
        .flex()
        .items_center()
        .justify_center()
        .child(
            div()
                .absolute()
                .top(px(0.0))
                .right(px(0.0))
                .bottom(px(0.0))
                .left(px(0.0))
                .bg(theme.colors.surfaces.scrim),
        )
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(content)
        .focus_trap(format!("{key_context}-focus-trap"), focus)
}

type VariableChangeHandler = Rc<dyn Fn(&str, String, &mut Window, &mut App)>;
type ManageEnvironmentsHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Default)]
pub(crate) struct VariableContext {
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) secrets: BTreeSet<String>,
    pub(crate) unavailable_message: String,
    pub(crate) on_change: Option<VariableChangeHandler>,
    pub(crate) on_manage_environments: Option<ManageEnvironmentsHandler>,
}

impl std::fmt::Debug for VariableContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VariableContext")
            .field("values", &self.values)
            .field("secrets", &self.secrets)
            .field("unavailable_message", &self.unavailable_message)
            .finish_non_exhaustive()
    }
}

const VARIABLE_TOOLTIP_OPEN_DELAY: Duration = Duration::from_millis(200);
const VARIABLE_TOOLTIP_CLOSE_DELAY: Duration = Duration::from_millis(200);

pub(super) struct VariableHoverState {
    pub(super) open: bool,
    pub(super) active: Option<(usize, String)>,
    pub(super) trigger_bounds: Bounds<Pixels>,
    pub(super) visible_width: Option<Pixels>,
    pub(super) overlay_origin: Option<Point<Pixels>>,
    hovering_trigger: bool,
    hovering_content: bool,
    input_focused: bool,
    epoch: usize,
    open_task: Option<Task<()>>,
    close_task: Option<Task<()>>,
    pub(super) value_input: Entity<InputState>,
    pub(super) on_value_change: RefCell<Option<VariableChangeHandler>>,
    _input_subscription: Subscription,
}

impl VariableHoverState {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let value_input = cx.new(|cx| InputState::new(window, cx).placeholder("Variable value"));
        let input_subscription = cx.subscribe_in(&value_input, window, Self::on_input_event);
        Self {
            open: false,
            active: None,
            trigger_bounds: Bounds {
                origin: point(px(0.0), px(0.0)),
                size: size(px(0.0), px(0.0)),
            },
            visible_width: None,
            overlay_origin: None,
            hovering_trigger: false,
            hovering_content: false,
            input_focused: false,
            epoch: 0,
            open_task: None,
            close_task: None,
            value_input,
            on_value_change: RefCell::new(None),
            _input_subscription: input_subscription,
        }
    }

    pub(super) fn on_input_event(
        this: &mut Self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let Some((_, name)) = this.active.clone() else {
                    return;
                };
                let Some(on_change) = this.on_value_change.borrow().clone() else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                on_change(&name, value, window, cx);
            }
            InputEvent::Focus => this.set_input_focused(true, cx),
            InputEvent::Blur => this.set_input_focused(false, cx),
            InputEvent::PressEnter { .. } => {}
        }
    }

    pub(super) fn on_trigger_hover(
        &mut self,
        index: usize,
        name: String,
        hovering: bool,
        cx: &mut Context<Self>,
    ) {
        if hovering {
            if self.input_focused {
                return;
            }
            self.active = Some((index, name));
            self.hovering_trigger = true;
            cx.notify();
            self.schedule_open(cx);
            return;
        }
        if self
            .active
            .as_ref()
            .is_some_and(|(active, _)| *active == index)
        {
            self.hovering_trigger = false;
            if !self.hovering_content && !self.input_focused {
                self.schedule_close(cx);
            }
        }
    }

    pub(super) fn on_content_hover(&mut self, hovering: bool, cx: &mut Context<Self>) {
        self.hovering_content = hovering;
        if hovering {
            self.cancel_tasks();
        } else if !self.hovering_trigger && !self.input_focused {
            self.schedule_close(cx);
        }
    }

    pub(super) fn set_input_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.input_focused = focused;
        if focused {
            self.cancel_tasks();
        } else if !self.hovering_trigger && !self.hovering_content {
            self.schedule_close(cx);
        }
    }

    pub(super) fn schedule_open(&mut self, cx: &mut Context<Self>) {
        self.cancel_tasks();
        if self.open {
            cx.notify();
            return;
        }
        let epoch = self.next_epoch();
        self.open_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(VARIABLE_TOOLTIP_OPEN_DELAY)
                .await;
            let _ = this.update(cx, |state, cx| {
                if state.epoch == epoch {
                    state.open = true;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn schedule_close(&mut self, cx: &mut Context<Self>) {
        self.cancel_tasks();
        let epoch = self.next_epoch();
        self.close_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(VARIABLE_TOOLTIP_CLOSE_DELAY)
                .await;
            let _ = this.update(cx, |state, cx| {
                if state.epoch == epoch
                    && !state.hovering_trigger
                    && !state.hovering_content
                    && !state.input_focused
                {
                    state.open = false;
                    state.active = None;
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.cancel_tasks();
        self.open = false;
        self.active = None;
        self.hovering_trigger = false;
        self.hovering_content = false;
        self.input_focused = false;
        cx.notify();
    }

    pub(super) fn cancel_tasks(&mut self) {
        self.epoch += 1;
        self.open_task = None;
        self.close_task = None;
    }

    pub(super) fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }
}

impl Render for VariableHoverState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

pub(super) fn variable_tooltip_popup(
    theme: Theme,
    name: String,
    presentation: VariableTooltipPresentation,
    hover: Entity<VariableHoverState>,
    value_input: Entity<InputState>,
    on_manage_environments: Option<ManageEnvironmentsHandler>,
) -> impl IntoElement {
    let hover_for_content = hover.clone();
    div()
        .id("variable-input-tooltip-popup")
        .debug_selector(|| "variable-input-tooltip-popup".into())
        .w(px(280.0))
        .max_w(px(360.0))
        .px(px(theme.metrics.spacing_3))
        .py(px(theme.metrics.spacing_2))
        .flex()
        .flex_col()
        .gap(px(theme.metrics.spacing_2))
        .rounded(px(theme.metrics.radius_medium))
        .bg(theme.colors.surfaces.overlay)
        .border_1()
        .border_color(theme.colors.borders.standard)
        .occlude()
        .on_mouse_down(MouseButton::Left, {
            let value_input = value_input.clone();
            move |_, window, cx| {
                cx.stop_propagation();
                value_input.update(cx, |input, cx| input.focus(window, cx));
            }
        })
        .on_hover(move |hovered, _, cx| {
            hover_for_content.update(cx, |state, cx| state.on_content_hover(*hovered, cx));
        })
        .child(
            truncated_label(format!("{{{{{name}}}}}"))
                .flex_none()
                .font_family(theme.typography.monospace_family)
                .text_size(px(theme.typography.caption_size))
                .text_color(theme.colors.syntax.string),
        )
        .when_some(presentation.hint, |popup, hint| {
            popup.child(
                truncated_label(hint)
                    .id("variable-tooltip-create-hint")
                    .debug_selector(|| "variable-tooltip-create-hint".into())
                    .flex_none()
                    .text_size(px(theme.typography.caption_size))
                    .text_color(theme.colors.text.muted),
            )
        })
        .child(variable_value_input(
            theme,
            format!("variable-tooltip-value-{name}"),
            value_input,
            presentation.value,
            presentation.placeholder,
            presentation.editable,
        ))
        .when_some(on_manage_environments, |popup, on_manage| {
            let hover = hover.clone();
            popup.child(
                text_button(
                    theme,
                    "variable-tooltip-manage-environments",
                    "Manage environments…",
                    move |_, window, cx| {
                        hover.update(cx, |state, cx| state.dismiss(cx));
                        on_manage(window, cx);
                    },
                )
                .on_mouse_down(MouseButton::Left, |_, _, cx| {
                    cx.stop_propagation();
                }),
            )
        })
}

fn variable_value_input(
    theme: Theme,
    id: impl Into<ElementId>,
    state: Entity<InputState>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    editable: bool,
) -> gpui::AnyElement {
    let mut input = text_input_base(theme, id, value, placeholder);
    input.font_family = theme.typography.monospace_family;
    input.text_size = theme.typography.caption_size;
    input.debug_selector = Some("variable-tooltip-value-input");
    input.readonly = !editable;
    input.shared_input = Some(state);
    input.into_any_element()
}

pub(super) type InputChangeHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
pub(super) type TextContextActionHandler = Rc<dyn Fn(Option<String>, usize, &mut Window, &mut App)>;
pub(super) type TextContextEnableHandler = Rc<dyn Fn(Option<&str>, usize) -> bool>;
pub(super) type DropdownChangeHandler<T> = Rc<dyn Fn(Option<&T>, &mut Window, &mut App)>;
pub(super) type DropdownActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;
pub(super) type EditorMouseDownHandler = Rc<dyn Fn(&mut Window, &mut App)>;
