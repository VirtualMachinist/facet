use super::*;

pub(super) fn focus_ring_shadow(ring_color: Hsla, gap_color: Hsla) -> Vec<BoxShadow> {
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
) -> Button {
    action_button(
        theme,
        id,
        label,
        ActionButtonKind::Primary,
        None,
        false,
        on_click,
    )
}

pub(crate) fn secondary_button(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    action_button(
        theme,
        id,
        label,
        ActionButtonKind::Secondary,
        None,
        false,
        on_click,
    )
}

pub(crate) fn secondary_menu_trigger(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    focus_handle: &FocusHandle,
    on_keyboard_activate: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    secondary_button(theme, id, label, move |event, window, cx| {
        if !matches!(event, ClickEvent::Mouse(_)) {
            on_keyboard_activate(window, cx);
        }
    })
    .track_focus(focus_handle)
    .key_context("ImportSubmenuTrigger")
}

#[derive(Clone, Copy)]
enum ActionButtonKind {
    Primary,
    Secondary,
    DialogSecondary,
    Destructive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DialogActionStyle {
    Primary,
    Secondary,
    Destructive,
}

fn action_button(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    kind: ActionButtonKind,
    shortcut_hint: Option<String>,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let label = label.into();
    let button = Button::new(id)
        .debug_selector(|| id.into())
        .h(px(theme.metrics.control_height))
        .min_w(px(COMPACT_ACTION_BUTTON_WIDTH))
        .px(px(theme.metrics.spacing_3))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .font_family(theme.typography.interface_family)
        .text_size(px(theme.typography.body_size))
        .border_1()
        .when(!disabled, |button| button.cursor_pointer());
    match kind {
        ActionButtonKind::Primary => button
            .text_color(theme.colors.text.inverse)
            .bg(theme.colors.actions.accent)
            .border_color(theme.colors.actions.accent)
            .when(!disabled, |button| {
                button.hover(move |button| {
                    button
                        .bg(theme.colors.actions.hover)
                        .border_color(theme.colors.actions.hover)
                })
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
            .disabled(disabled)
            .on_click(on_click)
            .child(action_button_label(
                theme,
                label,
                kind,
                shortcut_hint,
                disabled,
            )),
        ActionButtonKind::Secondary => button
            .text_color(theme.colors.text.primary)
            .bg(theme.colors.surfaces.raised)
            .border_color(theme.colors.borders.standard)
            .when(!disabled, |button| {
                button.hover(move |button| button.bg(theme.colors.surfaces.window))
            })
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
            .disabled(disabled)
            .on_click(on_click)
            .child(action_button_label(
                theme,
                label,
                kind,
                shortcut_hint,
                disabled,
            )),
        ActionButtonKind::DialogSecondary => button
            .text_color(theme.colors.text.secondary)
            .bg(gpui::transparent_black())
            .border_color(theme.colors.borders.standard)
            .when(!disabled, |button| {
                button.hover(move |button| {
                    button
                        .text_color(theme.colors.text.primary)
                        .bg(theme.colors.selection.inactive_background)
                        .border_color(theme.colors.borders.standard)
                })
            })
            .focus(move |button| {
                button
                    .border_color(theme.colors.borders.strong)
                    .shadow(focus_ring_shadow(
                        theme.colors.borders.focused.into(),
                        theme.colors.surfaces.overlay.into(),
                    ))
            })
            .styles(move |styles| {
                styles.disabled(move |button| {
                    button
                        .bg(gpui::transparent_black())
                        .text_color(theme.colors.actions.disabled_foreground)
                        .border_color(theme.colors.borders.subtle)
                })
            })
            .disabled(disabled)
            .on_click(on_click)
            .child(action_button_label(
                theme,
                label,
                kind,
                shortcut_hint,
                disabled,
            )),
        ActionButtonKind::Destructive => {
            let mut border: Hsla = theme.colors.status.error.into();
            border.a = match theme.appearance {
                crate::theme::ThemeAppearance::Light => 0.42,
                crate::theme::ThemeAppearance::Dark => 0.52,
            };
            button
                .text_color(theme.colors.status.error)
                .bg(gpui::transparent_black())
                .border_color(border)
                .when(!disabled, |button| {
                    button.hover(move |button| {
                        let mut hover: Hsla = theme.colors.status.error.into();
                        hover.a = match theme.appearance {
                            crate::theme::ThemeAppearance::Light => 0.09,
                            crate::theme::ThemeAppearance::Dark => 0.14,
                        };
                        button.bg(hover).border_color(theme.colors.status.error)
                    })
                })
                .focus(move |button| {
                    button
                        .border_color(theme.colors.status.error)
                        .shadow(focus_ring_shadow(
                            theme.colors.status.error.into(),
                            theme.colors.surfaces.overlay.into(),
                        ))
                })
                .styles(move |styles| {
                    styles.disabled(move |button| {
                        button
                            .bg(gpui::transparent_black())
                            .text_color(theme.colors.actions.disabled_foreground)
                            .border_color(theme.colors.borders.subtle)
                    })
                })
                .disabled(disabled)
                .on_click(on_click)
                .child(action_button_label(
                    theme,
                    label,
                    kind,
                    shortcut_hint,
                    disabled,
                ))
        }
    }
}

fn action_button_label(
    theme: Theme,
    label: String,
    kind: ActionButtonKind,
    shortcut_hint: Option<String>,
    disabled: bool,
) -> impl IntoElement {
    let Some(shortcut_hint) = shortcut_hint else {
        return div().child(label);
    };

    let key_color: Hsla = if disabled {
        theme.colors.actions.disabled_foreground.into()
    } else {
        match kind {
            ActionButtonKind::Primary => theme.colors.text.inverse.into(),
            ActionButtonKind::Destructive => theme.colors.status.error.into(),
            ActionButtonKind::Secondary | ActionButtonKind::DialogSecondary => {
                theme.colors.text.muted.into()
            }
        }
    };
    let mut hint_color = key_color;
    hint_color.a = 0.9;
    let mut hint_background = key_color;
    hint_background.a = match kind {
        ActionButtonKind::Primary => 0.16,
        ActionButtonKind::Destructive => 0.08,
        ActionButtonKind::Secondary | ActionButtonKind::DialogSecondary => 0.08,
    };
    let mut hint_border = key_color;
    hint_border.a = match kind {
        ActionButtonKind::Primary => 0.36,
        ActionButtonKind::Destructive => 0.26,
        ActionButtonKind::Secondary | ActionButtonKind::DialogSecondary => 0.22,
    };
    div()
        .flex()
        .items_center()
        .gap(px(theme.metrics.spacing_2))
        .child(div().child(label))
        .child(
            div()
                .flex_none()
                .h(px(20.0))
                .px(px(theme.metrics.spacing_1))
                .flex()
                .items_center()
                .rounded(px(theme.metrics.radius_small))
                .border_1()
                .bg(hint_background)
                .border_color(hint_border)
                .text_size(px(theme.typography.caption_size))
                .text_color(hint_color)
                .child(shortcut_hint),
        )
}

pub(crate) fn dialog_action_button(
    theme: Theme,
    id: &'static str,
    label: impl Into<String>,
    style: DialogActionStyle,
    shortcut_hint: Option<String>,
    disabled: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    let kind = match style {
        DialogActionStyle::Primary => ActionButtonKind::Primary,
        DialogActionStyle::Secondary => ActionButtonKind::DialogSecondary,
        DialogActionStyle::Destructive => ActionButtonKind::Destructive,
    };
    action_button(theme, id, label, kind, shortcut_hint, disabled, on_click)
}

pub(crate) fn dialog_choice_button(
    theme: Theme,
    id: impl Into<ElementId>,
    label: impl Into<String>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Button {
    Button::new(id)
        .h(px(theme.metrics.control_height + 4.0))
        .w_full()
        .px(px(theme.metrics.spacing_3))
        .flex()
        .items_center()
        .justify_start()
        .rounded(px(theme.metrics.radius_small))
        .font_family(theme.typography.interface_family)
        .text_size(px(theme.typography.body_size))
        .text_color(theme.colors.text.primary)
        .bg(theme.colors.surfaces.raised)
        .border_1()
        .border_color(theme.colors.borders.standard)
        .cursor_pointer()
        .hover(move |button| button.bg(theme.colors.selection.inactive_background))
        .focus(move |button| {
            button
                .border_color(theme.colors.borders.strong)
                .shadow(focus_ring_shadow(
                    theme.colors.borders.focused.into(),
                    theme.colors.surfaces.overlay.into(),
                ))
        })
        .on_click(on_click)
        .child(label.into())
}
