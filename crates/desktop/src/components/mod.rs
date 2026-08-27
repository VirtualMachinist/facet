//! Probe-styled compositions over headless Longbridge gpui-base behavior.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    rc::Rc,
    sync::{Arc, LazyLock},
    time::Duration,
};

use crate::response_viewer::{SearchMatch, join_header_lines};
use crate::shell::PaneLayout;
use crate::theme::Theme;
use gpui::{
    Anchor, App, AppContext as _, Bounds, BoxShadow, ClickEvent, ContentMask, Context, Edges,
    Element, ElementId, Entity, EntityId, FocusHandle, Focusable, FontWeight, GlobalElementId,
    HighlightStyle, Hsla, InspectorElementId, InteractiveElement as _, IntoElement, LayoutId,
    MouseButton, ParentElement as _, Pixels, Point, Render, RenderOnce, Role, ShapedLine,
    SharedString, StatefulInteractiveElement as _, Style, Styled as _, Subscription, Task,
    TextAlign, TextRun, TransformationMatrix, Window, canvas, deferred, div, fill, font, point,
    prelude::FluentBuilder as _, px, relative, size, transparent_black,
};
use gpui_base::{
    Align, Button, Editor, ElementExt as _, FocusTrapElement as _, Input, InputBase,
    POPUP_PRIORITY, Placement, Popup, Positioner, Select, Switch, SwitchThumb, SwitchTrack, Toggle,
    ToggleGroup,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    input::{
        Copy, Cut, EditorState, Escape, InputContextMenuCapabilities, InputEditorStyle, InputEvent,
        InputState, Paste, Search, SelectAll, TextDecoration, TextDecorationCollection,
    },
};
use probe_core::path_variable_ranges;

mod buttons;
mod icons;
mod menus;
pub(crate) use buttons::{
    DialogActionStyle, dialog_action_button, dialog_choice_button, primary_button,
    secondary_button, secondary_menu_trigger,
};
use icons::{CHECK_SVG, CHEVRON_RIGHT_SVG, SEARCH_SVG, folder_open_icon, library_icon};
pub(crate) use icons::{
    add_menu_button, chevron_icon, close_icon, home_button, hover_fill, locate_icon, plus_icon,
    save_icon, sidebar_toggle, trash_icon, tree_folder_icon,
};
use menus::{MenuButtonStyle, context_menu_separator, menu_button_with_style};
pub(crate) use menus::{
    app_menu_trigger, cascading_menu, checked_menu_button, destructive_menu_button,
    import_submenu_menu_button, menu_button, menu_separator, pane_layout_toggle,
    positioned_cascading_menu, shortcut_label_for_action, shortcut_label_for_action_in_context,
    switch,
};

/// Fixed width for compact primary actions such as Send.
pub(crate) const COMPACT_ACTION_BUTTON_WIDTH: f32 = 72.0;
pub(crate) const COMPACT_DIALOG_WIDTH: f32 = 420.0;
pub(crate) const WIDE_DIALOG_WIDTH: f32 = 520.0;

/// Single-line label that shows an ellipsis when the available width is too small.
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

fn temporary_surface_shadow(theme: Theme, y_offset: f32) -> Vec<BoxShadow> {
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
struct TextContextMenuState {
    position: Option<Point<Pixels>>,
    capabilities: InputContextMenuCapabilities,
    target_focus: Option<FocusHandle>,
    target_editor: Option<Entity<EditorState>>,
}

fn text_context_menu_id(id: &ElementId, child: &'static str) -> ElementId {
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

fn clipboard_has_pasteable_text(cx: &App) -> bool {
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
fn with_text_context_menu(
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

#[derive(Clone, Default)]
pub(crate) struct VariableContext {
    pub(crate) values: BTreeMap<String, String>,
    pub(crate) secrets: BTreeSet<String>,
    pub(crate) unavailable_message: String,
    pub(crate) on_change: Option<VariableChangeHandler>,
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

struct VariableHoverState {
    open: bool,
    active: Option<(usize, String)>,
    trigger_bounds: Bounds<Pixels>,
    visible_width: Option<Pixels>,
    overlay_origin: Option<Point<Pixels>>,
    hovering_trigger: bool,
    hovering_content: bool,
    input_focused: bool,
    epoch: usize,
    open_task: Option<Task<()>>,
    close_task: Option<Task<()>>,
    value_input: Entity<InputState>,
    on_value_change: RefCell<Option<VariableChangeHandler>>,
    _input_subscription: Subscription,
}

impl VariableHoverState {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
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

    fn on_input_event(
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

    fn on_trigger_hover(
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

    fn on_content_hover(&mut self, hovering: bool, cx: &mut Context<Self>) {
        self.hovering_content = hovering;
        if hovering {
            self.cancel_tasks();
        } else if !self.hovering_trigger && !self.input_focused {
            self.schedule_close(cx);
        }
    }

    fn set_input_focused(&mut self, focused: bool, cx: &mut Context<Self>) {
        self.input_focused = focused;
        if focused {
            self.cancel_tasks();
        } else if !self.hovering_trigger && !self.hovering_content {
            self.schedule_close(cx);
        }
    }

    fn schedule_open(&mut self, cx: &mut Context<Self>) {
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

    fn schedule_close(&mut self, cx: &mut Context<Self>) {
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

    fn cancel_tasks(&mut self) {
        self.epoch += 1;
        self.open_task = None;
        self.close_task = None;
    }

    fn next_epoch(&mut self) -> usize {
        self.epoch += 1;
        self.epoch
    }
}

impl Render for VariableHoverState {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

fn variable_tooltip_popup(
    theme: Theme,
    name: String,
    presentation: VariableTooltipPresentation,
    hover: Entity<VariableHoverState>,
    value_input: Entity<InputState>,
) -> impl IntoElement {
    let hover_for_content = hover;
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

type InputChangeHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
type TextContextActionHandler = Rc<dyn Fn(Option<String>, usize, &mut Window, &mut App)>;
type TextContextEnableHandler = Rc<dyn Fn(Option<&str>, usize) -> bool>;
type DropdownChangeHandler<T> = Rc<dyn Fn(Option<&T>, &mut Window, &mut App)>;
type DropdownActionHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type EditorMouseDownHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Copy)]
struct EditorInsets {
    top: f32,
    right: f32,
    bottom: f32,
    left: f32,
}

impl EditorInsets {
    fn standard(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: theme.metrics.spacing_2,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_2,
        }
    }

    fn response(theme: Theme) -> Self {
        Self {
            top: theme.metrics.spacing_2,
            right: 2.0,
            bottom: theme.metrics.spacing_2,
            left: theme.metrics.spacing_1,
        }
    }

    fn edges(self) -> Edges<Pixels> {
        Edges {
            top: px(self.top),
            right: px(self.right),
            bottom: px(self.bottom),
            left: px(self.left),
        }
    }
}

#[derive(Clone)]
struct TextContextMenuExtraAction {
    id: &'static str,
    label: &'static str,
    requires_selection: bool,
    is_enabled: TextContextEnableHandler,
    on_click: TextContextActionHandler,
}

pub(crate) struct ResponseBodyInputOptions<'a> {
    matches: &'a [SearchMatch],
    active_match: usize,
    inspection_reveal: Option<(Range<usize>, bool)>,
    language: SharedString,
    soft_wrap: bool,
    on_visible_range: VisibleRangeHandler,
    on_mouse_down: EditorMouseDownHandler,
    inspect_enabled: TextContextEnableHandler,
    on_inspect: TextContextActionHandler,
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

type VisibleRangeHandler = Rc<dyn Fn(Range<usize>, &mut App)>;
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
    theme: Theme,
    id: ElementId,
    value: SharedString,
    placeholder: SharedString,
    variables: VariableContext,
    highlight_path_variables: bool,
    variable_overlay: bool,
    font_family: &'static str,
    text_size: f32,
    height: f32,
    width: Option<f32>,
    debug_selector: Option<&'static str>,
    on_change: Option<InputChangeHandler>,
    on_enter: Option<InputChangeHandler>,
    on_focus: Option<FocusChangeHandler>,
    autofocus: bool,
    readonly: bool,
    shared_input: Option<Entity<InputState>>,
    flat: bool,
    leading_icon: Option<gpui::Div>,
    content_gap: f32,
    quiet_focus: bool,
    focus_on_render: bool,
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

fn text_input_base(
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

fn editor_value_needs_refresh(
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
fn editor_paint_style(theme: Theme) -> InputEditorStyle {
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
struct ProbeEditor {
    theme: Theme,
    id: ElementId,
    value: SharedString,
    placeholder: SharedString,
    decorations: Vec<TextDecoration>,
    language: SharedString,
    readonly: bool,
    min_height: Option<f32>,
    padding: EditorInsets,
    soft_wrap: bool,
    text_color: gpui::Rgba,
    scroll_to_range: Option<Range<usize>>,
    search_matches: Vec<(Range<usize>, bool)>,
    on_change: Option<InputChangeHandler>,
    on_mouse_down: Option<EditorMouseDownHandler>,
    on_visible_range: Option<VisibleRangeHandler>,
    extra_context_menu_actions: Vec<TextContextMenuExtraAction>,
    debug_selector: Option<&'static str>,
    variables: Option<VariableContext>,
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

fn search_match_bounds(
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

fn search_fallback_char_size(
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

fn search_match_char_ranges(text: &str, range: Range<usize>) -> Vec<Range<usize>> {
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

fn normalize_search_char_bounds(
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

fn body_text_highlights(theme: Theme, variables: &[(Range<usize>, String)]) -> Vec<TextDecoration> {
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
fn variable_input_overlay(
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
                    )),
            )
            .with_priority(POPUP_PRIORITY + 1),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn variable_span_layout(
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

struct VariableHighlightElement {
    state: Entity<InputState>,
    base_color: Hsla,
    highlight_color: Hsla,
    highlight_path_variables: bool,
}

struct VariableHighlightPrepaintState {
    line: Option<ShapedLine>,
    scroll_offset: Pixels,
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
fn variable_highlight_runs(
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
fn input_text_scroll_offset(
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

fn variable_tooltip_presentation(
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

struct VariableTooltipPresentation {
    value: String,
    placeholder: &'static str,
    editable: bool,
    hint: Option<&'static str>,
}

fn variable_ranges(value: &str) -> Vec<(Range<usize>, String)> {
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

fn input_variable_ranges(
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

#[cfg(test)]
mod components_tests;
