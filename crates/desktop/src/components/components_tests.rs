use std::collections::BTreeMap;

use gpui::{
    AppContext as _, ClipboardItem, Context, Entity, Image, IntoElement, KeyBinding, Modifiers,
    MouseButton, Render, SharedString, TestAppContext, VisualTestContext, div, hsla, point,
    prelude::*, px, size, transparent_black,
};
use gpui_base::{
    Button, Input, InputBase, Popover,
    input::{Copy, Cut, InputState, Paste, SelectAll},
};

use super::{
    EditorInsets, ProbeEditor, VariableContext, VariableHighlightElement, body_text_highlights,
    clipboard_has_pasteable_text, dropdown, editor_paint_style, input_text_scroll_offset,
    menu_button, single_line, variable_highlight_runs, variable_ranges, variable_span_layout,
    variable_tooltip_presentation,
};
use crate::theme::Theme;
use probe_core::path_variable_ranges;

struct MenuTestView {
    open: bool,
    activations: usize,
}

#[derive(Clone, Copy)]
enum TextContextMenuHarnessKind {
    Input,
    BodyEditor,
    ResponseEditor,
}

struct TextContextMenuHarness {
    kind: TextContextMenuHarnessKind,
    input: Option<Entity<InputState>>,
}

impl Render for TextContextMenuHarness {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::light();
        let content = match self.kind {
            TextContextMenuHarnessKind::Input => {
                let mut input = super::text_input_base(
                    theme,
                    "context-input",
                    "https://api.example.com",
                    "URL",
                );
                input.debug_selector = Some("context-input");
                input.shared_input = self.input.clone();
                input.into_any_element()
            }
            TextContextMenuHarnessKind::BodyEditor => ProbeEditor {
                theme,
                id: "context-body-editor".into(),
                value: "{\"ok\":true}".into(),
                placeholder: "Body content".into(),
                decorations: Vec::new(),
                language: "json".into(),
                readonly: false,
                min_height: Some(120.0),
                padding: EditorInsets::standard(theme),
                soft_wrap: true,
                text_color: theme.colors.text.primary,
                scroll_to_range: None,
                search_matches: Vec::new(),
                on_change: None,
                on_mouse_down: None,
                on_visible_range: None,
                extra_context_menu_actions: Vec::new(),
                debug_selector: Some("context-body-editor"),
                variables: Some(VariableContext::default()),
            }
            .into_any_element(),
            TextContextMenuHarnessKind::ResponseEditor => ProbeEditor {
                theme,
                id: "context-response-editor".into(),
                value: "{\"ok\":true}".into(),
                placeholder: SharedString::default(),
                decorations: Vec::new(),
                language: "json".into(),
                readonly: true,
                min_height: Some(120.0),
                padding: EditorInsets::response(theme),
                soft_wrap: false,
                text_color: theme.colors.text.primary,
                scroll_to_range: None,
                search_matches: Vec::new(),
                on_change: None,
                on_mouse_down: None,
                on_visible_range: None,
                extra_context_menu_actions: Vec::new(),
                debug_selector: Some("context-response-editor"),
                variables: None,
            }
            .into_any_element(),
        };
        div()
            .size_full()
            .p(px(20.0))
            .child(div().w_full().h(px(140.0)).child(content))
    }
}

fn open_text_context_menu(
    cx: &mut TestAppContext,
    kind: TextContextMenuHarnessKind,
    target: &'static str,
) -> gpui::WindowHandle<TextContextMenuHarness> {
    let window = cx.open_window(size(px(420.0), px(220.0)), |window, cx| {
        TextContextMenuHarness {
            kind,
            input: (matches!(kind, TextContextMenuHarnessKind::Input))
                .then(|| cx.new(|cx| InputState::new(window, cx))),
        }
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let target = visual
        .debug_bounds(target)
        .expect("text context-menu target should render");
    visual.simulate_mouse_down(target.center(), MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(target.center(), MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();
    window
}

#[gpui::test]
fn editable_input_and_body_editor_show_editing_context_menu(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    cx.update(|cx| {
        cx.bind_keys([
            KeyBinding::new("ctrl-x", Cut, None),
            KeyBinding::new("ctrl-c", Copy, None),
            KeyBinding::new("ctrl-v", Paste, None),
            KeyBinding::new("ctrl-a", SelectAll, None),
        ]);
    });
    for (kind, target) in [
        (TextContextMenuHarnessKind::Input, "context-input"),
        (
            TextContextMenuHarnessKind::BodyEditor,
            "context-body-editor",
        ),
    ] {
        let window = open_text_context_menu(cx, kind, target);
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("text-context-menu").is_some());
        assert!(visual.debug_bounds("text-context-cut").is_some());
        assert!(visual.debug_bounds("text-context-copy").is_some());
        assert!(visual.debug_bounds("text-context-paste").is_some());
        assert!(visual.debug_bounds("text-context-select-all").is_some());
        let shortcuts = window
            .update(cx, |_, window, _| {
                [
                    super::shortcut_label_for_action(window, &Cut),
                    super::shortcut_label_for_action(window, &Copy),
                    super::shortcut_label_for_action(window, &Paste),
                    super::shortcut_label_for_action(window, &SelectAll),
                ]
            })
            .expect("text context-menu window should remain open");
        assert!(shortcuts.iter().all(Option::is_some));
    }
}

#[gpui::test]
fn readonly_response_editor_shows_copy_context_menu(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    let window = open_text_context_menu(
        cx,
        TextContextMenuHarnessKind::ResponseEditor,
        "context-response-editor",
    );
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("text-context-menu").is_some());
    assert!(visual.debug_bounds("text-context-copy").is_some());
    assert!(visual.debug_bounds("text-context-select-all").is_some());
    assert!(visual.debug_bounds("text-context-cut").is_none());
    assert!(visual.debug_bounds("text-context-paste").is_none());
}

#[gpui::test]
fn text_context_menu_only_enables_paste_for_text_clipboard_content(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.write_to_clipboard(ClipboardItem::new_image(&Image::empty()));
        assert!(!clipboard_has_pasteable_text(cx));

        cx.write_to_clipboard(ClipboardItem::new_string("request body".into()));
        assert!(clipboard_has_pasteable_text(cx));
    });
}

#[gpui::test]
fn context_menu_actions_apply_to_the_target_input(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    let window = open_text_context_menu(cx, TextContextMenuHarnessKind::Input, "context-input");
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let select_all = visual
        .debug_bounds("text-context-select-all")
        .expect("Select All should render");
    visual.simulate_click(select_all.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let input = window
        .update(cx, |view, _, _| {
            view.input.clone().expect("input state should exist")
        })
        .expect("test window should remain open");
    assert_eq!(
        input.read_with(cx, |input, _| input.selected_range()),
        0..23
    );

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let target = visual
        .debug_bounds("context-input")
        .expect("input should remain rendered");
    let selected_text = point(target.left() + px(40.0), target.center().y);
    visual.simulate_mouse_down(selected_text, MouseButton::Right, Modifiers::default());
    visual.simulate_mouse_up(selected_text, MouseButton::Right, Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let cut = visual
        .debug_bounds("text-context-cut")
        .expect("Cut should render");
    visual.simulate_click(cut.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();
    assert_eq!(input.read_with(cx, |input, _| input.value()), "");
}

impl Render for MenuTestView {
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
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
    fn render(&mut self, _window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
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
            let unscrolled =
                variable_span_layout(window, value, &ranges, font_family, 14.0, px(0.0), 0, None);
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
fn search_match_char_ranges_split_long_matches_on_char_boundaries() {
    let value = "token.abc.def\nnext";
    let ranges = super::search_match_char_ranges(value, 0.."token.abc.def\nn".len());

    assert_eq!(ranges.first(), Some(&(0..1)));
    assert_eq!(ranges.last(), Some(&(14..15)));
    assert!(!ranges.iter().any(|range| &value[range.clone()] == "\n"));
    assert!(
        ranges.iter().all(|range| {
            value.is_char_boundary(range.start) && value.is_char_boundary(range.end)
        })
    );
}

#[test]
fn search_highlight_bounds_repair_wrapped_edge_characters() {
    let mut wrapped_edge =
        gpui::Bounds::new(point(px(120.0), px(24.0)), size(px(-120.0), px(32.0)));

    super::normalize_search_char_bounds(&mut wrapped_edge, Some(size(px(8.0), px(16.0))));

    assert_eq!(wrapped_edge.size.width, px(8.0));
    assert_eq!(wrapped_edge.size.height, px(16.0));
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
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
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
