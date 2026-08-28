use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
    time::Duration,
};

use gpui::{
    Action, Anchor, App, AppContext as _, Bounds, Context, CursorStyle, DragMoveEvent, ElementId,
    FocusHandle, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement as _, PathPromptOptions, Pixels, Point, Render,
    ScrollHandle, ScrollStrategy, StatefulInteractiveElement as _, Styled as _, Task,
    TitlebarOptions, UniformListScrollHandle, Window, WindowBounds, WindowControlArea,
    WindowOptions, deferred, div, point, prelude::FluentBuilder as _, px, relative, size,
    uniform_list,
};
#[cfg(target_os = "macos")]
use gpui::{Menu, MenuItem, OsAction, SystemMenuType};
use gpui_base::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_base::{
    AutoScroll, Button, POPUP_PRIORITY, Popover, Positioner, Scrollbar, ScrollbarMode, Tab, Tabs,
};
use probe_core::{
    AuthenticationKind, AuthenticationValue, Body, Environment, EnvironmentVariable, FileReference,
    FormField, Header, HttpRequest, MultipartPart, MultipartPartKind, MultipartValue,
    QueryParameter, RawBodyKind, RequestBody, RequestKey, Variable, VariableValue,
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
mod presentation;
mod render;
mod tree;

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
    tree_search::matching_tree_items,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EnvironmentDialogErrorResolution {
    ManagerDraftValid,
    CreateNameValid,
    ManagerClean,
    SavesIdle,
    Manual,
}

struct EnvironmentDialogError {
    message: String,
    resolution: EnvironmentDialogErrorResolution,
}

impl EnvironmentDialogError {
    fn new(message: impl Into<String>, resolution: EnvironmentDialogErrorResolution) -> Self {
        Self {
            message: message.into(),
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
    loaded_workspace: Option<LoadedWorkspace>,
    workspace_path: Option<PathBuf>,
    shell: ShellState,
    loading: bool,
    message: Option<String>,
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
    desktop_menu_open: Option<DesktopMenu>,
    desktop_submenu_open: Option<DesktopSubmenu>,
    workspace_switcher_open: bool,
    workspace_import_submenu_open: bool,
    sidebar_import_menu_open: bool,
    workspace_import_trigger_focus: FocusHandle,
    workspace_import_popup_focus: FocusHandle,
    sidebar_import_trigger_focus: FocusHandle,
    sidebar_import_popup_focus: FocusHandle,
    structure_add_menu_open: bool,
    tree_context_menu: Option<WorkspaceItemRef>,
    tree_context_menu_position: Option<Point<Pixels>>,
    tab_context_menu: Option<RequestKey>,
    tab_context_menu_position: Option<Point<Pixels>>,
    environment_manager_context_menu: Option<String>,
    environment_manager_context_menu_position: Option<Point<Pixels>>,
    request_tab_tooltip: Option<RequestTabTooltip>,
    request_tab_tooltip_epoch: usize,
    request_tab_tooltip_task: Option<Task<()>>,
    visible_tree_rows: Vec<TreeRow>,
    tree_search: String,
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
        let workspace_import_trigger_focus = cx.focus_handle();
        let workspace_import_popup_focus = cx.focus_handle();
        let sidebar_import_trigger_focus = cx.focus_handle();
        let sidebar_import_popup_focus = cx.focus_handle();
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
            loaded_workspace: None,
            workspace_path: None,
            shell: ShellState::default(),
            loading: false,
            message: None,
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
            desktop_menu_open: None,
            desktop_submenu_open: None,
            workspace_switcher_open: false,
            workspace_import_submenu_open: false,
            sidebar_import_menu_open: false,
            workspace_import_trigger_focus,
            workspace_import_popup_focus,
            sidebar_import_trigger_focus,
            sidebar_import_popup_focus,
            structure_add_menu_open: false,
            tree_context_menu: None,
            tree_context_menu_position: None,
            tab_context_menu: None,
            tab_context_menu_position: None,
            environment_manager_context_menu: None,
            environment_manager_context_menu_position: None,
            request_tab_tooltip: None,
            request_tab_tooltip_epoch: 0,
            request_tab_tooltip_task: None,
            visible_tree_rows: Vec::new(),
            tree_search: String::new(),
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

    fn reset_caret_blink(&mut self, cx: &mut Context<Self>) {
        let was_visible = crate::caret::CaretBlink::is_visible(cx);
        crate::caret::CaretBlink::show(cx);
        self._caret_blink = Self::spawn_caret_blink(cx);
        if !was_visible {
            cx.notify();
        }
    }

    fn restore_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.session_store.clone() else {
            return;
        };
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx.background_spawn(async move { store.load() }).await;
                let _ = view.update_in(cx, |view, window, cx| match result {
                    Ok(state) => {
                        let active_path = state.active_collection.clone();
                        view.session = state.clone();
                        if let Some(path) = active_path {
                            view.load_workspace_path(path, Some(state), window, cx);
                        }
                    }
                    Err(error) => {
                        view.message = Some(format!(
                            "Could not restore the previous desktop session: {error}"
                        ));
                        cx.notify();
                    }
                });
            })
            .detach();
    }

    fn choose_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Open Collection".into()),
        });
        let view = cx.weak_entity();

        window
            .spawn(cx, async move |cx| {
                let paths = match receiver.await {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.message = Some(format!("Could not open the file picker: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                    Err(_) => return,
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let _ = view.update_in(cx, |view, window, cx| {
                    view.request_load_workspace(path, None, window, cx);
                });
            })
            .detach();
    }

    fn choose_new_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let directory = self.new_collection_directory();
        let receiver = cx.prompt_for_new_path(&directory, Some("Untitled.yml"));
        let view = cx.weak_entity();

        window
            .spawn(cx, async move |cx| {
                let path = match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.message = Some(format!("Could not open the file picker: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                    Err(_) => return,
                };
                let _ = view.update_in(cx, |view, window, cx| {
                    view.request_create_workspace(path, window, cx);
                });
            })
            .detach();
    }

    fn request_import(
        &mut self,
        source: ImportSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Import(source), window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Import(source));
            self.start_next_environment_save(window, cx);
            return;
        }
        self.choose_import(source, window, cx);
    }

    fn choose_import(&mut self, source: ImportSource, window: &mut Window, cx: &mut Context<Self>) {
        match source {
            ImportSource::Postman => self.choose_postman_import(window, cx),
            ImportSource::Yaak => self.choose_yaak_import(window, cx),
        }
    }

    fn choose_yaak_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: true,
            multiple: false,
            prompt: Some("Import from Yaak".into()),
        });
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let paths = match receiver.await {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) | Err(_) => return,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.message =
                                Some(format!("Could not open the Yaak source picker: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                let Some(source) = paths.into_iter().next() else {
                    return;
                };
                let _ = view.update_in(cx, |view, _, cx| {
                    view.loading = true;
                    view.message = None;
                    cx.notify();
                });
                let preview = match cx
                    .background_spawn(async move { inspect_yaak_source(source) })
                    .await
                {
                    Ok(preview) => preview,
                    Err(error) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            view.message = Some(format!("Could not inspect Yaak data: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                let summaries = preview.workspaces();
                let _ = view.update_in(cx, |view, window, cx| {
                    if let [workspace] = summaries.as_slice() {
                        view.convert_yaak_import(preview, workspace.id.clone(), false, window, cx);
                    } else {
                        view.show_application_dialog(
                            ApplicationDialog::SelectYaakWorkspace {
                                preview,
                                workspaces: summaries,
                            },
                            window,
                            cx,
                        );
                    }
                });
            })
            .detach();
    }

    fn choose_postman_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Import from Postman".into()),
        });
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let paths = match receiver.await {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) | Err(_) => return,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.message =
                                Some(format!("Could not open the Postman source picker: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                let Some(source) = paths.into_iter().next() else {
                    return;
                };
                let _ = view.update_in(cx, |view, _, cx| {
                    view.loading = true;
                    view.message = None;
                    cx.notify();
                });
                let preview = match cx
                    .background_spawn(async move { inspect_postman_source(source) })
                    .await
                {
                    Ok(preview) => preview,
                    Err(error) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            view.message = Some(format!("Could not inspect Postman data: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                };
                let _ = view.update_in(cx, |view, window, cx| {
                    view.convert_postman_import(preview, false, window, cx);
                });
            })
            .detach();
    }

    fn convert_postman_import(
        &mut self,
        preview: PostmanImportPreview,
        allow_partial: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.application_dialog = None;
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx
                    .background_spawn(async move {
                        match preview.convert(allow_partial) {
                            Ok(imported) => PostmanConversionResult::Imported(Box::new(imported)),
                            Err(PostmanImportError::Unsupported(diagnostics)) if !allow_partial => {
                                PostmanConversionResult::NeedsPartialConfirmation {
                                    preview: Box::new(preview),
                                    detail: format_import_diagnostics(&diagnostics),
                                }
                            }
                            Err(error) => PostmanConversionResult::Failed(error.to_string()),
                        }
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| match result {
                    PostmanConversionResult::Imported(imported) => {
                        view.choose_postman_import_destination(*imported, window, cx);
                    }
                    PostmanConversionResult::NeedsPartialConfirmation { preview, detail } => {
                        view.loading = false;
                        view.show_application_dialog(
                            ApplicationDialog::ConfirmPartialPostmanImport { preview, detail },
                            window,
                            cx,
                        );
                    }
                    PostmanConversionResult::Failed(error) => {
                        view.loading = false;
                        view.message = Some(format!("Could not convert Postman data: {error}"));
                        cx.notify();
                    }
                });
            })
            .detach();
    }

    fn choose_postman_import_destination(
        &mut self,
        imported: ImportedPostmanCollection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let filename = suggested_collection_filename(&imported.source.name);
        let receiver = cx.prompt_for_new_path(&self.new_collection_directory(), Some(&filename));
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let destination = match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) | Err(_) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            cx.notify();
                        });
                        return;
                    }
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            view.message = Some(format!(
                                "Could not open the import destination picker: {error}"
                            ));
                            cx.notify();
                        });
                        return;
                    }
                };
                let warning_count = imported.diagnostics.len();
                let selected_environment = imported.collection_variables_environment.clone();
                let result = cx
                    .background_spawn(async move {
                        let workspace = create_bundled_workspace_from_collection(
                            &destination,
                            &imported.collection,
                        )
                        .map_err(|error| error.to_string())?;
                        let canonical_path = workspace
                            .source_path()
                            .ok_or_else(|| {
                                format!(
                                    "imported collection at {} has no filesystem path",
                                    destination.display()
                                )
                            })?
                            .to_owned();
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((path, workspace)) => {
                            view.set_workspace(path, workspace);
                            if let Some(environment) = selected_environment {
                                view.shell.select_environment(Some(environment));
                                view.capture_selected_environment();
                            }
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                            if warning_count > 0 {
                                view.message = Some(format!(
                                    "Imported Postman collection with {warning_count} warning(s)."
                                ));
                            }
                        }
                        Err(error) => {
                            view.message = Some(format!("Could not import Postman data: {error}"));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    fn convert_yaak_import(
        &mut self,
        preview: YaakImportPreview,
        workspace_id: String,
        allow_partial: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.application_dialog = None;
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx
                    .background_spawn(async move {
                        match preview.convert(Some(&workspace_id), allow_partial) {
                            Ok(imported) => YaakConversionResult::Imported(imported),
                            Err(YaakImportError::Unsupported(diagnostics)) if !allow_partial => {
                                YaakConversionResult::NeedsPartialConfirmation {
                                    preview,
                                    workspace_id,
                                    detail: format_import_diagnostics(&diagnostics),
                                }
                            }
                            Err(error) => YaakConversionResult::Failed(error.to_string()),
                        }
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| match result {
                    YaakConversionResult::Imported(imported) => {
                        view.choose_yaak_import_destination(imported, window, cx);
                    }
                    YaakConversionResult::NeedsPartialConfirmation {
                        preview,
                        workspace_id,
                        detail,
                    } => {
                        view.loading = false;
                        view.show_application_dialog(
                            ApplicationDialog::ConfirmPartialYaakImport {
                                preview,
                                workspace_id,
                                detail,
                            },
                            window,
                            cx,
                        );
                    }
                    YaakConversionResult::Failed(error) => {
                        view.loading = false;
                        view.message = Some(format!("Could not convert Yaak data: {error}"));
                        cx.notify();
                    }
                });
            })
            .detach();
    }

    fn choose_yaak_import_destination(
        &mut self,
        imported: ImportedYaakWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let filename = suggested_collection_filename(&imported.workspace.name);
        let receiver = cx.prompt_for_new_path(&self.new_collection_directory(), Some(&filename));
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let destination = match receiver.await {
                    Ok(Ok(Some(path))) => path,
                    Ok(Ok(None)) | Err(_) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            cx.notify();
                        });
                        return;
                    }
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            view.message = Some(format!(
                                "Could not open the import destination picker: {error}"
                            ));
                            cx.notify();
                        });
                        return;
                    }
                };
                let warning_count = imported.diagnostics.len();
                let result = cx
                    .background_spawn(async move {
                        let workspace = create_bundled_workspace_from_collection(
                            &destination,
                            &imported.collection,
                        )
                        .map_err(|error| error.to_string())?;
                        let canonical_path = workspace
                            .source_path()
                            .ok_or_else(|| {
                                format!(
                                    "imported collection at {} has no filesystem path",
                                    destination.display()
                                )
                            })?
                            .to_owned();
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((path, workspace)) => {
                            view.set_workspace(path, workspace);
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                            if warning_count > 0 {
                                view.message = Some(format!(
                                    "Imported Yaak workspace with {warning_count} warning(s)."
                                ));
                            }
                        }
                        Err(error) => {
                            view.message = Some(format!("Could not import Yaak data: {error}"));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    fn new_collection_directory(&self) -> PathBuf {
        if let Some(base) = self
            .workspace_path
            .as_deref()
            .and_then(workspace_base_directory)
            .filter(|path| path.is_dir())
        {
            return base;
        }
        directories::UserDirs::new()
            .and_then(|dirs| {
                dirs.document_dir()
                    .map(Path::to_owned)
                    .or_else(|| Some(dirs.home_dir().to_owned()))
            })
            .filter(|path| path.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
    }

    fn choose_file_path(
        &mut self,
        key: RequestKey,
        window: &mut Window,
        cx: &mut Context<Self>,
        apply: impl FnOnce(&mut HttpRequest, String) + Send + 'static,
    ) {
        let receiver = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Choose File".into()),
        });
        let workspace_path = self.workspace_path.clone();
        let view = cx.weak_entity();

        window
            .spawn(cx, async move |cx| {
                let paths = match receiver.await {
                    Ok(Ok(Some(paths))) => paths,
                    Ok(Ok(None)) => return,
                    Ok(Err(error)) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.message = Some(format!("Could not open the file picker: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                    Err(_) => return,
                };
                let Some(path) = paths.into_iter().next() else {
                    return;
                };
                let stored = body_file_path_for_storage(&path, workspace_path.as_deref());
                let _ = view.update_in(cx, |view, _, cx| {
                    view.edit_request(key, |request| apply(request, stored), cx);
                });
            })
            .detach();
    }

    fn choose_body_file(
        &mut self,
        key: RequestKey,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_file_path(key, window, cx, move |request, stored| {
            if let Some(RequestBody::Single(Body::File(files))) = request.body.as_mut()
                && let Some(file) = files.get_mut(index)
            {
                file.file_path = stored;
            }
        });
    }

    fn choose_multipart_file(
        &mut self,
        key: RequestKey,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_file_path(key, window, cx, move |request, stored| {
            if let Some(RequestBody::Single(Body::Multipart(parts))) = request.body.as_mut()
                && let Some(part) = parts.get_mut(index)
            {
                part.value = MultipartValue::Single(stored);
            }
        });
    }

    fn load_workspace_path(
        &mut self,
        path: PathBuf,
        restored_state: Option<SessionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_selected_environment();
        self.loading = true;
        self.message = None;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let load_path = path.clone();
                let result = cx
                    .background_spawn(async move {
                        let canonical_path = fs::canonicalize(&load_path).map_err(|error| {
                            format!("failed to locate {}: {error}", load_path.display())
                        })?;
                        let workspace =
                            load_workspace(&canonical_path).map_err(|error| error.to_string())?;
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((canonical_path, workspace)) => {
                            view.set_workspace(canonical_path, workspace);
                            if let Some(state) = restored_state {
                                view.session = state;
                                view.restore_shell_state(cx);
                            }
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                        }
                        Err(error) => {
                            if let Some(state) = restored_state {
                                view.session = state;
                                view.session.clear_active_collection();
                                view.persist_session(cx);
                                view.message = Some(format!(
                                    "Could not restore the previous collection. {error}"
                                ));
                            } else {
                                view.message = Some(error);
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    fn request_load_workspace(
        &mut self,
        path: PathBuf,
        restored_state: Option<SessionState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(
                dirty,
                PendingClose::Open {
                    path,
                    restored_state,
                },
                window,
                cx,
            );
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Open {
                path,
                restored_state,
            });
            self.start_next_environment_save(window, cx);
            return;
        }
        self.load_workspace_path(path, restored_state, window, cx);
    }

    fn request_create_workspace(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Create { path }, window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Create { path });
            self.start_next_environment_save(window, cx);
            return;
        }
        self.create_workspace_path(path, window, cx);
    }

    fn create_workspace_path(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.capture_selected_environment();
        self.loading = true;
        self.message = None;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx
                    .background_spawn(async move {
                        let workspace = create_bundled_workspace(&path, None, true)
                            .map_err(|error| error.to_string())?;
                        let canonical_path = workspace
                            .source_path()
                            .ok_or_else(|| {
                                format!(
                                    "created collection at {} has no filesystem path",
                                    path.display()
                                )
                            })?
                            .to_owned();
                        Ok::<_, String>((canonical_path, workspace))
                    })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    view.loading = false;
                    match result {
                        Ok((canonical_path, workspace)) => {
                            view.set_workspace(canonical_path, workspace);
                            view.start_workspace_watcher(window, cx);
                            view.persist_session(cx);
                        }
                        Err(error) => {
                            view.message = Some(error);
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    fn has_pending_environment_work(&self) -> bool {
        self.environment_save_task.is_some() || !self.pending_environment_saves.is_empty()
    }

    fn finish_pending_close_if_idle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_save_task.is_some()
            || self.environment_save_task.is_some()
            || self.persistence.has_outstanding_saves()
        {
            return;
        }
        if !self.pending_environment_saves.is_empty() {
            self.start_next_environment_save(window, cx);
            return;
        }
        if let Some(pending) = self.pending_close.take() {
            let dirty = match &pending {
                PendingClose::Tab(key) => self
                    .request_is_dirty(*key)
                    .then_some(vec![*key])
                    .unwrap_or_default(),
                PendingClose::OtherTabs { keep } => self.other_dirty_tab_keys(*keep),
                PendingClose::Workspace
                | PendingClose::Window
                | PendingClose::Quit
                | PendingClose::Open { .. }
                | PendingClose::Create { .. }
                | PendingClose::Import(_) => self.dirty_keys(),
            };
            if dirty.is_empty() {
                self.finish_pending_close(pending, window, cx);
            } else {
                self.prompt_unsaved(dirty, pending, window, cx);
            }
        }
    }

    fn set_workspace(&mut self, path: PathBuf, workspace: LoadedWorkspace) {
        self.persistence
            .reset(workspace.requests().iter().filter_map(|located| {
                workspace
                    .workspace()
                    .request(located.key())
                    .cloned()
                    .map(|request| (located.key(), request))
            }));
        self.execution.clear();
        self.response_viewer.clear();
        self.loaded_workspace = Some(workspace);
        self.workspace_path = Some(path);
        self.shell.reset_for_workspace();
        self.reset_collection_ui();
        self.pending_environment_saves.clear();
        self.environment_save_workspace_path = None;
        self.restore_selected_environment();
        self.rebuild_visible_tree_rows();
        self.message = None;
    }

    fn start_workspace_watcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filesystem_watch_task = None;
        self.filesystem_watcher = None;
        let Some(path) = self.workspace_path.clone() else {
            return;
        };
        let watcher = match WorkspaceWatcher::start(&path) {
            Ok(watcher) => watcher,
            Err(error) => {
                self.message = Some(format!("Could not watch this collection: {error}"));
                return;
            }
        };
        let WorkspaceWatcher {
            watcher,
            receiver,
            workspace_path,
            ..
        } = watcher;
        self.filesystem_watcher = Some(watcher);
        #[cfg(not(test))]
        let mut receiver = receiver;
        #[cfg(test)]
        let receiver = receiver;
        let view = cx.weak_entity();
        self.filesystem_watch_task = Some(window.spawn(cx, async move |cx| {
            loop {
                let (events, watch_error, disconnected) = {
                    #[cfg(not(test))]
                    {
                        let Some(first) = receiver.recv().await else {
                            return;
                        };
                        cx.background_executor().timer(WATCH_DEBOUNCE).await;
                        let mut events = Vec::new();
                        let mut watch_error = None;
                        match first {
                            Ok(event) => events.push(event),
                            Err(error) => watch_error = Some(error.to_string()),
                        }
                        while let Ok(event) = receiver.try_recv() {
                            match event {
                                Ok(event) => events.push(event),
                                Err(error) => watch_error = Some(error.to_string()),
                            }
                        }
                        (events, watch_error, false)
                    }

                    #[cfg(test)]
                    {
                        loop {
                            let mut events = Vec::new();
                            let mut watch_error = None;
                            let mut disconnected =
                                drain_watch_events(&receiver, &mut events, &mut watch_error);
                            if events.is_empty() && watch_error.is_none() {
                                if disconnected {
                                    return;
                                }
                                cx.background_executor().timer(WATCH_POLL).await;
                                continue;
                            }
                            cx.background_executor().timer(WATCH_DEBOUNCE).await;
                            disconnected |=
                                drain_watch_events(&receiver, &mut events, &mut watch_error);
                            break (events, watch_error, disconnected);
                        }
                    }
                };
                if let Some(error) = watch_error {
                    let _ = view.update_in(cx, |view, _, cx| {
                        view.message = Some(format!("Collection watcher error: {error}"));
                        cx.notify();
                    });
                }
                if !events
                    .iter()
                    .any(|event| event_affects_workspace(event, &workspace_path))
                {
                    if disconnected {
                        return;
                    }
                    continue;
                }
                let hints = rename_hints(&events, &workspace_path);
                let reload_path = workspace_path.clone();
                let result = cx
                    .background_spawn(async move { load_workspace(&reload_path) })
                    .await;
                let _ = view.update_in(cx, |view, window, cx| {
                    if view.workspace_path.as_ref() != Some(&workspace_path) {
                        return;
                    }
                    if view.structure_task.is_some() {
                        return;
                    }
                    match result {
                        Ok(fresh) => view.reconcile_filesystem_workspace(fresh, hints, window, cx),
                        Err(error) => {
                            view.message = Some(format!(
                                "The collection changed on disk but is not yet valid: {error}. The last valid version is still open."
                            ));
                            cx.notify();
                        }
                    }
                });
                if disconnected {
                    return;
                }
            }
        }));
    }

    fn local_request_states(&self) -> Vec<LocalRequestState> {
        let Some(loaded) = &self.loaded_workspace else {
            return Vec::new();
        };
        loaded
            .requests()
            .iter()
            .filter_map(|located| {
                Some(LocalRequestState {
                    selector: located.selector().to_owned(),
                    baseline: self.persistence.saved_request(located.key())?.clone(),
                    local: loaded.workspace().request(located.key())?.clone(),
                })
            })
            .collect()
    }

    fn reconcile_filesystem_workspace(
        &mut self,
        fresh: LoadedWorkspace,
        rename_hints: BTreeMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match reconcile(self.local_request_states(), fresh, &rename_hints) {
            ReconcileResult::Applied(reconciled) => {
                self.apply_reconciled_workspace(*reconciled, cx);
            }
            ReconcileResult::Conflicted(conflicts) => {
                self.prompt_filesystem_conflict(conflicts, window, cx);
            }
        }
    }

    fn show_application_dialog(
        &mut self,
        dialog: ApplicationDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.application_dialog.is_some() {
            self.enqueue_application_dialog(dialog);
            return;
        }
        self.structure_dialog = None;
        self.create_environment_dialog = None;
        self.dismiss_transient_surfaces();
        self.application_dialog = Some(dialog);
        self.application_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn enqueue_application_dialog(&mut self, dialog: ApplicationDialog) {
        if let ApplicationDialog::FilesystemConflict { path, .. } = &dialog
            && (matches!(
                &self.application_dialog,
                Some(ApplicationDialog::FilesystemConflict {
                    path: current_path,
                    ..
                }) if current_path == path
            ) || self.pending_application_dialogs.iter().any(|pending| {
                matches!(
                    pending,
                    ApplicationDialog::FilesystemConflict {
                        path: pending_path,
                        ..
                    } if pending_path == path
                )
            }))
        {
            return;
        }
        self.pending_application_dialogs.push_back(dialog);
    }

    fn show_next_application_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.application_dialog.is_none()
            && let Some(dialog) = self.pending_application_dialogs.pop_front()
        {
            self.show_application_dialog(dialog, window, cx);
        }
    }

    fn dismiss_transient_surfaces(&mut self) {
        self.desktop_menu_open = None;
        self.desktop_submenu_open = None;
        self.workspace_switcher_open = false;
        self.workspace_import_submenu_open = false;
        self.sidebar_import_menu_open = false;
        self.structure_add_menu_open = false;
        self.tree_context_menu = None;
        self.tree_context_menu_position = None;
        self.tab_context_menu = None;
        self.tab_context_menu_position = None;
        self.environment_manager_context_menu = None;
        self.environment_manager_context_menu_position = None;
    }

    fn open_desktop_menu(&mut self, menu: DesktopMenu, cx: &mut Context<Self>) {
        self.desktop_menu_open = Some(menu);
        self.desktop_submenu_open = None;
        cx.notify();
    }

    fn close_desktop_menu(&mut self, cx: &mut Context<Self>) {
        self.desktop_menu_open = None;
        self.desktop_submenu_open = None;
        cx.notify();
    }

    fn handle_application_dialog_action(
        &mut self,
        action: ApplicationDialogAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(dialog) = self.application_dialog.take() else {
            return;
        };
        match (dialog, action) {
            (ApplicationDialog::Unsaved { keys, pending }, ApplicationDialogAction::Save) => {
                self.pending_close = Some(pending);
                self.persistence.enqueue(keys);
                self.start_next_request_save(window, cx);
            }
            (ApplicationDialog::Unsaved { keys, pending }, ApplicationDialogAction::Discard) => {
                self.discard_dirty_requests(&keys);
                self.finish_pending_close(pending, window, cx);
            }
            (ApplicationDialog::UnsavedEnvironment, ApplicationDialogAction::Save) => {
                self.environment_manager_close_after_save = true;
                self.save_environment_manager_dialog(window, cx);
                if self.environment_save_task.is_none() {
                    self.environment_manager_close_after_save = false;
                    self.restore_environment_dialog_focus(window, cx);
                }
            }
            (ApplicationDialog::UnsavedEnvironment, ApplicationDialogAction::Discard) => {
                self.close_environment_manager_dialog(window, cx);
            }
            (ApplicationDialog::Delete { kind, selector, .. }, ApplicationDialogAction::Delete) => {
                let operation = match kind {
                    ItemKind::Request => StructureOperation::DeleteRequest { selector },
                    ItemKind::Folder => StructureOperation::DeleteFolder { selector },
                };
                self.apply_structure(operation, window, cx);
            }
            (
                ApplicationDialog::DeleteEnvironment { name, .. },
                ApplicationDialogAction::Delete,
            ) => self.delete_environment(name, window, cx),
            (
                ApplicationDialog::FilesystemConflict { path, .. },
                ApplicationDialogAction::UseDisk,
            ) => self.reload_conflicted_workspace(path, window, cx),
            (ApplicationDialog::FilesystemConflict { .. }, ApplicationDialogAction::KeepLocal) => {
                self.message = Some(
                    "Kept local edits. Probe will not overwrite the changed disk files; resolve the conflict before saving."
                        .to_owned(),
                );
                cx.notify();
            }
            (
                ApplicationDialog::SelectYaakWorkspace {
                    preview,
                    workspaces,
                },
                ApplicationDialogAction::SelectWorkspace(index),
            ) => {
                if let Some(workspace) = workspaces.get(index) {
                    self.convert_yaak_import(preview, workspace.id.clone(), false, window, cx);
                } else {
                    self.loading = false;
                    cx.notify();
                }
            }
            (
                ApplicationDialog::ConfirmPartialYaakImport {
                    preview,
                    workspace_id,
                    ..
                },
                ApplicationDialogAction::ImportSupportedData,
            ) => self.convert_yaak_import(preview, workspace_id, true, window, cx),
            (
                ApplicationDialog::ConfirmPartialPostmanImport { preview, .. },
                ApplicationDialogAction::ImportSupportedData,
            ) => self.convert_postman_import(*preview, true, window, cx),
            (
                ApplicationDialog::SelectYaakWorkspace { .. }
                | ApplicationDialog::ConfirmPartialYaakImport { .. }
                | ApplicationDialog::ConfirmPartialPostmanImport { .. },
                ApplicationDialogAction::Cancel,
            ) => {
                self.loading = false;
                self.focus_handle.focus(window, cx);
                cx.notify();
            }
            (_, ApplicationDialogAction::Cancel) => {
                self.restore_environment_dialog_focus(window, cx);
                cx.notify();
            }
            (dialog, _) => {
                self.application_dialog = Some(dialog);
                cx.notify();
            }
        }
        self.show_next_application_dialog(window, cx);
    }

    fn reload_conflicted_workspace(
        &mut self,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_path != path {
            return;
        }
        let Some(path) = path else {
            return;
        };
        self.loading = true;
        cx.notify();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let result = cx.background_spawn(async move { load_workspace(path) }).await;
                let _ = view.update_in(cx, |view, _, cx| {
                    view.loading = false;
                    match result {
                        Ok(workspace) => {
                            let clean_local = view
                                .local_request_states()
                                .into_iter()
                                .map(|mut state| {
                                    state.local.clone_from(&state.baseline);
                                    state
                                })
                                .collect();
                            if let ReconcileResult::Applied(reconciled) =
                                reconcile(clean_local, workspace, &BTreeMap::new())
                            {
                                view.apply_reconciled_workspace(*reconciled, cx);
                                view.message = Some(
                                    "Reloaded the collection from disk; conflicting local edits were discarded."
                                        .to_owned(),
                                );
                            }
                        }
                        Err(error) => {
                            view.message = Some(format!(
                                "Could not reload the collection from disk: {error}"
                            ));
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
    }

    fn prompt_filesystem_conflict(
        &mut self,
        conflicts: Vec<SynchronizationConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = conflicts
            .iter()
            .take(3)
            .map(SynchronizationConflict::description)
            .collect::<Vec<_>>()
            .join("; ");
        self.show_application_dialog(
            ApplicationDialog::FilesystemConflict {
                path: self.workspace_path.clone(),
                detail: format!(
                    "{detail}. Choose Use Disk to discard the conflicting local edits, or Keep Local to retain them without overwriting disk."
                ),
            },
            window,
            cx,
        );
    }

    fn reset_collection_ui(&mut self) {
        self.selected_tree_item = None;
        self.structure_dialog = None;
        self.create_environment_dialog = None;
        self.discard_environment_manager_dialog();
        self.application_dialog = None;
        self.pending_application_dialogs.clear();
        self.dismiss_transient_surfaces();
        self.clear_tree_drag();
        self.tree_search.clear();
        self.request_editor.clear();
    }

    fn snapshot_shell_selectors(&self, old: &LoadedWorkspace) -> ShellSelectors {
        ShellSelectors {
            tab_selectors: self
                .shell
                .tabs()
                .iter()
                .filter_map(|key| old.request_selector(*key).map(str::to_owned))
                .collect(),
            active_selector: self
                .shell
                .active_tab()
                .and_then(|key| old.request_selector(key))
                .map(str::to_owned),
            folder_selectors: self
                .shell
                .collapsed_folders()
                .filter_map(|key| old.folder_selector(key).map(str::to_owned))
                .collect(),
            selected: self.selected_tree_item.and_then(|item| match item {
                WorkspaceItemRef::Request(key) => old
                    .request_selector(key)
                    .map(|selector| (ItemKind::Request, selector.to_owned())),
                WorkspaceItemRef::Folder(key) => old
                    .folder_selector(key)
                    .map(|selector| (ItemKind::Folder, selector.to_owned())),
            }),
        }
    }

    fn install_reloaded_workspace(
        &mut self,
        workspace: LoadedWorkspace,
        baselines: Vec<(RequestKey, HttpRequest)>,
        key_remaps: &BTreeMap<RequestKey, RequestKey>,
    ) {
        self.persistence.reset(baselines);
        self.loaded_workspace = Some(workspace);
        self.shell.reset_for_workspace();
        self.execution.remap_requests(key_remaps);
        self.response_viewer.remap_requests(key_remaps);
        self.request_editor.remap_requests(key_remaps);
    }

    fn remap_structure_dialog(&mut self, remaps: &BTreeMap<String, String>) {
        let Some(dialog) = self.structure_dialog.as_mut() else {
            return;
        };
        let Some(loaded) = self.loaded_workspace.as_ref() else {
            self.structure_dialog = None;
            return;
        };

        let mut target_exists = true;
        match &mut dialog.mode {
            StructureDialogMode::CreateRequest | StructureDialogMode::CreateFolder => {}
            StructureDialogMode::Rename { kind, selector }
            | StructureDialogMode::Move { kind, selector } => {
                if let Some(mapped) = remaps.get(selector) {
                    selector.clone_from(mapped);
                }
                target_exists = match kind {
                    ItemKind::Request => loaded.request_key(selector).is_some(),
                    ItemKind::Folder => loaded.folder_key(selector).is_some(),
                };
            }
        }

        if !target_exists {
            self.structure_dialog = None;
            return;
        }

        if !dialog.parent.is_empty() {
            if let Some(mapped) = remaps.get(&dialog.parent) {
                dialog.parent.clone_from(mapped);
            }
            if loaded.folder_key(&dialog.parent).is_none() {
                self.structure_dialog = None;
            }
        }
    }

    fn restore_shell_selectors(
        &mut self,
        remaps: &BTreeMap<String, String>,
        selectors: ShellSelectors,
    ) {
        let loaded = self
            .loaded_workspace
            .as_ref()
            .expect("workspace was replaced");
        for selector in selectors.tab_selectors {
            if let Some(key) = remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
            {
                self.shell.open_request(key);
            }
        }
        if let Some(selector) = selectors.active_selector
            && let Some(key) = remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
        {
            self.shell.open_request(key);
        }
        for selector in selectors.folder_selectors {
            let selector = remaps
                .get(&selector)
                .map_or(selector.as_str(), String::as_str);
            if let Some(key) = loaded.folder_key(selector) {
                self.shell.collapse_folder(key);
            }
        }
        self.selected_tree_item = selectors.selected.and_then(|(kind, selector)| match kind {
            ItemKind::Request => remaps
                .get(&selector)
                .and_then(|selector| loaded.request_key(selector))
                .map(WorkspaceItemRef::Request),
            ItemKind::Folder => {
                let selector = remaps
                    .get(&selector)
                    .map_or(selector.as_str(), String::as_str);
                loaded.folder_key(selector).map(WorkspaceItemRef::Folder)
            }
        });
    }

    fn apply_reconciled_workspace(
        &mut self,
        mut reconciled: ReconciledWorkspace,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.loaded_workspace.as_ref() else {
            return;
        };
        let selectors = self.snapshot_shell_selectors(old);
        let key_remaps =
            request_key_remaps(old, &reconciled.workspace, &reconciled.selector_remaps);
        let baselines = reconciled
            .workspace
            .requests()
            .iter()
            .filter_map(|located| {
                reconciled
                    .disk_baselines
                    .remove(located.selector())
                    .map(|request| (located.key(), request))
            })
            .collect::<Vec<_>>();
        let environment_manager_reload = self.environment_manager_reload_snapshot(old);
        self.install_reloaded_workspace(reconciled.workspace, baselines, &key_remaps);
        self.restore_shell_selectors(&reconciled.selector_remaps, selectors);
        self.remap_structure_dialog(&reconciled.selector_remaps);
        self.create_environment_dialog = None;
        self.message = None;
        self.sync_environment_manager_after_reload(environment_manager_reload);
        if self.shell.selected_environment().is_some_and(|name| {
            !self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced")
                .workspace()
                .environments()
                .iter()
                .any(|environment| environment.name == name)
        }) {
            self.shell.select_environment(None);
        }
        self.rebuild_visible_tree_rows();
        self.persist_session(cx);
        cx.notify();
    }

    fn restore_shell_state(&mut self, cx: &mut Context<Self>) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let tabs: Vec<_> = self
            .session
            .open_tabs
            .iter()
            .filter_map(|selector| loaded.request_key(selector))
            .collect();
        let active_tab = self
            .session
            .active_tab
            .as_deref()
            .and_then(|selector| loaded.request_key(selector));
        let collapsed_folders: Vec<_> = self
            .session
            .collapsed_folders
            .iter()
            .filter_map(|selector| loaded.folder_key(selector))
            .collect();

        self.shell.restore_pane_sizes(
            self.session.sidebar_width,
            self.session.response_height,
            self.session.response_width,
        );
        self.shell.sidebar_collapsed = self.session.sidebar_collapsed;
        self.shell
            .set_pane_layout(if self.session.horizontal_panes {
                PaneLayout::Horizontal
            } else {
                PaneLayout::Vertical
            });
        self.refresh_system_menu(cx);
        for key in tabs {
            self.shell.open_request(key);
        }
        if let Some(key) = active_tab {
            self.shell.open_request(key);
            self.selected_tree_item = Some(WorkspaceItemRef::Request(key));
        }
        for key in collapsed_folders {
            self.shell.collapse_folder(key);
        }
        self.rebuild_visible_tree_rows();
        self.reveal_active_tab();
    }

    fn capture_session(&mut self) {
        self.session.sidebar_width = self.shell.sidebar_width;
        self.session.sidebar_collapsed = self.shell.sidebar_collapsed;
        self.session.response_height = self.shell.response_height;
        self.session.response_width = self.shell.response_width;
        self.session.horizontal_panes = self.shell.pane_layout == PaneLayout::Horizontal;
        let (Some(path), Some(loaded)) = (&self.workspace_path, &self.loaded_workspace) else {
            self.session.clear_active_collection();
            return;
        };
        self.session.activate_collection(path.clone());
        self.session.open_tabs = self
            .shell
            .tabs()
            .iter()
            .filter_map(|key| loaded.request_selector(*key).map(str::to_owned))
            .collect();
        self.session.active_tab = self
            .shell
            .active_tab()
            .and_then(|key| loaded.request_selector(key))
            .map(str::to_owned);
        self.session.collapsed_folders = self
            .shell
            .collapsed_folders()
            .filter_map(|key| loaded.folder_selector(key).map(str::to_owned))
            .collect();
        self.session.collapsed_folders.sort();
        self.session.remember_selected_environment(
            path.clone(),
            self.shell.selected_environment().map(str::to_owned),
        );
    }

    fn capture_selected_environment(&mut self) {
        let Some(path) = self.workspace_path.clone() else {
            return;
        };
        self.session.remember_selected_environment(
            path,
            self.shell.selected_environment().map(str::to_owned),
        );
    }

    fn restore_selected_environment(&mut self) {
        let (Some(path), Some(loaded)) = (&self.workspace_path, &self.loaded_workspace) else {
            self.shell.select_environment(None);
            return;
        };
        let name = self
            .session
            .selected_environment_for(path)
            .filter(|name| {
                loaded
                    .workspace()
                    .environments()
                    .iter()
                    .any(|environment| environment.name == *name)
            })
            .map(str::to_owned);
        self.shell.select_environment(name);
    }

    fn persist_session(&mut self, cx: &mut Context<Self>) {
        self.capture_session();
        let Some(store) = self.session_store.clone() else {
            return;
        };
        let state = self.session.clone();
        self.session_save_task = Some(cx.spawn(async move |view, cx| {
            let result = cx.background_spawn(async move { store.save(&state) }).await;
            if let Err(error) = result {
                let _ = view.update(cx, |view, cx| {
                    view.message = Some(format!("Could not save desktop session state: {error}"));
                    cx.notify();
                });
            }
        }));
    }

    fn close_workspace_now(&mut self, cx: &mut Context<Self>) {
        self.capture_selected_environment();
        self.execution.clear();
        self.response_viewer.clear();
        self.pending_environment_saves.clear();
        self.environment_save_workspace_path = None;
        self.loaded_workspace = None;
        self.workspace_path = None;
        self.shell.reset_for_workspace();
        self.shell.select_environment(None);
        self.reset_collection_ui();
        self.persistence.clear();
        self.filesystem_watch_task = None;
        self.filesystem_watcher = None;
        self.visible_tree_rows.clear();
        self.session.clear_active_collection();
        self.persist_session(cx);
        cx.notify();
    }

    fn select_environment(&mut self, environment: Option<String>, cx: &mut Context<Self>) {
        self.shell.select_environment(environment);
        self.persist_session(cx);
        cx.notify();
    }

    fn show_environment_dialog_error(
        &mut self,
        message: impl Into<String>,
        resolution: EnvironmentDialogErrorResolution,
    ) {
        self.environment_dialog_error = Some(EnvironmentDialogError::new(message, resolution));
    }

    fn environment_manager_draft_has_required_names(&self) -> bool {
        self.environment_manager_dialog
            .as_ref()
            .is_some_and(|dialog| {
                !dialog.draft.name.trim().is_empty()
                    && dialog.draft.variables.iter().all(|variable| {
                        !matches!(
                            variable,
                            EnvironmentVariable::Plain(variable)
                                if variable.name.as_deref().is_none_or(|name| name.trim().is_empty())
                        )
                    })
            })
    }

    fn clear_resolved_environment_dialog_error(&mut self) {
        let Some(resolution) = self
            .environment_dialog_error
            .as_ref()
            .map(|error| error.resolution)
        else {
            return;
        };
        let resolved = match resolution {
            EnvironmentDialogErrorResolution::ManagerDraftValid => {
                self.environment_manager_draft_has_required_names()
            }
            EnvironmentDialogErrorResolution::CreateNameValid => self
                .create_environment_dialog
                .as_deref()
                .is_some_and(|name| !name.trim().is_empty()),
            EnvironmentDialogErrorResolution::ManagerClean => !self.environment_manager_is_dirty(),
            EnvironmentDialogErrorResolution::SavesIdle => {
                self.environment_save_task.is_none()
                    && self.request_save_task.is_none()
                    && self.structure_task.is_none()
                    && self.pending_environment_saves.is_empty()
            }
            EnvironmentDialogErrorResolution::Manual => false,
        };
        if resolved {
            self.environment_dialog_error = None;
        }
    }

    fn open_environment_manager_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .shell
            .selected_environment()
            .and_then(|name| {
                loaded
                    .workspace()
                    .environments()
                    .iter()
                    .find(|environment| environment.name == name)
            })
            .or_else(|| loaded.workspace().environments().first());
        let Some(selected) = selected else {
            self.open_create_environment_dialog(window, cx);
            return;
        };
        self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(selected));
        self.environment_dialog_error = None;
        self.environment_manager_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn request_close_environment_manager_dialog(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_application_dialog(ApplicationDialog::UnsavedEnvironment, window, cx);
            return;
        }
        self.close_environment_manager_dialog(window, cx);
    }

    fn close_environment_manager_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some() {
            return;
        }
        self.discard_environment_manager_dialog();
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    fn discard_environment_manager_dialog(&mut self) {
        self.environment_manager_context_menu = None;
        self.environment_manager_context_menu_position = None;
        self.environment_manager_close_after_save = false;
        self.environment_manager_dialog = None;
        self.environment_dialog_error = None;
    }

    fn environment_manager_reload_snapshot(
        &self,
        old: &LoadedWorkspace,
    ) -> Option<(EnvironmentManagerDialog, Option<Environment>)> {
        let dialog = self.environment_manager_dialog.as_ref()?;
        let original = old
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
            .cloned();
        Some((dialog.clone(), original))
    }

    fn sync_environment_manager_after_reload(
        &mut self,
        previous: Option<(EnvironmentManagerDialog, Option<Environment>)>,
    ) {
        let Some((dialog, previous_original)) = previous else {
            return;
        };
        self.environment_manager_context_menu = None;
        self.environment_manager_context_menu_position = None;
        let Some(disk) = self.loaded_workspace.as_ref().and_then(|loaded| {
            loaded
                .workspace()
                .environments()
                .iter()
                .find(|environment| environment.name == dialog.original_name)
                .cloned()
        }) else {
            self.discard_environment_manager_dialog();
            return;
        };
        let dirty = previous_original
            .as_ref()
            .is_none_or(|original| original != &dialog.draft);
        let environment_changed_on_disk = previous_original
            .as_ref()
            .is_none_or(|original| original != &disk);
        if !dirty {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&disk));
            return;
        }
        if environment_changed_on_disk {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&disk));
            self.show_environment_dialog_error(
                "This environment changed on disk. Unsaved environment edits were discarded."
                    .to_owned(),
                EnvironmentDialogErrorResolution::Manual,
            );
            return;
        }
        self.environment_manager_dialog = Some(dialog);
    }

    fn apply_environment_manager_draft(
        &mut self,
        cx: &mut Context<Self>,
        update: impl FnOnce(&mut EnvironmentManagerDialog),
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        if let Some(dialog) = self.environment_manager_dialog.as_mut() {
            update(dialog);
            cx.notify();
        }
    }

    fn select_environment_manager_environment(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some() {
            return;
        }
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let Some(original) = loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
        else {
            return;
        };
        if &dialog.draft != original {
            self.show_environment_dialog_error(
                "Save or cancel the current environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
            );
            cx.notify();
            return;
        }
        if let Some(environment) = loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == name)
        {
            self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(environment));
            self.environment_dialog_error = None;
            cx.notify();
        }
    }

    fn save_environment_manager_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
            || !self.pending_environment_saves.is_empty()
        {
            self.show_environment_dialog_error(
                "Wait for the current save to finish.",
                EnvironmentDialogErrorResolution::SavesIdle,
            );
            cx.notify();
            return;
        }
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return;
        };
        let mut replacement = dialog.draft.clone();
        replacement.name = replacement.name.trim().to_owned();
        for variable in &mut replacement.variables {
            if let EnvironmentVariable::Plain(variable) = variable
                && let Some(name) = variable.name.as_mut()
            {
                *name = name.trim().to_owned();
            }
        }
        let invalid_variable = replacement.variables.iter().any(|variable| {
            matches!(
                variable,
                EnvironmentVariable::Plain(variable)
                    if variable.name.as_deref().is_none_or(str::is_empty)
            )
        });
        if replacement.name.is_empty() || invalid_variable {
            self.show_environment_dialog_error(
                "Environment and variable names are required.",
                EnvironmentDialogErrorResolution::ManagerDraftValid,
            );
            cx.notify();
            return;
        }
        let original_name = dialog.original_name.clone();
        let saved_name = replacement.name.clone();
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let prepared = match loaded.prepare_environment_replace(&original_name, replacement) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not save environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.complete_environment_replace(saved);
                            let environment = loaded
                                .workspace()
                                .environments()
                                .iter()
                                .find(|environment| environment.name == saved_name)
                                .cloned();
                            if let Some(environment) = environment {
                                if view.shell.selected_environment() == Some(original_name.as_str())
                                {
                                    view.select_environment(Some(environment.name.clone()), cx);
                                }
                                if view.environment_manager_close_after_save {
                                    view.close_environment_manager_dialog(window, cx);
                                } else {
                                    view.environment_manager_dialog =
                                        Some(EnvironmentManagerDialog::new(&environment));
                                }
                            }
                        }
                        view.environment_manager_close_after_save = false;
                        view.environment_dialog_error = None;
                    }
                    Err(error) => {
                        view.environment_manager_close_after_save = false;
                        view.show_environment_dialog_error(
                            format!("Could not save environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                        );
                    }
                }
                view.environment_save_workspace_path = None;
                view.start_next_request_save(window, cx);
                view.start_next_environment_save(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn confirm_delete_environment(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_application_dialog(
            ApplicationDialog::DeleteEnvironment {
                name,
                detail: "The environment and its variables will be removed. This cannot be undone."
                    .to_owned(),
            },
            window,
            cx,
        );
    }

    fn delete_selected_environment_from_manager(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some() {
            return;
        }
        let Some(name) = self
            .environment_manager_dialog
            .as_ref()
            .map(|dialog| dialog.original_name.clone())
        else {
            return;
        };
        self.confirm_delete_environment(name, window, cx);
    }

    fn delete_environment(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            self.show_environment_dialog_error(
                "Wait for the current save to finish.",
                EnvironmentDialogErrorResolution::SavesIdle,
            );
            cx.notify();
            return;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let prepared = match loaded.prepare_environment_delete(&name) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not delete environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        let close_manager = view.complete_deleted_environment(saved, &name, cx);
                        view.environment_dialog_error = None;
                        if close_manager {
                            view.close_environment_manager_dialog(window, cx);
                        }
                    }
                    Err(error) => {
                        view.show_environment_dialog_error(
                            format!("Could not delete environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                        );
                    }
                }
                view.environment_save_workspace_path = None;
                view.start_next_request_save(window, cx);
                view.start_next_environment_save(window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn complete_deleted_environment(
        &mut self,
        saved: CompletedEnvironmentDelete,
        name: &str,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.environment_save_workspace_path != self.workspace_path
            || self.loaded_workspace.is_none()
        {
            return false;
        }
        let deleted_current = self
            .environment_manager_dialog
            .as_ref()
            .is_some_and(|dialog| dialog.original_name == name);
        let deleted_index = self.loaded_workspace.as_ref().and_then(|loaded| {
            loaded
                .workspace()
                .environments()
                .iter()
                .position(|environment| environment.name == name)
        });
        self.loaded_workspace
            .as_mut()
            .expect("workspace was present")
            .complete_environment_delete(saved);
        if self.shell.selected_environment() == Some(name) {
            self.select_environment(None, cx);
        }
        if !deleted_current {
            return false;
        }
        let next = self.loaded_workspace.as_ref().and_then(|loaded| {
            let environments = loaded.workspace().environments();
            deleted_index.and_then(|index| {
                environments
                    .get(index)
                    .or_else(|| environments.get(index.saturating_sub(1)))
                    .cloned()
            })
        });
        match next {
            Some(environment) => {
                self.environment_manager_dialog = Some(EnvironmentManagerDialog::new(&environment));
                false
            }
            None => true,
        }
    }

    fn environment_manager_is_dirty(&self) -> bool {
        let Some(dialog) = self.environment_manager_dialog.as_ref() else {
            return false;
        };
        let Some(loaded) = self.loaded_workspace.as_ref() else {
            return false;
        };
        loaded
            .workspace()
            .environments()
            .iter()
            .find(|environment| environment.name == dialog.original_name)
            .is_none_or(|environment| environment != &dialog.draft)
    }

    fn restore_environment_dialog_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_manager_dialog.is_some() {
            self.environment_manager_dialog_focus.focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
    }

    fn open_create_environment_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_workspace.is_none()
            || self.structure_task.is_some()
            || self.environment_save_task.is_some()
            || self.request_save_task.is_some()
        {
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_environment_dialog_error(
                "Save or discard unsaved environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
            );
            cx.notify();
            return;
        }
        self.structure_dialog = None;
        self.environment_dialog_error = None;
        self.create_environment_dialog = Some(String::new());
        self.create_environment_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn close_create_environment_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.environment_dialog_error = None;
        self.restore_environment_dialog_focus(window, cx);
        cx.notify();
    }

    fn submit_create_environment_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.create_environment_dialog.as_ref() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() {
            self.show_environment_dialog_error(
                "Environment name is required.",
                EnvironmentDialogErrorResolution::CreateNameValid,
            );
            cx.notify();
            return;
        }
        self.create_named_environment(name, window, cx);
    }

    fn create_named_environment(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            self.show_environment_dialog_error(
                "Wait for the current save before creating an environment.",
                EnvironmentDialogErrorResolution::SavesIdle,
            );
            cx.notify();
            return;
        }
        if self.environment_manager_is_dirty() {
            self.show_environment_dialog_error(
                "Save or discard unsaved environment changes first.",
                EnvironmentDialogErrorResolution::ManagerClean,
            );
            cx.notify();
            return;
        }
        let Some(loaded) = self.loaded_workspace.as_mut() else {
            return;
        };
        let prepared = match loaded.prepare_environment_create(name.clone(), None) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.show_environment_dialog_error(
                    format!("Could not create environment: {error}"),
                    EnvironmentDialogErrorResolution::Manual,
                );
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_dialog_error = None;
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.complete_environment_create(saved);
                            view.environment_manager_dialog = loaded
                                .workspace()
                                .environments()
                                .iter()
                                .find(|environment| environment.name == name)
                                .map(EnvironmentManagerDialog::new);
                            view.select_environment(Some(name), cx);
                        }
                        view.environment_save_workspace_path = None;
                        view.create_environment_dialog = None;
                        view.environment_dialog_error = None;
                        view.restore_environment_dialog_focus(window, cx);
                        view.start_next_request_save(window, cx);
                        view.start_next_environment_save(window, cx);
                    }
                    Err(error) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.revert_created_environment(&name);
                            if view.shell.selected_environment() == Some(name.as_str()) {
                                view.select_environment(None, cx);
                            }
                        }
                        view.environment_save_workspace_path = None;
                        view.pending_close = None;
                        view.show_environment_dialog_error(
                            format!("Could not create environment: {error}"),
                            EnvironmentDialogErrorResolution::Manual,
                        );
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn select_request(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        if self
            .loaded_workspace
            .as_ref()
            .is_some_and(|loaded| loaded.workspace().request(key).is_some())
        {
            self.selected_tree_item = Some(WorkspaceItemRef::Request(key));
            self.shell.open_request(key);
            self.response_viewer.ensure_available_tab(key);
            self.start_base64_encoding(key, cx);
            self.reveal_active_tab();
            if self
                .loaded_workspace
                .as_mut()
                .and_then(|loaded| loaded.request_mut(key))
                .is_some_and(ensure_path_parameters_from_url)
            {
                self.persistence.edited(key);
            }
            self.persist_session(cx);
            cx.notify();
        }
    }

    fn select_tree_item(&mut self, item: WorkspaceItemRef, cx: &mut Context<Self>) {
        self.selected_tree_item = Some(item);
        cx.notify();
    }

    fn selected_parent_selector(&self) -> Option<String> {
        let loaded = self.loaded_workspace.as_ref()?;
        let selected = self.selected_tree_item?;
        if let WorkspaceItemRef::Folder(key) = selected {
            return loaded.folder_selector(key).map(str::to_owned);
        }
        let (parent, _) = item_position(loaded.workspace(), selected)?;
        parent.and_then(|key| loaded.folder_selector(key).map(str::to_owned))
    }

    fn open_create_request_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_workspace.is_none() || self.structure_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.structure_dialog = Some(StructureDialog::create_request(
            self.selected_parent_selector(),
        ));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn open_create_folder_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.loaded_workspace.is_none() || self.structure_task.is_some() {
            return;
        }
        self.create_environment_dialog = None;
        self.structure_dialog = Some(StructureDialog::create_folder(
            self.selected_parent_selector(),
        ));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn selected_item_details(&self) -> Option<(ItemKind, String, String)> {
        let loaded = self.loaded_workspace.as_ref()?;
        match self.selected_tree_item? {
            WorkspaceItemRef::Request(key) => Some((
                ItemKind::Request,
                loaded.request_selector(key)?.to_owned(),
                loaded
                    .workspace()
                    .request(key)?
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "Untitled request".to_owned()),
            )),
            WorkspaceItemRef::Folder(key) => Some((
                ItemKind::Folder,
                loaded.folder_selector(key)?.to_owned(),
                loaded
                    .workspace()
                    .folder(key)?
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "Untitled folder".to_owned()),
            )),
        }
    }

    fn open_rename_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, name)) = self.selected_item_details() else {
            return;
        };
        self.structure_dialog = Some(StructureDialog::rename(kind, selector, name));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn open_move_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, _)) = self.selected_item_details() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .selected_tree_item
            .expect("details require a selection");
        let Some((parent, _)) = item_position(loaded.workspace(), selected) else {
            return;
        };
        let parent = parent.and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        self.structure_dialog = Some(StructureDialog::move_item(kind, selector, parent));
        self.structure_dialog_focus.focus(window, cx);
        cx.notify();
    }

    fn reorder_selected(&mut self, offset: isize, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, _)) = self.selected_item_details() else {
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let selected = self
            .selected_tree_item
            .expect("details require a selection");
        let Some((_, index)) = item_position(loaded.workspace(), selected) else {
            return;
        };
        let Some(index) = index.checked_add_signed(offset) else {
            return;
        };
        let operation = match kind {
            ItemKind::Request => StructureOperation::ReorderRequest { selector, index },
            ItemKind::Folder => StructureOperation::ReorderFolder { selector, index },
        };
        self.apply_structure(operation, window, cx);
    }

    fn duplicate_selected_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((ItemKind::Request, selector, _)) = self.selected_item_details() else {
            return;
        };
        self.apply_structure(
            StructureOperation::DuplicateRequest { selector },
            window,
            cx,
        );
    }

    fn request_delete_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.structure_task.is_some() {
            return;
        }
        let Some((kind, selector, name)) = self.selected_item_details() else {
            return;
        };
        let dirty_count = match self.selected_tree_item {
            Some(WorkspaceItemRef::Request(key)) => self
                .loaded_workspace
                .as_ref()
                .and_then(|loaded| {
                    loaded
                        .workspace()
                        .request(key)
                        .map(|request| (key, request))
                })
                .is_some_and(|(key, request)| self.persistence.is_dirty(key, request))
                as usize,
            Some(WorkspaceItemRef::Folder(key)) => {
                let mut requests = Vec::new();
                if let Some(loaded) = &self.loaded_workspace {
                    descendant_requests(loaded.workspace(), key, &mut requests);
                }
                requests
                    .into_iter()
                    .filter(|key| {
                        self.loaded_workspace
                            .as_ref()
                            .and_then(|loaded| loaded.workspace().request(*key))
                            .is_some_and(|request| self.persistence.is_dirty(*key, request))
                    })
                    .count()
            }
            None => 0,
        };
        let detail = if dirty_count == 0 {
            "This cannot be undone.".to_owned()
        } else {
            format!(
                "This will discard unsaved changes in {dirty_count} request(s) and cannot be undone."
            )
        };
        self.show_application_dialog(
            ApplicationDialog::Delete {
                kind,
                selector,
                name,
                detail,
            },
            window,
            cx,
        );
    }

    fn submit_structure_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dialog) = self.structure_dialog.as_ref() else {
            return;
        };
        match dialog.operation() {
            Ok(operation) => {
                self.structure_dialog = None;
                self.focus_handle.focus(window, cx);
                self.apply_structure(operation, window, cx);
            }
            Err(message) => {
                self.message = Some(message);
                cx.notify();
            }
        }
    }

    fn submit_application_dialog_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(action) = self
            .application_dialog
            .as_ref()
            .and_then(ApplicationDialog::primary_action)
        else {
            return;
        };
        self.handle_application_dialog_action(action, window, cx);
    }

    fn submit_application_dialog_destructive(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self
            .application_dialog
            .as_ref()
            .and_then(ApplicationDialog::destructive_action)
        else {
            return;
        };
        self.handle_application_dialog_action(action, window, cx);
    }

    fn apply_structure(
        &mut self,
        operation: StructureOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        if self.request_save_task.is_some() || self.environment_save_task.is_some() {
            self.message =
                Some("Wait for the current save before changing collection structure.".to_owned());
            cx.notify();
            return;
        }
        let (Some(mut workspace), Some(path)) =
            (self.loaded_workspace.clone(), self.workspace_path.clone())
        else {
            return;
        };
        self.loading = true;
        self.message = None;
        let operation_for_task = operation.clone();
        self.structure_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move {
                    let structure_result = workspace
                        .apply_structure(operation_for_task)
                        .map_err(|error| error.to_string())?;
                    let disk_workspace =
                        load_workspace(&path).map_err(|error| error.to_string())?;
                    Ok::<_, String>((workspace, disk_workspace, structure_result))
                })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.structure_task = None;
                view.loading = false;
                match result {
                    Ok((workspace, disk_workspace, result)) => {
                        view.apply_structure_result(
                            workspace,
                            disk_workspace,
                            result,
                            &operation,
                            window,
                            cx,
                        );
                    }
                    Err(error) => {
                        view.message =
                            Some(format!("Could not edit collection structure: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn apply_structure_result(
        &mut self,
        mut workspace: LoadedWorkspace,
        disk_workspace: LoadedWorkspace,
        result: StructureResult,
        operation: &StructureOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(old) = self.loaded_workspace.as_ref() else {
            return;
        };
        let selectors = self.snapshot_shell_selectors(old);
        let key_remaps = request_key_remaps(old, &workspace, &result.selector_remaps);
        let current_requests = old
            .requests()
            .iter()
            .filter_map(|located| {
                old.workspace()
                    .request(located.key())
                    .cloned()
                    .map(|request| (located.selector().to_owned(), request))
            })
            .collect::<Vec<_>>();

        for (old_selector, mut request) in current_requests {
            let Some(new_selector) = result.selector_remaps.get(&old_selector) else {
                continue;
            };
            let Some(new_key) = workspace.request_key(new_selector) else {
                continue;
            };
            let persisted = disk_workspace
                .request_key(new_selector)
                .and_then(|key| disk_workspace.workspace().request(key));
            if let Some(persisted) = persisted {
                request.metadata.sequence = persisted.metadata.sequence;
                if matches!(
                    operation,
                    StructureOperation::RenameRequest { selector, .. }
                        if selector == &old_selector
                ) {
                    request.metadata.name.clone_from(&persisted.metadata.name);
                }
            }
            if let Some(target) = workspace.request_mut(new_key) {
                *target = request;
            }
        }

        let baselines = workspace
            .requests()
            .iter()
            .filter_map(|located| {
                let disk_key = disk_workspace.request_key(located.selector())?;
                let baseline = disk_workspace.workspace().request(disk_key)?.clone();
                Some((located.key(), baseline))
            })
            .collect::<Vec<_>>();
        self.install_reloaded_workspace(workspace, baselines, &key_remaps);
        self.restore_shell_selectors(&result.selector_remaps, selectors);
        let should_select_result = matches!(
            operation,
            StructureOperation::CreateRequest { .. }
                | StructureOperation::CreateFolder { .. }
                | StructureOperation::DuplicateRequest { .. }
        );
        if matches!(
            operation,
            StructureOperation::CreateRequest { .. } | StructureOperation::CreateFolder { .. }
        ) && let Some(parent) = result.parent.as_deref()
        {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            if let Some(key) = loaded.folder_key(parent) {
                self.shell.expand_folder(key);
            }
        }
        if should_select_result && let Some(selector) = result.selector.as_deref() {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            self.selected_tree_item = match result.kind {
                ItemKind::Request => loaded.request_key(selector).map(WorkspaceItemRef::Request),
                ItemKind::Folder => loaded.folder_key(selector).map(WorkspaceItemRef::Folder),
            };
            if matches!(operation, StructureOperation::DuplicateRequest { .. })
                && self.structure_dialog.is_none()
            {
                self.tree_focus_handle.focus(window, cx);
            }
        }
        if self.selected_tree_item.is_none()
            && let Some(selector) = result.selector.as_deref()
        {
            let loaded = self
                .loaded_workspace
                .as_ref()
                .expect("workspace was replaced after structural edit");
            self.selected_tree_item = match result.kind {
                ItemKind::Request => loaded.request_key(selector).map(WorkspaceItemRef::Request),
                ItemKind::Folder => loaded.folder_key(selector).map(WorkspaceItemRef::Folder),
            };
        }
        if let Some(WorkspaceItemRef::Request(key)) = self.selected_tree_item
            && result.previous_selector.is_none()
        {
            self.shell.open_request(key);
        }
        self.rebuild_visible_tree_rows();
        if should_select_result {
            self.scroll_selected_tree_item_into_view();
        }
        self.reveal_active_tab();
        self.message = None;
        self.persist_session(cx);
    }

    fn scroll_selected_tree_item_into_view(&self) {
        let Some(selected) = self.selected_tree_item else {
            return;
        };
        if let Some(index) = self
            .visible_tree_rows
            .iter()
            .position(|row| row.item == selected)
        {
            self.tree_scroll
                .scroll_to_item(index, ScrollStrategy::Nearest);
        }
    }

    fn select_tree_offset(&mut self, offset: isize, cx: &mut Context<Self>) {
        if self.visible_tree_rows.is_empty() {
            return;
        }
        let current = self
            .selected_tree_item
            .and_then(|item| {
                self.visible_tree_rows
                    .iter()
                    .position(|row| row.item == item)
            })
            .unwrap_or(if offset < 0 {
                self.visible_tree_rows.len()
            } else {
                0
            });
        let next = current
            .checked_add_signed(offset)
            .unwrap_or(0)
            .min(self.visible_tree_rows.len() - 1);
        self.selected_tree_item = Some(self.visible_tree_rows[next].item);
        self.tree_scroll
            .scroll_to_item(next, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn activate_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        match self.selected_tree_item {
            Some(WorkspaceItemRef::Request(key)) => self.select_request(key, cx),
            Some(WorkspaceItemRef::Folder(key)) => {
                self.shell.toggle_folder(key);
                self.rebuild_visible_tree_rows();
                self.persist_session(cx);
                cx.notify();
            }
            None => self.select_tree_offset(0, cx),
        }
    }

    fn collapse_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        let Some(selected) = self.selected_tree_item else {
            return;
        };
        match selected {
            WorkspaceItemRef::Folder(key) if self.shell.folder_is_expanded(key) => {
                self.shell.collapse_folder(key);
                self.rebuild_visible_tree_rows();
                self.persist_session(cx);
                cx.notify();
            }
            _ => {
                let Some(loaded) = &self.loaded_workspace else {
                    return;
                };
                if let Some((Some(parent), _)) = item_position(loaded.workspace(), selected) {
                    self.selected_tree_item = Some(WorkspaceItemRef::Folder(parent));
                    cx.notify();
                }
            }
        }
    }

    fn expand_selected_tree_item(&mut self, cx: &mut Context<Self>) {
        let Some(WorkspaceItemRef::Folder(key)) = self.selected_tree_item else {
            return;
        };
        if !self.shell.folder_is_expanded(key) {
            self.shell.toggle_folder(key);
            self.rebuild_visible_tree_rows();
            self.persist_session(cx);
            cx.notify();
        }
    }

    fn close_tab_now(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        self.shell.close_tab(key);
        self.reveal_active_tab();
        self.persist_session(cx);
        cx.notify();
    }

    fn close_other_tabs_now(&mut self, keep: RequestKey, cx: &mut Context<Self>) {
        let open_tabs = self.shell.tabs().to_vec();
        if !open_tabs.contains(&keep) {
            return;
        }
        for key in open_tabs {
            if key != keep {
                self.shell.close_tab(key);
            }
        }
        self.shell.open_request(keep);
        self.reveal_active_tab();
        self.persist_session(cx);
        cx.notify();
    }

    fn dirty_keys(&self) -> Vec<RequestKey> {
        let Some(loaded) = &self.loaded_workspace else {
            return Vec::new();
        };
        self.persistence
            .dirty_keys(loaded.requests().iter().filter_map(|located| {
                loaded
                    .workspace()
                    .request(located.key())
                    .map(|request| (located.key(), request))
            }))
    }

    fn request_is_dirty(&self, key: RequestKey) -> bool {
        self.loaded_workspace
            .as_ref()
            .and_then(|loaded| loaded.workspace().request(key))
            .is_some_and(|request| self.persistence.is_dirty(key, request))
    }

    fn request_close_tab(&mut self, key: RequestKey, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_context_menu(cx);
        if self.request_is_dirty(key) {
            self.prompt_unsaved(vec![key], PendingClose::Tab(key), window, cx);
        } else {
            self.close_tab_now(key, cx);
        }
    }

    fn other_dirty_tab_keys(&self, keep: RequestKey) -> Vec<RequestKey> {
        self.shell
            .tabs()
            .iter()
            .copied()
            .filter(|key| *key != keep)
            .filter(|key| self.request_is_dirty(*key))
            .collect()
    }

    fn request_close_other_tabs(
        &mut self,
        keep: RequestKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_tab_context_menu(cx);
        if !self.shell.tabs().contains(&keep) {
            return;
        }
        let dirty = self.other_dirty_tab_keys(keep);
        if dirty.is_empty() {
            self.close_other_tabs_now(keep, cx);
        } else {
            self.prompt_unsaved(dirty, PendingClose::OtherTabs { keep }, window, cx);
        }
    }

    fn request_close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Workspace, window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Workspace);
            self.start_next_environment_save(window, cx);
            return;
        }
        self.close_workspace_now(cx);
    }

    fn request_close_window(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.application_dialog.is_some() {
            return false;
        }
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Window, window, cx);
            return false;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Window);
            self.start_next_environment_save(window, cx);
            return false;
        }
        true
    }

    fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.shell.toggle_sidebar();
        self.persist_session(cx);
        cx.notify();
    }

    fn set_pane_layout(&mut self, layout: PaneLayout, cx: &mut Context<Self>) {
        self.shell.set_pane_layout(layout);
        self.refresh_system_menu(cx);
        self.persist_session(cx);
        cx.notify();
    }

    #[cfg(target_os = "macos")]
    fn refresh_system_menu(&self, cx: &mut Context<Self>) {
        cx.set_menus(system_menus(self.shell.pane_layout));
    }

    #[cfg(not(target_os = "macos"))]
    fn refresh_system_menu(&self, _: &mut Context<Self>) {}

    fn quit_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.application_dialog.is_some() {
            return;
        }
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::Quit, window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::Quit);
            self.start_next_environment_save(window, cx);
            return;
        }
        cx.quit();
    }

    fn prompt_unsaved(
        &mut self,
        keys: Vec<RequestKey>,
        pending: PendingClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_close.is_some() {
            return;
        }
        self.show_application_dialog(ApplicationDialog::Unsaved { keys, pending }, window, cx);
    }

    fn discard_dirty_requests(&mut self, keys: &[RequestKey]) {
        let Some(loaded) = self.loaded_workspace.as_mut() else {
            return;
        };
        for key in keys {
            let Some(saved) = self.persistence.saved_request(*key).cloned() else {
                continue;
            };
            if let Some(request) = loaded.request_mut(*key) {
                *request = saved;
            }
        }
    }

    fn save_active_request(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(key) = self.shell.active_tab() {
            let dirty = self
                .loaded_workspace
                .as_ref()
                .and_then(|loaded| loaded.workspace().request(key))
                .is_some_and(|request| self.persistence.is_dirty(key, request));
            if dirty {
                self.persistence.enqueue([key]);
                self.start_next_request_save(window, cx);
            }
        }
    }

    fn start_next_request_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.request_save_task.is_some() || self.environment_save_task.is_some() {
            return;
        }
        let Some(key) = self.persistence.next() else {
            self.finish_pending_close_if_idle(window, cx);
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            self.persistence.fail(key);
            return;
        };
        let Some(request) = loaded.workspace().request(key) else {
            self.persistence.fail(key);
            return;
        };
        let Some(selector) = loaded.request_selector(key).map(str::to_owned) else {
            self.persistence.fail(key);
            return;
        };
        let (_revision, snapshot, update) = self.persistence.begin(key, request);
        let prepared = match loaded.prepare_request_save(&selector, update) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.persistence.fail(key);
                self.pending_close = None;
                self.message = Some(format!("Could not save request: {error}"));
                cx.notify();
                return;
            }
        };
        self.request_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.request_save_task = None;
                match result {
                    Ok(saved) => {
                        if let Some(loaded) = view.loaded_workspace.as_mut() {
                            loaded.complete_request_save(saved);
                        }
                        view.persistence.complete(key, snapshot);
                        view.message = None;
                        view.start_next_request_save(window, cx);
                        view.start_next_environment_save(window, cx);
                    }
                    Err(error) => {
                        view.persistence.fail(key);
                        view.pending_close = None;
                        view.message = Some(format!("Could not save request: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn finish_pending_close(
        &mut self,
        pending: PendingClose,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending_close = None;
        match pending {
            PendingClose::Tab(key) => self.close_tab_now(key, cx),
            PendingClose::OtherTabs { keep } => self.close_other_tabs_now(keep, cx),
            PendingClose::Workspace => self.close_workspace_now(cx),
            PendingClose::Window => window.remove_window(),
            PendingClose::Quit => cx.quit(),
            PendingClose::Open {
                path,
                restored_state,
            } => self.load_workspace_path(path, restored_state, window, cx),
            PendingClose::Create { path } => self.create_workspace_path(path, window, cx),
            PendingClose::Import(source) => self.choose_import(source, window, cx),
        }
    }

    fn reveal_active_tab(&mut self) {
        self.scroll_active_tab_into_view();
        self.pending_tab_reveal = true;
    }

    fn scroll_active_tab_into_view(&self) {
        let Some(active) = self.shell.active_tab() else {
            return;
        };
        let Some(index) = self.shell.tabs().iter().position(|tab| *tab == active) else {
            return;
        };
        self.tab_bar_scroll.scroll_to_item(index);
    }

    fn send_request(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        let Some(request) = self
            .loaded_workspace
            .as_ref()
            .and_then(|loaded| loaded.workspace().request(key))
            .cloned()
        else {
            return;
        };
        let selected_environment = self.shell.selected_environment().map(str::to_owned);
        let request = if let Some(environment_name) = selected_environment {
            let Some(loaded) = &self.loaded_workspace else {
                return;
            };
            match resolve_environment(loaded.workspace().environments(), &environment_name)
                .and_then(|environment| resolve_request(&request, &environment))
            {
                Ok(request) => request,
                Err(error) => {
                    self.execution.fail(key, error.to_string());
                    self.response_viewer.remove(key);
                    cx.notify();
                    return;
                }
            }
        } else {
            request
        };
        let options = ExecutionOptions {
            base_directory: self
                .workspace_path
                .as_deref()
                .and_then(workspace_base_directory),
            response_cache: Some(self.response_cache.clone()),
        };
        let (cancellation_sender, cancellation_receiver) = tokio::sync::oneshot::channel();
        let (result_sender, result_receiver) = tokio::sync::oneshot::channel();
        let generation = self.execution.begin(key, cancellation_sender);
        let spawn_result = thread::Builder::new()
            .name("probe-http-request".to_owned())
            .spawn(move || {
                let result = execute_http_request(request, options, cancellation_receiver);
                let _ = result_sender.send(result);
            });
        if let Err(error) = spawn_result {
            self.execution
                .fail(key, format!("Could not start HTTP execution: {error}"));
            self.response_viewer.remove(key);
            cx.notify();
            return;
        }

        cx.spawn(async move |view, cx| {
            let result = result_receiver.await.unwrap_or_else(|_| {
                Err(HttpError::Transport(
                    "HTTP execution ended without a result".to_owned(),
                ))
            });
            let _ = view.update(cx, |view, cx| {
                view.complete_execution(key, generation, result, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn complete_execution(
        &mut self,
        key: RequestKey,
        generation: u64,
        result: Result<HttpResponse, HttpError>,
        cx: &mut Context<Self>,
    ) {
        self.execution.finish(key, generation, result);
        self.refresh_response_document(key, cx);
    }

    fn refresh_response_document(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        let Some(ResponseState::Complete(response)) = self.execution.response(key) else {
            self.response_viewer.remove(key);
            return;
        };
        let generation = self.response_viewer.allocate_generation();
        let (document, pretty_pending, inspection_pending) = prepare_document(response, generation);
        let pretty_job = pretty_pending.then(|| (response.body.clone(), document.syntax));
        let inspection_source = inspection_pending.then(|| {
            response.body_file.clone().map_or_else(
                || Ok(response.body.clone()),
                |file| Err((file, document.syntax)),
            )
        });
        self.response_viewer.insert(key, document);
        if self.shell.active_tab() == Some(key) {
            self.response_viewer.ensure_available_tab(key);
        }
        self.start_base64_encoding(key, cx);
        if let Some((body, syntax)) = pretty_job {
            cx.spawn(async move |view, cx| {
                let pretty = cx
                    .background_spawn(async move { pretty_body(&body, syntax) })
                    .await;
                let _ = view.update(cx, |view, cx| {
                    view.response_viewer.apply_pretty(key, generation, pretty);
                    cx.notify();
                });
            })
            .detach();
        }
        if let Some(source) = inspection_source {
            cx.spawn(async move |view, cx| {
                let inspection = cx
                    .background_spawn(async move {
                        match source {
                            Ok(body) => inspect_response_body(&body),
                            Err((file, ResponseBodySyntax::Json)) => inspect_json_file(file.path()),
                            Err((file, ResponseBodySyntax::Xml)) => inspect_xml_file(file.path()),
                            Err((_, ResponseBodySyntax::Plain)) => Default::default(),
                        }
                    })
                    .await;
                let _ = view.update(cx, |view, cx| {
                    view.response_viewer
                        .apply_inspection(key, generation, inspection);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn set_response_tab(&mut self, tab: ResponseViewerTab, cx: &mut Context<Self>) {
        self.response_viewer.set_tab(tab);
        if let Some(key) = self.shell.active_tab() {
            self.start_base64_encoding(key, cx);
        }
        cx.notify();
    }

    fn set_raw_body_view(&mut self, view: RawBodyView, cx: &mut Context<Self>) {
        self.response_viewer.set_raw_view(view);
        if let Some(key) = self.shell.active_tab() {
            self.start_base64_encoding(key, cx);
        }
        cx.notify();
    }

    fn start_base64_encoding(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        let Some((generation, bytes)) = self.response_viewer.take_base64_job(key) else {
            return;
        };
        cx.spawn(async move |view, cx| {
            let encoded = cx
                .background_spawn(async move { encode_base64(&bytes) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.response_viewer.apply_base64(key, generation, encoded);
                cx.notify();
            });
        })
        .detach();
    }

    fn load_response_page(
        &mut self,
        key: RequestKey,
        direction: PageDirection,
        cx: &mut Context<Self>,
    ) {
        let Some(ResponseState::Complete(response)) = self.execution.response(key) else {
            return;
        };
        let Some(file) = response.body_file.clone() else {
            return;
        };
        let Some((generation, offset)) = self.response_viewer.begin_page(key, direction) else {
            return;
        };
        let length = response
            .size
            .saturating_sub(offset)
            .min(RESPONSE_PAGE_BYTES);
        cx.spawn(async move |view, cx| {
            let result = cx
                .background_spawn(async move { read_response_page(&file, offset, length) })
                .await;
            let _ = view.update(cx, |view, cx| {
                match result {
                    Ok(body) => {
                        view.response_viewer
                            .apply_page(key, generation, offset, body);
                        view.start_base64_encoding(key, cx);
                    }
                    Err(error) => view.response_viewer.fail_page(
                        key,
                        generation,
                        format!("Could not read retained response: {error}"),
                    ),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn cancel_request(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        self.execution.cancel(key);
        self.response_viewer.remove(key);
        cx.notify();
    }

    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some((start_x, start_width)) = self.inspector_resize_start {
            let delta = f32::from(event.position.x) - start_x;
            self.inspector_list_width =
                (start_width + delta).clamp(MIN_INSPECT_LIST_WIDTH, MAX_INSPECT_LIST_WIDTH);
            cx.notify();
            return;
        }
        match self.shell.resizing {
            Some(ResizePane::Sidebar) => self.shell.resize_sidebar(event.position.x.into()),
            Some(ResizePane::Response) => match self.shell.pane_layout {
                PaneLayout::Vertical => self.shell.resize_response(
                    window.window_bounds().get_bounds().size.height.into(),
                    event.position.y.into(),
                ),
                PaneLayout::Horizontal => self.shell.resize_response_width(
                    window.window_bounds().get_bounds().size.width.into(),
                    event.position.x.into(),
                ),
            },
            None => return,
        }
        cx.notify();
    }

    fn finish_resize(&mut self, cx: &mut Context<Self>) {
        let was_inspector_resizing = self.inspector_resize_start.take().is_some();
        if self.shell.resizing.take().is_none() && !was_inspector_resizing {
            return;
        }
        self.persist_session(cx);
        cx.notify();
    }

    fn set_tree_search(&mut self, query: String, cx: &mut Context<Self>) {
        if self.tree_search == query {
            return;
        }
        self.tree_search = query;
        let expanded = self.expand_folders_for_tree_search();
        self.rebuild_visible_tree_rows();
        if expanded {
            self.persist_session(cx);
        }
        cx.notify();
    }

    fn expand_folders_for_tree_search(&mut self) -> bool {
        let query = self.tree_search.trim();
        if query.is_empty() {
            return false;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return false;
        };
        let hits = matching_tree_items(loaded.workspace(), query);
        let mut expanded = false;
        for folder in hits.folders() {
            if !self.shell.folder_is_expanded(folder) {
                self.shell.expand_folder(folder);
                expanded = true;
            }
        }
        expanded
    }

    fn rebuild_visible_tree_rows(&mut self) {
        let Some(loaded) = &self.loaded_workspace else {
            self.visible_tree_rows.clear();
            return;
        };
        let workspace = loaded.workspace();
        let query = self.tree_search.trim();
        let filter = if query.is_empty() {
            None
        } else {
            Some(matching_tree_items(workspace, query))
        };
        let mut rows = Vec::with_capacity(workspace.request_count());
        flatten_visible_tree_rows(
            workspace,
            workspace.root_items(),
            0,
            &self.shell,
            filter.as_ref(),
            &mut rows,
        );
        self.visible_tree_rows = rows;
    }

    fn clear_tree_drag(&mut self) {
        self.tree_drag_source = None;
        self.tree_drop_target = None;
        self.tree_list_bounds = None;
        self.tree_auto_scroll.stop();
    }

    fn scroll_tree_by(&mut self, delta: Pixels) {
        let handle = self.tree_scroll.0.borrow().base_handle.clone();
        let mut offset = handle.offset();
        let max = handle.max_offset();
        offset.y = (offset.y - delta).max(-max.y).min(px(0.0));
        handle.set_offset(offset);
    }

    fn on_tree_drag_move(
        &mut self,
        event: &DragMoveEvent<TreeDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let source = event.drag(cx).item;
        self.tree_drag_source = Some(source);
        self.tree_list_bounds = Some(event.bounds);
        self.tree_auto_scroll.last_drag_position = Some(event.event.position);
        self.tree_row_height = Theme::for_window_appearance(window.appearance())
            .metrics
            .tree_row_height;
        let in_x = event.event.position.x >= event.bounds.left()
            && event.event.position.x <= event.bounds.right();
        let delta = in_x
            .then(|| AutoScroll::compute_delta(event.event.position.y, event.bounds))
            .flatten();
        self.tree_auto_scroll.set(delta, cx, |delta, view, cx| {
            view.scroll_tree_by(delta);
            view.recompute_tree_drop_from_stored_pointer(None, cx);
            cx.notify();
        });
        if in_x || event.bounds.contains(&event.event.position) {
            self.recompute_tree_drop(source, event.event.position, event.bounds, Some(window), cx);
        } else if delta.is_none() {
            self.tree_drop_target = None;
            cx.set_active_drag_cursor_style(CursorStyle::OperationNotAllowed, window);
            cx.notify();
        }
    }

    fn recompute_tree_drop_from_stored_pointer(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let (Some(source), Some(pointer), Some(bounds)) = (
            self.tree_drag_source,
            self.tree_auto_scroll.last_drag_position,
            self.tree_list_bounds,
        ) else {
            return;
        };
        self.recompute_tree_drop(source, pointer, bounds, window, cx);
    }

    fn recompute_tree_drop(
        &mut self,
        source: WorkspaceItemRef,
        pointer: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) {
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let pointer_y = pointer.y.into();
        let list_top = bounds.top().into();
        let scroll_y = self.tree_scroll.0.borrow().base_handle.offset().y.into();
        let Some((hovered_index, relative_y)) = hovered_row_index(
            pointer_y,
            list_top,
            TREE_LIST_PADDING_Y,
            scroll_y,
            self.tree_row_height,
            self.visible_tree_rows.len() + 1,
        ) else {
            self.tree_drop_target = None;
            return;
        };
        let root_end_drop = hovered_index == self.visible_tree_rows.len();
        let hovered = if root_end_drop {
            self.visible_tree_rows.last().copied()
        } else {
            self.visible_tree_rows.get(hovered_index).copied()
        };
        let Some(hovered) = hovered else {
            self.tree_drop_target = None;
            return;
        };
        let folder_expanded = match hovered.item {
            WorkspaceItemRef::Folder(key) => self.shell.folder_is_expanded(key),
            WorkspaceItemRef::Request(_) => false,
        };
        let zone = drop_zone(
            matches!(hovered.item, WorkspaceItemRef::Folder(_)),
            relative_y,
        );
        let intent = if root_end_drop {
            TreeDropIntent {
                parent: None,
                index: loaded.workspace().root_items().len(),
                indicator: DropIndicator::RootEnd,
            }
        } else {
            let Some(intent) = drop_intent(loaded.workspace(), hovered.item, zone, folder_expanded)
            else {
                self.tree_drop_target = None;
                return;
            };
            intent
        };
        let Some((source_parent, source_index)) = item_position(loaded.workspace(), source) else {
            self.tree_drop_target = None;
            return;
        };
        let source_selector = match source {
            WorkspaceItemRef::Request(key) => loaded.request_selector(key).map(str::to_owned),
            WorkspaceItemRef::Folder(key) => loaded.folder_selector(key).map(str::to_owned),
        };
        let Some(source_selector) = source_selector else {
            self.tree_drop_target = None;
            return;
        };
        let dest_parent_selector = intent
            .parent
            .and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        let duplicate_path = would_duplicate_path(
            loaded.uses_path_locators(),
            &source_selector,
            dest_parent_selector.as_deref(),
            |selector| {
                loaded.request_key(selector).is_some() || loaded.folder_key(selector).is_some()
            },
        );
        let cursor = match validate_tree_drop(
            loaded.workspace(),
            source,
            source_parent,
            source_index,
            intent,
            duplicate_path,
        ) {
            Ok(intent) => {
                self.tree_drop_target = Some(intent);
                CursorStyle::ClosedHand
            }
            Err(DropReject::NoOp) => {
                self.tree_drop_target = None;
                CursorStyle::ClosedHand
            }
            Err(_) => {
                self.tree_drop_target = None;
                CursorStyle::OperationNotAllowed
            }
        };
        if let Some(window) = window {
            cx.set_active_drag_cursor_style(cursor, window);
        }
        cx.notify();
    }

    fn drop_tree_item(&mut self, drag: &TreeDrag, window: &mut Window, cx: &mut Context<Self>) {
        let intent = self.tree_drop_target.take();
        self.clear_tree_drag();
        let Some(intent) = intent else {
            return;
        };
        if self.structure_task.is_some() {
            return;
        }
        let Some(loaded) = &self.loaded_workspace else {
            return;
        };
        let Some(selector) = (match drag.item {
            WorkspaceItemRef::Request(key) => loaded.request_selector(key),
            WorkspaceItemRef::Folder(key) => loaded.folder_selector(key),
        })
        .map(str::to_owned) else {
            return;
        };
        let Some((source_parent, source_index)) = item_position(loaded.workspace(), drag.item)
        else {
            return;
        };
        let dest_parent_selector = intent
            .parent
            .and_then(|key| loaded.folder_selector(key).map(str::to_owned));
        let Some(operation) = structure_operation_for_drop(
            drag.kind,
            selector,
            source_parent,
            source_index,
            intent.parent,
            dest_parent_selector,
            intent.index,
        ) else {
            return;
        };
        self.apply_structure(operation, window, cx);
    }

    fn open_tree_context_menu(
        &mut self,
        item: WorkspaceItemRef,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.structure_task.is_some() {
            return;
        }
        self.tree_context_menu = Some(item);
        self.tree_context_menu_position = Some(position);
        self.select_tree_item(item, cx);
    }

    fn close_tree_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.tree_context_menu.is_none() {
            return;
        }
        self.tree_context_menu = None;
        self.tree_context_menu_position = None;
        cx.notify();
    }

    fn open_tab_context_menu(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.tabs().contains(&key) {
            return;
        }
        self.tab_context_menu = Some(key);
        self.tab_context_menu_position = Some(position);
        self.request_tab_tooltip = None;
        cx.notify();
    }

    fn close_tab_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.tab_context_menu.is_none() {
            return;
        }
        self.tab_context_menu = None;
        self.tab_context_menu_position = None;
        cx.notify();
    }

    fn open_environment_manager_context_menu(
        &mut self,
        name: String,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if self.environment_manager_dialog.is_none() || self.environment_save_task.is_some() {
            return;
        }
        self.environment_manager_context_menu = Some(name);
        self.environment_manager_context_menu_position = Some(position);
        cx.notify();
    }

    fn close_environment_manager_context_menu(&mut self, cx: &mut Context<Self>) {
        if self.environment_manager_context_menu.is_none() {
            return;
        }
        self.environment_manager_context_menu = None;
        self.environment_manager_context_menu_position = None;
        cx.notify();
    }

    fn open_request_tab_tooltip(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !self.shell.tabs().contains(&key) {
            return;
        }
        self.request_tab_tooltip_epoch = self.request_tab_tooltip_epoch.wrapping_add(1);
        let epoch = self.request_tab_tooltip_epoch;
        self.request_tab_tooltip = Some(RequestTabTooltip {
            key,
            position,
            open: false,
        });
        self.request_tab_tooltip_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(REQUEST_TAB_TOOLTIP_DELAY)
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.request_tab_tooltip_epoch == epoch
                    && let Some(tooltip) = view.request_tab_tooltip.as_mut()
                {
                    tooltip.open = true;
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn update_request_tab_tooltip_position(
        &mut self,
        key: RequestKey,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let Some(tooltip) = self.request_tab_tooltip.as_mut() else {
            return;
        };
        if tooltip.key != key {
            return;
        }
        tooltip.position = position;
        if tooltip.open {
            cx.notify();
        }
    }

    fn close_request_tab_tooltip(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        if self
            .request_tab_tooltip
            .is_none_or(|tooltip| tooltip.key != key)
        {
            return;
        }
        self.request_tab_tooltip_epoch = self.request_tab_tooltip_epoch.wrapping_add(1);
        self.request_tab_tooltip_task = None;
        self.request_tab_tooltip = None;
        cx.notify();
    }

    fn update_environment_variable(
        &mut self,
        name: &str,
        value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(environment) = self.shell.selected_environment().map(str::to_owned) else {
            return;
        };
        let Some(loaded) = self.loaded_workspace.as_mut() else {
            return;
        };
        if loaded
            .set_environment_variable(&environment, name, value)
            .is_ok()
        {
            self.pending_environment_saves
                .insert((environment, name.to_owned()));
            self.start_next_environment_save(window, cx);
            cx.notify();
        }
    }

    fn start_next_environment_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.environment_save_task.is_some()
            || self.request_save_task.is_some()
            || self.structure_task.is_some()
        {
            return;
        }
        let Some((environment, name)) = self.pending_environment_saves.pop_first() else {
            self.finish_pending_close_if_idle(window, cx);
            return;
        };
        let Some(loaded) = &self.loaded_workspace else {
            self.pending_environment_saves.insert((environment, name));
            self.finish_pending_close_if_idle(window, cx);
            return;
        };
        let prepared = match loaded.prepare_environment_variable_save(&environment, &name) {
            Ok(prepared) => prepared,
            Err(error) => {
                self.pending_environment_saves.insert((environment, name));
                self.pending_close = None;
                self.message = Some(format!("Could not save environment variable: {error}"));
                cx.notify();
                return;
            }
        };
        self.environment_save_workspace_path = self.workspace_path.clone();
        self.environment_save_task = Some(cx.spawn_in(window, async move |view, window| {
            let result = window
                .background_spawn(async move { prepared.execute() })
                .await;
            let _ = view.update_in(window, |view, window, cx| {
                view.environment_save_task = None;
                match result {
                    Ok(saved) => {
                        if view.environment_save_workspace_path == view.workspace_path
                            && let Some(loaded) = view.loaded_workspace.as_mut()
                        {
                            loaded.complete_environment_save(saved);
                        }
                        view.environment_save_workspace_path = None;
                        view.message = None;
                        view.start_next_request_save(window, cx);
                        view.start_next_environment_save(window, cx);
                    }
                    Err(error) => {
                        view.environment_save_workspace_path = None;
                        view.pending_close = None;
                        view.message =
                            Some(format!("Could not save environment variable: {error}"));
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
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
                    Some(point(px(9.0), px(9.0)))
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
        KeyBinding::new(
            "enter",
            SubmitEnvironmentManagerDialog,
            Some("EnvironmentManagerDialog"),
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

    #[cfg(target_os = "windows")]
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenWorkspace, None),
        KeyBinding::new("ctrl-n", NewCollection, None),
        KeyBinding::new("ctrl-s", SaveRequest, None),
        KeyBinding::new("ctrl-w", CloseActiveTab, None),
        KeyBinding::new("alt-f4", CloseWindow, None),
    ]);

    #[cfg(target_os = "linux")]
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenWorkspace, None),
        KeyBinding::new("ctrl-n", NewCollection, None),
        KeyBinding::new("ctrl-s", SaveRequest, None),
        KeyBinding::new("ctrl-w", CloseActiveTab, None),
        KeyBinding::new("ctrl-q", QuitApplication, None),
    ]);
}

#[cfg(test)]
mod tests;
