use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{
    KeyDownEvent, KeyUpEvent, Keystroke, Modifiers, MouseButton, TestAppContext, VisualTestContext,
    point, px, size,
};
use probe_core::{
    EnvironmentVariable, HttpRequest, QueryParameter, Variable, VariableValue, VariableValueSet,
    WorkspaceItemRef,
};
use probe_http::{HttpResponse, ResponseHeader};
use probe_postman::{COLLECTION_VARIABLES_ENVIRONMENT, inspect_postman_source};
use probe_yaak::{ImportDiagnostic, ImportDiagnosticSeverity};
use tokio::sync::oneshot;

use super::{
    ApplicationDialog, ApplicationDialogAction, CloseImportSubmenu, DesktopMenu,
    IMPORT_DIAGNOSTIC_GROUP_LIMIT, ImportSource, OpenFileMenu, OpenImportSubmenu, PendingClose,
    PrettyRevealState, ProbeApp, SubmitEnvironmentManagerDialog, bind_platform_hotkeys,
    format_import_diagnostics, request_key_remaps,
};
use crate::{
    request_editor::{BodyEditorKind, EditorSection},
    response_inspector::InspectSelection,
    response_viewer::{RawBodyView, ResponseViewerTab},
    shell::PaneLayout,
    structure_editor::StructureDialogMode,
    synchronization::ReconciledWorkspace,
    theme::Theme,
    toast::ToastIntent,
};

fn toast_debug(view: &ProbeApp) -> Vec<String> {
    view.toasts
        .iter()
        .map(|(_, toast, status)| format!("{:?}: {} ({status:?})", toast.intent, toast.message))
        .collect()
}

fn has_active_toast(view: &ProbeApp, intent: ToastIntent, text: &str) -> bool {
    view.toasts.iter().any(|(_, toast, status)| {
        status != gpui_base::ToastTransitionStatus::Ending
            && toast.intent == intent
            && toast.message.contains(text)
    })
}

#[gpui::test]
fn toast_stack_renders_and_the_front_toast_can_be_dismissed(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.show_toast(ToastIntent::Info, "Copied", cx);
            view.show_toast(ToastIntent::Warning, "Needs attention", cx);
            view.show_toast(ToastIntent::Error, "Save failed", cx);
            assert_eq!(
                view.toasts
                    .iter()
                    .map(|(_, toast, _)| toast.intent)
                    .collect::<Vec<_>>(),
                vec![ToastIntent::Info, ToastIntent::Warning, ToastIntent::Error,]
            );
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    visual.run_until_parked();
    let collapsed_close = visual
        .debug_bounds("toast-close-2")
        .expect("the front toast should expose a visible close action");
    visual.simulate_mouse_move(collapsed_close.center(), None, Modifiers::default());
    visual.run_until_parked();
    let close = visual
        .debug_bounds("toast-close-2")
        .expect("the close action should remain visible when the stack expands");
    visual.simulate_click(close.center(), Modifiers::default());
    visual.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            assert_eq!(
                view.toasts
                    .iter()
                    .find(|(id, _, _)| *id == 2)
                    .map(|(_, _, status)| status),
                Some(gpui_base::ToastTransitionStatus::Ending),
                "close bounds: {close:?}"
            );
            view.close_workspace_now(cx);
            assert!(
                view.toasts.is_empty(),
                "closing a workspace should drop stale toasts: {:?}",
                toast_debug(view)
            );
        })
        .unwrap();
}

fn bundled_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase1-bundled.yml")
}

fn large_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase2-large-workspace.yml")
}

fn nested_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase16-bundled.yml")
}

fn postman_fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/postman")
        .join(name)
}

#[test]
fn about_dialog_reports_the_packaged_version() {
    let dialog = ApplicationDialog::About;

    assert_eq!(dialog.title(), "Probe");
    assert!(dialog.description().contains(env!("CARGO_PKG_VERSION")));
    assert_eq!(dialog.action_specs().unwrap()[0].label, "Done");
    assert_eq!(
        dialog.primary_action(),
        Some(ApplicationDialogAction::Cancel)
    );
    assert_eq!(dialog.destructive_action(), None);
}

#[test]
fn destructive_only_dialogs_do_not_take_enter_as_primary_action() {
    let dialog = ApplicationDialog::Delete {
        kind: probe_opencollection::ItemKind::Request,
        selector: "products/list".to_owned(),
        name: "List products".to_owned(),
        detail: "This cannot be undone.".to_owned(),
    };

    assert_eq!(dialog.primary_action(), None);
    assert_eq!(
        dialog.destructive_action(),
        Some(ApplicationDialogAction::Delete)
    );
}

#[test]
fn unsaved_environment_dialog_warns_before_discard() {
    let dialog = ApplicationDialog::UnsavedEnvironment;

    assert_eq!(dialog.title(), "Save changes to this environment?");
    assert_eq!(
        dialog.description(),
        "Unsaved changes will be lost if you discard them."
    );
    assert_eq!(dialog.primary_action(), Some(ApplicationDialogAction::Save));
    assert_eq!(
        dialog.destructive_action(),
        Some(ApplicationDialogAction::Discard)
    );
}

#[cfg(target_os = "macos")]
fn destructive_dialog_shortcut() -> &'static str {
    "cmd-backspace"
}

#[cfg(not(target_os = "macos"))]
fn destructive_dialog_shortcut() -> &'static str {
    "ctrl-delete"
}

#[cfg(target_os = "macos")]
fn tree_delete_shortcut() -> &'static str {
    "backspace"
}

#[cfg(not(target_os = "macos"))]
fn tree_delete_shortcut() -> &'static str {
    "delete"
}

#[cfg(target_os = "macos")]
fn rename_shortcut() -> &'static str {
    "cmd-e"
}

#[cfg(not(target_os = "macos"))]
fn rename_shortcut() -> &'static str {
    "f2"
}

#[cfg(target_os = "macos")]
fn save_shortcut() -> &'static str {
    "cmd-s"
}

#[cfg(not(target_os = "macos"))]
fn save_shortcut() -> &'static str {
    "ctrl-s"
}

#[cfg(target_os = "macos")]
fn find_shortcut() -> &'static str {
    "cmd-f"
}

#[cfg(not(target_os = "macos"))]
fn find_shortcut() -> &'static str {
    "ctrl-f"
}

fn assert_save_shortcut_after_clicking_remove_row_persists_removal(
    cx: &mut TestAppContext,
    fixture_name: &str,
    section: EditorSection,
    add_selector: &'static str,
    remove_selector: &'static str,
    assert_request: impl FnOnce(&HttpRequest),
) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = writable_bundled_fixture(fixture_name);
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture.clone(), workspace);
            view.select_request(key, cx);
            view.request_editor.section = section;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let add = visual
        .debug_bounds(add_selector)
        .expect("add row button should render");
    visual.simulate_click(add.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| view.save_active_request(window, cx))
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let remove = visual
        .debug_bounds(remove_selector)
        .expect("remove row button should render");
    visual.simulate_click(remove.center(), Modifiers::default());
    visual.run_until_parked();
    cx.simulate_keystrokes(window.into(), save_shortcut());
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
    assert_request(request);
    fs::remove_file(fixture).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn macos_system_menu_delegates_services_to_the_operating_system() {
    use gpui::{MenuItem, SystemMenuType};

    let menus = super::system_menus(PaneLayout::Horizontal);

    assert_eq!(
        menus
            .iter()
            .map(|menu| menu.name.to_string())
            .collect::<Vec<_>>(),
        ["Probe", "File", "Edit", "View", "Window"]
    );
    assert!(menus[0].items.iter().any(|item| {
        matches!(
            item,
            MenuItem::SystemMenu(menu) if menu.menu_type == SystemMenuType::Services
        )
    }));
    assert!(menus[1].items.iter().any(|item| {
        matches!(
            item,
            MenuItem::Submenu(menu)
                if menu.name == "Import From…"
                    && matches!(
                        menu.items.as_slice(),
                        [
                            MenuItem::Action { name: postman, .. },
                            MenuItem::Action { name: yaak, .. },
                        ] if postman == "Postman Export…" && yaak == "Yaak Export…"
                    )
        )
    }));
    assert!(matches!(
        menus[3].items.as_slice(),
        [
            MenuItem::Action { name, .. },
            MenuItem::Submenu(layout),
            MenuItem::Separator,
        ] if name == "Show/Hide Sidebar"
            && layout.name == "Editor Layout"
            && matches!(
                layout.items.as_slice(),
                [
                    MenuItem::Action { name: vertical, .. },
                    MenuItem::Action { name: horizontal, .. },
                ] if vertical == "Vertical" && horizontal == "Horizontal"
            )
    ));
    let MenuItem::Submenu(layout) = &menus[3].items[1] else {
        panic!("Editor Layout should be a submenu");
    };
    assert!(!layout.items[0].is_checked());
    assert!(layout.items[1].is_checked());
}

#[gpui::test]
fn view_menu_state_changes_are_persisted(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });

    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.toggle_sidebar(cx);
            view.set_pane_layout(PaneLayout::Horizontal, cx);

            assert!(view.shell.sidebar_collapsed);
            assert!(view.session.sidebar_collapsed);
            assert_eq!(view.shell.pane_layout, PaneLayout::Horizontal);
            assert!(view.session.horizontal_panes);
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn desktop_menu_action_opens_the_requested_menu(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    cx.run_until_parked();

    window
        .update(cx, |_, window, cx| {
            window.dispatch_action(Box::new(OpenFileMenu), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    assert_eq!(
        window
            .update(cx, |view, _, _| view.transient.desktop_menu_open)
            .expect("test window should remain open"),
        Some(DesktopMenu::File)
    );
}

fn visible_tree_names(view: &ProbeApp) -> Vec<String> {
    let Some(loaded) = &view.loaded_workspace else {
        return Vec::new();
    };
    view.visible_tree_rows
        .iter()
        .filter_map(|row| match row.item {
            WorkspaceItemRef::Request(key) => loaded
                .workspace()
                .request(key)
                .and_then(|request| request.metadata.name.clone()),
            WorkspaceItemRef::Folder(key) => loaded
                .workspace()
                .folder(key)
                .and_then(|folder| folder.metadata.name.clone()),
        })
        .collect()
}

#[test]
fn import_diagnostics_aggregate_repeated_issues() {
    let diagnostics = (0..889)
        .map(|index| ImportDiagnostic {
            code: "unsupported_field",
            severity: ImportDiagnosticSeverity::Lossy,
            resource_type: "http_request".to_owned(),
            resource_id: Some(format!("rq_{index}")),
            field: Some("description".to_owned()),
            message: "request descriptions cannot be represented by the current Probe domain"
                .to_owned(),
        })
        .collect::<Vec<_>>();

    let detail = format_import_diagnostics(&diagnostics);

    assert!(detail.contains("Found 889 compatibility issue(s): 889 lossy, 0 warning(s)."));
    assert!(detail.contains("• 889 lossy — http_request.description"));
    assert_eq!(
        detail
            .matches("request descriptions cannot be represented")
            .count(),
        1
    );
    assert!(!detail.contains("rq_0"));
    assert!(detail.lines().count() <= 5);
}

#[test]
fn import_diagnostics_bound_the_number_of_issue_groups() {
    let diagnostics = (0..IMPORT_DIAGNOSTIC_GROUP_LIMIT + 2)
        .map(|index| ImportDiagnostic {
            code: "unsupported_field",
            severity: ImportDiagnosticSeverity::Lossy,
            resource_type: "http_request".to_owned(),
            resource_id: Some(format!("rq_{index}")),
            field: Some(format!("field_{index}")),
            message: "field cannot be represented".to_owned(),
        })
        .collect::<Vec<_>>();

    let detail = format_import_diagnostics(&diagnostics);

    assert!(detail.contains("• 2 more issue(s) across 2 additional type(s)"));
    assert_eq!(
        detail.lines().filter(|line| line.starts_with('•')).count(),
        IMPORT_DIAGNOSTIC_GROUP_LIMIT + 1
    );
}

#[gpui::test]
fn postman_lossy_import_requires_desktop_confirmation(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let preview = inspect_postman_source(postman_fixture("collection-lossy.json")).unwrap();
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.convert_postman_import(preview, false, window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::ConfirmPartialPostmanImport { .. })
            ));
            assert!(!view.loading);
        })
        .unwrap();
}

#[gpui::test]
fn successful_postman_import_selects_collection_variables_environment(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let imported = inspect_postman_source(postman_fixture("collection-v2.1.json"))
        .unwrap()
        .convert(false)
        .unwrap();
    let destination = std::env::temp_dir().join(format!(
        "probe-desktop-postman-{}-{}.yml",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.choose_postman_import_destination(imported, window, cx);
        })
        .unwrap();
    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection({
        let destination = destination.clone();
        move |_| Some(destination)
    });
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert_eq!(
                view.shell.selected_environment(),
                Some(COLLECTION_VARIABLES_ENVIRONMENT)
            );
            assert_eq!(
                view.workspace_path.as_deref(),
                Some(destination.canonicalize().unwrap().as_path())
            );
        })
        .unwrap();
    fs::remove_file(destination).unwrap();
}

#[gpui::test]
fn postman_import_protects_dirty_requests(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture().canonicalize().unwrap();
    let workspace = probe_opencollection::load_workspace(&fixture).unwrap();
    let key = workspace.requests()[0].key();
    window
        .update(cx, |view, window, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.edit_request(
                key,
                |request| request.url = Some("https://dirty.example".to_owned()),
                cx,
            );
            view.request_import(ImportSource::Postman, window, cx);
            assert!(matches!(
                view.application_dialog,
                Some(ApplicationDialog::Unsaved {
                    pending: PendingClose::Import(ImportSource::Postman),
                    ..
                })
            ));
        })
        .unwrap();
}

#[gpui::test]
fn import_submenu_actions_open_and_close_provider_state(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();

    let mut visual = VisualTestContext::from_window(window.into(), cx);
    let workspace_trigger = visual
        .debug_bounds("workspace-switcher-trigger")
        .expect("workspace switcher trigger should render");
    visual.simulate_click(workspace_trigger.center(), Modifiers::default());
    visual.run_until_parked();
    cx.run_until_parked();

    cx.dispatch_action(window.into(), OpenImportSubmenu);
    window
        .update(cx, |view, window, cx| {
            assert!(view.transient.workspace_import_submenu_open);
            assert!(!view.transient.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.transient.workspace_import_popup_focus.clone())
            );
        })
        .unwrap();
    cx.dispatch_action(window.into(), CloseImportSubmenu);
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(!view.transient.workspace_import_submenu_open);
            assert!(!view.transient.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.transient.workspace_import_trigger_focus.clone())
            );
        })
        .unwrap();
}

#[gpui::test]
fn sidebar_import_submenu_keyboard_activation_restores_trigger_focus(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    cx.update(bind_platform_hotkeys);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual
            .debug_bounds("sidebar-import-from")
            .expect("sidebar import trigger should render");
    }
    window
        .update(cx, |view, window, cx| {
            view.transient
                .sidebar_import_trigger_focus
                .focus(window, cx);
        })
        .unwrap();
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let keystroke = Keystroke::parse("enter").unwrap();
        visual.simulate_event(KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        visual.simulate_event(KeyUpEvent { keystroke });
    }
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(view.transient.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.transient.sidebar_import_popup_focus.clone())
            );
        })
        .unwrap();

    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(!view.transient.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.transient.sidebar_import_trigger_focus.clone())
            );
        })
        .unwrap();
}

fn environment_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase4-environments.yml")
}

fn http_environment_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase5-http.yml")
}

fn writable_fixture(source: PathBuf, prefix: &str, suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{unique}-{suffix}.yml",
        std::process::id()
    ));
    fs::copy(source, &path).unwrap();
    path
}

fn writable_environment_fixture(suffix: &str) -> PathBuf {
    writable_fixture(environment_fixture(), "probe-desktop-env", suffix)
}

fn reconciled_workspace(workspace: probe_opencollection::LoadedWorkspace) -> ReconciledWorkspace {
    let disk_baselines = workspace
        .requests()
        .iter()
        .filter_map(|located| {
            workspace
                .workspace()
                .request(located.key())
                .cloned()
                .map(|request| (located.selector().to_owned(), request))
        })
        .collect();
    let selector_remaps = workspace
        .requests()
        .iter()
        .map(|located| (located.selector().to_owned(), located.selector().to_owned()))
        .chain(
            workspace
                .folders()
                .iter()
                .map(|located| (located.selector().to_owned(), located.selector().to_owned())),
        )
        .collect();
    ReconciledWorkspace {
        workspace,
        disk_baselines,
        selector_remaps,
    }
}

fn hover_and_wait(
    cx: &mut TestAppContext,
    window: gpui::WindowHandle<ProbeApp>,
    point: gpui::Point<gpui::Pixels>,
) {
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.simulate_mouse_move(point, None, Modifiers::default());
        visual.run_until_parked();
    }
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
}

fn writable_bundled_fixture(suffix: &str) -> PathBuf {
    writable_fixture(bundled_fixture(), "probe-desktop", suffix)
}

fn writable_large_fixture(suffix: &str) -> PathBuf {
    writable_fixture(large_fixture(), "probe-desktop-large", suffix)
}

fn writable_structure_fixture(suffix: &str) -> PathBuf {
    writable_fixture(nested_fixture(), "probe-desktop-structure", suffix)
}

mod environments;
mod interactions;
mod response;
mod workspace;
