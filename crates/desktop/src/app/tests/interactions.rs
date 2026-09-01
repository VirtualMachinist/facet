use super::*;

#[gpui::test]
fn middle_clicking_a_request_tab_closes_it(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let first = workspace.requests()[0].key();
    let second = workspace.requests()[1].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(first, cx);
            view.select_request(second, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let tab = visual
        .debug_bounds("request-tab-label")
        .expect("active request tab should render");
    visual.simulate_mouse_down(tab.center(), MouseButton::Middle, Modifiers::default());
    visual.simulate_mouse_up(tab.center(), MouseButton::Middle, Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let (tabs, active, selected) = window
        .update(cx, |view, _, _| {
            (
                view.shell.tabs().to_vec(),
                view.shell.active_tab(),
                view.selected_tree_item,
            )
        })
        .expect("test window should remain open");
    assert_eq!(tabs, vec![first]);
    assert_eq!(active, Some(first));
    assert_eq!(selected, Some(WorkspaceItemRef::Request(first)));
}

#[gpui::test]
fn request_tab_context_menu_closes_other_tabs_and_selects_its_target(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let first = workspace.requests()[0].key();
    let second = workspace.requests()[1].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(first, cx);
            view.select_request(second, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let tab = visual
        .debug_bounds("request-tab-label")
        .expect("active request tab should render");
    window
        .update(cx, |view, _, cx| {
            view.open_tab_context_menu(first, tab.center(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    let menu_target = window
        .update(cx, |view, _, _| {
            view.transient
                .tab_context_menu
                .as_ref()
                .map(|menu| menu.target)
        })
        .expect("test window should remain open");
    assert_eq!(menu_target, Some(first));

    window
        .update(cx, |view, window, cx| {
            view.request_close_other_tabs(first, window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let (tabs, active, selected) = window
        .update(cx, |view, _, _| {
            (
                view.shell.tabs().to_vec(),
                view.shell.active_tab(),
                view.selected_tree_item,
            )
        })
        .expect("test window should remain open");
    assert_eq!(tabs, vec![first]);
    assert_eq!(active, Some(first));
    assert_eq!(selected, Some(WorkspaceItemRef::Request(first)));
}

#[gpui::test]
fn tree_context_menu_duplicate_restores_tree_focus_for_delete_shortcut(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("context-duplicate-focus");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let request = workspace.request_key("items/0").unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_tree_item(WorkspaceItemRef::Request(request), cx);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let row = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("tree-row-items/0")
            .expect("request row should render")
    };
    window
        .update(cx, |view, _, cx| {
            view.open_tree_context_menu(WorkspaceItemRef::Request(request), row.center(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let duplicate = visual
            .debug_bounds("tree-context-duplicate-0")
            .expect("duplicate menu item should render");
        visual.simulate_click(duplicate.center(), Modifiers::default());
        visual.run_until_parked();
    }
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            assert_eq!(window.focused(cx), Some(view.tree_focus_handle.clone()));
            let selected = view
                .selected_tree_item
                .expect("duplicated request should be selected");
            assert_ne!(selected, WorkspaceItemRef::Request(request));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            assert!(matches!(
                selected,
                WorkspaceItemRef::Request(key) if loaded.workspace().request(key).is_some()
            ));
        })
        .expect("test window should remain open");

    cx.simulate_keystrokes(window.into(), tree_delete_shortcut());
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::Delete { .. })
            ));
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn tree_context_menu_duplicate_keeps_keyboard_rename_dialog_open_after_reconcile(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("context-duplicate-rename-focus");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let request = workspace.request_key("items/0").unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_tree_item(WorkspaceItemRef::Request(request), cx);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let row = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("tree-row-items/0")
            .expect("request row should render")
    };
    window
        .update(cx, |view, _, cx| {
            view.open_tree_context_menu(WorkspaceItemRef::Request(request), row.center(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let duplicate = visual
            .debug_bounds("tree-context-duplicate-0")
            .expect("duplicate menu item should render");
        visual.simulate_click(duplicate.center(), Modifiers::default());
        visual.run_until_parked();
    }
    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), rename_shortcut());
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            assert!(matches!(
                view.structure_dialog,
                Some(crate::structure_editor::StructureDialog {
                    mode: StructureDialogMode::Rename { .. },
                    ..
                })
            ));
            let fresh = probe_opencollection::load_workspace(&fixture).unwrap();
            let disk_baselines = fresh
                .requests()
                .iter()
                .filter_map(|located| {
                    fresh
                        .workspace()
                        .request(located.key())
                        .cloned()
                        .map(|request| (located.selector().to_owned(), request))
                })
                .collect();
            let selector_remaps =
                fresh
                    .requests()
                    .iter()
                    .map(|located| (located.selector().to_owned(), located.selector().to_owned()))
                    .chain(fresh.folders().iter().map(|located| {
                        (located.selector().to_owned(), located.selector().to_owned())
                    }))
                    .collect();
            view.apply_reconciled_workspace(
                ReconciledWorkspace {
                    workspace: fresh,
                    disk_baselines,
                    selector_remaps,
                },
                cx,
            );
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(matches!(
                view.structure_dialog,
                Some(crate::structure_editor::StructureDialog {
                    mode: StructureDialogMode::Rename { .. },
                    ..
                })
            ));
            assert_ne!(window.focused(cx), Some(view.tree_focus_handle.clone()));
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn platform_close_tab_hotkey_closes_the_active_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let first = workspace.requests()[0].key();
    let second = workspace.requests()[1].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(first, cx);
            view.select_request(second, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let shortcut = if cfg!(target_os = "macos") {
        "cmd-w"
    } else {
        "ctrl-w"
    };
    cx.simulate_keystrokes(window.into(), shortcut);
    cx.run_until_parked();

    let (tabs, active) = window
        .update(cx, |view, _, _| {
            (view.shell.tabs().to_vec(), view.shell.active_tab())
        })
        .expect("test window should remain open");
    assert_eq!(tabs, vec![first]);
    assert_eq!(active, Some(first));
}

#[gpui::test]
fn tab_and_shift_tab_move_focus_between_controls(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let input_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let url_bar = visual
            .debug_bounds("request-url-bar")
            .expect("request URL input should render");
        gpui::point(url_bar.right() - px(110.0), url_bar.center().y)
    };
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_click(input_point, Modifiers::default());
        visual.run_until_parked();
    }
    let input = window
        .update(cx, |_, window, cx| window.focused(cx))
        .expect("test window should remain open")
        .expect("clicking the request URL should focus its input");

    cx.simulate_keystrokes(window.into(), "tab");
    let next = window
        .update(cx, |_, window, cx| window.focused(cx))
        .expect("test window should remain open")
        .expect("Tab should focus the next control");
    assert_ne!(next, input);

    cx.simulate_keystrokes(window.into(), "shift-tab");
    let previous = window
        .update(cx, |_, window, cx| window.focused(cx))
        .expect("test window should remain open")
        .expect("Shift-Tab should focus the previous control");
    assert_eq!(previous, input);
}

#[gpui::test]
fn opening_many_request_tabs_scrolls_to_the_active_tab(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = large_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace =
        probe_opencollection::load_workspace(&fixture).expect("large fixture should load");
    let keys: Vec<_> = workspace
        .requests()
        .iter()
        .take(12)
        .map(|request| request.key())
        .collect();
    assert!(keys.len() >= 12, "large fixture should have many requests");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            for key in &keys {
                view.select_request(*key, cx);
            }
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();

    let (offset, max_offset, child_count, last_visible) = window
        .update(cx, |view, _, _| {
            let last = view.tab_bar_scroll.children_count().saturating_sub(1);
            let viewport = view.tab_bar_scroll.bounds();
            let offset = view.tab_bar_scroll.offset();
            let last_visible = view
                .tab_bar_scroll
                .bounds_for_item(last)
                .is_some_and(|bounds| {
                    bounds.right() + offset.x <= viewport.right() + px(1.0)
                        && bounds.left() + offset.x >= viewport.left() - viewport.size.width
                });
            (
                offset,
                view.tab_bar_scroll.max_offset(),
                view.tab_bar_scroll.children_count(),
                last_visible,
            )
        })
        .expect("test window should remain open");

    assert!(
        child_count >= 12,
        "tab strip should track opened request tabs, got {child_count}"
    );
    assert!(
        max_offset.x > px(0.0),
        "opening many tabs should overflow the tab bar, max_offset={max_offset:?}"
    );
    assert!(
        offset.x < px(0.0),
        "tab bar should scroll right to reveal the newest tab, offset={offset:?}"
    );
    assert!(
        last_visible,
        "the newly opened tab should be visible in the tab bar"
    );
}

#[gpui::test]
fn hovering_a_request_tab_shows_the_full_label_tooltip(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = large_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace =
        probe_opencollection::load_workspace(&fixture).expect("large fixture should load");
    let keys: Vec<_> = workspace
        .requests()
        .iter()
        .take(12)
        .map(|request| request.key())
        .collect();
    assert!(keys.len() >= 12, "large fixture should have many requests");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            for key in &keys {
                view.select_request(*key, cx);
            }
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.update(|window, cx| {
        window.simulate_next_frame(cx);
    });
    cx.run_until_parked();

    let tab = visual
        .debug_bounds("request-tab-label")
        .expect("active request tab should render");
    hover_and_wait(cx, window, tab.center());

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(
        visual.debug_bounds("request-tab-tooltip-popup").is_some(),
        "hovering a request tab should show its full label tooltip"
    );
    assert!(
        visual.debug_bounds("request-tab-tooltip-method").is_some(),
        "the request tab tooltip should include the request method"
    );
}
