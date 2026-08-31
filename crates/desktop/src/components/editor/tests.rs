use std::collections::BTreeMap;

use gpui::{
    Context, Entity, IntoElement, Render, SharedString, TestAppContext, VisualTestContext, hsla,
    point, prelude::*, px, size, transparent_black,
};
use gpui_base::{
    Editor, Input, InputBase,
    input::{EditorState, InputState},
};

use super::*;
use crate::theme::Theme;

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
fn variable_tooltip_presentation_creates_missing_writable_variables() {
    let mut values = BTreeMap::new();
    values.insert("host".to_owned(), "api.example".to_owned());
    let variables = VariableContext {
        values,
        secrets: ["token".to_owned()].into_iter().collect(),
        unavailable_message: "unavailable".to_owned(),
        on_change: Some(std::rc::Rc::new(|_, _, _, _| {})),
        on_manage_environments: None,
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

struct SearchHighlightHarness {
    editor: Entity<EditorState>,
}

impl Render for SearchHighlightHarness {
    fn render(&mut self, _window: &mut gpui::Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::light();
        InputBase::new("search-highlight-harness-editor")
            .size_full()
            .font_family(theme.typography.monospace_family)
            .text_size(px(theme.typography.body_size))
            .child(Editor::new(&self.editor))
    }
}

#[gpui::test]
fn search_highlight_bounds_include_last_character_before_newline(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    let value = SharedString::from("aaaaaaaaaf\n234");
    let window = cx.open_window(size(px(320.0), px(120.0)), |window, cx| {
        SearchHighlightHarness {
            editor: cx.new(|cx| {
                let mut editor = EditorState::new(window, cx).soft_wrap(false);
                editor.set_value(value.clone(), window, cx);
                editor
            }),
        }
    });
    cx.run_until_parked();

    let (fallback, cross_line, single_character) = window
        .update(cx, |harness, window, cx| {
            let editor = harness.editor.read(cx);
            let fallback = super::search_fallback_char_size(Theme::light(), editor, window);
            (
                fallback,
                super::search_match_bounds(editor, &value, &(9..14), fallback),
                super::search_match_bounds(editor, &value, &(9..10), fallback),
            )
        })
        .expect("search highlight test window should be open");

    assert_eq!(cross_line.len(), 2, "f234 should highlight on both rows");
    assert!(
        cross_line.iter().all(|bounds| bounds.size.width > px(1.0)),
        "both rows of a cross-line match should have visible highlight bounds"
    );
    assert_eq!(
        single_character.len(),
        1,
        "a lone match at the end of a row should have highlight bounds"
    );
    assert_eq!(single_character[0].size.height, fallback.height);
    assert!(
        (single_character[0].size.width - fallback.width - px(1.0)).abs() < px(0.01),
        "a lone line-end match should use the editor font's measured glyph width"
    );
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
