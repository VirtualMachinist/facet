use super::*;

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
        .focus_visible(move |button| button.border_color(theme.colors.borders.focused))
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
        .focus_visible(move |tab| tab.border_color(theme.colors.borders.focused))
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
        .focus_visible(move |button| button.border_color(theme.colors.borders.focused))
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
                .focus_visible(move |button| button.border_color(theme.colors.borders.focused))
                .on_click(on_click)
                .child(icon),
        )
}
