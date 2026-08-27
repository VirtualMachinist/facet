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
use probe_core::{HttpRequest, QueryParameter, WorkspaceItemRef};
use probe_http::{HttpResponse, ResponseHeader};
use probe_postman::{COLLECTION_VARIABLES_ENVIRONMENT, inspect_postman_source};
use probe_yaak::{ImportDiagnostic, ImportDiagnosticSeverity};
use tokio::sync::oneshot;

use super::{
    ApplicationDialog, ApplicationDialogAction, CloseImportSubmenu, DesktopMenu,
    IMPORT_DIAGNOSTIC_GROUP_LIMIT, ImportSource, OpenFileMenu, OpenImportSubmenu, PendingClose,
    PrettyRevealState, ProbeApp, bind_platform_hotkeys, format_import_diagnostics,
    request_key_remaps,
};
use crate::{
    request_editor::{BodyEditorKind, EditorSection},
    response_inspector::InspectSelection,
    response_viewer::ResponseViewerTab,
    shell::PaneLayout,
    structure_editor::StructureDialogMode,
    synchronization::ReconciledWorkspace,
    theme::Theme,
};

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
            (
                view.persistence.is_dirty(key, request),
                view.message.clone(),
            )
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
            .update(cx, |view, _, _| view.desktop_menu_open)
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
            assert!(view.workspace_import_submenu_open);
            assert!(!view.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.workspace_import_popup_focus.clone())
            );
        })
        .unwrap();
    cx.dispatch_action(window.into(), CloseImportSubmenu);
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(!view.workspace_import_submenu_open);
            assert!(!view.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.workspace_import_trigger_focus.clone())
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
            view.sidebar_import_trigger_focus.focus(window, cx);
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
            assert!(view.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.sidebar_import_popup_focus.clone())
            );
        })
        .unwrap();

    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    window
        .update(cx, |view, window, cx| {
            assert!(!view.sidebar_import_menu_open);
            assert_eq!(
                window.focused(cx),
                Some(view.sidebar_import_trigger_focus.clone())
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

fn writable_environment_fixture(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "probe-desktop-env-{}-{unique}-{suffix}.yml",
        std::process::id()
    ));
    fs::copy(environment_fixture(), &path).unwrap();
    path
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
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "probe-desktop-{}-{unique}-{suffix}.yml",
        std::process::id()
    ));
    fs::copy(bundled_fixture(), &path).unwrap();
    path
}

fn writable_large_fixture(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "probe-desktop-large-{}-{unique}-{suffix}.yml",
        std::process::id()
    ));
    fs::copy(large_fixture(), &path).unwrap();
    path
}

fn writable_structure_fixture(suffix: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "probe-desktop-structure-{}-{unique}-{suffix}.yml",
        std::process::id()
    ));
    fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/opencollection/phase16-bundled.yml"),
        &path,
    )
    .unwrap();
    path
}

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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
            let loaded = view.loaded_workspace.as_ref().unwrap();
            assert!(loaded.request_key("items/0").is_some());
            assert!(loaded.folder_key("items/1").is_some());
            assert!(view.message.is_none());
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
            assert!(view.message.is_some());
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
            (
                view.persistence.is_dirty(key, request),
                view.message.clone(),
            )
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
            (
                view.persistence.is_dirty(key, request),
                view.message.clone(),
            )
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
            assert!(
                view.message
                    .as_deref()
                    .is_some_and(|message| message.contains("externally modified"))
            );
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
            (
                view.workspace_path.clone(),
                view.loading,
                view.message.clone(),
            )
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
                view.message.clone(),
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
fn large_sidebar_only_renders_the_visible_rows(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = large_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace =
        probe_opencollection::load_workspace(&fixture).expect("large fixture should load");
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
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
    assert_eq!(folder_only, ["Folder"]);
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

#[gpui::test]
fn request_editor_sections_render_for_an_open_request(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
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

    for section in EditorSection::ALL {
        window
            .update(cx, |view, _, cx| {
                view.request_editor.section = section;
                if section == EditorSection::Body {
                    view.change_body_kind(request_key, BodyEditorKind::Json, cx);
                }
                cx.notify();
            })
            .expect("test window should remain open");
        cx.run_until_parked();
        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            assert!(visual.debug_bounds("request-url-bar").is_some());
            assert!(visual.debug_bounds("request-breadcrumb").is_some());
            assert!(visual.debug_bounds("request-breadcrumb-folder-0").is_some());
            assert!(visual.debug_bounds("request-breadcrumb-request").is_some());
            assert!(visual.debug_bounds("request-method-trigger").is_some());
            assert!(visual.debug_bounds("request-environment-trigger").is_some());
            if section == EditorSection::Body {
                assert!(
                    visual.debug_bounds("request-body-editor").is_some(),
                    "JSON body editor should render"
                );
            }
        }
    }
}

#[gpui::test]
fn completed_response_renders_pretty_raw_headers_and_search(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
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
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            let body = br#"{"createdAt":1787482800,"ok":true}"#.to_vec();
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 201,
                    reason: "Created".to_owned(),
                    url: "https://api.example.test/users".to_owned(),
                    duration: Duration::from_millis(42),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "application/json".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-status").is_some());
        assert!(visual.debug_bounds("response-status-code").is_some());
        assert!(visual.debug_bounds("response-metadata").is_some());
        assert!(visual.debug_bounds("response-tab-pretty").is_some());
        assert!(visual.debug_bounds("response-tab-raw").is_some());
        assert!(visual.debug_bounds("response-tab-headers").is_some());
        assert!(visual.debug_bounds("response-search").is_none());
        assert!(visual.debug_bounds("editor-search-card").is_none());
        assert!(visual.debug_bounds("response-body").is_some());
        assert!(visual.debug_bounds("response-headers").is_none());
    }

    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let body_bounds = visual
            .debug_bounds("response-body")
            .expect("response body should render");
        visual.simulate_click(body_bounds.center(), Modifiers::default());
    }
    cx.simulate_keystrokes(window.into(), find_shortcut());
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_some());
        assert!(visual.debug_bounds("editor-search-input").is_some());
        assert!(visual.debug_bounds("response-search").is_none());
    }
    cx.simulate_keystrokes(window.into(), "o k");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_some());
        assert!(visual.debug_bounds("response-body").is_some());
    }
    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("editor-search-card").is_none());
    }

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Headers);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        assert!(visual.debug_bounds("response-headers").is_some());
        assert!(visual.debug_bounds("response-body").is_none());
    }

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Pretty);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Inspect);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let reveal = visual
            .debug_bounds("response-inspector-reveal-pretty")
            .expect("selected inspection should expose a reveal button");
        visual.simulate_mouse_down(reveal.center(), MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(reveal.center(), MouseButton::Left, Modifiers::default());
    }
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert_eq!(view.response_viewer.tab(), ResponseViewerTab::Pretty);
            assert_eq!(
                view.pretty_reveal.get(),
                Some(PrettyRevealState {
                    selection: InspectSelection::Timestamp(0),
                    scroll_pending: false,
                })
            );
        })
        .expect("test window should remain open");
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        let body = visual
            .debug_bounds("response-body")
            .expect("Pretty response body should render after reveal");
        assert!(
            visual
                .debug_bounds("response-inspector-reveal-pretty")
                .is_none()
        );
        visual.simulate_mouse_down(body.center(), MouseButton::Left, Modifiers::default());
        visual.simulate_mouse_up(body.center(), MouseButton::Left, Modifiers::default());
    }
    cx.run_until_parked();
    window
        .update(cx, |view, _, _| {
            assert!(view.pretty_reveal.get().is_none());
        })
        .expect("test window should remain open");
}

#[gpui::test]
fn xml_response_inspects_values_and_keeps_syntax_after_visiting_raw(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
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
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            let body = br#"<root createdAt="1787482800"><item/></root>"#.to_vec();
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 200,
                    reason: "OK".to_owned(),
                    url: "https://api.example.test/data.xml".to_owned(),
                    duration: Duration::from_millis(12),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "application/xml".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            assert_eq!(
                view.response_viewer
                    .document(request_key)
                    .expect("response document")
                    .syntax
                    .language(),
                "xml"
            );
            let document = view
                .response_viewer
                .document(request_key)
                .expect("response document");
            assert_eq!(document.inspection.timestamps.len(), 1);
            assert_eq!(document.inspection.timestamps[0].path, "/root/@createdAt");
            assert_eq!(document.inspection_ranges.len(), 1);
            assert_eq!(
                &document.pretty_text[document.inspection_ranges[0].range.clone()],
                "1787482800"
            );
            view.response_viewer.set_tab(ResponseViewerTab::Raw);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, cx| {
            view.response_viewer.set_tab(ResponseViewerTab::Pretty);
            cx.notify();
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    window
        .update(cx, |view, _, _| {
            assert_eq!(view.response_viewer.tab(), ResponseViewerTab::Pretty);
            assert_eq!(
                view.response_viewer
                    .document(request_key)
                    .expect("response document")
                    .syntax
                    .language(),
                "xml"
            );
        })
        .expect("test window should remain open");
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("response-body").is_some());
}

#[gpui::test]
fn large_response_body_only_renders_visible_rows(cx: &mut TestAppContext) {
    cx.update(Theme::init);
    let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
        ProbeApp::new(window, cx)
    });
    let fixture = bundled_fixture()
        .canonicalize()
        .expect("fixture should exist");
    let workspace = probe_opencollection::load_workspace(&fixture).expect("fixture should load");
    let request_key = workspace.requests()[0].key();
    let body = (0..20_000)
        .map(|index| format!("line-{index:05}"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes();
    window
        .update(cx, |view, _, cx| {
            view.session_store = None;
            view.set_workspace(fixture, workspace);
            view.select_request(request_key, cx);
            view.shell.response_height = 220.0;
            let (cancellation, _) = tokio::sync::oneshot::channel();
            let generation = view.execution.begin(request_key, cancellation);
            view.complete_execution(
                request_key,
                generation,
                Ok(HttpResponse {
                    status: 200,
                    reason: "OK".to_owned(),
                    url: "https://api.example.test/lines".to_owned(),
                    duration: Duration::from_millis(12),
                    size: body.len(),
                    headers: vec![ResponseHeader {
                        name: "content-type".to_owned(),
                        value: "text/plain".to_owned(),
                    }],
                    body,
                    body_complete: true,
                    body_file: None,
                    body_retention_error: None,
                }),
                cx,
            );
            view.response_viewer.set_tab(ResponseViewerTab::Raw);
            cx.notify();
        })
        .expect("test window should be open");
    cx.run_until_parked();
    {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.update(|window, cx| {
            window.simulate_next_frame(cx);
            window.simulate_next_frame(cx);
        });
    }
    cx.run_until_parked();

    let (total_rows, rendered_rows) = window
        .update(cx, |view, _, _| {
            (
                view.response_viewer.visible_line_count(request_key),
                view.rendered_response_rows,
            )
        })
        .expect("test window should remain open");
    assert!(total_rows >= 20_000);
    assert!(rendered_rows > 0);
    assert!(
        rendered_rows < total_rows,
        "virtualized response viewer rendered all {total_rows} rows"
    );
}

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
fn environment_switcher_includes_create_environment(cx: &mut TestAppContext) {
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
            assert!(view.environment_save_task.is_none(), "{:?}", view.message);
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
            assert_eq!(
                view.message.as_deref(),
                Some("Environment name is required.")
            );
            assert!(view.create_environment_dialog.is_some());
        })
        .expect("test window should be open");
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

    let (tabs, active) = window
        .update(cx, |view, _, _| {
            (view.shell.tabs().to_vec(), view.shell.active_tab())
        })
        .expect("test window should remain open");
    assert_eq!(tabs, vec![first]);
    assert_eq!(active, Some(first));
}

#[gpui::test]
fn request_tab_context_menu_closes_other_tabs(cx: &mut TestAppContext) {
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
            view.open_tab_context_menu(second, tab.center(), cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();
    let menu_target = window
        .update(cx, |view, _, _| view.tab_context_menu)
        .expect("test window should remain open");
    assert_eq!(menu_target, Some(second));

    window
        .update(cx, |view, window, cx| {
            view.request_close_other_tabs(second, window, cx);
        })
        .expect("test window should remain open");
    cx.run_until_parked();

    let (tabs, active) = window
        .update(cx, |view, _, _| {
            (view.shell.tabs().to_vec(), view.shell.active_tab())
        })
        .expect("test window should remain open");
    assert_eq!(tabs, vec![second]);
    assert_eq!(active, Some(second));
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
            assert!(view.structure_task.is_none(), "{:?}", view.message);
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
