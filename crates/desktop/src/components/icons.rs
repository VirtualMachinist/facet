use super::*;

static CHEVRON_DOWN_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuChevronDown));
static CHEVRON_UP_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuChevronUp));
pub(super) static CHEVRON_RIGHT_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuChevronRight));
pub(super) static CHECK_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuCheck));
static FOLDER_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuFolder));
static FOLDER_OPEN_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuFolderOpen));
static PLUS_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuPlus));
pub(super) static SEARCH_SVG: LazyLock<Vec<u8>> =
    LazyLock::new(|| icon_svg_bytes(icondata::LuSearch));
static SAVE_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuSave));
static CLOSE_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuX));
static TRASH_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuTrash2));
static LOCATE_SVG: LazyLock<Vec<u8>> = LazyLock::new(|| icon_svg_bytes(icondata::LuLocateFixed));
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

pub(super) fn library_icon(
    cache_key: &'static str,
    data: &'static LazyLock<Vec<u8>>,
    size: f32,
) -> gpui::Div {
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

pub(super) fn plus_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-plus", &PLUS_SVG, theme.metrics.icon_small)
}

pub(crate) fn hover_fill(color: gpui::Rgba) -> gpui::Rgba {
    let mut hover: Hsla = color.into();
    hover.l = if hover.l < 0.5 {
        (hover.l + 0.08).min(1.0)
    } else {
        (hover.l * 0.92).max(0.0)
    };
    hover.into()
}

pub(crate) fn add_menu_button(theme: Theme, open: bool, enabled: bool) -> Button {
    icon_button_base(
        theme,
        "tree-add-menu-trigger",
        "tree-add-menu-trigger",
        "Add request or folder",
        enabled,
        open && enabled,
        hover_fill(theme.colors.surfaces.window),
    )
    .child(
        library_icon("lucide-plus", &PLUS_SVG, theme.metrics.icon_standard).text_color(
            if enabled {
                theme.colors.text.primary
            } else {
                theme.colors.actions.disabled_foreground
            },
        ),
    )
}

pub(crate) fn save_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-save", &SAVE_SVG, theme.metrics.icon_standard)
        .text_color(theme.colors.text.primary)
}

pub(crate) fn close_icon(theme: Theme) -> gpui::Div {
    library_icon("lucide-x", &CLOSE_SVG, theme.metrics.icon_standard)
        .text_color(theme.colors.text.secondary)
}

pub(crate) fn locate_icon(theme: Theme) -> gpui::Div {
    library_icon(
        "lucide-locate-fixed",
        &LOCATE_SVG,
        theme.metrics.icon_standard,
    )
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
    icon_button_base(
        theme,
        "sidebar-toggle",
        "sidebar-toggle",
        label,
        true,
        false,
        theme.colors.surfaces.sidebar,
    )
    .child(sidebar_icon(theme, collapsed))
    .on_click(move |_, window, cx| on_toggle(window, cx))
}

pub(crate) fn home_button(
    theme: Theme,
    enabled: bool,
    on_click: impl Fn(&mut Window, &mut App) + 'static,
) -> Button {
    icon_button_base(
        theme,
        "home-button",
        "home-button",
        "Close collection",
        enabled,
        false,
        theme.colors.surfaces.sidebar,
    )
    .child(
        library_icon("lucide-house", &HOME_SVG, theme.metrics.icon_standard).text_color(
            if enabled {
                theme.colors.text.secondary
            } else {
                theme.colors.actions.disabled_foreground
            },
        ),
    )
    .on_click(move |_, window, cx| on_click(window, cx))
}

fn icon_button_base(
    theme: Theme,
    id: &'static str,
    debug_selector: &'static str,
    accessibility_label: &'static str,
    enabled: bool,
    selected: bool,
    active_background: gpui::Rgba,
) -> Button {
    Button::new(id)
        .accessibility_label(accessibility_label)
        .debug_selector(move || debug_selector.into())
        .selected(selected)
        .disabled(!enabled)
        .size(px(theme.metrics.control_height))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(theme.metrics.radius_small))
        .when(!enabled, |button| button.bg(theme.colors.actions.disabled))
        .when(enabled, |button| {
            button
                .hover(move |button| button.bg(active_background))
                .focus(move |button| button.bg(active_background))
                .styles(move |styles| styles.selected(move |button| button.bg(active_background)))
        })
}

pub(super) fn folder_open_icon(color: gpui::Rgba) -> gpui::Div {
    library_icon("lucide-folder-open", &FOLDER_OPEN_SVG, 14.0).text_color(color)
}

pub(super) fn trash_icon(color: gpui::Rgba) -> gpui::Div {
    library_icon("lucide-trash-2", &TRASH_SVG, 14.0).text_color(color)
}
