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
    App, AppContext as _, Bounds, BoxShadow, ClickEvent, ContentMask, Context, Element, ElementId,
    Entity, EntityId, FocusHandle, Focusable, FontWeight, GlobalElementId, HighlightStyle, Hsla,
    InspectorElementId, InteractiveElement as _, IntoElement, LayoutId, MouseButton,
    ParentElement as _, Pixels, Point, Render, RenderOnce, Role, ShapedLine, SharedString,
    StatefulInteractiveElement as _, Style, Styled as _, Subscription, Task, TextAlign, TextRun,
    TransformationMatrix, Window, canvas, deferred, div, font, point, prelude::FluentBuilder as _,
    px, relative, size, transparent_black,
};
use gpui_base::{
    Align, Button, Editor, ElementExt as _, Input, InputBase, POPUP_PRIORITY, Placement, Popup,
    Positioner, Select, Switch, SwitchThumb, SwitchTrack, Toggle, ToggleGroup,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
    input::{
        EditorState, InputEditorStyle, InputEvent, InputState, TextDecoration,
        TextDecorationCollection,
    },
};
use probe_core::path_variable_ranges;

/// Single-line label that shows an ellipsis when the available width is too small.
pub(crate) fn truncated_label(text: impl Into<String>) -> gpui::Div {
    div().min_w(px(0.0)).truncate().child(text.into())
}

static CHEVRON_DOWN_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuChevronDown));
static CHEVRON_UP_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuChevronUp));
static FOLDER_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuFolder));
static FOLDER_OPEN_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuFolderOpen));
static PLUS_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuPlus));
static SAVE_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuSave));
static CLOSE_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuX));
static TRASH_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuTrash2));
static SIDEBAR_COLLAPSE_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuPanelLeftClose));
static SIDEBAR_EXPAND_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuPanelLeftOpen));
static HOME_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuHouse));

fn icon_svg_bytes(icon: icondata::Icon) -> Vec<u8> {
    format!(
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="{}" fill="{}" "#,
            r#"stroke="{}" stroke-width="{}" stroke-linecap="{}" stroke-linejoin="{}">"#,
            "{}",
            "</svg>"
        ),
        icon.view_box.unwrap_or("0 0 24 24"),
        icon.fill.unwrap_or("none"),
        icon.stroke.unwrap_or("currentColor"),
        icon.stroke_width.unwrap_or("2"),
        icon.stroke_linecap.unwrap_or("round"),
        icon.stroke_linejoin.unwrap_or("round"),
        icon.data,
    )
    .into_bytes()
}

fn library_icon(cache_key: &'static str, data: &'static LazyLock<Vec<u8>>, size: f32) -> gpui::Div {
    let size = px(size);
    div()
        .flex_none()
        .size(size)
        .flex()
        .items_center()
        .justify_center()
        .child(
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    let _ = window.paint_svg(
                        bounds,
                        SharedString::from(cache_key),
                        Some(data.as_slice()),
                        TransformationMatrix::default(),
                        window.text_style().color,
                        cx,
                    );
                },
            )
            .size(size),
        )
}

pub(crate) fn chevron_icon(theme: Theme, expanded: bool) -> gpui::Div {
    let icon = if expanded {
        library_icon(
            "lucide-chevron-up",
            &CHEVRON_UP_SVG,
            theme.metrics.icon_small,
        )
    } else {
        library_icon(
            "lucide-chevron-down",
            &CHEVRON_DOWN_SVG,
            theme.metrics.icon_small,
        )
    };
    icon.text_color(theme.colors.text.muted)
}

fn tree_item_icon_color(theme: Theme, selected: bool) -> gpui::Rgba {
    if selected {
        theme.colors.selection.active_foreground
    } else {
        theme.colors.text.muted
    }
}

pub(crate) fn tree_folder_icon(theme: Theme, expanded: bool, selected: bool) -> gpui::Div {
    let icon = if expanded {
        library_icon(
            "lucide-folder-open",
            &FOLDER_OPEN_SVG,
            theme.metrics.icon_standard,
        )
    } else {
        library_icon("lucide-folder", &FOLDER_SVG, theme.metrics.icon_standard)
    };
    icon.text_color(tree_item_icon_color(theme, selected))
}

fn plus_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-plus", &PLUS_SVG, theme.metrics.icon_small)
}

pub(crate) fn add_menu_button(theme: Theme, open: bool, enabled: bool) -> Button {
    let disabled_background = theme.colors.actions.disabled;
    let disabled_border = theme.colors.actions.disabled;
    let disabled_foreground = theme.colors.actions.disabled_foreground;
    let mut button = Button::new("tree-add-menu-trigger")
        .accessibility_label("Add request or folder")
        .debug_selector(|| "tree-add-menu-trigger".into())
        .selected(open && enabled)
        .disabled(!enabled)
        .size(px(theme.metrics.control_height - 4.0))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .border_1()
        .border_color(if enabled {
            theme.colors.borders.subtle
        } else {
            disabled_border
        })
        .when(!enabled, |button| button.bg(disabled_background));

    if enabled {
        button = button
            .hover(move |button| button.bg(theme.colors.surfaces.window))
            .focus(move |button| button.border_color(theme.colors.borders.focused))
            .styles(move |styles| {
                styles.selected(move |button| button.bg(theme.colors.surfaces.window))
            });
    }

    button.child(plus_icon(theme).text_color(if enabled {
        theme.colors.text.secondary
    } else {
        disabled_foreground
    }))
}

pub(crate) fn save_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-save", &SAVE_SVG, theme.metrics.icon_standard)
        .text_color(theme.colors.text.primary)
}

pub(crate) fn close_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-x", &CLOSE_SVG, theme.metrics.icon_standard)
        .text_color(theme.colors.text.secondary)
}

fn sidebar_icon(theme: Theme, collapsed: bool) -> gpui::Div {
    if collapsed {
        library_icon(
            "lucide-panel-left-open",
            &SIDEBAR_EXPAND_SVG,
            theme.metrics.icon_standard,
        )
    } else {
        library_icon(
            "lucide-panel-left-close",
            &SIDEBAR_COLLAPSE_SVG,
            theme.metrics.icon_standard,
        )
    }
    .text_color(theme.colors.text.secondary)
}

pub(crate) fn sidebar_toggle(
    theme: Theme,
    collapsed: bool,
    on_toggle: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = if collapsed {
        "Show sidebar"
    } else {
        "Hide sidebar"
    };
    Button::new("sidebar-toggle")
        .accessibility_label(label)
        .debug_selector(|| "sidebar-toggle".into())
        .w(px(theme.metrics.control_height))
        .h(px(theme.metrics.control_height))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .hover(move |button| button.bg(theme.colors.surfaces.sidebar))
        .focus(move |button| button.bg(theme.colors.surfaces.sidebar))
        .child(sidebar_icon(theme, collapsed))
        .on_click(move |_, window, cx| on_toggle(window, cx))
}

pub(crate) fn home_button(
    theme: Theme,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let disabled_background = theme.colors.actions.disabled;
    let disabled_foreground = theme.colors.actions.disabled_foreground;
    let mut button = Button::new("home-button")
        .accessibility_label("Close collection")
        .debug_selector(|| "home-button".into())
        .disabled(!enabled)
        .w(px(theme.metrics.control_height))
        .h(px(theme.metrics.control_height))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .when(!enabled, |button| button.bg(disabled_background));

    if enabled {
        button = button
            .hover(move |button| button.bg(theme.colors.surfaces.sidebar))
            .focus(move |button| button.bg(theme.colors.surfaces.sidebar));
    }

    button
        .child(
            library_icon("lucide-house", &HOME_SVG, theme.metrics.icon_standard).text_color(
                if enabled {
                    theme.colors.text.secondary
                } else {
                    disabled_foreground
                },
            ),
        )
        .on_click(move |_, window, cx| on_click(window, cx))
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
    ProbeTextInput {
        theme,
        id: id.into(),
        value: single_line(value),
        placeholder: placeholder.into(),
        variables: VariableContext::default(),
        highlight_path_variables: false,
        variable_overlay: false,
        font_family: theme.typography.monospace_family,
        text_size: theme.typography.caption_size,
        height: theme.metrics.control_height,
        width: None,
        debug_selector: Some("variable-tooltip-value-input"),
        on_change: None,
        on_enter: None,
        autofocus: false,
        readonly: !editable,
        on_focus: None,
        shared_input: Some(state),
    }
    .into_any_element()
}

fn focus_ring_shadow(ring_color: Hsla, gap_color: Hsla) -> Vec<BoxShadow> {
    vec![
        BoxShadow::new(px(0.0), px(0.0), ring_color).spread_radius(px(2.0)),
        BoxShadow::new(px(0.0), px(0.0), gap_color).spread_radius(px(0.5)),
    ]
}

pub(crate) fn primary_button(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    Button::new(id)
        .debug_selector(|| id.into())
        .h(px(theme.metrics.control_height))
        .min_w(px(72.0))
        .px(px(theme.metrics.spacing_3))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .font_family(theme.typography.interface_family)
        .text_size(px(theme.typography.body_size))
        .text_color(theme.colors.text.inverse)
        .bg(theme.colors.actions.accent)
        .border_1()
        .border_color(theme.colors.actions.accent)
        .cursor_pointer()
        .hover(move |button| {
            button
                .bg(theme.colors.actions.hover)
                .border_color(theme.colors.actions.hover)
        })
        .focus(move |button| {
            button.shadow(focus_ring_shadow(
                theme.colors.actions.accent.into(),
                theme.colors.text.inverse.into(),
            ))
        })
        .styles(move |styles| {
            styles.disabled(move |button| {
                button
                    .bg(theme.colors.actions.disabled)
                    .text_color(theme.colors.actions.disabled_foreground)
                    .border_color(theme.colors.actions.disabled)
            })
        })
        .on_click(on_click)
        .child(label)
}

pub(crate) fn secondary_button(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    Button::new(id)
        .debug_selector(|| id.into())
        .h(px(theme.metrics.control_height))
        .min_w(px(72.0))
        .px(px(theme.metrics.spacing_3))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .font_family(theme.typography.interface_family)
        .text_size(px(theme.typography.body_size))
        .text_color(theme.colors.text.primary)
        .bg(theme.colors.surfaces.raised)
        .border_1()
        .border_color(theme.colors.borders.standard)
        .cursor_pointer()
        .hover(move |button| button.bg(theme.colors.surfaces.window))
        .focus(move |button| {
            button
                .border_color(theme.colors.borders.focused)
                .shadow(focus_ring_shadow(
                    theme.colors.borders.focused.into(),
                    theme.colors.surfaces.raised.into(),
                ))
        })
        .styles(move |styles| {
            styles.disabled(move |button| {
                button
                    .bg(theme.colors.actions.disabled)
                    .text_color(theme.colors.actions.disabled_foreground)
                    .border_color(theme.colors.actions.disabled)
            })
        })
        .on_click(on_click)
        .child(label)
}

type InputChangeHandler = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;
type DropdownChangeHandler<T> = Rc<dyn Fn(Option<&T>, &mut Window, &mut App)>;

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
struct ProbeTextInput {
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
}

impl RenderOnce for ProbeTextInput {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
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
            field.read(cx).state.clone()
        };
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        state.update(cx, |input, cx| {
            input.set_editor_style(editor_paint_style(self.theme));
            input.set_readonly(self.readonly, cx);
            input.set_placeholder(placeholder, window, cx);
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
            .bg(theme.colors.surfaces.raised)
            .border_1()
            .border_color(if focused {
                theme.colors.borders.focused
            } else {
                theme.colors.borders.standard
            })
            .focused(focused)
            .styles(move |styles| {
                styles.focused(move |input| input.border_color(theme.colors.borders.focused))
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
            .child(Input::new(&state));
        if !self.variable_overlay {
            return input.into_any_element();
        }
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
    ProbeTextInput {
        theme,
        id: id.into(),
        value: single_line(value),
        placeholder: placeholder.into(),
        variables,
        highlight_path_variables: false,
        variable_overlay: true,
        font_family: theme.typography.monospace_family,
        text_size: theme.typography.body_size,
        height: theme.metrics.control_height,
        width: None,
        debug_selector: None,
        on_change: Some(Rc::new(on_value_change)),
        on_enter: None,
        on_focus: None,
        autofocus: false,
        readonly: false,
        shared_input: None,
    }
    .into_any_element()
}

pub(crate) fn url_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    variables: VariableContext,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
    ProbeTextInput {
        theme,
        id: id.into(),
        value: single_line(value),
        placeholder: placeholder.into(),
        variables,
        highlight_path_variables: true,
        variable_overlay: true,
        font_family: theme.typography.monospace_family,
        text_size: theme.typography.body_size,
        height: theme.metrics.control_height,
        width: None,
        debug_selector: None,
        on_change: Some(Rc::new(on_value_change)),
        on_enter: None,
        on_focus: None,
        autofocus: false,
        readonly: false,
        shared_input: None,
    }
    .into_any_element()
}

pub(crate) fn dialog_text_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    autofocus: bool,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    on_enter: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> gpui::AnyElement {
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
        on_change: Some(Rc::new(on_value_change)),
        on_enter: Some(Rc::new(on_enter)),
        on_focus: None,
        autofocus,
        readonly: false,
        shared_input: None,
    }
    .into_any_element()
}

pub(crate) fn sidebar_search_input(
    theme: Theme,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    ProbeTextInput {
        theme,
        id: "tree-search-input".into(),
        value: single_line(value),
        placeholder: placeholder.into(),
        variables: VariableContext::default(),
        highlight_path_variables: false,
        variable_overlay: false,
        font_family: theme.typography.interface_family,
        text_size: theme.typography.caption_size,
        height: theme.metrics.control_height - 4.0,
        width: None,
        debug_selector: Some("tree-search"),
        on_change: Some(Rc::new(on_value_change)),
        on_enter: None,
        on_focus: None,
        autofocus: false,
        readonly: false,
        shared_input: None,
    }
}

pub(crate) fn search_input(
    theme: Theme,
    id: impl Into<ElementId>,
    value: impl Into<SharedString>,
    placeholder: impl Into<SharedString>,
    on_value_change: impl Fn(SharedString, &mut Window, &mut App) + 'static,
    on_enter: impl Fn(SharedString, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
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
        height: theme.metrics.control_height - 2.0,
        width: Some(180.0),
        debug_selector: Some("response-search"),
        on_change: Some(Rc::new(on_value_change)),
        on_enter: Some(Rc::new(on_enter)),
        on_focus: None,
        autofocus: false,
        readonly: false,
        shared_input: None,
    }
}

fn editor_button_base(
    theme: Theme,
    id: impl Into<ElementId>,
    selected: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
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

pub(crate) fn editor_add_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
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
    Button::new(id)
        .role(Role::Tab)
        .selected(selected)
        .aria_selected(selected)
        .aria_position_in_set(position)
        .aria_size_of_set(size)
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
        .focus(move |tab| tab.border_color(theme.colors.borders.focused))
        .cursor_pointer()
        .on_click(on_click)
        .child(label)
}

pub(crate) fn remove_row_button(
    theme: Theme,
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = theme.colors.text.secondary;
    Button::new(id)
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
        .child(trash_icon(color))
}

pub(crate) fn browse_file_button(
    theme: Theme,
    id: impl Into<ElementId>,
    aria_label: impl Into<SharedString>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let color = theme.colors.text.secondary;
    Button::new(id)
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
        .child(folder_open_icon(color))
}

fn folder_open_icon(color: gpui::Rgba) -> gpui::Div {
    library_icon("lucide-folder-open", &FOLDER_OPEN_SVG, 14.0).text_color(color)
}

fn trash_icon(color: gpui::Rgba) -> gpui::Div {
    library_icon("lucide-trash-2", &TRASH_SVG, 14.0).text_color(color)
}

pub(crate) fn dropdown<T: Clone + Eq + 'static>(
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String)>,
    width: f32,
    on_value_change: impl Fn(Option<&T>, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    dropdown_with_option_colors(
        theme,
        id,
        aria_label,
        value,
        options
            .into_iter()
            .map(|(value, label)| (value, label, None))
            .collect(),
        width,
        on_value_change,
    )
}

pub(crate) fn dropdown_with_option_colors<T: Clone + Eq + 'static>(
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String, Option<gpui::Rgba>)>,
    width: f32,
    on_value_change: impl Fn(Option<&T>, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    ProbeDropdown {
        theme,
        id,
        aria_label,
        value,
        options,
        width,
        on_value_change: Rc::new(on_value_change),
    }
}

#[derive(IntoElement)]
struct ProbeDropdown<T: Clone + Eq + 'static> {
    theme: Theme,
    id: &'static str,
    aria_label: &'static str,
    value: Option<T>,
    options: Vec<(T, String, Option<gpui::Rgba>)>,
    width: f32,
    on_value_change: DropdownChangeHandler<T>,
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
            if state.highlighted >= self.options.len() {
                state.highlighted = selected_index;
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
        let on_value_change = self.on_value_change.clone();
        let selected_value = self.value;
        let options = self.options;
        let list = div()
            .id(format!("{id}-list"))
            .track_focus(&list_focus)
            .role(Role::ListBox)
            .key_context("Select")
            .on_action({
                let controller = controller.clone();
                move |_: &SelectDown, _, cx| controller.move_highlight(1, option_count, cx)
            })
            .on_action({
                let controller = controller.clone();
                move |_: &SelectUp, _, cx| controller.move_highlight(-1, option_count, cx)
            })
            .on_action({
                let controller = controller.clone();
                let on_value_change = on_value_change.clone();
                let options = options.clone();
                move |_: &Confirm, window, cx| {
                    let index = controller.highlighted(cx);
                    if let Some((value, _, _)) = options.get(index) {
                        on_value_change(Some(value), window, cx);
                        controller.close_and_restore_trigger(window, cx);
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
            .text_color(selected_color)
            .debug_selector(move || format!("{id}-trigger"))
            .hover(move |trigger| trigger.bg(theme.colors.selection.inactive_background))
            .focus(move |trigger| trigger.border_color(theme.colors.borders.focused))
            .on_click({
                let controller = controller.clone();
                let list_focus = list_focus.clone();
                move |_, window, cx| controller.toggle_open(&list_focus, window, cx)
            })
            .child(truncated_label(selected_label).min_w(px(0.0)).flex_1())
            .child(chevron_icon(theme, open));

        let select_root = Select::new(format!("{id}-select"))
            .open(open)
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
        padded: true,
        soft_wrap: true,
        text_color: theme.colors.text.primary,
        scroll_to_offset: None,
        on_change: Some(Rc::new(on_value_change)),
        on_visible_range: None,
        debug_selector: None,
        variables: Some(variables),
    }
    .into_any_element()
}

pub(crate) fn response_body_input(
    theme: Theme,
    id: impl Into<ElementId>,
    text: &str,
    matches: &[SearchMatch],
    active_match: usize,
    language: impl Into<SharedString>,
    on_visible_range: impl Fn(Range<usize>, &mut App) + 'static,
) -> gpui::AnyElement {
    let mut decorations = Vec::new();
    let mut scroll_to_offset = None;
    for (index, found) in matches.iter().enumerate() {
        let active = index == active_match;
        if active {
            scroll_to_offset = Some(found.range.start);
        }
        decorations.push(search_match_decoration(theme, found.range.clone(), active));
    }
    ProbeEditor {
        theme,
        id: id.into(),
        value: text.to_owned().into(),
        placeholder: SharedString::default(),
        decorations,
        language: language.into(),
        readonly: true,
        min_height: None,
        padded: true,
        soft_wrap: false,
        text_color: theme.colors.syntax.plain,
        scroll_to_offset,
        on_change: None,
        on_visible_range: Some(Rc::new(on_visible_range)),
        debug_selector: None,
        variables: None,
    }
    .into_any_element()
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
    let mut scroll_to_offset = None;
    for (index, found) in matches.iter().enumerate() {
        let active = index == active_match;
        if active {
            scroll_to_offset = Some(found.range.start);
        }
        decorations.push(search_match_decoration(theme, found.range.clone(), active));
    }
    ProbeEditor {
        theme,
        id: id.into(),
        value: joined.text.into(),
        placeholder: SharedString::default(),
        decorations,
        language: SharedString::default(),
        readonly: true,
        min_height: None,
        padded: true,
        soft_wrap: false,
        text_color: theme.colors.text.primary,
        scroll_to_offset,
        on_change: None,
        on_visible_range: Some(Rc::new(on_visible_range)),
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
    last_scroll_offset: Option<usize>,
    language: SharedString,
    _subscription: Subscription,
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
    padded: bool,
    soft_wrap: bool,
    text_color: gpui::Rgba,
    scroll_to_offset: Option<usize>,
    on_change: Option<InputChangeHandler>,
    on_visible_range: Option<VisibleRangeHandler>,
    debug_selector: Option<&'static str>,
    variables: Option<VariableContext>,
}

impl RenderOnce for ProbeEditor {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let placeholder = self.placeholder.clone();
        let on_change = self.on_change.clone();
        let soft_wrap = self.soft_wrap;
        let language = self.language.clone();
        let field = window.use_keyed_state(self.id.clone(), cx, |window, cx| {
            let state = cx.new(|cx| {
                let mut editor = EditorState::new(window, cx)
                    .placeholder(placeholder.clone())
                    .folding(false)
                    .searchable(false)
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
                last_scroll_offset: None,
                language: language.clone(),
                _subscription: subscription,
            }
        });
        field.update(cx, |field, cx| {
            field.on_change = self.on_change.clone();
            let language_changed = field.language != self.language;
            if language_changed {
                field.language = self.language.clone();
            }
            field.state.update(cx, |editor, cx| {
                editor.set_editor_style(editor_paint_style(self.theme));
                editor.set_readonly(self.readonly, cx);
                if language_changed {
                    editor.set_highlighter(self.language.clone(), cx);
                }
                if editor.value() != self.value {
                    editor.set_value(self.value.clone(), window, cx);
                }
            });
            if field.last_decorations != self.decorations {
                field.last_decorations = self.decorations.clone();
                field.decorations.set(self.decorations.clone(), cx);
            }
            if self.scroll_to_offset != field.last_scroll_offset {
                if let Some(offset) = self.scroll_to_offset {
                    let laid_out = field.state.read(cx).visible_row_range().is_some();
                    field.state.update(cx, |editor, cx| {
                        editor.set_selected_range(offset..offset, cx);
                    });
                    if laid_out {
                        field.last_scroll_offset = self.scroll_to_offset;
                    }
                } else {
                    field.last_scroll_offset = None;
                }
            }
        });
        let state = field.read(cx).state.clone();
        if let Some(on_visible_range) = self.on_visible_range {
            match state.read(cx).visible_row_range() {
                Some(range) => on_visible_range(range, cx),
                None => window.request_animation_frame(),
            }
        }
        let focused = state.read(cx).focus_handle(cx).is_focused(window);
        let theme = self.theme;
        let editor_id = self.id.clone();
        let editor = InputBase::new(editor_id.clone())
            .size_full()
            .when_some(self.min_height, |editor, height| editor.min_h(px(height)))
            .when(self.padded, |editor| editor.p(px(theme.metrics.spacing_2)))
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
            .on_mouse_down(MouseButton::Left, {
                let state = state.clone();
                move |_, window, cx| {
                    state.update(cx, |editor, cx| editor.focus(window, cx));
                }
            })
            .child(div().size_full().child(Editor::new(&state)));
        let Some(variables) = self.variables else {
            return editor.into_any_element();
        };
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
        active.then(|| theme.colors.selection.active_foreground.into()),
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
            .with_priority(POPUP_PRIORITY),
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

pub(crate) fn menu_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    shortcut: Option<String>,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    menu_button_with_style(
        theme,
        id,
        label,
        shortcut,
        MenuButtonStyle {
            padding_x: theme.metrics.spacing_2,
            text_color: theme.colors.text.primary,
            shortcut_color: theme.colors.text.muted,
        },
        on_activate,
    )
}

pub(crate) fn destructive_menu_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    shortcut: Option<String>,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    menu_button_with_style(
        theme,
        id,
        label,
        shortcut,
        MenuButtonStyle {
            padding_x: theme.metrics.spacing_2,
            text_color: theme.colors.status.error,
            shortcut_color: theme.colors.status.error,
        },
        on_activate,
    )
}

#[derive(Clone, Copy)]
struct MenuButtonStyle {
    padding_x: f32,
    text_color: gpui::Rgba,
    shortcut_color: gpui::Rgba,
}

fn menu_button_with_style(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    shortcut: Option<String>,
    style: MenuButtonStyle,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let on_activate = Rc::new(on_activate);
    let pointer_activate = on_activate.clone();
    let keyboard_activate = on_activate;

    // A controlled popover can rerender after its button receives focus on
    // mouse-down. Activate before that rerender so the subsequent click is not
    // lost when the popup is unmounted. Keyboard activation still follows the
    // headless button's normal action path.
    gpui::div()
        .w_full()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            pointer_activate(window, cx);
        })
        .child(
            Button::new(id)
                .w_full()
                .h(px(theme.metrics.control_height + 4.0))
                .px(px(style.padding_x))
                .flex()
                .items_center()
                .justify_start()
                .overflow_hidden()
                .rounded(px(theme.metrics.radius_small))
                .font_family(theme.typography.interface_family)
                .text_size(px(theme.typography.body_size))
                .text_color(style.text_color)
                .cursor_pointer()
                .hover(move |button| button.bg(theme.colors.surfaces.sidebar))
                .focus(move |button| button.border_1().border_color(theme.colors.borders.focused))
                .on_click(move |event, window, cx| {
                    if !matches!(event, ClickEvent::Mouse(_)) {
                        keyboard_activate(window, cx);
                    }
                })
                .child(
                    div()
                        .w_full()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(truncated_label(label).flex_1())
                        .when_some(shortcut, |row, shortcut| {
                            row.child(
                                div()
                                    .flex_none()
                                    .text_size(px(theme.typography.caption_size))
                                    .text_color(style.shortcut_color)
                                    .child(shortcut),
                            )
                        }),
                ),
        )
}

pub(crate) fn switch(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    checked: bool,
    on_checked_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    Switch::new(id)
        .checked(checked)
        .accessibility_label(label)
        .w(px(36.0))
        .h(px(20.0))
        .flex()
        .items_center()
        .cursor_pointer()
        .on_change(move |value, _, window, cx| on_checked_change(value, window, cx))
        .child(
            SwitchTrack::new("switch-track")
                .checked(checked)
                .w(px(36.0))
                .h(px(20.0))
                .p(px(2.0))
                .flex()
                .items_center()
                .rounded(px(9999.0))
                .bg(if checked {
                    theme.colors.actions.accent
                } else {
                    theme.colors.borders.standard
                })
                .child(
                    SwitchThumb::new(checked)
                        .size(px(16.0))
                        .rounded(px(8.0))
                        .bg(theme.colors.text.inverse)
                        .ml(if checked { px(16.0) } else { px(0.0) }),
                ),
        )
}

pub(crate) fn pane_layout_toggle(
    theme: Theme,
    layout: PaneLayout,
    on_change: impl Fn(PaneLayout, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let on_change = Rc::new(on_change);
    let item = |index: usize, label: &'static str, item_layout: PaneLayout| {
        let pressed = layout == item_layout;
        let color = if pressed {
            theme.colors.selection.active_foreground
        } else {
            theme.colors.text.secondary
        };
        let on_change = on_change.clone();
        Toggle::new(("pane-layout", index))
            .pressed(pressed)
            .accessibility_label(label)
            .w(px(32.0))
            .h(px(theme.metrics.control_height))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(theme.metrics.radius_small))
            .text_color(color)
            .border_1()
            .border_color(transparent_black())
            .cursor_pointer()
            .when(pressed, |toggle| {
                toggle.bg(theme.colors.selection.active_background)
            })
            .when(!pressed, |toggle| {
                toggle.hover(move |toggle| toggle.bg(theme.colors.selection.inactive_background))
            })
            .focus(move |toggle| {
                toggle.shadow(focus_ring_shadow(
                    theme.colors.borders.focused.into(),
                    theme.colors.text.inverse.into(),
                ))
            })
            .on_change(move |next, _, window, cx| {
                if next {
                    on_change(item_layout, window, cx);
                }
            })
            .child(pane_layout_icon(item_layout, color))
    };

    ToggleGroup::new("pane-layout-toggle")
        .flex()
        .items_center()
        .gap(px(theme.metrics.spacing_1))
        .pr(px(theme.metrics.spacing_2))
        .child(item(
            0,
            "Stack response below request",
            PaneLayout::Vertical,
        ))
        .child(item(
            1,
            "Place response beside request",
            PaneLayout::Horizontal,
        ))
}

fn pane_layout_icon(layout: PaneLayout, color: gpui::Rgba) -> gpui::Div {
    let divider = match layout {
        PaneLayout::Vertical => div().w_full().h(px(1.0)).bg(color),
        PaneLayout::Horizontal => div().h_full().w(px(1.0)).bg(color),
    };
    div()
        .w(px(14.0))
        .h(px(12.0))
        .rounded(px(2.0))
        .border_1()
        .border_color(color)
        .overflow_hidden()
        .flex()
        .when(layout == PaneLayout::Vertical, |icon| icon.flex_col())
        .child(div().flex_1())
        .child(divider)
        .child(div().flex_1())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gpui::{
        AppContext as _, Context, Entity, IntoElement, Modifiers, Render, SharedString,
        TestAppContext, VisualTestContext, div, hsla, point, prelude::*, px, size,
        transparent_black,
    };
    use gpui_base::{Button, Input, InputBase, Popover, input::InputState};

    use super::{
        VariableContext, VariableHighlightElement, body_text_highlights, dropdown,
        editor_paint_style, input_text_scroll_offset, menu_button, single_line,
        variable_highlight_runs, variable_ranges, variable_span_layout,
        variable_tooltip_presentation,
    };
    use crate::theme::Theme;
    use probe_core::path_variable_ranges;

    struct MenuTestView {
        open: bool,
        activations: usize,
    }

    impl Render for MenuTestView {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            cx: &mut Context<Self>,
        ) -> impl IntoElement {
            let open_view = cx.weak_entity();
            let activate_view = cx.weak_entity();

            div().size_full().p(px(20.0)).child(
                Popover::new("menu-test-popover")
                    .open(self.open)
                    .on_open_change(move |open, _, cx| {
                        let _ = open_view.update(cx, |view, cx| {
                            view.open = *open;
                            cx.notify();
                        });
                    })
                    .trigger(
                        Button::new("menu-test-trigger")
                            .w(px(100.0))
                            .h(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .debug_selector(|| "menu-test-trigger".into())
                            .child("Open"),
                    )
                    .content(move |_, _, _| {
                        div()
                            .id("menu-test-popup")
                            .w(px(180.0))
                            .debug_selector(|| "menu-test-popup".into())
                            .child(menu_button(
                                Theme::light(),
                                "menu-test-item",
                                "Workspace",
                                None,
                                move |_, cx| {
                                    let _ = activate_view.update(cx, |view, cx| {
                                        view.activations += 1;
                                        view.open = false;
                                        cx.notify();
                                    });
                                },
                            ))
                    }),
            )
        }
    }

    #[gpui::test]
    fn controlled_popover_menu_item_activates_on_pointer_press(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let window = cx.open_window(size(px(320.0), px(180.0)), |_, _| MenuTestView {
            open: false,
            activations: 0,
        });
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let trigger = visual
            .debug_bounds("menu-test-trigger")
            .expect("trigger should be rendered");
        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.run_until_parked();
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let popup = visual
            .debug_bounds("menu-test-popup")
            .expect("popup should be rendered");
        visual.simulate_click(popup.center(), Modifiers::default());
        visual.run_until_parked();
        cx.run_until_parked();

        let (open, activations) = window
            .update(cx, |view, _, _| (view.open, view.activations))
            .expect("test window should remain open");
        assert!(!open);
        assert_eq!(activations, 1);
    }

    struct DropdownHoverLeakView {
        value: Option<&'static str>,
        underlay_hovered: bool,
    }

    impl Render for DropdownHoverLeakView {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            cx: &mut Context<Self>,
        ) -> impl IntoElement {
            let select_view = cx.weak_entity();
            let underlay_view = cx.weak_entity();

            div()
                .size_full()
                .p(px(12.0))
                .flex()
                .flex_col()
                .child(dropdown(
                    Theme::light(),
                    "hover-leak-select",
                    "Method",
                    self.value,
                    vec![
                        ("GET", "GET".to_owned()),
                        ("POST", "POST".to_owned()),
                        ("PUT", "PUT".to_owned()),
                        ("PATCH", "PATCH".to_owned()),
                        ("DELETE", "DELETE".to_owned()),
                    ],
                    120.0,
                    move |value, _, cx| {
                        let value = value.copied();
                        let _ = select_view.update(cx, |view, cx| {
                            view.value = value;
                            cx.notify();
                        });
                    },
                ))
                .child(
                    div()
                        .id("dropdown-underlay")
                        .flex_1()
                        .w_full()
                        .mt(px(8.0))
                        .debug_selector(|| "dropdown-underlay".into())
                        .hover(|underlay| underlay.bg(Theme::light().colors.surfaces.raised))
                        .on_hover(move |hovered, _, cx| {
                            let hovered = *hovered;
                            let _ = underlay_view.update(cx, |view, cx| {
                                view.underlay_hovered = hovered;
                                cx.notify();
                            });
                        })
                        .child("Underlay"),
                )
        }
    }

    #[gpui::test]
    fn dropdown_menu_does_not_hover_elements_underneath(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let window = cx.open_window(size(px(360.0), px(280.0)), |_, _| DropdownHoverLeakView {
            value: Some("GET"),
            underlay_hovered: false,
        });
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let underlay = visual
            .debug_bounds("dropdown-underlay")
            .expect("underlay should be rendered");
        visual.simulate_mouse_move(underlay.center(), None, Modifiers::default());
        visual.run_until_parked();
        cx.run_until_parked();
        let hovered = window
            .update(cx, |view, _, _| view.underlay_hovered)
            .expect("test window should remain open");
        assert!(hovered, "underlay should hover when the menu is closed");

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let trigger = visual
            .debug_bounds("hover-leak-select-trigger")
            .expect("select trigger should be rendered");
        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.run_until_parked();
        cx.run_until_parked();

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let item = visual
            .debug_bounds("hover-leak-select-item-3")
            .expect("select item over the underlay should be rendered");
        visual.simulate_mouse_move(item.center(), None, Modifiers::default());
        visual.run_until_parked();
        cx.run_until_parked();

        let hovered = window
            .update(cx, |view, _, _| view.underlay_hovered)
            .expect("test window should remain open");
        assert!(
            !hovered,
            "hovering a dropdown item should not hover the element underneath"
        );

        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_click(point(px(340.0), px(260.0)), Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual.debug_bounds("hover-leak-select-item-3").is_none(),
            "clicking outside should dismiss the dropdown"
        );
    }

    #[gpui::test]
    fn dropdown_opens_from_keyboard_focused_trigger(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let window = cx.open_window(size(px(360.0), px(280.0)), |_, _| DropdownHoverLeakView {
            value: Some("GET"),
            underlay_hovered: false,
        });
        cx.run_until_parked();

        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let trigger = visual
                .debug_bounds("hover-leak-select-trigger")
                .expect("select trigger should render");
            visual.simulate_click(trigger.center(), Modifiers::default());
            visual.run_until_parked();
        }
        cx.simulate_keystrokes(window.into(), "escape");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "down");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "down");
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), "enter");
        cx.run_until_parked();

        let value = window
            .update(cx, |view, _, _| view.value)
            .expect("test window should remain open");
        assert_eq!(value, Some("POST"));
    }

    #[gpui::test]
    fn dropdown_keyboard_navigation_selects_and_dismisses(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let window = cx.open_window(size(px(360.0), px(280.0)), |_, _| DropdownHoverLeakView {
            value: Some("GET"),
            underlay_hovered: false,
        });
        cx.run_until_parked();

        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            let trigger = visual
                .debug_bounds("hover-leak-select-trigger")
                .expect("select trigger should render");
            visual.simulate_click(trigger.center(), Modifiers::default());
            visual.run_until_parked();
        }

        cx.simulate_keystrokes(window.into(), "down enter");
        cx.run_until_parked();

        let value = window
            .update(cx, |view, _, _| view.value)
            .expect("test window should remain open");
        assert_eq!(value, Some("POST"));
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual.debug_bounds("hover-leak-select-item-1").is_none(),
            "keyboard selection should dismiss the dropdown"
        );

        cx.simulate_keystrokes(window.into(), "down");
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual.debug_bounds("hover-leak-select-item-1").is_some(),
            "trigger should stay focused so the next arrow key reopens the menu"
        );
    }

    #[test]
    fn editor_paint_style_uses_visible_caret_and_selection() {
        for theme in [Theme::light(), Theme::dark()] {
            let style = editor_paint_style(theme);
            assert!(
                style.caret.a > 0.0,
                "gpui-base default caret is transparent"
            );
            assert!(
                style.selection.a > 0.0,
                "gpui-base default selection is transparent"
            );
        }
    }

    #[test]
    fn single_line_replaces_line_breaks_with_spaces() {
        assert_eq!(single_line("abc"), SharedString::from("abc"));
        assert_eq!(single_line("a\nb\rc"), SharedString::from("a b c"));
        assert_eq!(
            single_line("{\n  \"name\": \"Milo\"\n}"),
            SharedString::from("{   \"name\": \"Milo\" }")
        );
    }

    #[test]
    fn variable_ranges_find_mustache_placeholders() {
        let value = "{{host}}/users/{{id}}";
        let ranges = variable_ranges(value);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&value[ranges[0].0.clone()], "{{host}}");
        assert_eq!(ranges[0].1, "host");
        assert_eq!(&value[ranges[1].0.clone()], "{{id}}");
        assert_eq!(ranges[1].1, "id");
    }

    #[test]
    fn variable_ranges_trim_names_and_find_placeholders_in_json() {
        let value = "{\n  \"tenant\": \"{{ tenant }}\"\n}";
        let ranges = variable_ranges(value);
        assert_eq!(ranges.len(), 1);
        assert_eq!(&value[ranges[0].0.clone()], "{{ tenant }}");
        assert_eq!(ranges[0].1, "tenant");
    }

    #[test]
    fn path_variable_ranges_only_highlight_colon_placeholders_in_the_url_path() {
        let value = "https://api.example.com:8443/users/:userId/posts/:post_id?next=:ignored";
        let ranges = path_variable_ranges(value);
        assert_eq!(ranges.len(), 2);
        assert_eq!(&value[ranges[0].0.clone()], ":userId");
        assert_eq!(ranges[0].1, "userId");
        assert_eq!(&value[ranges[1].0.clone()], ":post_id");
        assert_eq!(ranges[1].1, "post_id");
    }

    #[test]
    fn variable_tooltip_presentation_creates_missing_writable_variables() {
        let mut values = BTreeMap::new();
        values.insert("host".to_owned(), "api.example".to_owned());
        let variables = VariableContext {
            values,
            secrets: ["token".to_owned()].into_iter().collect(),
            unavailable_message: "unavailable".to_owned(),
            on_change: Some(std::rc::Rc::new(|_, _, _, _| {})),
        };
        let existing = variable_tooltip_presentation("host", &variables);
        assert_eq!(existing.value, "api.example");
        assert!(existing.editable);
        assert!(existing.hint.is_none());

        let missing = variable_tooltip_presentation("created", &variables);
        assert_eq!(missing.value, "");
        assert_eq!(missing.placeholder, "Enter a value to create");
        assert!(missing.editable);
        assert_eq!(missing.hint, Some("Not defined in this environment"));

        let secret = variable_tooltip_presentation("token", &variables);
        assert_eq!(secret.value, "unavailable");
        assert!(!secret.editable);
        assert!(secret.hint.is_none());
    }

    #[test]
    fn variable_tooltip_presentation_keeps_unavailable_when_not_writable() {
        let variables = VariableContext {
            unavailable_message: "Select an environment".to_owned(),
            ..VariableContext::default()
        };
        let missing = variable_tooltip_presentation("created", &variables);
        assert_eq!(missing.value, "Select an environment");
        assert!(!missing.editable);
        assert!(missing.hint.is_none());
    }

    #[gpui::test]
    fn variable_span_layout_keeps_duplicate_names_and_follows_scroll(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let window = cx.open_window(size(px(240.0), px(48.0)), |window, cx| HighlightHarness {
            input: cx.new(|cx| InputState::new(window, cx)),
        });
        window
            .update(cx, |_, window, _| {
                let font_family = Theme::light().typography.monospace_family;
                let value = "{{host}}/{{host}}";
                let ranges = variable_ranges(value);
                let unscrolled = variable_span_layout(
                    window,
                    value,
                    &ranges,
                    font_family,
                    14.0,
                    px(0.0),
                    0,
                    None,
                );
                assert_eq!(unscrolled.len(), 2);
                assert_eq!(unscrolled[0].0, "host");
                assert_eq!(unscrolled[1].0, "host");
                assert!(
                    unscrolled[1].1 > unscrolled[0].1,
                    "duplicate names should keep separate span origins, got {unscrolled:?}"
                );

                let scrolled = variable_span_layout(
                    window,
                    value,
                    &ranges,
                    font_family,
                    14.0,
                    px(-12.0),
                    0,
                    None,
                );
                assert_eq!(scrolled[0].1, unscrolled[0].1 - px(12.0));
                assert_eq!(scrolled[1].1, unscrolled[1].1 - px(12.0));

                let followed = variable_span_layout(
                    window,
                    value,
                    &ranges,
                    font_family,
                    14.0,
                    px(0.0),
                    value.len(),
                    Some(px(40.0)),
                );
                assert!(
                    followed[0].1 < px(0.0),
                    "caret past a narrow field should shift hover spans left, got {followed:?}"
                );
            })
            .expect("span layout test window should remain open");
    }

    #[test]
    fn body_text_highlights_overlay_mustache_variables() {
        let theme = Theme::light();
        let value = "{\"host\":\"{{host}}\"}";
        let ranges = variable_ranges(value);
        let highlights = body_text_highlights(theme, &ranges);
        assert_eq!(highlights.len(), 1);
        assert_eq!(&value[highlights[0].range.clone()], "{{host}}");
    }

    #[test]
    fn variable_highlight_runs_color_only_mustache_spans() {
        let value = "{{host}}/users";
        let ranges = variable_ranges(value)
            .into_iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        let highlight = hsla(0.33, 0.6, 0.5, 1.0);
        let base = gpui::TextRun {
            len: value.len(),
            font: gpui::Font::default(),
            color: transparent_black(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = variable_highlight_runs(value, &ranges, &base, highlight);
        assert_eq!(runs.len(), 2);
        assert_eq!(runs[0].color, highlight);
        assert_eq!(runs[0].len, "{{host}}".len());
        assert_eq!(runs[1].color, transparent_black());
        assert_eq!(runs[1].len, "/users".len());
    }

    #[test]
    fn variable_highlight_runs_paint_non_variable_text_with_the_base_color() {
        let value = "https://{{host}}/users";
        let ranges = variable_ranges(value)
            .into_iter()
            .map(|(range, _)| range)
            .collect::<Vec<_>>();
        let base_color = hsla(0.0, 0.0, 0.25, 1.0);
        let highlight = hsla(0.33, 0.6, 0.5, 1.0);
        let base = gpui::TextRun {
            len: value.len(),
            font: gpui::Font::default(),
            color: base_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };

        let runs = variable_highlight_runs(value, &ranges, &base, highlight);

        assert_eq!(runs.len(), 3);
        assert_eq!(runs[0].color, base_color);
        assert_eq!(runs[1].color, highlight);
        assert_eq!(runs[2].color, base_color);
    }

    #[test]
    fn input_text_scroll_offset_shifts_left_when_caret_overflows() {
        assert_eq!(
            input_text_scroll_offset(px(50.0), px(50.0), px(100.0), px(0.0)),
            px(0.0),
            "caret inside the field should not scroll"
        );
        assert_eq!(
            input_text_scroll_offset(px(200.0), px(200.0), px(100.0), px(0.0)),
            px(-110.0),
            "caret past the right edge should match gpui-base scroll_to"
        );
    }

    #[test]
    fn input_text_scroll_offset_keeps_existing_scroll_while_caret_stays_visible() {
        assert_eq!(
            input_text_scroll_offset(px(550.0), px(700.0), px(200.0), px(-500.0)),
            px(-500.0),
            "caret still visible in the scrolled viewport should not reset scroll"
        );
    }

    #[test]
    fn input_text_scroll_offset_scrolls_back_when_caret_moves_left_offscreen() {
        assert_eq!(
            input_text_scroll_offset(px(150.0), px(700.0), px(200.0), px(-500.0)),
            px(-140.0),
            "caret off the left edge should scroll right to reveal it"
        );
    }

    #[test]
    fn input_text_scroll_offset_clamps_immediately_when_deleting_at_end() {
        assert_eq!(
            input_text_scroll_offset(px(692.0), px(692.0), px(200.0), px(-510.0)),
            px(-502.0),
            "a shorter line should move the overlay with the input on the deletion frame"
        );
    }

    struct HighlightHarness {
        input: Entity<InputState>,
    }

    impl Render for HighlightHarness {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut Context<Self>,
        ) -> impl IntoElement {
            InputBase::new("variable-highlight-harness-input")
                .w(px(160.0))
                .h(px(24.0))
                .child(Input::new(&self.input))
        }
    }

    fn long_variable_url() -> SharedString {
        SharedString::from(format!(
            "{{{{sdfsdfsd}}}}{}",
            "kjlkjlkjlkjlkjlkjlkjflsdjflkjsdlfkjsldkjflskdjflkjlfjlsdj".repeat(2)
        ))
    }

    #[gpui::test]
    fn variable_highlight_scrolls_with_caret_at_end_of_long_url(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let value = long_variable_url();
        let window = cx.open_window(size(px(240.0), px(48.0)), |window, cx| HighlightHarness {
            input: cx.new(|cx| InputState::new(window, cx)),
        });
        cx.run_until_parked();
        let input = window
            .update(cx, |harness, window, cx| {
                harness.input.update(cx, |input, cx| {
                    input.focus(window, cx);
                });
                harness.input.clone()
            })
            .expect("highlight test window should be open");
        cx.simulate_input(window.into(), value.as_ref());
        cx.run_until_parked();
        let native_scroll_offset = input.read_with(cx, |input, _| input.scroll_offset().x);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let visible = size(px(160.0), px(24.0));
        let (_, prepaint) = visual.draw(point(px(0.0), px(0.0)), visible, |_, _| {
            VariableHighlightElement {
                state: input.clone(),
                base_color: transparent_black(),
                highlight_color: hsla(0.33, 0.6, 0.5, 1.0),
                highlight_path_variables: false,
            }
        });

        assert!(
            prepaint.scroll_offset < px(0.0),
            "long URL with caret at the end should scroll highlights left, got {:?}",
            prepaint.scroll_offset
        );
        assert_eq!(
            prepaint.scroll_offset, native_scroll_offset,
            "the variable overlay must use gpui-base's finalized scroll offset"
        );
    }

    #[gpui::test]
    fn variable_highlight_stays_at_origin_when_caret_is_at_start(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let value = long_variable_url();
        let window = cx.open_window(size(px(240.0), px(48.0)), |window, cx| HighlightHarness {
            input: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(value.clone(), window, cx);
                input
            }),
        });
        cx.run_until_parked();
        let input = window
            .update(cx, |harness, _window, cx| {
                harness.input.update(cx, |input, cx| {
                    input.set_selected_range(0..0, cx);
                });
                harness.input.clone()
            })
            .expect("highlight test window should be open");
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let (_, prepaint) = visual.draw(
            point(px(0.0), px(0.0)),
            size(px(160.0), px(24.0)),
            |_, _| VariableHighlightElement {
                state: input,
                base_color: transparent_black(),
                highlight_color: hsla(0.33, 0.6, 0.5, 1.0),
                highlight_path_variables: false,
            },
        );

        assert_eq!(
            prepaint.scroll_offset,
            px(0.0),
            "caret at the start should keep the variable highlight at its origin"
        );
    }

    #[gpui::test]
    fn variable_highlight_shapes_multiline_value_without_panicking(cx: &mut TestAppContext) {
        cx.update(crate::theme::Theme::init);
        let value = SharedString::from("{{host}}\n/users");
        let window = cx.open_window(size(px(240.0), px(48.0)), |window, cx| HighlightHarness {
            input: cx.new(|cx| {
                let mut input = InputState::new(window, cx);
                input.set_value(single_line(value.clone()), window, cx);
                input
            }),
        });
        let input = window
            .update(cx, |harness, _window, _cx| harness.input.clone())
            .expect("highlight test window should be open");
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let _ = visual.draw(
            point(px(0.0), px(0.0)),
            size(px(160.0), px(24.0)),
            |_, _| VariableHighlightElement {
                state: input,
                base_color: transparent_black(),
                highlight_color: hsla(0.33, 0.6, 0.5, 1.0),
                highlight_path_variables: false,
            },
        );
    }
}
