use super::*;

#[gpui::test]
fn environment_switcher_is_visible_without_a_selected_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            assert!(view.shell.active_tab().is_none());
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("request-environment-trigger").is_some());
}

#[gpui::test]
fn environment_switcher_includes_environment_actions(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let trigger = visual
        .debug_bounds("request-environment-trigger")
        .expect("environment switcher should render");
    visual.simulate_click(trigger.center(), Modifiers::default());
    visual.run_until_parked();
    visual
        .debug_bounds("request-environment-action-0")
        .expect("environment switcher should include Create environment");
    visual
        .debug_bounds("request-environment-action-1")
        .expect("environment switcher should include Manage environments");
}

#[gpui::test]
fn environment_manager_renders_editable_and_readonly_variable_fields(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("base".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("environment-manager-dialog")
            .expect("environment manager should render");
        visual
            .debug_bounds("environment-manager-parent-trigger")
            .expect("extends dropdown should render");
        visual
            .debug_bounds("environment-manager-variables")
            .expect("variable table should render");
        visual
            .debug_bounds("environment-manager-add")
            .expect("compact add-environment control should render");
        assert!(
            visual.debug_bounds("environment-manager-delete").is_none(),
            "delete should live in the environment context menu, not the sidebar"
        );
        visual
            .debug_bounds("environment-manager-add-variable")
            .expect("inline add-variable action should render");
        assert!(
            visual
                .debug_bounds("environment-manager-save-status")
                .is_none(),
            "save status should not render in the environment manager footer"
        );
        visual
            .debug_bounds("environment-variable-value-host")
            .expect("string values should remain editable");
        visual
            .debug_bounds("environment-variable-variant-tenant")
            .expect("direct selectable-variant values should render as read-only");
        assert!(
            visual
                .debug_bounds("environment-variable-value-tenant")
                .is_none(),
            "direct selectable-variant values must not use an editable input"
        );
    }

    window
        .update(cx, |view, _, cx| {
            view.select_environment_manager_environment("development", cx);
            let dialog = view
                .environment_manager_dialog
                .as_mut()
                .expect("manager should remain open");
            assert_eq!(dialog.original_name, "development");
            dialog
                .draft
                .variables
                .push(EnvironmentVariable::Plain(Variable {
                    name: Some("retries".to_owned()),
                    value: Some(VariableValueSet::Single(VariableValue::Typed {
                        kind: probe_core::VariableValueType::Number,
                        data: "3".to_owned(),
                    })),
                    disabled: false,
                }));
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual
        .debug_bounds("environment-variable-name-host")
        .expect("direct variable names should be editable");
    assert!(
        visual
            .debug_bounds("environment-variable-name-baseUrl")
            .is_none(),
        "inherited variable names should remain read-only"
    );
    visual
        .debug_bounds("environment-variable-value-host")
        .expect("string values should remain editable");
    visual
        .debug_bounds("environment-variable-value-retries")
        .expect("typed single values should remain editable");
    visual
        .debug_bounds("environment-variable-variant-tenant")
        .expect("inherited selectable-variant values should render as read-only");
    assert!(
        visual
            .debug_bounds("environment-variable-value-tenant")
            .is_none(),
        "inherited selectable-variant values must not use an editable input"
    );
    visual
        .debug_bounds("environment-manager-dirty")
        .expect("unsaved environment changes should show a dirty indicator");
}

#[gpui::test]
fn environment_manager_protects_dirty_draft_and_restores_create_focus(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            assert_eq!(
                window.focused(cx),
                Some(view.environment_manager_dialog_focus.clone())
            );
            view.open_create_environment_dialog(window, cx);
            assert!(view.create_environment_dialog.is_some());
            assert_eq!(
                window.focused(cx),
                Some(view.create_environment_dialog_focus.clone())
            );
            view.close_create_environment_dialog(window, cx);
            assert!(view.create_environment_dialog.is_none());
            assert_eq!(
                window.focused(cx),
                Some(view.environment_manager_dialog_focus.clone())
            );
            let dialog = view
                .environment_manager_dialog
                .as_mut()
                .expect("manager should remain open");
            dialog.draft.name = "renamed-development".to_owned();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let add = visual
            .debug_bounds("environment-manager-add")
            .expect("add-environment control should render");
        visual.simulate_click(add.center(), Modifiers::default());
        visual.run_until_parked();
    }

    window
        .update(cx, |view, window, cx| {
            assert!(view.create_environment_dialog.is_none());
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.draft.name.as_str()),
                Some("renamed-development")
            );
            view.open_create_environment_dialog(window, cx);
            assert!(view.create_environment_dialog.is_none());
            assert!(
                has_active_toast(
                    view,
                    ToastIntent::Error,
                    "Save or discard unsaved environment changes first."
                ),
                "{:?}",
                toast_debug(view)
            );
            view.create_named_environment("staging".to_owned(), window, cx);
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.draft.name.as_str()),
                Some("renamed-development")
            );
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn environment_manager_validation_errors_are_scoped_and_dismissible(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.show_toast(ToastIntent::Error, "App-level error", cx);
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should open")
                .draft
                .name = "  ".to_owned();
            view.save_environment_manager_dialog(window, cx);
            assert!(
                has_active_toast(
                    view,
                    ToastIntent::Error,
                    "Environment and variable names are required."
                ),
                "{:?}",
                toast_debug(view)
            );
            assert!(
                has_active_toast(view, ToastIntent::Error, "App-level error"),
                "{:?}",
                toast_debug(view)
            );
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    visual.run_until_parked();
    let close = visual
        .debug_bounds("toast-close-1")
        .expect("the validation toast should expose a close action");
    visual.simulate_click(close.center(), Modifiers::default());
    visual.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.environment_dialog_error.is_none());
            assert!(
                !has_active_toast(
                    view,
                    ToastIntent::Error,
                    "Environment and variable names are required."
                ),
                "{:?}",
                toast_debug(view)
            );
            assert!(
                has_active_toast(view, ToastIntent::Error, "App-level error"),
                "{:?}",
                toast_debug(view)
            );
            assert!(view.environment_manager_dialog.is_some());
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn environment_manager_routes_blocked_save_and_create_failures_to_its_error(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);

            view.pending_environment_saves
                .insert(("development".to_owned(), "host".to_owned()));
            view.save_environment_manager_dialog(window, cx);
            assert!(
                has_active_toast(
                    view,
                    ToastIntent::Error,
                    "Wait for the current save to finish."
                ),
                "{:?}",
                toast_debug(view)
            );
            view.pending_environment_saves.clear();

            view.environment_manager_dialog
                .as_mut()
                .expect("manager should remain open")
                .draft
                .name = "base".to_owned();
            view.save_environment_manager_dialog(window, cx);
            assert!(
                has_active_toast(view, ToastIntent::Error, "Could not save environment:"),
                "{:?}",
                toast_debug(view)
            );

            view.environment_manager_dialog
                .as_mut()
                .expect("manager should remain open")
                .draft
                .name = "development".to_owned();
            view.open_create_environment_dialog(window, cx);
            *view
                .create_environment_dialog
                .as_mut()
                .expect("create dialog should open") = "base".to_owned();
            view.submit_create_environment_dialog(window, cx);
            assert!(view.create_environment_dialog.is_some());
            assert!(
                has_active_toast(view, ToastIntent::Error, "Could not create environment:"),
                "{:?}",
                toast_debug(view)
            );
        })
        .expect("test window should be open");
}

#[gpui::test]
fn environment_dialog_auto_dismisses_errors_when_their_condition_resolves(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should open")
                .draft
                .name = "  ".to_owned();
            view.save_environment_manager_dialog(window, cx);
            view.apply_environment_manager_draft(cx, |dialog| {
                dialog.draft.name = "development".to_owned();
            });
        })
        .expect("test window should be open");
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.environment_dialog_error.is_none());
        })
        .expect("test window should remain open");

    window
        .update(cx, |view, window, cx| {
            view.pending_environment_saves
                .insert(("development".to_owned(), "host".to_owned()));
            view.save_environment_manager_dialog(window, cx);
            view.pending_environment_saves.clear();
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.environment_dialog_error.is_none());
        })
        .expect("test window should remain open");

    window
        .update(cx, |view, window, cx| {
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should remain open")
                .draft
                .name = "base".to_owned();
            view.save_environment_manager_dialog(window, cx);
            view.apply_environment_manager_draft(cx, |dialog| {
                dialog.draft.name = "development".to_owned();
            });
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(
                has_active_toast(view, ToastIntent::Error, "Could not save environment:"),
                "{:?}",
                toast_debug(view)
            );
            view.open_create_environment_dialog(window, cx);
            view.submit_create_environment_dialog(window, cx);
            *view
                .create_environment_dialog
                .as_mut()
                .expect("create dialog should remain open") = "staging".to_owned();
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.environment_dialog_error.is_none());
            assert_eq!(view.create_environment_dialog.as_deref(), Some("staging"));
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn environment_manager_saves_plain_variables_and_parent(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-save")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            let dialog = view
                .environment_manager_dialog
                .as_mut()
                .expect("manager should open");
            dialog.draft.extends = None;
            dialog
                .draft
                .variables
                .push(EnvironmentVariable::Plain(Variable {
                    name: Some("region".to_owned()),
                    value: Some(VariableValueSet::Single(VariableValue::String(
                        "ap-southeast-2".to_owned(),
                    ))),
                    disabled: false,
                }));
            view.save_environment_manager_dialog(window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert!(
                has_active_toast(view, ToastIntent::Success, "Environment saved."),
                "{:?}",
                toast_debug(view)
            );
        })
        .expect("test window should remain open");
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    let development = reloaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .unwrap();
    assert_eq!(development.extends, None);
    assert!(development.variables.iter().any(|variable| matches!(
        variable,
        EnvironmentVariable::Plain(variable) if variable.name.as_deref() == Some("region")
    )));
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn platform_save_hotkey_saves_dirty_environment_manager(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-save-hotkey")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.apply_environment_manager_draft(cx, |dialog| {
                dialog.draft.extends = None;
            });
            assert!(!view.environment_manager_save_disabled());
        })
        .expect("test window should be open");
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), save_shortcut());
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert!(
                has_active_toast(view, ToastIntent::Success, "Environment saved."),
                "{:?}",
                toast_debug(view)
            );
            assert!(view.environment_manager_save_disabled());
        })
        .expect("test window should remain open");
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    let development = reloaded
        .workspace()
        .environments()
        .iter()
        .find(|environment| environment.name == "development")
        .unwrap();
    assert_eq!(development.extends, None);
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn platform_save_hotkey_is_disabled_when_environment_manager_has_nothing_to_save(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-save-hotkey-clean")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request_key, cx);
            view.edit_request(
                request_key,
                |request| request.url = Some("https://dirty.example".to_owned()),
                cx,
            );
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            assert!(view.environment_manager_save_disabled());
            assert!(view.request_is_dirty(request_key));
        })
        .expect("test window should be open");
    cx.run_until_parked();

    cx.simulate_keystrokes(window.into(), save_shortcut());
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.environment_manager_dialog.is_some());
            assert!(view.environment_manager_save_disabled());
            assert!(view.request_is_dirty(request_key));
            assert!(
                !has_active_toast(view, ToastIntent::Success, "Environment saved."),
                "{:?}",
                toast_debug(view)
            );
            assert!(
                !has_active_toast(view, ToastIntent::Success, "Request saved."),
                "{:?}",
                toast_debug(view)
            );
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn platform_save_hotkey_is_disabled_while_environment_manager_save_is_busy(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-save-hotkey-busy")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.apply_environment_manager_draft(cx, |dialog| {
                dialog.draft.extends = None;
            });
            view.save_environment_manager_dialog(window, cx);
            assert!(view.environment_save_task.is_some());
            assert!(view.environment_manager_save_disabled());
            window.dispatch_action(Box::new(SubmitEnvironmentManagerDialog), cx);
            assert!(
                !has_active_toast(
                    view,
                    ToastIntent::Error,
                    "Wait for the current save to finish."
                ),
                "{:?}",
                toast_debug(view)
            );
            assert!(view.environment_save_task.is_some());
        })
        .expect("test window should be open");
    cx.run_until_parked();
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_save_ignores_edits_made_while_busy(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-save-busy")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.save_environment_manager_dialog(window, cx);
            assert!(view.environment_save_task.is_some());
            view.apply_environment_manager_draft(cx, |dialog| {
                dialog.draft.name = "hijacked".to_owned();
            });
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should stay open during save")
                .draft
                .name = "hijacked".to_owned();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            let dialog = view
                .environment_manager_dialog
                .as_ref()
                .expect("manager should rebind to the saved environment");
            assert_eq!(dialog.original_name, "development");
            assert_eq!(dialog.draft.name, "development");
        })
        .expect("test window should remain open");
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .any(|environment| environment.name == "development")
    );
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .all(|environment| environment.name != "hijacked")
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_deletes_a_leaf_environment(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-delete")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.delete_environment("development".to_owned(), window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.original_name.as_str()),
                Some("base")
            );
            assert_eq!(
                window.focused(cx),
                Some(view.environment_manager_dialog_focus.clone())
            );
        })
        .expect("test window should remain open");
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .all(|environment| environment.name != "development")
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_delete_preserves_a_dirty_draft_for_another_environment(
    cx: &mut TestAppContext,
) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-delete-other")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.create_named_environment("staging".to_owned(), window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            view.select_environment_manager_environment("development", cx);
            let dialog = view
                .environment_manager_dialog
                .as_mut()
                .expect("manager should be editing development");
            assert_eq!(dialog.original_name, "development");
            dialog.draft.name = "renamed-development".to_owned();
            view.delete_environment("staging".to_owned(), window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            let dialog = view
                .environment_manager_dialog
                .as_ref()
                .expect("manager should keep the current draft");
            assert_eq!(dialog.original_name, "development");
            assert_eq!(dialog.draft.name, "renamed-development");
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_delete_selects_a_neighbor(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-delete-neighbor")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.create_named_environment("staging".to_owned(), window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            view.select_environment_manager_environment("development", cx);
            view.delete_environment("development".to_owned(), window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.original_name.as_str()),
                Some("staging")
            );
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_delete_of_the_last_environment_closes(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-delete-last")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.delete_environment("development".to_owned(), window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.original_name.as_str()),
                Some("base")
            );
            view.delete_environment("base".to_owned(), window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert!(view.environment_manager_dialog.is_none());
            assert_eq!(window.focused(cx), Some(view.focus_handle.clone()));
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_closes_when_the_workspace_resets(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let other = http_environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let other_workspace =
        probe_opencollection::load_workspace(&other).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.open_environment_manager_dialog(window, cx);
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should open")
                .draft
                .name = "renamed-development".to_owned();
            view.set_workspace(other, other_workspace);
            assert!(view.environment_manager_dialog.is_none());
            let reloaded =
                probe_opencollection::load_workspace(&fixture).expect("fixture should reload");
            view.set_workspace(fixture, reloaded);
            view.open_environment_manager_dialog(window, cx);
            view.close_workspace_now(cx);
            assert!(view.environment_manager_dialog.is_none());
        })
        .expect("test window should be open");
}

#[gpui::test]
fn environment_manager_rebinds_after_workspace_reload(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-reload")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            view.apply_reconciled_workspace(
                reconciled_workspace(
                    probe_opencollection::load_workspace(&fixture).expect("fixture should reload"),
                ),
                cx,
            );
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.original_name.as_str()),
                Some("development")
            );
            view.environment_manager_dialog
                .as_mut()
                .expect("manager should remain open")
                .draft
                .name = "renamed-development".to_owned();
            view.apply_reconciled_workspace(
                reconciled_workspace(
                    probe_opencollection::load_workspace(&fixture).expect("fixture should reload"),
                ),
                cx,
            );
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.draft.name.as_str()),
                Some("renamed-development")
            );
            assert!(view.toasts.is_empty(), "{:?}", toast_debug(view));
        })
        .expect("test window should be open");

    let mut changed = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let mut replacement = changed.workspace().environments()[1].clone();
    replacement.extends = None;
    let saved = changed
        .prepare_environment_replace("development", replacement)
        .unwrap()
        .execute()
        .unwrap();
    changed.complete_environment_replace(saved);

    window
        .update(cx, |view, _, cx| {
            view.apply_reconciled_workspace(reconciled_workspace(changed), cx);
            let dialog = view
                .environment_manager_dialog
                .as_ref()
                .expect("manager should rebind to disk");
            assert_eq!(dialog.original_name, "development");
            assert_eq!(dialog.draft.name, "development");
            assert_eq!(dialog.draft.extends, None);
            assert!(
                has_active_toast(
                    view,
                    ToastIntent::Error,
                    "This environment changed on disk. Unsaved environment edits were discarded."
                ),
                "{:?}",
                toast_debug(view)
            );
        })
        .expect("test window should remain open");

    let mut remaining =
        probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let saved = remaining
        .prepare_environment_delete("development")
        .unwrap()
        .execute()
        .unwrap();
    remaining.complete_environment_delete(saved);

    window
        .update(cx, |view, _, cx| {
            view.apply_reconciled_workspace(reconciled_workspace(remaining), cx);
            assert!(view.environment_manager_dialog.is_none());
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_manager_cancel_with_unsaved_changes_prompts(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
            let dialog = view
                .environment_manager_dialog
                .as_mut()
                .expect("manager should open");
            dialog.draft.name = "renamed-development".to_owned();
            view.request_close_environment_manager_dialog(window, cx);
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::UnsavedEnvironment)
            ));
            assert!(view.environment_manager_dialog.is_some());
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let cancel = visual
            .debug_bounds("application-dialog-cancel")
            .expect("unsaved environment warning should render Cancel");
        visual.simulate_click(cancel.center(), Modifiers::default());
        visual.run_until_parked();
    }

    window
        .update(cx, |view, window, cx| {
            assert!(view.application_dialog.is_none());
            assert_eq!(
                view.environment_manager_dialog
                    .as_ref()
                    .map(|dialog| dialog.draft.name.as_str()),
                Some("renamed-development")
            );
            view.request_close_environment_manager_dialog(window, cx);
            view.handle_application_dialog_action(ApplicationDialogAction::Discard, window, cx);
            assert!(view.application_dialog.is_none());
            assert!(view.environment_manager_dialog.is_none());
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn environment_manager_context_menu_deletes_an_environment(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("manager-context-delete")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.open_environment_manager_dialog(window, cx);
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let dialog = visual
            .debug_bounds("environment-manager-dialog")
            .expect("environment manager should render");
        window
            .update(cx, |view, _, cx| {
                view.open_environment_manager_context_menu(
                    "development".to_owned(),
                    dialog.center(),
                    cx,
                );
            })
            .expect("test window should remain open");
        visual.run_until_parked();
        let delete = visual
            .debug_bounds("environment-manager-delete")
            .expect("environment context menu should include Delete");
        visual.simulate_click(delete.center(), Modifiers::default());
        visual.run_until_parked();
    }

    window
        .update(cx, |view, _, _| {
            assert!(matches!(
                view.application_dialog.as_ref(),
                Some(ApplicationDialog::DeleteEnvironment { name, .. }) if name == "development"
            ));
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn create_environment_dialog_cannot_close_while_persistence_is_running(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("create-env-cancel-busy")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.open_create_environment_dialog(window, cx);
            *view
                .create_environment_dialog
                .as_mut()
                .expect("create dialog should open") = "staging".to_owned();
            view.submit_create_environment_dialog(window, cx);
            assert!(view.environment_save_task.is_some());

            view.close_create_environment_dialog(window, cx);
            assert!(view.create_environment_dialog.is_some());
            assert_eq!(
                window.focused(cx),
                Some(view.create_environment_dialog_focus.clone())
            );
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(view.environment_save_task.is_none());
            assert!(view.create_environment_dialog.is_none());
            assert_eq!(view.shell.selected_environment(), Some("staging"));
        })
        .expect("test window should remain open");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn creating_an_environment_from_the_switcher_persists_and_selects_it(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("create-env")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let trigger = visual
            .debug_bounds("request-environment-trigger")
            .expect("environment switcher should render");
        visual.simulate_click(trigger.center(), Modifiers::default());
        visual.run_until_parked();
        let create = visual
            .debug_bounds("request-environment-action-0")
            .expect("Create environment action should render");
        visual.simulate_click(create.center(), Modifiers::default());
        visual.run_until_parked();
    }
    cx.run_until_parked();

    window
        .update(cx, |view, window, cx| {
            assert!(view.create_environment_dialog.is_some());
            if let Some(name) = view.create_environment_dialog.as_mut() {
                *name = "staging".to_owned();
            }
            view.submit_create_environment_dialog(window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(
                view.environment_save_task.is_none(),
                "{:?}",
                toast_debug(view)
            );
            assert_eq!(view.shell.selected_environment(), Some("staging"));
            let loaded = view.loaded_workspace.as_ref().expect("workspace");
            assert!(
                loaded
                    .workspace()
                    .environments()
                    .iter()
                    .any(|environment| environment.name == "staging")
            );
        })
        .expect("test window should remain open");

    let reloaded = probe_opencollection::load_workspace(&fixture).expect("created env should load");
    assert!(
        reloaded
            .workspace()
            .environments()
            .iter()
            .any(|environment| environment.name == "staging")
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn create_environment_dialog_rejects_an_empty_name(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("create-env-empty")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.open_create_environment_dialog(window, cx);
            view.submit_create_environment_dialog(window, cx);
            assert!(
                has_active_toast(view, ToastIntent::Error, "Environment name is required."),
                "{:?}",
                toast_debug(view)
            );
            assert!(view.create_environment_dialog.is_some());
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual
        .debug_bounds("toast-0")
        .expect("validation error should render as a toast");
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn environment_selection_is_shared_when_opening_another_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
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
            view.shell
                .select_environment(Some("development".to_owned()));
            view.select_request(second, cx);
            assert_eq!(view.shell.selected_environment(), Some("development"));
        })
        .expect("test window should be open");
}

#[gpui::test]
fn environment_selection_is_restored_when_reopening_a_workspace(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request_key, cx);
            view.select_environment(Some("development".to_owned()), cx);
            view.close_workspace_now(cx);
        })
        .expect("test window should be open");

    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should reload");
    window
        .update(cx, |view, _, _| {
            view.set_workspace(fixture, workspace);
            assert_eq!(view.shell.selected_environment(), Some("development"));
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn environment_selection_is_remembered_per_workspace(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let first_fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let second_fixture = http_environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let first_workspace =
        probe_opencollection::load_workspace(&first_fixture).expect("fixture should load");
    let second_workspace =
        probe_opencollection::load_workspace(&second_fixture).expect("fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(first_fixture.clone(), first_workspace);
            view.select_environment(Some("development".to_owned()), cx);
            view.capture_selected_environment();
            view.set_workspace(second_fixture.clone(), second_workspace);
            view.select_environment(Some("local".to_owned()), cx);
            view.capture_selected_environment();
        })
        .expect("test window should be open");

    let first_workspace =
        probe_opencollection::load_workspace(&first_fixture).expect("fixture should reload");
    let second_workspace =
        probe_opencollection::load_workspace(&second_fixture).expect("fixture should reload");
    window
        .update(cx, |view, _, _| {
            view.set_workspace(first_fixture, first_workspace);
            assert_eq!(view.shell.selected_environment(), Some("development"));
            view.capture_selected_environment();
            view.set_workspace(second_fixture, second_workspace);
            assert_eq!(view.shell.selected_environment(), Some("local"));
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn missing_environment_is_not_restored(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = environment_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.session.remember_selected_environment(
                fixture.clone(),
                Some("missing-environment".to_owned()),
            );
            view.set_workspace(fixture, workspace);
            assert_eq!(view.shell.selected_environment(), None);
            cx.notify();
        })
        .expect("test window should be open");
}

#[gpui::test]
fn request_variables_render_inline_and_show_resolved_tooltips(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("tooltip")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request_key, cx);
            view.shell
                .select_environment(Some("development".to_owned()));
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let (variable_point, input_point, trigger_left) = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let variable = visual
            .debug_bounds("variable-hover-trigger")
            .expect("variable hover trigger should render");
        let url_bar = visual
            .debug_bounds("request-url-bar")
            .expect("request URL bar should render");
        (
            variable.center(),
            gpui::point(url_bar.right() - px(110.0), url_bar.center().y),
            variable.left(),
        )
    };
    hover_and_wait(cx, window, variable_point);
    let popup_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let popup = visual
            .debug_bounds("variable-input-tooltip-popup")
            .expect("hovered variable tooltip should render");
        assert!(
            (popup.left() - trigger_left).abs() < px(1.0),
            "tooltip left edge should align to the variable, popup={:?} trigger_left={:?}",
            popup,
            trigger_left
        );
        popup.center()
    };
    hover_and_wait(cx, window, popup_point);
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual
                .debug_bounds("variable-input-tooltip-popup")
                .is_some(),
            "tooltip should stay visible while moving from the variable onto it"
        );
        let value_input = visual
            .debug_bounds("variable-tooltip-value-input")
            .expect("tooltip value input should render");
        visual.simulate_click(value_input.center(), Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("variable-input-tooltip-popup")
                .is_some(),
            "tooltip should stay visible while interacting with its value field"
        );
    }
    window
        .update(cx, |view, window, cx| {
            view.update_environment_variable(
                "baseUrl",
                "https://changed.example".to_owned(),
                window,
                cx,
            );
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    let updated = window
        .update(cx, |view, _, _| {
            let environment = view.shell.selected_environment()?.to_owned();
            probe_core::resolve_environment(
                view.loaded_workspace.as_ref()?.workspace().environments(),
                &environment,
            )
            .ok()
            .and_then(|resolved| resolved.variable("baseUrl").map(str::to_owned))
        })
        .expect("test window should remain open");
    assert_eq!(updated.as_deref(), Some("https://changed.example"));
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    assert_eq!(
        probe_core::resolve_environment(reloaded.workspace().environments(), "development")
            .unwrap()
            .variable("baseUrl"),
        Some("https://changed.example")
    );
    let button_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("variable-tooltip-manage-environments")
            .expect("tooltip should include Manage environments")
            .center()
    };
    hover_and_wait(cx, window, button_point);
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_click(button_point, Modifiers::default());
        visual.run_until_parked();
    }
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("environment-manager-dialog")
            .expect("clicking Manage environments should open the environment manager");
        assert!(
            visual
                .debug_bounds("variable-input-tooltip-popup")
                .is_none(),
            "opening the environment manager should dismiss the variable tooltip"
        );
    }
    window
        .update(cx, |view, window, cx| {
            view.request_close_environment_manager_dialog(window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_click(input_point, Modifiers::default());
        visual.run_until_parked();
    }
    let select_all = if cfg!(target_os = "macos") {
        "cmd-a"
    } else {
        "ctrl-a"
    };
    cx.simulate_keystrokes(window.into(), select_all);
    cx.simulate_input(window.into(), "https://url.example");
    cx.run_until_parked();
    let edited_url = window
        .update(cx, |view, _, _| {
            view.active_request()
                .and_then(|request| request.url.clone())
        })
        .expect("test window should remain open");
    assert_eq!(edited_url.as_deref(), Some("https://url.example"));
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn missing_url_variable_tooltip_creates_the_variable(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("create-var")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request_key, cx);
            view.shell
                .select_environment(Some("development".to_owned()));
            view.edit_request(
                request_key,
                |request| request.url = Some("https://{{created}}/users".to_owned()),
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    let variable_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("variable-hover-trigger")
            .expect("missing variable hover trigger should render")
            .center()
    };
    hover_and_wait(cx, window, variable_point);
    let popup_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(
            visual
                .debug_bounds("variable-tooltip-create-hint")
                .is_some(),
            "missing variable tooltip should invite creating the variable"
        );
        visual
            .debug_bounds("variable-input-tooltip-popup")
            .expect("create-variable tooltip should render")
            .center()
    };
    hover_and_wait(cx, window, popup_point);
    let value_point = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("variable-tooltip-value-input")
            .expect("create-variable value input should render")
            .center()
    };
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_mouse_move(value_point, None, Modifiers::default());
        visual.simulate_click(value_point, Modifiers::default());
        visual.run_until_parked();
        assert!(
            visual
                .debug_bounds("variable-input-tooltip-popup")
                .is_some(),
            "create-variable tooltip should stay open while focusing its value field"
        );
    }
    cx.simulate_input(window.into(), "createdhost");
    cx.run_until_parked();
    let created = window
        .update(cx, |view, _, _| {
            let url = view
                .active_request()
                .and_then(|request| request.url.clone());
            assert_eq!(url.as_deref(), Some("https://{{created}}/users"));
            let environment = view.shell.selected_environment()?.to_owned();
            probe_core::resolve_environment(
                view.loaded_workspace.as_ref()?.workspace().environments(),
                &environment,
            )
            .ok()
            .and_then(|resolved| resolved.variable("created").map(str::to_owned))
        })
        .expect("test window should remain open");
    assert_eq!(created.as_deref(), Some("createdhost"));
    cx.run_until_parked();
    let reloaded = probe_opencollection::load_workspace(&fixture).expect("saved env should load");
    assert_eq!(
        probe_core::resolve_environment(reloaded.workspace().environments(), "development")
            .unwrap()
            .variable("created"),
        Some("createdhost")
    );
    fs::remove_file(fixture).unwrap();
}

#[gpui::test]
fn json_body_variables_show_resolved_tooltips(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_environment_fixture("body-tooltip")
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(request_key, cx);
            view.shell
                .select_environment(Some("development".to_owned()));
            view.request_editor.section = EditorSection::Body;
            view.edit_request(
                request_key,
                |request| {
                    request.body = Some(probe_core::RequestBody::Single(probe_core::Body::Raw(
                        probe_core::RawBody {
                            kind: probe_core::RawBodyKind::Json,
                            data: "{\n  \"tenant\": \"{{tenant}}\"\n}".to_owned(),
                        },
                    )));
                },
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();
    // Hits are placed after the editor reports its overlay origin on a later frame.
    window
        .update(cx, |_, _, cx| cx.notify())
        .expect("test window should remain open");
    cx.run_until_parked();

    let (variable_point, trigger_left) = {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let editor = visual
            .debug_bounds("request-body-editor")
            .expect("JSON body editor should render");
        let variable = visual
                .debug_bounds("body-variable-hover-trigger")
                .unwrap_or_else(|| {
                    panic!(
                        "body variable hover trigger should render inside the JSON editor, editor={editor:?}"
                    )
                });
        (variable.center(), variable.left())
    };
    hover_and_wait(cx, window, variable_point);
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let popup = visual
            .debug_bounds("variable-input-tooltip-popup")
            .expect("hovered JSON body variable tooltip should render");
        assert!(
            (popup.left() - trigger_left).abs() < px(8.0),
            "tooltip should appear near the body variable, popup={:?} trigger_left={:?}",
            popup,
            trigger_left
        );
        let value_input = visual
            .debug_bounds("variable-tooltip-value-input")
            .expect("tooltip value input should render");
        assert!(
            value_input.size.width > px(0.0),
            "resolved variable value should be visible"
        );
    }
    fs::remove_file(fixture).unwrap();
}
