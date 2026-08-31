use std::{cell::Cell, rc::Rc};

use gpui::{
    AppContext as _, Axis, ClipboardItem, Context, Entity, Image, IntoElement, KeyBinding,
    Modifiers, MouseButton, Render, SharedString, TestAppContext, VisualTestContext, div, point,
    prelude::*, px, size,
};
use gpui_base::{
    Button, Popover,
    input::{Copy, Cut, InputState, Paste, SelectAll},
};

use super::{
    EditorInsets, ProbeEditor, VariableContext, clipboard_has_pasteable_text, dropdown,
    editor_value_needs_refresh, menu_button, pane_splitter,
};
use crate::theme::Theme;

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

#[test]
fn changing_editor_language_refreshes_unchanged_text() {
    let xml: SharedString = r#"<root id="1"/>"#.into();
    assert!(editor_value_needs_refresh(true, &xml, &xml));
    assert!(!editor_value_needs_refresh(false, &xml, &xml));
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

struct SplitterHarness {
    presses: Rc<Cell<usize>>,
}

impl Render for SplitterHarness {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let presses = self.presses.clone();
        div().size_full().p(px(8.0)).child(
            div()
                .id("splitter-pane")
                .debug_selector(|| "splitter-pane".into())
                .size_full()
                .relative()
                .child(
                    pane_splitter(Theme::light(), "test-splitter", Axis::Horizontal)
                        .debug_selector("test-splitter")
                        .on_mouse_down(move |_, _, _| {
                            presses.set(presses.get() + 1);
                        }),
                ),
        )
    }
}

#[gpui::test]
fn pane_splitter_activates_on_pointer_press(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    let presses = Rc::new(Cell::new(0));
    let window = cx.open_window(size(px(240.0), px(80.0)), {
        let presses = presses.clone();
        move |_, _| SplitterHarness { presses }
    });
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let handle = visual
        .debug_bounds("test-splitter")
        .expect("splitter hit target should render");
    visual.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(handle.center(), MouseButton::Left, Modifiers::default());
    let pane = visual
        .debug_bounds("splitter-pane")
        .expect("splitter parent pane should render");
    assert_eq!(handle.size.width, px(5.0));
    assert!(handle.size.height > px(10.0));
    assert_eq!(handle.center().x, pane.left());
    assert_eq!(presses.get(), 1);
}

struct HiddenLineSplitterHarness {
    presses: Rc<Cell<usize>>,
}

impl Render for HiddenLineSplitterHarness {
    fn render(&mut self, _: &mut gpui::Window, _: &mut Context<Self>) -> impl IntoElement {
        let presses = self.presses.clone();
        div().size_full().p(px(8.0)).child(
            div()
                .id("hidden-splitter-pane")
                .debug_selector(|| "hidden-splitter-pane".into())
                .size_full()
                .relative()
                .child(
                    pane_splitter(Theme::light(), "hidden-splitter", Axis::Horizontal)
                        .show_line(false)
                        .trailing()
                        .debug_selector("hidden-splitter")
                        .on_mouse_down(move |_, _, _| {
                            presses.set(presses.get() + 1);
                        }),
                ),
        )
    }
}

#[gpui::test]
fn pane_splitter_without_idle_line_still_exposes_a_hit_target(cx: &mut TestAppContext) {
    cx.update(crate::theme::Theme::init);
    let presses = Rc::new(Cell::new(0));
    let window = cx.open_window(size(px(240.0), px(80.0)), {
        let presses = presses.clone();
        move |_, _| HiddenLineSplitterHarness { presses }
    });
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let handle = visual
        .debug_bounds("hidden-splitter")
        .expect("hidden-line splitter should still render a hit target");
    visual.simulate_mouse_down(handle.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(handle.center(), MouseButton::Left, Modifiers::default());
    let pane = visual
        .debug_bounds("hidden-splitter-pane")
        .expect("hidden-line splitter parent pane should render");
    assert_eq!(handle.size.width, px(5.0));
    assert!(handle.size.height > px(10.0));
    assert_eq!(handle.center().x, pane.right());
    assert_eq!(presses.get(), 1);
}
