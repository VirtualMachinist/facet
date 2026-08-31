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
    Anchor, Animation, AnimationExt as _, App, AppContext as _, Bounds, BoxShadow, ClickEvent,
    ContentMask, Context, Edges, Element, ElementId, Entity, EntityId, FocusHandle, Focusable,
    FontWeight, GlobalElementId, HighlightStyle, Hsla, InspectorElementId, InteractiveElement as _,
    IntoElement, LayoutId, MouseButton, ParentElement as _, Pixels, Point, Render, RenderOnce,
    Role, ShapedLine, SharedString, StatefulInteractiveElement as _, Style, Styled as _,
    Subscription, Task, TextAlign, TextRun, TransformationMatrix, Window, canvas, deferred, div,
    fill, font, point, prelude::FluentBuilder as _, px, relative, size, transparent_black,
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
mod controls;
mod editor;
use controls::{EditorInsets, TextContextMenuExtraAction, VisibleRangeHandler, text_input_base};
pub(crate) use controls::{
    ResponseBodyInputOptions, browse_file_button, compact_icon_button, dialog_text_input, dropdown,
    dropdown_with_option_colors, editor_add_button, editor_button, editor_key_value_row,
    editor_subtab, icon_button, remove_row_button, sidebar_search_input, text_tab, url_text_input,
    variable_text_input,
};

mod icons;
pub(crate) use editor::{
    BodySyntax, body_text_input, response_body_input, response_headers_input,
    response_inspector_input, single_line,
};
#[cfg(test)]
use editor::{
    ProbeEditor, VariableHighlightElement, body_text_highlights, editor_value_needs_refresh,
    input_text_scroll_offset, normalize_search_char_bounds, search_fallback_char_size,
    search_match_bounds, search_match_char_ranges, variable_highlight_runs, variable_ranges,
    variable_span_layout, variable_tooltip_presentation,
};
use editor::{
    VariableTooltipPresentation, editor_paint_style, input_variable_ranges, variable_input_overlay,
};
mod menus;
mod splitter;
mod surfaces;
mod toasts;
#[cfg(test)]
use surfaces::clipboard_has_pasteable_text;
use surfaces::{
    DropdownActionHandler, DropdownChangeHandler, EditorMouseDownHandler, InputChangeHandler,
    TextContextActionHandler, TextContextEnableHandler, TextContextMenuState, VariableHoverState,
    temporary_surface_shadow, text_context_menu_id, variable_tooltip_popup, with_text_context_menu,
};
pub(crate) use surfaces::{
    VariableContext, context_menu_surface, dialog_actions, dialog_description, dialog_field,
    dialog_field_label, dialog_layer, dialog_surface, dialog_title, popup_surface, truncated_label,
};

pub(crate) use buttons::{
    DialogActionStyle, dialog_action_button, dialog_choice_button, primary_button,
    secondary_button, secondary_menu_trigger, text_button,
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
pub(crate) use splitter::pane_splitter;
pub(crate) use toasts::{TOAST_STACK_WIDTH, toast};

/// Fixed width for compact primary actions such as Send.
pub(crate) const COMPACT_ACTION_BUTTON_WIDTH: f32 = 72.0;
pub(crate) const COMPACT_DIALOG_WIDTH: f32 = 420.0;
pub(crate) const WIDE_DIALOG_WIDTH: f32 = 520.0;

#[cfg(test)]
mod components_tests;
