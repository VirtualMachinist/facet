use super::*;

#[gpui::test]
fn structural_move_remaps_tabs_and_preserves_dirty_drafts(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("move");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let request = workspace.request_key("items/0").unwrap();
    let folder = workspace.folder_key("items/1").unwrap();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request, cx);
            view.shell.collapse_folder(folder);
            view.edit_request(
                request,
                |request| request.url = Some("https://local.example/dirty".to_owned()),
                cx,
            );
            view.apply_structure(
                probe_opencollection::StructureOperation::MoveRequest {
                    selector: "items/0".to_owned(),
                    parent: Some("items/1".to_owned()),
                    index: Some(1),
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            let moved = loaded.request_key("items/0/items/1").unwrap();
            let request = loaded.workspace().request(moved).unwrap();
            assert_eq!(request.url.as_deref(), Some("https://local.example/dirty"));
            assert!(view.persistence.is_dirty(moved, request));
            assert_eq!(view.shell.active_tab(), Some(moved));
            assert!(view.shell.tabs().contains(&moved));
            let remapped_folder = loaded.folder_key("items/0").unwrap();
            assert!(!view.shell.folder_is_expanded(remapped_folder));
        })
        .unwrap();

    let disk = probe_opencollection::load_workspace(&fixture).unwrap();
    let persisted = disk
        .workspace()
        .request(disk.request_key("items/0/items/1").unwrap())
        .unwrap();
    assert_ne!(
        persisted.url.as_deref(),
        Some("https://local.example/dirty"),
        "structural moves must not silently save an unrelated dirty draft"
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn creating_root_request_without_selection_selects_opens_and_reveals_it(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_large_fixture("create-root-request-selection");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.selected_tree_item = None;
            view.apply_structure(
                probe_opencollection::StructureOperation::CreateRequest {
                    parent: None,
                    index: None,
                    name: "Created Root".to_owned(),
                    method: Some("GET".to_owned()),
                    url: None,
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();

    let created_selector = window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            let created = loaded
                .requests()
                .iter()
                .find_map(|located| {
                    let request = loaded.workspace().request(located.key())?;
                    (request.metadata.name.as_deref() == Some("Created Root"))
                        .then(|| located.key())
                })
                .expect("created request should exist");
            assert_eq!(
                view.selected_tree_item,
                Some(WorkspaceItemRef::Request(created))
            );
            assert_eq!(view.shell.active_tab(), Some(created));
            loaded.request_selector(created).unwrap().to_owned()
        })
        .unwrap();
    assert_eq!(created_selector, "items/1001");
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual
        .debug_bounds("tree-row-items/1001")
        .expect("created request should be revealed in the tree");

    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn creating_request_in_selected_folder_selects_child_and_expands_parent(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("create-folder-child-selection");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let folder = workspace.folder_key("items/0").unwrap();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_tree_item(WorkspaceItemRef::Folder(folder), cx);
            view.shell.collapse_folder(folder);
            view.rebuild_visible_tree_rows();
            view.apply_structure(
                probe_opencollection::StructureOperation::CreateRequest {
                    parent: Some("items/0".to_owned()),
                    index: None,
                    name: "Created Child".to_owned(),
                    method: Some("GET".to_owned()),
                    url: None,
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            let created = loaded.request_key("items/0/items/1").unwrap();
            let folder = loaded.folder_key("items/0").unwrap();
            assert_eq!(
                view.selected_tree_item,
                Some(WorkspaceItemRef::Request(created))
            );
            assert_eq!(view.shell.active_tab(), Some(created));
            assert!(view.shell.folder_is_expanded(folder));
            assert!(
                view.visible_tree_rows
                    .iter()
                    .any(|row| row.item == WorkspaceItemRef::Request(created)),
                "created child should be visible after expanding its parent"
            );
        })
        .unwrap();

    fs::remove_file(fixture).unwrap();
}

fn simulate_tree_drag(
    visual: &mut VisualTestContext,
    from: gpui::Bounds<gpui::Pixels>,
    to: gpui::Point<gpui::Pixels>,
) {
    visual.simulate_mouse_down(from.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        point(from.center().x + px(8.0), from.center().y + px(8.0)),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn tree_drag_moves_a_request_into_a_folder(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("tree-drag-move");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let source = visual
        .debug_bounds("tree-row-items/0")
        .expect("request row should render");
    let folder = visual
        .debug_bounds("tree-row-items/1")
        .expect("folder row should render");
    simulate_tree_drag(&mut visual, source, folder.center());
    visual.run_until_parked();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            assert!(loaded.request_key("items/0/items/1").is_some());
            assert!(loaded.folder_key("items/0").is_some());
            assert!(loaded.request_key("items/0").is_none());
        })
        .unwrap();

    let disk = probe_opencollection::load_workspace(&fixture).unwrap();
    assert!(disk.request_key("items/0/items/1").is_some());
    assert!(disk.folder_key("items/0").is_some());
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn tree_drag_reorders_a_folder_before_its_sibling(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("tree-drag-reorder");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let folder = visual
        .debug_bounds("tree-row-items/1")
        .expect("folder row should render");
    let request = visual
        .debug_bounds("tree-row-items/0")
        .expect("request row should render");
    simulate_tree_drag(
        &mut visual,
        folder,
        point(request.center().x, request.top() + px(2.0)),
    );
    visual.run_until_parked();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            let root = loaded.workspace().root_items();
            assert!(matches!(root[0], probe_core::WorkspaceItemRef::Folder(_)));
            assert!(matches!(root[1], probe_core::WorkspaceItemRef::Request(_)));
        })
        .unwrap();

    let disk = probe_opencollection::load_workspace(&fixture).unwrap();
    assert!(disk.folder_key("items/0").is_some());
    assert!(disk.request_key("items/1").is_some());
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn tree_drag_moves_a_nested_request_to_root_end(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("tree-drag-root-end");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let nested = visual
        .debug_bounds("tree-row-items/1/items/0")
        .expect("nested request row should render");
    simulate_tree_drag(
        &mut visual,
        nested,
        point(nested.center().x, nested.bottom() + px(48.0)),
    );
    visual.run_until_parked();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            let root = loaded.workspace().root_items();
            assert_eq!(root.len(), 3);
            assert!(matches!(root[2], probe_core::WorkspaceItemRef::Request(_)));
            assert!(loaded.request_key("items/2").is_some());
            let folder = loaded.folder_key("items/1").unwrap();
            assert!(
                loaded
                    .workspace()
                    .folder(folder)
                    .unwrap()
                    .children
                    .is_empty()
            );
        })
        .unwrap();

    let disk = probe_opencollection::load_workspace(&fixture).unwrap();
    assert!(disk.request_key("items/2").is_some());
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn tree_drag_rejects_dropping_a_folder_into_itself(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("tree-drag-invalid");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let folder = visual
        .debug_bounds("tree-row-items/1")
        .expect("folder row should render");
    simulate_tree_drag(&mut visual, folder, folder.center());
    visual.run_until_parked();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none(), "{:?}", toast_debug(view));
            let loaded = view.loaded_workspace.as_ref().unwrap();
            assert!(loaded.request_key("items/0").is_some());
            assert!(loaded.folder_key("items/1").is_some());
            assert!(view.toasts.is_empty(), "{:?}", toast_debug(view));
        })
        .unwrap();
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn failed_structure_edit_keeps_the_previous_workspace(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_structure_fixture("tree-drag-conflict");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    fs::write(
        &fixture,
        "opencollection: 1.0.0\ninfo:\n  name: changed\nbundled: true\nitems: []\n",
    )
    .unwrap();

    window
        .update(cx, |view, window, cx| {
            view.apply_structure(
                probe_opencollection::StructureOperation::ReorderFolder {
                    selector: "items/1".to_owned(),
                    index: 0,
                },
                window,
                cx,
            );
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.structure_task.is_none());
            assert!(
                view.toasts.iter().any(|(_, toast, _)| {
                    toast.intent == ToastIntent::Error
                        && toast
                            .message
                            .contains("Could not edit collection structure")
                }),
                "{:?}",
                toast_debug(view)
            );
            let loaded = view.loaded_workspace.as_ref().unwrap();
            assert!(loaded.request_key("items/0").is_some());
            assert!(loaded.folder_key("items/1").is_some());
        })
        .unwrap();
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn request_save_runs_in_background_and_clears_dirty_state(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("save");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| request.url = Some("https://saved.example/pets".to_owned()),
                cx,
            );
            assert!(
                view.persistence.is_dirty(
                    key,
                    view.loaded_workspace
                        .as_ref()
                        .unwrap()
                        .workspace()
                        .request(key)
                        .unwrap()
                )
            );
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let save = visual
        .debug_bounds("request-save")
        .expect("dirty request should show its save icon");
    let breadcrumb = visual
        .debug_bounds("request-breadcrumb")
        .expect("request breadcrumb should render");
    assert_eq!(
        save.right(),
        breadcrumb.right(),
        "save icon should be anchored to the breadcrumb's right edge"
    );
    visual.simulate_click(save.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let clean_save = visual
        .debug_bounds("request-save")
        .expect("save icon should remain visible when the request is clean");
    let breadcrumb = visual
        .debug_bounds("request-breadcrumb")
        .expect("request breadcrumb should remain visible");
    assert_eq!(clean_save.right(), breadcrumb.right());

    let (dirty, message) = window
        .update(cx, |view, _, _| {
            let request = view
                .loaded_workspace
                .as_ref()
                .unwrap()
                .workspace()
                .request(key)
                .unwrap();
            (view.persistence.is_dirty(key, request), toast_debug(view))
        })
        .unwrap();
    assert!(!dirty, "save failed: {message:?}");
    let reloaded = probe_opencollection::load_workspace(&fixture).unwrap();
    assert_eq!(
        reloaded
            .workspace()
            .request(reloaded.requests()[0].key())
            .unwrap()
            .url
            .as_deref(),
        Some("https://saved.example/pets")
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn save_shortcut_after_clicking_remove_query_row_persists_removal(cx: &mut TestAppContext) {
    assert_save_shortcut_after_clicking_remove_row_persists_removal(
        cx,
        "remove-query-row-shortcut-save",
        EditorSection::Query,
        "add-query-parameter",
        "remove-query-1",
        |request| {
            assert_eq!(request.query_parameters.len(), 1);
            assert_eq!(request.query_parameters[0].name, "limit");
        },
    );
}

#[gpui::test]
fn save_shortcut_after_clicking_remove_header_row_persists_removal(cx: &mut TestAppContext) {
    assert_save_shortcut_after_clicking_remove_row_persists_removal(
        cx,
        "remove-header-row-shortcut-save",
        EditorSection::Headers,
        "add-header",
        "remove-header-2",
        |request| {
            assert_eq!(request.headers.len(), 2);
            assert_eq!(request.headers[0].name, "Accept");
            assert_eq!(request.headers[1].name, "X-Debug");
        },
    );
}

#[gpui::test]
fn workspace_reload_preserves_running_request_execution(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("reload-running-execution");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);

            let old = view.loaded_workspace.as_ref().unwrap();
            let old_key = old.requests()[0].key();
            let (sender, mut receiver) = oneshot::channel();
            view.execution.begin(old_key, sender);

            let fresh = probe_opencollection::load_workspace(&fixture).unwrap();
            let selector_remaps = old
                .requests()
                .iter()
                .map(|located| (located.selector().to_owned(), located.selector().to_owned()))
                .collect::<BTreeMap<_, _>>();
            let key_remaps = request_key_remaps(old, &fresh, &selector_remaps);
            let baselines = fresh
                .requests()
                .iter()
                .filter_map(|located| {
                    fresh
                        .workspace()
                        .request(located.key())
                        .cloned()
                        .map(|request| (located.key(), request))
                })
                .collect::<Vec<_>>();
            let new_key = key_remaps[&old_key];

            view.install_reloaded_workspace(fresh, baselines, &key_remaps);

            assert!(receiver.try_recv().is_err());
            assert!(matches!(
                view.execution.response(new_key),
                Some(crate::execution::ResponseState::Running { .. })
            ));
            cx.notify();
        })
        .unwrap();

    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn saving_after_removing_empty_query_parameter_during_in_flight_save_clears_dirty_state(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("save-empty-query-removal");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| {
                    request.query_parameters.push(QueryParameter {
                        name: String::new(),
                        value: String::new(),
                        disabled: false,
                    });
                },
                cx,
            );
            view.save_active_request(window, cx);
            view.edit_request(
                key,
                |request| {
                    request.query_parameters.retain(|parameter| {
                        !parameter.name.is_empty() || !parameter.value.is_empty()
                    });
                },
                cx,
            );
            assert!(
                view.persistence.is_dirty(
                    key,
                    view.loaded_workspace
                        .as_ref()
                        .unwrap()
                        .workspace()
                        .request(key)
                        .unwrap()
                ),
                "removing a saved empty parameter should make the request dirty before save"
            );
            view.save_active_request(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    let (dirty, message) = window
        .update(cx, |view, _, _| {
            let request = view
                .loaded_workspace
                .as_ref()
                .unwrap()
                .workspace()
                .request(key)
                .unwrap();
            (view.persistence.is_dirty(key, request), toast_debug(view))
        })
        .unwrap();
    assert!(!dirty, "save failed: {message:?}");
    let reloaded = probe_opencollection::load_workspace(&fixture).unwrap();
    let request = reloaded
        .workspace()
        .request(reloaded.requests()[0].key())
        .unwrap();
    assert_eq!(request.query_parameters.len(), 1);
    assert_eq!(request.query_parameters[0].name, "limit");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn discarding_a_dirty_tab_restores_the_workspace_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let key = workspace.requests()[0].key();
    let original_url = workspace
        .workspace()
        .request(key)
        .and_then(|request| request.url.clone());

    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| request.url = Some("https://discarded.example".to_owned()),
                cx,
            );
            assert_eq!(view.dirty_keys(), vec![key]);

            view.discard_dirty_requests(&[key]);
            view.close_tab_now(key, cx);

            let request = view
                .loaded_workspace
                .as_ref()
                .unwrap()
                .workspace()
                .request(key)
                .unwrap();
            assert_eq!(request.url, original_url);
            assert!(view.dirty_keys().is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn unsaved_changes_use_the_custom_dialog_and_cancel_preserves_the_tab(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("enter-create-environment");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| request.url = Some("https://unsaved.example".to_owned()),
                cx,
            );
            view.request_close_tab(key, window, cx);
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::Unsaved { .. })
            ));
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let cancel = visual
        .debug_bounds("application-dialog-cancel")
        .expect("custom dialog Cancel action should be rendered");
    visual.simulate_click(cancel.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.application_dialog.is_none());
            assert!(view.shell.tabs().contains(&key));
            assert!(view.request_is_dirty(key));
        })
        .unwrap();
}

#[gpui::test]
fn enter_triggers_application_dialog_primary_action(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });

    window
        .update(cx, |view, window, cx| {
            view.show_application_dialog(ApplicationDialog::About, window, cx);
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::About)
            ));
        })
        .unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.application_dialog.is_none());
        })
        .unwrap();
}

#[gpui::test]
fn enter_triggers_create_environment_dialog_primary_action(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("enter-create-environment");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();

    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.open_create_environment_dialog(window, cx);
            if let Some(name) = view.create_environment_dialog.as_mut() {
                *name = "Staging".to_owned();
            }
            assert!(view.create_environment_dialog.is_some());
        })
        .unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.create_environment_dialog.is_none());
            assert!(
                view.loaded_workspace
                    .as_ref()
                    .unwrap()
                    .workspace()
                    .environments()
                    .iter()
                    .any(|environment| environment.name == "Staging")
            );
        })
        .unwrap();
}

#[gpui::test]
fn destructive_shortcut_triggers_application_dialog_destructive_action(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("destructive-shortcut");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();

    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| request.url = Some("https://discard.example".to_owned()),
                cx,
            );
            view.request_close_tab(key, window, cx);
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::Unsaved { .. })
            ));
        })
        .unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), destructive_dialog_shortcut());
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.application_dialog.is_none());
            assert!(!view.shell.tabs().contains(&key));
            assert!(!view.request_is_dirty(key));
        })
        .unwrap();
}

#[gpui::test]
fn application_dialogs_queue_without_repeating_the_same_filesystem_conflict(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let conflict_path = PathBuf::from("collection.yml");

    window
        .update(cx, |view, window, cx| {
            view.show_application_dialog(
                ApplicationDialog::Delete {
                    kind: probe_opencollection::ItemKind::Request,
                    selector: "products/list".to_owned(),
                    name: "List products".to_owned(),
                    detail: "This cannot be undone.".to_owned(),
                },
                window,
                cx,
            );
            for detail in ["First conflict", "Repeated conflict"] {
                view.show_application_dialog(
                    ApplicationDialog::FilesystemConflict {
                        path: Some(conflict_path.clone()),
                        detail: detail.to_owned(),
                    },
                    window,
                    cx,
                );
            }

            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::Delete { .. })
            ));
            assert_eq!(view.pending_application_dialogs.len(), 1);

            view.handle_application_dialog_action(ApplicationDialogAction::Cancel, window, cx);

            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::FilesystemConflict { .. })
            ));
            assert!(view.pending_application_dialogs.is_empty());
        })
        .unwrap();
}

#[gpui::test]
fn save_conflict_keeps_the_request_dirty_and_visible(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture("conflict");
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(key, cx);
            view.edit_request(
                key,
                |request| request.url = Some("https://local.example".to_owned()),
                cx,
            );
            let mut external = fs::read_to_string(&fixture).unwrap();
            external.push_str("external: true\n");
            fs::write(&fixture, external).unwrap();
            view.save_active_request(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            let request = view
                .loaded_workspace
                .as_ref()
                .unwrap()
                .workspace()
                .request(key)
                .unwrap();
            assert!(view.persistence.is_dirty(key, request));
            assert!(view.toasts.iter().any(|(_, toast, _)| {
                toast.intent == ToastIntent::Error && toast.message.contains("externally modified")
            }));
            assert_eq!(request.url.as_deref(), Some("https://local.example"));
        })
        .unwrap();
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn recent_collection_in_sidebar_loads_the_workspace(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.session.recent_collections = vec![fixture.clone()];
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let recent = visual
        .debug_bounds("recent-collection-0")
        .expect("recent collection should be rendered");
    visual.simulate_click(recent.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let expected = fixture.canonicalize().expect("fixture should exist");
    let (actual, loading, message) = window
        .update(cx, |view, _, _| {
            (view.workspace_path.clone(), view.loading, toast_debug(view))
        })
        .expect("test window should remain open");
    assert_eq!(
        actual.as_deref(),
        Some(expected.as_path()),
        "loading={loading}, message={message:?}"
    );
}

#[gpui::test]
fn empty_sidebar_new_collection_creates_and_loads_a_workspace(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let destination_dir = std::env::temp_dir().join(format!(
        "probe-desktop-new-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&destination_dir).unwrap();
    let destination = destination_dir.join("pets.yml");
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let button = visual
        .debug_bounds("sidebar-new-collection")
        .expect("new collection button should be rendered");
    visual.simulate_click(button.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection({
        let destination = destination.clone();
        move |_| Some(destination)
    });
    cx.run_until_parked();

    let expected = destination
        .canonicalize()
        .expect("created collection should exist");
    let (actual, name, requests, message) = window
        .update(cx, |view, _, _| {
            (
                view.workspace_path.clone(),
                view.loaded_workspace
                    .as_ref()
                    .and_then(|loaded| loaded.workspace().metadata().name.clone()),
                view.loaded_workspace
                    .as_ref()
                    .map(|loaded| loaded.workspace().request_count()),
                toast_debug(view),
            )
        })
        .expect("test window should remain open");
    assert_eq!(
        actual.as_deref(),
        Some(expected.as_path()),
        "message={message:?}"
    );
    assert_eq!(name.as_deref(), Some("pets"));
    assert_eq!(requests, Some(0));
    fs::remove_dir_all(destination_dir).unwrap();
}

#[gpui::test]
fn empty_sidebar_import_menu_lists_postman_and_yaak(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let trigger = visual
        .debug_bounds("sidebar-import-from")
        .expect("empty sidebar should include Import From");
    visual.simulate_click(trigger.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    visual
        .debug_bounds("sidebar-import-postman")
        .expect("provider menu should include Postman");
    visual
        .debug_bounds("sidebar-import-yaak")
        .expect("provider menu should include Yaak");
}

#[gpui::test]
fn workspace_switcher_includes_new_collection(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let trigger = visual
        .debug_bounds("workspace-switcher-trigger")
        .expect("workspace switcher trigger should render");
    visual.simulate_click(trigger.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    visual
        .debug_bounds("workspace-switcher-new")
        .expect("workspace switcher should include New Collection");
    visual
        .debug_bounds("workspace-switcher-open")
        .expect("workspace switcher should include Open Collection");
    let import = visual
        .debug_bounds("workspace-switcher-import-from")
        .expect("workspace switcher should include Import From");
    visual.simulate_click(import.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    let submenu = visual
        .debug_bounds("workspace-switcher-import-popup")
        .expect("workspace switcher import popup should render");
    let postman = visual
        .debug_bounds("workspace-switcher-import-postman")
        .expect("workspace switcher import menu should include Postman");
    visual
        .debug_bounds("workspace-switcher-import-yaak")
        .expect("workspace switcher import menu should include Yaak");
    assert!(
        submenu.center().x > import.center().x,
        "import submenu should open beside its trigger: trigger={import:?}, submenu={submenu:?}"
    );
    assert_eq!(
        submenu.left(),
        import.right(),
        "the submenu should meet the trigger edge while its surfaces overlap"
    );
    assert_eq!(
        postman.top(),
        import.top(),
        "the first submenu row should align with its trigger row"
    );
}

#[gpui::test]
fn large_sidebar_virtualizes_rows_and_reveals_the_restored_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = large_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace =
        probe_opencollection::load_workspace(&fixture).expect("large fixture should load");
    let last = workspace
        .requests()
        .last()
        .expect("request should exist")
        .key();
    let last_selector = workspace
        .request_selector(last)
        .expect("request should have a selector")
        .to_owned();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.session.open_tabs = vec![last_selector.clone()];
            view.session.active_tab = Some(last_selector);
            view.restore_shell_state(cx);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let (total_rows, rendered_rows) = window
        .update(cx, |view, _, _| {
            (view.visible_tree_rows.len(), view.rendered_sidebar_rows)
        })
        .expect("test window should remain open");
    assert!(total_rows >= 1_000);
    assert!(rendered_rows > 0);
    assert!(
        rendered_rows < total_rows,
        "virtualized sidebar rendered all {total_rows} rows"
    );
    window
        .update(cx, |view, _, _| {
            assert_eq!(
                view.selected_tree_item,
                Some(WorkspaceItemRef::Request(last))
            );
            let expected_index = view
                .visible_tree_rows
                .iter()
                .position(|row| row.item == WorkspaceItemRef::Request(last))
                .expect("active request should be visible in the tree");
            let scroll = view.tree_scroll.0.borrow();
            let sizes = scroll
                .last_item_size
                .expect("sidebar list should have completed layout");
            let row_count = view.visible_tree_rows.len() + 1;
            let row_height = sizes.contents.height / row_count as f32;
            let scroll_top = -scroll.base_handle.offset().y;
            let item_top = row_height * expected_index as f32;
            let item_bottom = item_top + row_height;
            assert!(
                item_top >= scroll_top - px(1.0)
                    && item_bottom <= scroll_top + sizes.item.height + px(1.0),
                "active request should be inside the sidebar viewport"
            );
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn sidebar_search_filters_tree_and_expands_collapsed_match_folders(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = nested_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let folder = workspace
        .folder_key("items/1")
        .expect("folder should exist");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.shell.collapse_folder(folder);
            view.rebuild_visible_tree_rows();
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual
        .debug_bounds("tree-search")
        .expect("sidebar search input should render");

    let collapsed_names = window
        .update(cx, |view, _, _| visible_tree_names(view))
        .expect("test window should remain open");
    assert_eq!(collapsed_names, ["Alpha", "Folder"]);

    window
        .update(cx, |view, _, cx| {
            view.set_tree_search("nstd".to_owned(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let (names, folder_expanded) = window
        .update(cx, |view, _, _| {
            (
                visible_tree_names(view),
                view.shell.folder_is_expanded(folder),
            )
        })
        .expect("test window should remain open");
    assert!(folder_expanded, "matching request should expand its folder");
    assert_eq!(names, ["Folder", "Nested"]);

    window
        .update(cx, |view, _, cx| {
            view.set_tree_search("fldr".to_owned(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let folder_only = window
        .update(cx, |view, _, _| visible_tree_names(view))
        .expect("test window should remain open");
    assert_eq!(folder_only, ["Folder", "Nested"]);
}

#[gpui::test]
fn restored_active_tab_highlights_matching_sidebar_request(cx: &mut TestAppContext) {
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
    let first_selector = workspace
        .request_selector(first)
        .expect("first request should have a selector")
        .to_owned();
    let second_selector = workspace
        .request_selector(second)
        .expect("second request should have a selector")
        .to_owned();

    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.session.open_tabs = vec![first_selector, second_selector.clone()];
            view.session.active_tab = Some(second_selector);
            view.restore_shell_state(cx);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert_eq!(view.shell.active_tab(), Some(second));
            assert_eq!(
                view.selected_tree_item,
                Some(WorkspaceItemRef::Request(second))
            );
        })
        .expect("test window should remain open");

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual
        .debug_bounds("request-tree-label")
        .expect("active sidebar request label should render");
}
