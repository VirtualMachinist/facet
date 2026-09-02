use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
    time::Duration,
};

use gpui::{
    Action, Anchor, App, AppContext as _, Axis, Bounds, Context, CursorStyle, DragMoveEvent,
    ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    MouseButton, MouseDownEvent, MouseMoveEvent, ParentElement as _, PathPromptOptions, Pixels,
    Point, Render, ScrollHandle, ScrollStrategy, StatefulInteractiveElement as _, Styled as _,
    Task, TitlebarOptions, UniformListScrollHandle, Window, WindowBounds, WindowControlArea,
    WindowOptions, deferred, div, point, prelude::FluentBuilder as _, px, relative, size,
    transparent_black, uniform_list,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction, SystemMenuType};
use gpui_base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_base::{
    AutoScroll, Button, POPUP_PRIORITY, Popover, Positioner, Scrollbar, ScrollbarMode, Tab, Tabs,
    ToastStack,
};
use probe_core::{
    AuthenticationKind, AuthenticationValue, Body, Collection, Environment, EnvironmentVariable,
    FileReference, FormField, Header, HttpRequest, MultipartPart, MultipartPartKind,
    MultipartValue, QueryParameter, RawBodyKind, RequestBody, RequestKey, Variable, VariableValue,
    VariableValueSet, WorkspaceItemRef, add_path_parameter, ensure_path_parameters_from_url,
    remove_path_parameter_at, rename_path_parameter_at, resolve_environment, resolve_request,
};
use probe_http::{ExecutionOptions, HttpError, HttpResponse};
use probe_opencollection::{
    CompletedEnvironmentDelete, ItemKind, LoadedWorkspace, StructureOperation, StructureResult,
    create_bundled_workspace, create_bundled_workspace_from_collection, load_workspace,
};
use probe_postman::{
    ImportedPostmanCollection, PostmanImportError, PostmanImportPreview, inspect_postman_source,
};
use probe_yaak::{ImportedYaakWorkspace, YaakImportError, YaakImportPreview, inspect_yaak_source};

mod chrome;
mod dialogs;
mod environments;
mod imports;
mod interactions;
mod presentation;
mod render;
mod response;
mod session_state;
mod structure;
mod tabs;
mod transient;
mod tree;
mod workspace;

#[cfg(test)]
pub(crate) use dialogs::IMPORT_DIAGNOSTIC_GROUP_LIMIT;
use dialogs::{
    ApplicationDialog, ApplicationDialogAction, CANCEL_DIALOG_ACTION, DesktopMenu,
    DesktopMenuDefinition, DesktopMenuItem, DesktopSubmenu, DialogActionSpec,
    EnvironmentManagerDialog, ImportSource, PendingClose, PostmanConversionResult,
    YaakConversionResult, format_import_diagnostics, suggested_collection_filename,
};
use presentation::{
    InspectListRow, PrettyRevealState, ShellSelectors, inspect_list_rows, inspect_row_index,
    inspect_row_label, placeholder_message, request_method_options, response_status_color,
};
use transient::TransientSurfaces;
use tree::{
    TreeDrag, TreeRow, TreeRowSpec, flatten_visible_tree_rows, tree_hierarchy_guides,
    tree_level_indent, tree_method_font_size, tree_method_label, tree_row_button,
};

use crate::{
    components,
    execution::{
        ExecutionState, ResponseState, body_file_path_for_storage, execute_http_request,
        format_duration, format_size, read_response_page, response_cache,
    },
    filesystem::{
        WATCH_DEBOUNCE, WorkspaceWatcher, event_affects_workspace, rename_hints,
        workspace_base_directory,
    },
    persistence::PersistenceState,
    request_editor::{
        BodyEditorKind, EditorSection, RequestEditorState, apply_url_bar_value, auth_label,
        auth_value, body_kind, raw_body_mut, set_auth_property, set_authentication, url_bar_value,
    },
    response_inspector::{
        InspectSelection, inspect_json_file, inspect_response_body, inspect_xml_file,
        inspection_detail_text,
    },
    response_viewer::{
        PageDirection, PreparedDocument, RESPONSE_PAGE_BYTES, RawBodyView, ResponseBodySyntax,
        ResponseViewerState, ResponseViewerTab, encode_base64, prepare_document, pretty_body,
    },
    session::{SessionState, SessionStore},
    shell::{PaneLayout, ResizePane, ShellState},
    structure_editor::{
        DropIndicator, DropReject, ROOT_PARENT, StructureDialog, StructureDialogMode,
        TreeDropIntent, descendant_requests, drop_intent, drop_zone, hovered_row_index,
        item_position, structure_operation_for_drop, validate_tree_drop, would_duplicate_path,
    },
    synchronization::{
        LocalRequestState, ReconcileResult, ReconciledWorkspace, SynchronizationConflict, reconcile,
    },
    theme::Theme,
    toast::{ToastCenter, ToastId, ToastIntent, toast_stack_motion},
    tree_search::{TreeSearchMatches, matching_tree_items},
};

const APPLICATION_ID: &str = "dev.probe.desktop";
const APPLICATION_NAME: &str = "Probe";
const WORKSPACE_SWITCHER_MENU_WIDTH: f32 = 300.0;
const RESPONSE_ELAPSED_REFRESH_INTERVAL: Duration = Duration::from_millis(50);
const REQUEST_TAB_TOOLTIP_DELAY: Duration = Duration::from_millis(200);
const DEFAULT_INSPECT_LIST_WIDTH: f32 = 220.0;
const MIN_INSPECT_LIST_WIDTH: f32 = 160.0;
const MAX_INSPECT_LIST_WIDTH: f32 = 360.0;

#[cfg(test)]
use crate::filesystem::{WATCH_POLL, drain_watch_events};

gpui::actions!(
    probe,
    [
        OpenWorkspace,
        NewCollection,
        ImportPostmanExport,
        ImportYaakExport,
        SaveRequest,
        CloseActiveTab,
        AboutProbe,
        CloseWindow,
        MinimizeWindow,
        ZoomWindow,
        ToggleSidebar,
        UseVerticalEditorLayout,
        UseHorizontalEditorLayout,
        OpenFileMenu,
        OpenEditMenu,
        OpenViewMenu,
        OpenHelpMenu,
        HideApplication,
        HideOtherApplications,
        ShowAllApplications,
        QuitApplication,
        FocusNextControl,
        FocusPreviousControl,
        NewRequest,
        NewFolder,
        DuplicateRequest,
        RenameTreeItem,
        DeleteTreeItem,
        MoveTreeItem,
        MoveTreeItemUp,
        MoveTreeItemDown,
        SelectPreviousTreeItem,
        SelectNextTreeItem,
        CollapseTreeItem,
        ExpandTreeItem,
        ActivateTreeItem,
        OpenImportSubmenu,
        CloseImportSubmenu,
        SubmitStructureDialog,
        SubmitCreateEnvironmentDialog,
        SubmitEnvironmentManagerDialog,
        SubmitApplicationDialog,
        SubmitApplicationDialogDestructive,
        CancelStructureDialog,
        CancelCreateEnvironmentDialog,
        CancelEnvironmentManagerDialog,
        DeleteSelectedEnvironment,
        CancelApplicationDialog
    ]
);

const TREE_LIST_PADDING_Y: f32 = 2.0;

fn request_key_remaps(
    old: &LoadedWorkspace,
    new: &LoadedWorkspace,
    selector_remaps: &BTreeMap<String, String>,
) -> BTreeMap<RequestKey, RequestKey> {
    old.requests()
        .iter()
        .filter_map(|located| {
            let selector = selector_remaps.get(located.selector())?;
            new.request_key(selector)
                .map(|new_key| (located.key(), new_key))
        })
        .collect()
}

#[derive(Clone, Copy)]
struct RequestTabTooltip {
    key: RequestKey,
    position: Point<Pixels>,
    open: bool,
}

struct PositionedContextMenu<T> {
    target: T,
    position: Point<Pixels>,
}

#[derive(Clone, Copy)]
enum ImportedCollectionKind {
    Postman,
    Yaak,
}

impl ImportedCollectionKind {
    const fn source_label(self) -> &'static str {
        match self {
            Self::Postman => "Postman",
            Self::Yaak => "Yaak",
        }
    }

    const fn imported_kind(self) -> &'static str {
        match self {
            Self::Postman => "collection",
            Self::Yaak => "workspace",
        }
    }
}

struct CollectionImport {
    source_name: String,
    collection: Collection,
    warning_count: usize,
    selected_environment: Option<String>,
    kind: ImportedCollectionKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvironmentDialogErrorResolution {
    ManagerDraftValid,
    CreateNameValid,
    ManagerClean,
    SavesIdle,
    Manual,
}

struct EnvironmentDialogError {
    toast_id: ToastId,
    resolution: EnvironmentDialogErrorResolution,
}

impl EnvironmentDialogError {
    fn new(toast_id: ToastId, resolution: EnvironmentDialogErrorResolution) -> Self {
        Self {
            toast_id,
            resolution,
        }
    }
}

pub(crate) struct ProbeApp {
    focus_handle: FocusHandle,
    tree_focus_handle: FocusHandle,
    structure_dialog_focus: FocusHandle,
    create_environment_dialog_focus: FocusHandle,
    environment_manager_dialog_focus: FocusHandle,
    application_dialog_focus: FocusHandle,
    toast_focus_handle: FocusHandle,
    loaded_workspace: Option<LoadedWorkspace>,
    workspace_path: Option<PathBuf>,
    shell: ShellState,
    loading: bool,
    session_store: Option<SessionStore>,
    session: SessionState,
    session_save_task: Option<Task<()>>,
    request_save_task: Option<Task<()>>,
    environment_save_task: Option<Task<()>>,
    environment_save_workspace_path: Option<PathBuf>,
    environment_manager_close_after_save: bool,
    pending_environment_saves: BTreeSet<(String, String)>,
    structure_task: Option<Task<()>>,
    filesystem_watcher: Option<notify::RecommendedWatcher>,
    filesystem_watch_task: Option<Task<()>>,
    persistence: PersistenceState,
    pending_close: Option<PendingClose>,
    transient: TransientSurfaces,
    visible_tree_rows: Vec<TreeRow>,
    tree_search: String,
    tree_search_matches: Option<TreeSearchMatches>,
    selected_tree_item: Option<WorkspaceItemRef>,
    tree_drag_source: Option<WorkspaceItemRef>,
    tree_drop_target: Option<TreeDropIntent>,
    tree_list_bounds: Option<Bounds<Pixels>>,
    tree_row_height: f32,
    tree_auto_scroll: AutoScroll,
    structure_dialog: Option<StructureDialog>,
    create_environment_dialog: Option<String>,
    environment_manager_dialog: Option<EnvironmentManagerDialog>,
    environment_dialog_error: Option<EnvironmentDialogError>,
    application_dialog: Option<ApplicationDialog>,
    pending_application_dialogs: VecDeque<ApplicationDialog>,
    toasts: ToastCenter,
    toast_lifecycle_generation: u64,
    toast_paused: bool,
    request_editor: RequestEditorState,
    execution: ExecutionState,
    response_cache: probe_http::ResponseCache,
    response_viewer: ResponseViewerState,
    tree_scroll: UniformListScrollHandle,
    inspector_scroll: UniformListScrollHandle,
    inspector_list_width: f32,
    inspector_resize_start: Option<(f32, f32)>,
    pending_inspector_reveal: Cell<Option<InspectSelection>>,
    pretty_reveal: Cell<Option<PrettyRevealState>>,
    tab_bar_scroll: ScrollHandle,
    pending_tab_reveal: bool,
    #[cfg(test)]
    rendered_sidebar_rows: usize,
    #[cfg(test)]
    rendered_response_rows: usize,
    _caret_blink: Task<()>,
    _response_elapsed_refresh: Task<()>,
    _keystrokes: gpui::Subscription,
    _quit_subscription: gpui::Subscription,
}

impl ProbeApp {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::sync_gpui_base(window.appearance(), cx);
            window.refresh();
        })
        .detach();
        Theme::sync_gpui_base(window.appearance(), cx);
        let quit_subscription = cx.on_app_quit(|view, cx| {
            view.capture_session();
            let store = view.session_store.clone();
            let state = view.session.clone();
            let executor = cx.background_executor().clone();
            async move {
                if let Some(store) = store {
                    let _ = executor.spawn(async move { store.save(&state) }).await;
                }
            }
        });
        crate::caret::CaretBlink::show(cx);
        let keystrokes = cx.observe_keystrokes(|this, _, _, cx| {
            this.reset_caret_blink(cx);
        });
        let close_view = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            close_view
                .update(cx, |view, cx| view.request_close_window(window, cx))
                .unwrap_or(true)
        });
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        let tree_focus_handle = cx.focus_handle();
        let structure_dialog_focus = cx.focus_handle();
        let create_environment_dialog_focus = cx.focus_handle();
        let environment_manager_dialog_focus = cx.focus_handle();
        let application_dialog_focus = cx.focus_handle();
        let toast_focus_handle = cx.focus_handle();
        let response_cache = response_cache();
        let initializing_response_cache = response_cache.clone();
        cx.background_spawn(async move {
            let _ = initializing_response_cache.initialize();
        })
        .detach();

        Self {
            focus_handle,
            tree_focus_handle,
            structure_dialog_focus,
            create_environment_dialog_focus,
            environment_manager_dialog_focus,
            application_dialog_focus,
            toast_focus_handle,
            loaded_workspace: None,
            workspace_path: None,
            shell: ShellState::default(),
            loading: false,
            session_store: SessionStore::for_application(),
            session: SessionState::default(),
            session_save_task: None,
            request_save_task: None,
            environment_save_task: None,
            environment_save_workspace_path: None,
            environment_manager_close_after_save: false,
            pending_environment_saves: BTreeSet::new(),
            structure_task: None,
            filesystem_watcher: None,
            filesystem_watch_task: None,
            persistence: PersistenceState::default(),
            pending_close: None,
            transient: TransientSurfaces::new(cx),
            visible_tree_rows: Vec::new(),
            tree_search: String::new(),
            tree_search_matches: None,
            selected_tree_item: None,
            tree_drag_source: None,
            tree_drop_target: None,
            tree_list_bounds: None,
            tree_row_height: 28.0,
            tree_auto_scroll: AutoScroll::default(),
            structure_dialog: None,
            create_environment_dialog: None,
            environment_manager_dialog: None,
            environment_dialog_error: None,
            application_dialog: None,
            pending_application_dialogs: VecDeque::new(),
            toasts: ToastCenter::default(),
            toast_lifecycle_generation: 0,
            toast_paused: false,
            request_editor: RequestEditorState::default(),
            execution: ExecutionState::default(),
            response_cache,
            response_viewer: ResponseViewerState::default(),
            tree_scroll: UniformListScrollHandle::new(),
            inspector_scroll: UniformListScrollHandle::new(),
            inspector_list_width: DEFAULT_INSPECT_LIST_WIDTH,
            inspector_resize_start: None,
            pending_inspector_reveal: Cell::new(None),
            pretty_reveal: Cell::new(None),
            tab_bar_scroll: ScrollHandle::new(),
            pending_tab_reveal: false,
            #[cfg(test)]
            rendered_sidebar_rows: 0,
            #[cfg(test)]
            rendered_response_rows: 0,
            _caret_blink: Self::spawn_caret_blink(cx),
            _response_elapsed_refresh: Self::spawn_response_elapsed_refresh(cx),
            _keystrokes: keystrokes,
            _quit_subscription: quit_subscription,
        }
    }

    fn spawn_caret_blink(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(crate::caret::CARET_BLINK_INTERVAL)
                    .await;
                if this
                    .update(cx, |_, cx| {
                        crate::caret::CaretBlink::toggle(cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn spawn_response_elapsed_refresh(cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(RESPONSE_ELAPSED_REFRESH_INTERVAL)
                    .await;
                if this
                    .update(cx, |view, cx| {
                        if view
                            .shell
                            .active_tab()
                            .and_then(|key| view.execution.response(key))
                            .is_some_and(ResponseState::is_running)
                        {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
    }

    fn show_toast(
        &mut self,
        intent: ToastIntent,
        message: impl Into<String>,
        cx: &mut Context<Self>,
    ) -> ToastId {
        let now = cx.background_executor().now();
        let id = self.toasts.push(intent, message, now);
        self.schedule_toast_lifecycle(cx);
        cx.notify();
        id
    }

    fn dismiss_toast(&mut self, id: ToastId, cx: &mut Context<Self>) {
        if self
            .environment_dialog_error
            .as_ref()
            .is_some_and(|error| error.toast_id == id)
        {
            self.environment_dialog_error = None;
        }
        if self.toasts.dismiss(id, cx.background_executor().now()) {
            self.schedule_toast_lifecycle(cx);
            cx.notify();
        }
    }

    fn clear_environment_dialog_error(&mut self, cx: &mut Context<Self>) {
        if let Some(error) = self.environment_dialog_error.take() {
            self.dismiss_toast(error.toast_id, cx);
        }
    }

    fn clear_toasts(&mut self) {
        self.toasts.clear();
        self.toast_lifecycle_generation = self.toast_lifecycle_generation.wrapping_add(1);
        self.toast_paused = false;
    }

    fn schedule_toast_lifecycle(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let paused = self.toasts.stack_state.is_expanded();
        if paused != self.toast_paused {
            let changed = self.toasts.advance(now, !paused);
            self.toast_paused = paused;
            if changed {
                cx.notify();
            }
        }
        self.toast_lifecycle_generation = self.toast_lifecycle_generation.wrapping_add(1);
        let generation = self.toast_lifecycle_generation;
        let Some(delay) = self.toasts.next_wake(now, paused) else {
            return;
        };
        cx.spawn(async move |view, cx| {
            cx.background_executor().timer(delay).await;
            let _ = view.update(cx, |view, cx| {
                if view.toast_lifecycle_generation != generation {
                    return;
                }
                let paused = view.toasts.stack_state.is_expanded();
                let now = cx.background_executor().now();
                if view.toasts.advance(now, paused) {
                    cx.notify();
                }
                view.schedule_toast_lifecycle(cx);
            });
        })
        .detach();
    }

    fn reset_caret_blink(&mut self, cx: &mut Context<Self>) {
        let was_visible = crate::caret::CaretBlink::is_visible(cx);
        crate::caret::CaretBlink::show(cx);
        self._caret_blink = Self::spawn_caret_blink(cx);
        if !was_visible {
            cx.notify();
        }
    }

    fn workspace_name(&self) -> String {
        if let Some(name) = self
            .loaded_workspace
            .as_ref()
            .and_then(|loaded| loaded.workspace().metadata().name.as_deref())
        {
            return name.to_owned();
        }
        self.workspace_path
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("No collection open")
            .to_owned()
    }
}

#[cfg(target_os = "windows")]
fn render_windows_controls(theme: Theme) -> gpui::Div {
    let control = move |id: &'static str,
                        label: &'static str,
                        area,
                        destructive: bool,
                        action: fn(&mut Window)| {
        Button::new(id)
            .focusable(false)
            .tab_stop(false)
            .w(px(44.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .window_control_area(area)
            .hover(move |control| {
                if destructive {
                    control
                        .bg(theme.colors.status.error)
                        .text_color(theme.colors.text.inverse)
                } else {
                    control.bg(theme.colors.surfaces.sidebar)
                }
            })
            .on_click(move |_, window, _| action(window))
            .child(label)
    };

    div()
        .h_full()
        .flex()
        .child(control(
            "window-minimize",
            "—",
            WindowControlArea::Min,
            false,
            |window| window.minimize_window(),
        ))
        .child(control(
            "window-maximize",
            "□",
            WindowControlArea::Max,
            false,
            |window| window.zoom_window(),
        ))
        .child(control(
            "window-close",
            "×",
            WindowControlArea::Close,
            true,
            Window::remove_window,
        ))
}

#[cfg(not(target_os = "windows"))]
fn render_windows_controls(_: Theme) -> gpui::Div {
    div()
}

pub fn run() {
    let app = gpui_platform::application();
    app.on_reopen(|cx| {
        if cx.windows().is_empty() {
            open_probe_window(cx);
        } else if let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) {
            let _ = window.update(cx, |_, window, _| window.activate_window());
        }
    });
    app.run(|cx: &mut App| {
        cx.set_app_identity(APPLICATION_ID, APPLICATION_NAME);
        Theme::init(cx);
        bind_platform_hotkeys(cx);
        install_system_menu(cx);

        open_probe_window(cx);
        cx.activate(true);
    });
}

fn open_probe_window(cx: &mut App) {
    let bounds = Bounds::centered(None, size(px(1180.0), px(780.0)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: None,
                appears_transparent: cfg!(any(target_os = "macos", target_os = "windows")),
                traffic_light_position: if cfg!(target_os = "macos") {
                    Some(point(px(9.0), px(11.0)))
                } else {
                    None
                },
            }),
            app_owns_titlebar_drag: cfg!(target_os = "macos"),
            window_min_size: Some(size(px(760.0), px(560.0))),
            app_id: Some(APPLICATION_ID.to_owned()),
            ..Default::default()
        },
        |window, cx| {
            let view = cx.new(|cx| ProbeApp::new(window, cx));
            view.update(cx, |view, cx| view.restore_session(window, cx));
            view
        },
    )
    .expect("failed to open Probe's application window");
}

#[cfg(target_os = "macos")]
fn install_system_menu(cx: &mut App) {
    cx.on_action(|_: &HideApplication, cx| cx.hide());
    cx.on_action(|_: &HideOtherApplications, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAllApplications, cx| cx.unhide_other_apps());

    cx.set_menus(system_menus(PaneLayout::Vertical));
}

#[cfg(target_os = "macos")]
fn system_menus(pane_layout: PaneLayout) -> [Menu; 5] {
    [
        Menu::new(APPLICATION_NAME).items([
            MenuItem::action("About Probe", AboutProbe),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide Probe", HideApplication),
            MenuItem::action("Hide Others", HideOtherApplications),
            MenuItem::action("Show All", ShowAllApplications),
            MenuItem::separator(),
            MenuItem::action("Quit Probe", QuitApplication),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Collection…", NewCollection),
            MenuItem::action("Open Collection…", OpenWorkspace),
            MenuItem::submenu(Menu::new("Import From…").items([
                MenuItem::action("Postman Export…", ImportPostmanExport),
                MenuItem::action("Yaak Export…", ImportYaakExport),
            ])),
            MenuItem::separator(),
            MenuItem::action("Save Request", SaveRequest),
            MenuItem::separator(),
            MenuItem::action("Close Tab", CloseActiveTab),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::os_action("Undo", Undo, OsAction::Undo),
            MenuItem::os_action("Redo", Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action("Cut", Cut, OsAction::Cut),
            MenuItem::os_action("Copy", Copy, OsAction::Copy),
            MenuItem::os_action("Paste", Paste, OsAction::Paste),
            MenuItem::separator(),
            MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Show/Hide Sidebar", ToggleSidebar),
            MenuItem::submenu(
                Menu::new("Editor Layout").items([
                    MenuItem::action("Vertical", UseVerticalEditorLayout)
                        .checked(pane_layout == PaneLayout::Vertical),
                    MenuItem::action("Horizontal", UseHorizontalEditorLayout)
                        .checked(pane_layout == PaneLayout::Horizontal),
                ]),
            ),
            MenuItem::separator(),
        ]),
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
            MenuItem::separator(),
        ]),
    ]
}

#[cfg(not(target_os = "macos"))]
fn install_system_menu(_: &mut App) {}

fn bind_platform_hotkeys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNextControl, None),
        KeyBinding::new("shift-tab", FocusPreviousControl, None),
        KeyBinding::new("up", SelectPreviousTreeItem, Some("RequestTree")),
        KeyBinding::new("down", SelectNextTreeItem, Some("RequestTree")),
        KeyBinding::new("left", CollapseTreeItem, Some("RequestTree")),
        KeyBinding::new("right", ExpandTreeItem, Some("RequestTree")),
        KeyBinding::new("enter", ActivateTreeItem, Some("RequestTree")),
        KeyBinding::new("space", ActivateTreeItem, Some("RequestTree")),
        KeyBinding::new("right", OpenImportSubmenu, Some("ImportSubmenuTrigger")),
        KeyBinding::new("left", CloseImportSubmenu, Some("ImportSubmenu")),
        KeyBinding::new("escape", CloseImportSubmenu, Some("ImportSubmenu")),
        KeyBinding::new("n", NewRequest, Some("RequestTree")),
        KeyBinding::new("shift-n", NewFolder, Some("RequestTree")),
        KeyBinding::new("m", MoveTreeItem, Some("RequestTree")),
        KeyBinding::new("alt-up", MoveTreeItemUp, Some("RequestTree")),
        KeyBinding::new("alt-down", MoveTreeItemDown, Some("RequestTree")),
        KeyBinding::new("enter", SubmitStructureDialog, Some("StructureDialog")),
        KeyBinding::new(
            "enter",
            SubmitCreateEnvironmentDialog,
            Some("CreateEnvironmentDialog"),
        ),
        KeyBinding::new("enter", SubmitApplicationDialog, Some("ApplicationDialog")),
        KeyBinding::new("escape", CancelStructureDialog, Some("StructureDialog")),
        KeyBinding::new(
            "escape",
            CancelCreateEnvironmentDialog,
            Some("CreateEnvironmentDialog"),
        ),
        KeyBinding::new(
            "escape",
            CancelEnvironmentManagerDialog,
            Some("EnvironmentManagerDialog"),
        ),
        KeyBinding::new("escape", CancelApplicationDialog, Some("ApplicationDialog")),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenWorkspace, None),
        KeyBinding::new("cmd-n", NewCollection, None),
        KeyBinding::new("cmd-s", SaveRequest, None),
        KeyBinding::new(
            "cmd-s",
            SubmitEnvironmentManagerDialog,
            Some("EnvironmentManagerDialog"),
        ),
        KeyBinding::new("cmd-w", CloseActiveTab, None),
        KeyBinding::new("cmd-q", QuitApplication, None),
        KeyBinding::new("cmd-shift-w", CloseWindow, None),
        KeyBinding::new("cmd-m", MinimizeWindow, None),
        KeyBinding::new("cmd-h", HideApplication, None),
        KeyBinding::new("cmd-alt-h", HideOtherApplications, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-x", Cut, None),
        KeyBinding::new("cmd-c", Copy, None),
        KeyBinding::new("cmd-v", Paste, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-d", DuplicateRequest, Some("RequestTree")),
        KeyBinding::new("cmd-e", RenameTreeItem, Some("RequestTree")),
        KeyBinding::new("backspace", DeleteTreeItem, Some("RequestTree")),
        KeyBinding::new(
            "backspace",
            DeleteSelectedEnvironment,
            Some("EnvironmentManagerDialog"),
        ),
        KeyBinding::new(
            "cmd-backspace",
            SubmitApplicationDialogDestructive,
            Some("ApplicationDialog"),
        ),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("f10", OpenFileMenu, None),
        KeyBinding::new("alt-f", OpenFileMenu, None),
        KeyBinding::new("alt-e", OpenEditMenu, None),
        KeyBinding::new("alt-v", OpenViewMenu, None),
        KeyBinding::new("alt-h", OpenHelpMenu, None),
        KeyBinding::new("ctrl-z", Undo, None),
        KeyBinding::new("ctrl-shift-z", Redo, None),
        KeyBinding::new("ctrl-x", Cut, None),
        KeyBinding::new("ctrl-c", Copy, None),
        KeyBinding::new("ctrl-v", Paste, None),
        KeyBinding::new("ctrl-a", SelectAll, None),
        KeyBinding::new("ctrl-shift-w", CloseWindow, None),
        KeyBinding::new("ctrl-d", DuplicateRequest, Some("RequestTree")),
        KeyBinding::new("f2", RenameTreeItem, Some("RequestTree")),
        KeyBinding::new("delete", DeleteTreeItem, Some("RequestTree")),
        KeyBinding::new(
            "delete",
            DeleteSelectedEnvironment,
            Some("EnvironmentManagerDialog"),
        ),
        KeyBinding::new(
            "ctrl-delete",
            SubmitApplicationDialogDestructive,
            Some("ApplicationDialog"),
        ),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenWorkspace, None),
        KeyBinding::new("ctrl-n", NewCollection, None),
        KeyBinding::new("ctrl-s", SaveRequest, None),
        KeyBinding::new(
            "ctrl-s",
            SubmitEnvironmentManagerDialog,
            Some("EnvironmentManagerDialog"),
        ),
        KeyBinding::new("ctrl-w", CloseActiveTab, None),
    ]);

    #[cfg(target_os = "windows")]
    cx.bind_keys([KeyBinding::new("alt-f4", CloseWindow, None)]);

    #[cfg(target_os = "linux")]
    cx.bind_keys([KeyBinding::new("ctrl-q", QuitApplication, None)]);
}

#[cfg(test)]
mod tests;
