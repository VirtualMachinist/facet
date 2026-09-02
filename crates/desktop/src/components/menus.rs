use super::buttons::focus_ring_shadow;
use super::*;

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
        true,
        MenuButtonStyle {
            padding_x: theme.metrics.spacing_2,
            text_color: theme.colors.text.primary,
            shortcut_color: theme.colors.text.muted,
        },
        on_activate,
    )
}

pub(crate) fn shortcut_label_for_action(
    window: &Window,
    action: &dyn gpui::Action,
) -> Option<String> {
    window
        .highest_precedence_binding_for_action(action)
        .map(|binding| shortcut_label_for_binding(&binding))
}

pub(crate) fn shortcut_label_for_action_in_context(
    window: &Window,
    action: &dyn gpui::Action,
    context: &str,
) -> Option<String> {
    let context = gpui::KeyContext::parse(context).ok()?;
    window
        .highest_precedence_binding_for_action_in_context(action, context)
        .map(|binding| shortcut_label_for_binding(&binding))
}

fn shortcut_label_for_binding(binding: &gpui::KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(shortcut_label_for_keystroke)
        .collect::<Vec<_>>()
        .join(" ")
}

fn shortcut_label_for_keystroke(keystroke: &gpui::KeybindingKeystroke) -> String {
    let label = keystroke.to_string();
    if let Some(prefix) = label.strip_suffix("enter") {
        format!("{prefix}⏎")
    } else {
        label
    }
}

pub(crate) fn app_menu_trigger(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    open: bool,
    on_hover: impl Fn(&mut App) + 'static,
) -> Button {
    let label = label.into();
    Button::new(id)
        .selected(open)
        .h(px(theme.metrics.control_height))
        .px(px(theme.metrics.spacing_2))
        .flex()
        .items_center()
        .rounded(px(theme.metrics.radius_small))
        .font_family(theme.typography.interface_family)
        .text_size(px(theme.typography.body_size))
        .text_color(theme.colors.text.primary)
        .hover(move |button| button.bg(theme.colors.surfaces.sidebar))
        .focus_visible(move |button| button.border_1().border_color(theme.colors.borders.focused))
        .styles(move |styles| {
            styles.selected(move |button| button.bg(theme.colors.surfaces.sidebar))
        })
        .on_mouse_move(move |_, _, cx| on_hover(cx))
        .child(label)
}

fn menu_row_button(
    theme: Theme,
    id: impl Into<ElementId>,
    selected: bool,
    style: MenuButtonStyle,
    content: impl IntoElement,
) -> Button {
    Button::new(id)
        .selected(selected)
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
        .focus_visible(move |button| button.border_1().border_color(theme.colors.borders.focused))
        .styles(move |styles| {
            styles.selected(move |button| button.bg(theme.colors.surfaces.sidebar))
        })
        .child(content)
}

pub(crate) fn submenu_menu_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    open: bool,
    on_open: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let on_open = Rc::new(on_open);
    let pointer_open = on_open.clone();
    let keyboard_open = on_open;

    div()
        .w_full()
        .on_mouse_move(move |_, _, cx| pointer_open(cx))
        .child(
            menu_row_button(
                theme,
                id,
                open,
                MenuButtonStyle::standard(theme),
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(truncated_label(label).flex_1())
                    .child(
                        library_icon(
                            "lucide-chevron-right",
                            &CHEVRON_RIGHT_SVG,
                            theme.metrics.icon_small,
                        )
                        .text_color(theme.colors.text.muted),
                    ),
            )
            .on_click(move |_, _, cx| keyboard_open(cx)),
        )
}

pub(crate) fn import_submenu_menu_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    open: bool,
    focus_handle: &FocusHandle,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    let id = id.into();
    let debug_selector = id.to_string();
    let label = label.into();
    menu_row_button(
        theme,
        id,
        open,
        MenuButtonStyle::standard(theme),
        div()
            .w_full()
            .flex()
            .items_center()
            .child(truncated_label(label).flex_1())
            .child(
                library_icon(
                    "lucide-chevron-right",
                    &CHEVRON_RIGHT_SVG,
                    theme.metrics.icon_small,
                )
                .text_color(theme.colors.text.muted),
            ),
    )
    .debug_selector(move || debug_selector)
    .track_focus(focus_handle)
    .key_context("ImportSubmenuTrigger")
    .on_click(move |_, window, cx| on_activate(window, cx))
}

pub(crate) fn positioned_cascading_menu(
    theme: Theme,
    open: bool,
    parent_width: f32,
    trigger: impl IntoElement,
    popup: gpui::AnyElement,
) -> impl IntoElement {
    let surface_inset = theme.metrics.spacing_1 + 1.0;
    div().relative().w_full().child(trigger).when(open, |row| {
        row.child(
            deferred(
                div()
                    .absolute()
                    .occlude()
                    // Let the popup surface overlap the parent like a native
                    // cascading menu, and offset its padding plus border so the
                    // first child row aligns with the trigger row.
                    .top(px(-surface_inset))
                    .left(px(parent_width - surface_inset * 2.0))
                    .child(popup),
            )
            // The parent surface paints its border after normal children.
            // Deferring the submenu keeps that border behind the overlap.
            .with_priority(POPUP_PRIORITY),
        )
    })
}

pub(crate) fn cascading_menu(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    open: bool,
    parent_width: f32,
    popup: gpui::AnyElement,
    on_open: impl Fn(&mut App) + 'static,
) -> impl IntoElement {
    positioned_cascading_menu(
        theme,
        open,
        parent_width,
        submenu_menu_button(theme, id, label, open, on_open),
        popup,
    )
}

pub(crate) fn checked_menu_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    checked: bool,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    let on_activate = Rc::new(on_activate);
    let pointer_activate = on_activate.clone();
    let keyboard_activate = on_activate;

    div()
        .w_full()
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            pointer_activate(window, cx);
        })
        .child(
            menu_row_button(
                theme,
                id,
                false,
                MenuButtonStyle::standard(theme),
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .child(div().w(px(theme.metrics.icon_standard)).flex_none().when(
                        checked,
                        |slot| {
                            slot.child(
                                library_icon(
                                    "lucide-check",
                                    &CHECK_SVG,
                                    theme.metrics.icon_standard,
                                )
                                .text_color(theme.colors.text.primary),
                            )
                        },
                    ))
                    .child(truncated_label(label).ml(px(theme.metrics.spacing_1))),
            )
            .on_click(move |event, window, cx| {
                if !matches!(event, ClickEvent::Mouse(_)) {
                    keyboard_activate(window, cx);
                }
            }),
        )
}

pub(crate) fn menu_separator(theme: Theme) -> gpui::Div {
    div()
        .mx(px(theme.metrics.spacing_2))
        .my(px(theme.metrics.spacing_1))
        .h(px(1.0))
        .flex_none()
        .bg(theme.colors.borders.subtle)
}

pub(super) fn context_menu_separator(theme: Theme) -> gpui::Div {
    div()
        .mx(px(theme.metrics.spacing_2))
        .h(px(1.0))
        .flex_none()
        .bg(theme.colors.borders.subtle)
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
        true,
        MenuButtonStyle {
            padding_x: theme.metrics.spacing_2,
            text_color: theme.colors.status.error,
            shortcut_color: theme.colors.status.error,
        },
        on_activate,
    )
}

#[derive(Clone, Copy)]
pub(super) struct MenuButtonStyle {
    pub(super) padding_x: f32,
    pub(super) text_color: gpui::Rgba,
    pub(super) shortcut_color: gpui::Rgba,
}

impl MenuButtonStyle {
    pub(super) fn standard(theme: Theme) -> Self {
        Self {
            padding_x: theme.metrics.spacing_2,
            text_color: theme.colors.text.primary,
            shortcut_color: theme.colors.text.muted,
        }
    }
}

pub(super) fn menu_button_with_style(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    shortcut: Option<String>,
    enabled: bool,
    style: MenuButtonStyle,
    on_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let id = id.into();
    let debug_selector = id.to_string();
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
            if enabled {
                pointer_activate(window, cx);
            }
        })
        .child(
            menu_row_button(
                theme,
                id,
                false,
                style,
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
            )
            .disabled(!enabled)
            .styles(move |styles| {
                styles.disabled(move |button| {
                    button
                        .text_color(theme.colors.actions.disabled)
                        .cursor_default()
                })
            })
            .debug_selector(move || debug_selector)
            .on_click(move |event, window, cx| {
                if enabled && !matches!(event, ClickEvent::Mouse(_)) {
                    keyboard_activate(window, cx);
                }
            }),
        )
}

pub(crate) fn switch(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    checked: bool,
    disabled: bool,
    on_checked_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> impl IntoElement {
    let label = label.into();
    Switch::new(id)
        .checked(checked)
        .disabled(disabled)
        .accessibility_label(label)
        .w(px(36.0))
        .h(px(20.0))
        .flex()
        .items_center()
        .when(!disabled, |switch| switch.cursor_pointer())
        .on_change(move |value, _, window, cx| {
            if !disabled {
                on_checked_change(value, window, cx);
            }
        })
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
            .focus_visible(move |toggle| {
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
