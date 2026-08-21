use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    thread,
};

use gpui::{
    Anchor, App, AppContext as _, Bounds, Context, CursorStyle, DragMoveEvent, FocusHandle,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, MouseMoveEvent, ParentElement as _, PathPromptOptions, Pixels, Point,
    PromptButton, PromptLevel, Render, ScrollHandle, ScrollStrategy,
    StatefulInteractiveElement as _, Styled as _, Task, TitlebarOptions, UniformListScrollHandle,
    Window, WindowBounds, WindowControlArea, WindowOptions, deferred, div, point,
    prelude::FluentBuilder as _, px, relative, size, uniform_list,
};
use gpui_base::{AutoScroll, Button, POPUP_PRIORITY, Popover, Positioner, Tab, Tabs};
use probe_core::{
    AuthenticationKind, AuthenticationValue, Body, FileReference, FormField, Header, HttpRequest,
    MultipartPart, MultipartPartKind, MultipartValue, QueryParameter, RawBodyKind, RequestBody,
    RequestKey, Workspace, WorkspaceItemRef, add_path_parameter, ensure_path_parameters_from_url,
    remove_path_parameter_at, rename_path_parameter_at, resolve_environment, resolve_request,
};
use probe_http::{ExecutionOptions, HttpError, HttpResponse};
use probe_opencollection::{
    ItemKind, LoadedWorkspace, StructureOperation, StructureResult, create_bundled_workspace,
    create_bundled_workspace_from_collection, load_workspace,
};
use probe_yaak::{
    ImportDiagnostic, ImportDiagnosticSeverity, YaakImportError, inspect_yaak_source,
};

use crate::{
    components,
    execution::{
        ExecutionState, ResponseState, body_file_path_for_storage, execute_http_request,
        format_duration, format_size,
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
    response_viewer::{
        PreparedDocument, ResponseViewerState, ResponseViewerTab, prepare_document,
        pretty_json_body,
    },
    session::{SessionState, SessionStore},
    shell::{PaneLayout, ResizePane, ShellState},
    structure_editor::{
        DropIndicator, DropReject, ROOT_PARENT, StructureDialog, TreeDropIntent,
        descendant_requests, drop_intent, drop_zone, hovered_row_index, item_position,
        structure_operation_for_drop, validate_tree_drop, would_duplicate_path,
    },
    synchronization::{
        LocalRequestState, ReconcileResult, ReconciledWorkspace, SynchronizationConflict, reconcile,
    },
    theme::Theme,
    tree_search::{TreeSearchMatches, matching_tree_items},
};

const APPLICATION_ID: &str = "dev.probe.desktop";
const APPLICATION_NAME: &str = "Probe";
const IMPORT_DIAGNOSTIC_GROUP_LIMIT: usize = 8;

fn suggested_collection_filename(name: &str) -> String {
    let stem = name
        .trim()
        .chars()
        .map(|character| {
            if matches!(character, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                character
            }
        })
        .collect::<String>();
    let stem = stem.trim_matches([' ', '.', '-']);
    format!("{}.yml", if stem.is_empty() { "Imported" } else { stem })
}

fn format_import_diagnostics(diagnostics: &[ImportDiagnostic]) -> String {
    if diagnostics.is_empty() {
        return "No compatibility issues found.".to_owned();
    }

    let lossy_count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == ImportDiagnosticSeverity::Lossy)
        .count();
    let warning_count = diagnostics.len() - lossy_count;
    let mut groups = BTreeMap::new();
    for diagnostic in diagnostics {
        *groups
            .entry((
                diagnostic.severity,
                diagnostic.resource_type.as_str(),
                diagnostic.field.as_deref(),
                diagnostic.code,
                diagnostic.message.as_str(),
            ))
            .or_insert(0_usize) += 1;
    }

    let mut lines = vec![format!(
        "Found {} compatibility issue(s): {lossy_count} lossy, {warning_count} warning(s).",
        diagnostics.len()
    )];
    lines.push(String::new());
    for ((severity, resource_type, field, _, message), count) in
        groups.iter().take(IMPORT_DIAGNOSTIC_GROUP_LIMIT)
    {
        let resource = field
            .map(|field| format!("{resource_type}.{field}"))
            .unwrap_or_else(|| (*resource_type).to_owned());
        lines.push(format!(
            "• {count} {} — {resource}: {message}",
            severity.as_str()
        ));
    }
    if groups.len() > IMPORT_DIAGNOSTIC_GROUP_LIMIT {
        let hidden_group_count = groups.len() - IMPORT_DIAGNOSTIC_GROUP_LIMIT;
        let hidden_issue_count = groups
            .values()
            .skip(IMPORT_DIAGNOSTIC_GROUP_LIMIT)
            .sum::<usize>();
        lines.push(format!(
            "• {hidden_issue_count} more issue(s) across {hidden_group_count} additional type(s)"
        ));
    }
    if lossy_count > 0 {
        lines.push(String::new());
        lines.push(
            "Import Supported Data will omit or change the lossy fields listed above.".to_owned(),
        );
    }
    lines.join("\n")
}

#[cfg(test)]
use crate::filesystem::{WATCH_POLL, drain_watch_events};

gpui::actions!(
    probe,
    [
        OpenWorkspace,
        NewCollection,
        SaveRequest,
        CloseActiveTab,
        QuitApplication,
        FocusNextControl,
        FocusPreviousControl,
        NewRequest,
        NewFolder,
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
        CancelStructureDialog
    ]
);

#[derive(Clone, Debug)]
enum PendingClose {
    Tab(RequestKey),
    OtherTabs {
        keep: RequestKey,
    },
    Workspace,
    Window,
    Quit,
    Open {
        path: PathBuf,
        restored_state: Option<SessionState>,
    },
    Create {
        path: PathBuf,
    },
    ImportYaak,
}

const TREE_LIST_PADDING_Y: f32 = 2.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TreeRow {
    item: WorkspaceItemRef,
    depth: usize,
}

#[derive(Clone, Debug)]
struct TreeDrag {
    item: WorkspaceItemRef,
    kind: ItemKind,
    label: String,
    method: Option<String>,
}

struct TreeRowSpec {
    item: WorkspaceItemRef,
    kind: ItemKind,
    selector: String,
    label: String,
    method: Option<String>,
    depth: usize,
    selected: bool,
}

struct ShellSelectors {
    tab_selectors: Vec<String>,
    active_selector: Option<String>,
    folder_selectors: Vec<String>,
    selected: Option<(ItemKind, String)>,
}

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

impl Render for TreeDrag {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_window_appearance(window.appearance());
        let mut preview = div()
            .px(px(theme.metrics.spacing_2))
            .py(px(theme.metrics.spacing_1))
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_small))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .text_size(px(theme.typography.caption_size));
        if let Some(method) = &self.method {
            preview = preview.child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.method_color(method))
                    .child(method.clone()),
            );
        } else if self.kind == ItemKind::Folder {
            preview = preview.child(components::tree_folder_icon(theme, false, false));
        }
        preview.child(self.label.clone())
    }
}

pub(crate) struct ProbeApp {
    focus_handle: FocusHandle,
    structure_dialog_focus: FocusHandle,
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
    pending_environment_saves: BTreeSet<(String, String)>,
    structure_task: Option<Task<()>>,
    filesystem_watcher: Option<notify::RecommendedWatcher>,
    filesystem_watch_task: Option<Task<()>>,
    persistence: PersistenceState,
    pending_close: Option<PendingClose>,
    workspace_switcher_open: bool,
    structure_add_menu_open: bool,
    tree_context_menu: Option<WorkspaceItemRef>,
    tree_context_menu_position: Option<Point<Pixels>>,
    tab_context_menu: Option<RequestKey>,
    tab_context_menu_position: Option<Point<Pixels>>,
    visible_tree_rows: Vec<TreeRow>,
    tree_search: String,
    selected_tree_item: Option<WorkspaceItemRef>,
    tree_drag_source: Option<WorkspaceItemRef>,
    tree_drop_target: Option<TreeDropIntent>,
    tree_list_bounds: Option<Bounds<Pixels>>,
    tree_row_height: f32,
    tree_auto_scroll: AutoScroll,
    structure_dialog: Option<StructureDialog>,
    request_editor: RequestEditorState,
    execution: ExecutionState,
    response_viewer: ResponseViewerState,
    tree_scroll: UniformListScrollHandle,
    tab_bar_scroll: ScrollHandle,
    pending_tab_reveal: bool,
    #[cfg(test)]
    rendered_sidebar_rows: usize,
    #[cfg(test)]
    rendered_response_rows: usize,
    _caret_blink: Task<()>,
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
        let structure_dialog_focus = cx.focus_handle();

        Self {
            focus_handle,
            structure_dialog_focus,
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
            pending_environment_saves: BTreeSet::new(),
            structure_task: None,
            filesystem_watcher: None,
            filesystem_watch_task: None,
            persistence: PersistenceState::default(),
            pending_close: None,
            workspace_switcher_open: false,
            structure_add_menu_open: false,
            tree_context_menu: None,
            tree_context_menu_position: None,
            tab_context_menu: None,
            tab_context_menu_position: None,
            visible_tree_rows: Vec::new(),
            tree_search: String::new(),
            selected_tree_item: None,
            tree_drag_source: None,
            tree_drop_target: None,
            tree_list_bounds: None,
            tree_row_height: 28.0,
            tree_auto_scroll: AutoScroll::default(),
            structure_dialog: None,
            request_editor: RequestEditorState::default(),
            execution: ExecutionState::default(),
            response_viewer: ResponseViewerState::default(),
            tree_scroll: UniformListScrollHandle::new(),
            tab_bar_scroll: ScrollHandle::new(),
            pending_tab_reveal: false,
            #[cfg(test)]
            rendered_sidebar_rows: 0,
            #[cfg(test)]
            rendered_response_rows: 0,
            _caret_blink: Self::spawn_caret_blink(cx),
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

    fn request_import_yaak(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dirty = self.dirty_keys();
        if !dirty.is_empty() {
            self.prompt_unsaved(dirty, PendingClose::ImportYaak, window, cx);
            return;
        }
        if self.has_pending_environment_work() {
            self.pending_close = Some(PendingClose::ImportYaak);
            self.start_next_environment_save(window, cx);
            return;
        }
        self.choose_yaak_import(window, cx);
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
                let selected_id = if summaries.len() == 1 {
                    summaries[0].id.clone()
                } else {
                    let detail = summaries
                        .iter()
                        .map(|workspace| format!("{} — {}", workspace.name, workspace.id))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let mut buttons = summaries
                        .iter()
                        .map(|workspace| {
                            PromptButton::new(format!("{} — {}", workspace.name, workspace.id))
                        })
                        .collect::<Vec<_>>();
                    buttons.push(PromptButton::cancel("Cancel"));
                    let prompt = match view.update_in(cx, |_, window, cx| {
                        window.prompt(
                            PromptLevel::Info,
                            "Select a Yaak workspace",
                            Some(&detail),
                            &buttons,
                            cx,
                        )
                    }) {
                        Ok(prompt) => prompt,
                        Err(_) => return,
                    };
                    let Ok(answer) = prompt.await else {
                        return;
                    };
                    let Some(workspace) = summaries.get(answer) else {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            cx.notify();
                        });
                        return;
                    };
                    workspace.id.clone()
                };

                let imported = match preview.convert(Some(&selected_id), false) {
                    Ok(imported) => imported,
                    Err(YaakImportError::Unsupported(diagnostics)) => {
                        let detail = format_import_diagnostics(&diagnostics);
                        let prompt = match view.update_in(cx, |_, window, cx| {
                            window.prompt(
                                PromptLevel::Warning,
                                "Some Yaak data cannot be represented",
                                Some(&detail),
                                &[
                                    PromptButton::cancel("Cancel"),
                                    PromptButton::ok("Import Supported Data"),
                                ],
                                cx,
                            )
                        }) {
                            Ok(prompt) => prompt,
                            Err(_) => return,
                        };
                        let Ok(answer) = prompt.await else {
                            return;
                        };
                        if answer != 1 {
                            let _ = view.update_in(cx, |view, _, cx| {
                                view.loading = false;
                                cx.notify();
                            });
                            return;
                        }
                        match preview.convert(Some(&selected_id), true) {
                            Ok(imported) => imported,
                            Err(error) => {
                                let _ = view.update_in(cx, |view, _, cx| {
                                    view.loading = false;
                                    view.message =
                                        Some(format!("Could not convert Yaak data: {error}"));
                                    cx.notify();
                                });
                                return;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = view.update_in(cx, |view, _, cx| {
                            view.loading = false;
                            view.message = Some(format!("Could not convert Yaak data: {error}"));
                            cx.notify();
                        });
                        return;
                    }
                };

                let filename = suggested_collection_filename(&imported.workspace.name);
                let destination_receiver = match view.update_in(cx, |view, _, cx| {
                    cx.prompt_for_new_path(&view.new_collection_directory(), Some(&filename))
                }) {
                    Ok(receiver) => receiver,
                    Err(_) => return,
                };
                let destination = match destination_receiver.await {
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
                                view.restore_shell_state();
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
                | PendingClose::ImportYaak => self.dirty_keys(),
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
        let prompt = window.prompt(
            PromptLevel::Warning,
            "Collection changes conflict with local edits",
            Some(&format!(
                "{detail}. Choose Use Disk to discard the conflicting local edits, or Keep Local to retain them without overwriting disk."
            )),
            &["Use Disk", "Keep Local"],
            cx,
        );
        let path = self.workspace_path.clone();
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let Ok(answer) = prompt.await else {
                    return;
                };
                let _ = view.update_in(cx, |view, _, cx| {
                    if view.workspace_path != path {
                        return;
                    }
                    if answer == 0 {
                        let Some(path) = path else {
                            return;
                        };
                        view.loading = true;
                        let reload = cx.background_spawn(async move { load_workspace(path) });
                        cx.spawn(async move |view, cx| {
                            let result = reload.await;
                            let _ = view.update(cx, |view, cx| {
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
                                        if let ReconcileResult::Applied(reconciled) = reconcile(
                                            clean_local,
                                            workspace,
                                            &BTreeMap::new(),
                                        ) {
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
                    } else {
                        view.message = Some(
                            "Kept local edits. Probe will not overwrite the changed disk files; resolve the conflict before saving."
                                .to_owned(),
                        );
                        cx.notify();
                    }
                });
            })
            .detach();
    }

    fn reset_collection_ui(&mut self) {
        self.selected_tree_item = None;
        self.structure_dialog = None;
        self.structure_add_menu_open = false;
        self.tree_context_menu = None;
        self.tree_context_menu_position = None;
        self.tab_context_menu = None;
        self.tab_context_menu_position = None;
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
        self.execution.clear();
        self.response_viewer.clear();
        self.request_editor.remap_requests(key_remaps);
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
        self.install_reloaded_workspace(reconciled.workspace, baselines, &key_remaps);
        self.restore_shell_selectors(&reconciled.selector_remaps, selectors);
        self.structure_dialog = None;
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
        self.message = None;
        self.persist_session(cx);
        cx.notify();
    }

    fn restore_shell_state(&mut self) {
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

    fn select_request(&mut self, key: RequestKey, cx: &mut Context<Self>) {
        if self
            .loaded_workspace
            .as_ref()
            .is_some_and(|loaded| loaded.workspace().request(key).is_some())
        {
            self.selected_tree_item = Some(WorkspaceItemRef::Request(key));
            self.shell.open_request(key);
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
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Delete “{name}”?"),
            Some(&detail),
            &["Cancel", "Delete"],
            cx,
        );
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let Ok(answer) = prompt.await else {
                    return;
                };
                if answer != 1 {
                    return;
                }
                let _ = view.update_in(cx, |view, window, cx| {
                    let operation = match kind {
                        ItemKind::Request => StructureOperation::DeleteRequest { selector },
                        ItemKind::Folder => StructureOperation::DeleteFolder { selector },
                    };
                    view.apply_structure(operation, window, cx);
                });
            })
            .detach();
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
            let _ = view.update_in(window, |view, _, cx| {
                view.structure_task = None;
                view.loading = false;
                match result {
                    Ok((workspace, disk_workspace, result)) => {
                        view.apply_structure_result(
                            workspace,
                            disk_workspace,
                            result,
                            &operation,
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
        self.reveal_active_tab();
        self.message = None;
        self.persist_session(cx);
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

    fn quit_application(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        let noun = if keys.len() == 1 {
            "request"
        } else {
            "requests"
        };
        let prompt = window.prompt(
            PromptLevel::Warning,
            &format!("Save changes to {} {noun}?", keys.len()),
            Some("Unsaved changes will be lost if you discard them."),
            &["Save", "Discard", "Cancel"],
            cx,
        );
        let view = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let Ok(answer) = prompt.await else {
                    return;
                };
                let _ = view.update_in(cx, |view, window, cx| match answer {
                    0 => {
                        view.pending_close = Some(pending);
                        view.persistence.enqueue(keys);
                        view.start_next_request_save(window, cx);
                    }
                    1 => {
                        view.discard_dirty_requests(&keys);
                        view.finish_pending_close(pending, window, cx);
                    }
                    _ => {}
                });
            })
            .detach();
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
            PendingClose::ImportYaak => self.choose_yaak_import(window, cx),
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
        let (document, pending) = prepare_document(response, generation);
        let body = pending.then(|| response.body.clone());
        self.response_viewer.insert(key, document);
        let Some(body) = body else {
            return;
        };
        cx.spawn(async move |view, cx| {
            let pretty = cx
                .background_spawn(async move { pretty_json_body(&body) })
                .await;
            let _ = view.update(cx, |view, cx| {
                view.response_viewer.apply_pretty(key, generation, pretty);
                cx.notify();
            });
        })
        .detach();
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
        if self.shell.resizing.take().is_none() {
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

    fn render_tab_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(key) = self.tab_context_menu else {
            return div().into_any_element();
        };
        let Some(position) = self.tab_context_menu_position else {
            return div().into_any_element();
        };
        if !self.shell.tabs().contains(&key) {
            return div().into_any_element();
        }

        let close_view = cx.weak_entity();
        let close_other_view = cx.weak_entity();
        let dismiss_view = cx.weak_entity();
        let menu = div()
            .id("tab-context-menu")
            .w(px(220.0))
            .p(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out({
                let dismiss_view = dismiss_view.clone();
                move |_, _, cx| {
                    let _ = dismiss_view.update(cx, |view, cx| {
                        view.close_tab_context_menu(cx);
                    });
                }
            })
            .child(components::menu_button(
                theme,
                "tab-context-close",
                "Close Tab",
                shortcut_label_for_action(window, &CloseActiveTab),
                move |window, cx| {
                    let _ = close_view.update(cx, |view, cx| {
                        view.request_close_tab(key, window, cx);
                    });
                },
            ))
            .child(components::menu_button(
                theme,
                "tab-context-close-other",
                "Close Other Tabs",
                None,
                move |window, cx| {
                    let _ = close_other_view.update(cx, |view, cx| {
                        view.request_close_other_tabs(key, window, cx);
                    });
                },
            ));
        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(8.0))
                .child(menu),
        )
        .with_priority(POPUP_PRIORITY)
        .into_any_element()
    }

    fn render_tree_context_menu(
        &self,
        theme: Theme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(item) = self.tree_context_menu else {
            return div().into_any_element();
        };
        let Some(position) = self.tree_context_menu_position else {
            return div().into_any_element();
        };
        let rename_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-context-rename", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-context-rename", key.slot()),
        };
        let delete_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-context-delete", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-context-delete", key.slot()),
        };
        let rename_view = cx.weak_entity();
        let delete_view = cx.weak_entity();
        let dismiss_view = cx.weak_entity();
        let menu = div()
            .id("tree-context-menu")
            .w(px(200.0))
            .p(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .on_mouse_down_out({
                let dismiss_view = dismiss_view.clone();
                move |_, _, cx| {
                    let _ = dismiss_view.update(cx, |view, cx| {
                        view.close_tree_context_menu(cx);
                    });
                }
            })
            .child(components::menu_button(
                theme,
                rename_id,
                "Rename",
                shortcut_label_for_action_in_context(window, &RenameTreeItem, "RequestTree"),
                move |window, cx| {
                    let _ = rename_view.update(cx, |view, cx| {
                        view.tree_context_menu = None;
                        view.tree_context_menu_position = None;
                        view.select_tree_item(item, cx);
                        view.open_rename_dialog(window, cx);
                    });
                },
            ))
            .child(components::destructive_menu_button(
                theme,
                delete_id,
                "Delete",
                shortcut_label_for_action_in_context(window, &DeleteTreeItem, "RequestTree"),
                move |window, cx| {
                    let _ = delete_view.update(cx, |view, cx| {
                        view.tree_context_menu = None;
                        view.tree_context_menu_position = None;
                        view.select_tree_item(item, cx);
                        view.request_delete_selected(window, cx);
                    });
                },
            ));
        deferred(
            Positioner::corner(Anchor::TopLeft, position)
                .margin(px(8.0))
                .child(menu),
        )
        .with_priority(POPUP_PRIORITY)
        .into_any_element()
    }

    fn render_tree_row(
        &self,
        row: TreeRow,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(loaded) = &self.loaded_workspace else {
            return div().into_any_element();
        };
        let TreeRow { item, depth } = row;
        let can_edit = self.structure_task.is_none();
        match item {
            WorkspaceItemRef::Request(key) => {
                let Some(request) = loaded.workspace().request(key) else {
                    return div().into_any_element();
                };
                let label = request
                    .metadata
                    .name
                    .as_deref()
                    .unwrap_or("Untitled request");
                let method = request.method.as_deref().unwrap_or("HTTP").to_uppercase();
                let method_label = tree_method_label(&method).to_owned();
                let selected = self.selected_tree_item == Some(WorkspaceItemRef::Request(key));
                let view = cx.weak_entity();
                let context_menu_view = cx.weak_entity();
                let item = WorkspaceItemRef::Request(key);
                let button = Button::new(("request-tree-item", key.slot()))
                    .focusable(true)
                    .tab_stop(true)
                    .key_context("RequestTree")
                    .accessibility_label(format!("Request {label}"))
                    .w_full()
                    .h(px(theme.metrics.tree_row_height))
                    .pl(px(tree_level_indent(theme, depth)))
                    .pr(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_1))
                    .overflow_hidden()
                    .rounded(px(theme.metrics.radius_small))
                    .when(selected, |row| {
                        row.bg(theme.colors.selection.active_background)
                            .text_color(theme.colors.selection.active_foreground)
                    })
                    .when(!selected, |row| {
                        row.hover(move |row| row.bg(theme.colors.surfaces.window))
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |view, cx| view.select_request(key, cx));
                    })
                    .when(can_edit, |row| {
                        row.on_mouse_down(
                            MouseButton::Right,
                            move |event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                let _ = context_menu_view.update(cx, |view, cx| {
                                    view.open_tree_context_menu(item, event.position, cx);
                                });
                            },
                        )
                    })
                    .child(
                        div()
                            .w(px(26.0))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .truncate()
                            .font_family(theme.typography.monospace_family)
                            .text_size(px(tree_method_font_size(theme, &method_label)))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(if selected {
                                theme.colors.selection.active_foreground
                            } else {
                                theme.method_color(&method)
                            })
                            .child(method_label.clone()),
                    )
                    .child(
                        components::truncated_label(label.to_owned())
                            .flex_1()
                            .h_full()
                            .flex()
                            .items_center()
                            .when(selected, |label| {
                                label.debug_selector(|| "request-tree-label".into())
                            }),
                    );
                self.wrap_tree_row(
                    TreeRowSpec {
                        item,
                        kind: ItemKind::Request,
                        selector: loaded.request_selector(key).unwrap_or_default().to_owned(),
                        label: label.to_owned(),
                        method: Some(method_label),
                        depth,
                        selected,
                    },
                    can_edit,
                    button,
                    theme,
                    cx,
                )
            }
            WorkspaceItemRef::Folder(key) => {
                let Some(folder) = loaded.workspace().folder(key) else {
                    return div().into_any_element();
                };
                let expanded = self.shell.folder_is_expanded(key);
                let label = folder.metadata.name.as_deref().unwrap_or("Untitled folder");
                let selected = self.selected_tree_item == Some(WorkspaceItemRef::Folder(key));
                let view = cx.weak_entity();
                let context_menu_view = cx.weak_entity();
                let item = WorkspaceItemRef::Folder(key);
                let button = Button::new(("folder-tree-item", key.slot()))
                    .focusable(true)
                    .tab_stop(true)
                    .key_context("RequestTree")
                    .accessibility_label(format!("Folder {label}"))
                    .w_full()
                    .h(px(theme.metrics.tree_row_height))
                    .pl(px(tree_level_indent(theme, depth)))
                    .pr(px(theme.metrics.spacing_1))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_1))
                    .overflow_hidden()
                    .rounded(px(theme.metrics.radius_small))
                    .when(selected, |row| {
                        row.bg(theme.colors.selection.active_background)
                            .text_color(theme.colors.selection.active_foreground)
                    })
                    .when(!selected, |row| {
                        row.hover(move |row| row.bg(theme.colors.surfaces.window))
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |view, cx| {
                            view.select_tree_item(WorkspaceItemRef::Folder(key), cx);
                            view.shell.toggle_folder(key);
                            view.rebuild_visible_tree_rows();
                            view.persist_session(cx);
                            cx.notify();
                        });
                    })
                    .when(can_edit, |row| {
                        row.on_mouse_down(
                            MouseButton::Right,
                            move |event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                let _ = context_menu_view.update(cx, |view, cx| {
                                    view.open_tree_context_menu(item, event.position, cx);
                                });
                            },
                        )
                    })
                    .child(components::tree_folder_icon(theme, expanded, selected))
                    .child(
                        components::truncated_label(label.to_owned())
                            .flex_1()
                            .when(selected, |label| {
                                label.text_color(theme.colors.selection.active_foreground)
                            })
                            .font_weight(FontWeight::SEMIBOLD),
                    );
                self.wrap_tree_row(
                    TreeRowSpec {
                        item,
                        kind: ItemKind::Folder,
                        selector: loaded.folder_selector(key).unwrap_or_default().to_owned(),
                        label: label.to_owned(),
                        method: None,
                        depth,
                        selected,
                    },
                    can_edit,
                    button,
                    theme,
                    cx,
                )
            }
        }
    }

    fn wrap_tree_row(
        &self,
        spec: TreeRowSpec,
        can_edit: bool,
        button: Button,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let TreeRowSpec {
            item,
            kind,
            selector,
            label,
            method,
            depth,
            selected,
        } = spec;
        let indicator = self.tree_drop_target.map(|intent| intent.indicator);
        let show_before =
            matches!(indicator, Some(DropIndicator::Before(target)) if target == item);
        let show_after = matches!(indicator, Some(DropIndicator::After(target)) if target == item);
        let drop_into = matches!(
            indicator,
            Some(DropIndicator::IntoFolder(folder)) if item == WorkspaceItemRef::Folder(folder)
        );
        let indent = match kind {
            ItemKind::Folder | ItemKind::Request => tree_level_indent(theme, depth),
        };
        let drag_view = cx.weak_entity();
        let row_id = match item {
            WorkspaceItemRef::Request(key) => ("tree-drop-request", key.slot()),
            WorkspaceItemRef::Folder(key) => ("tree-drop-folder", key.slot()),
        };
        let button = if can_edit {
            button.on_drag(
                TreeDrag {
                    item,
                    kind,
                    label,
                    method,
                },
                move |drag, _, _, cx| {
                    let preview = drag.clone();
                    let item = drag.item;
                    let _ = drag_view.update(cx, |view, cx| {
                        view.tree_drag_source = Some(item);
                        view.select_tree_item(item, cx);
                    });
                    cx.new(|_| preview)
                },
            )
        } else {
            button
        };
        let line = |top: bool| {
            div()
                .absolute()
                .when(top, |line| line.top(px(0.0)))
                .when(!top, |line| line.bottom(px(0.0)))
                .left(px(indent))
                .right(px(theme.metrics.spacing_1))
                .h(px(2.0))
                .rounded(px(1.0))
                .bg(theme.colors.actions.accent)
        };
        div()
            .id(row_id)
            .relative()
            .w_full()
            .h(px(theme.metrics.tree_row_height))
            .debug_selector(move || format!("tree-row-{selector}"))
            .when(drop_into, |row| {
                row.rounded(px(theme.metrics.radius_small))
                    .bg(theme.colors.selection.inactive_background)
            })
            .child(button)
            .child(tree_hierarchy_guides(theme, depth, selected))
            .when(show_before, |row| row.child(line(true)))
            .when(show_after, |row| row.child(line(false)))
            .into_any_element()
    }

    fn render_tree_root_drop_row(&self, theme: Theme) -> gpui::AnyElement {
        let show_line = matches!(
            self.tree_drop_target.map(|intent| intent.indicator),
            Some(DropIndicator::RootEnd)
        );
        div()
            .id("tree-drop-root-end")
            .relative()
            .w_full()
            .h(px(theme.metrics.tree_row_height))
            .when(show_line, |row| {
                row.child(
                    div()
                        .absolute()
                        .top(px(0.0))
                        .left(px(tree_level_indent(theme, 0)))
                        .right(px(theme.metrics.spacing_1))
                        .h(px(2.0))
                        .rounded(px(1.0))
                        .bg(theme.colors.actions.accent),
                )
            })
            .into_any_element()
    }

    fn render_sidebar(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let new_request_view = cx.weak_entity();
        let new_folder_view = cx.weak_entity();
        let new_collection_view = cx.weak_entity();
        let open_collection_view = cx.weak_entity();
        let import_yaak_view = cx.weak_entity();
        let can_edit = self.loaded_workspace.is_some() && self.structure_task.is_none();
        let add_menu_state_view = cx.weak_entity();
        let add_popup = div()
            .w(px(180.0))
            .p(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1))
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .child(components::menu_button(
                theme,
                "tree-new-request",
                "Add Request",
                None,
                move |window, cx| {
                    let _ = new_request_view.update(cx, |view, cx| {
                        view.structure_add_menu_open = false;
                        view.open_create_request_dialog(window, cx);
                    });
                },
            ))
            .child(components::menu_button(
                theme,
                "tree-new-folder",
                "Add Folder",
                None,
                move |window, cx| {
                    let _ = new_folder_view.update(cx, |view, cx| {
                        view.structure_add_menu_open = false;
                        view.open_create_folder_dialog(window, cx);
                    });
                },
            ));
        let add_trigger =
            components::add_menu_button(theme, self.structure_add_menu_open, can_edit);
        let add_menu = if can_edit {
            Popover::new("tree-add-menu")
                .open(self.structure_add_menu_open)
                .on_open_change(move |open, _, cx| {
                    let _ = add_menu_state_view.update(cx, |view, cx| {
                        view.structure_add_menu_open = *open;
                        cx.notify();
                    });
                })
                .trigger(add_trigger)
                .content(move |_, _, _| add_popup)
                .into_any_element()
        } else {
            add_trigger.into_any_element()
        };
        let search_view = cx.weak_entity();
        let tree = if self.loaded_workspace.is_some() {
            let row_count = self.visible_tree_rows.len() + 1;
            let drag_view = cx.weak_entity();
            let drop_view = cx.weak_entity();
            uniform_list("request-tree", row_count, {
                cx.processor(move |view, range: std::ops::Range<usize>, _, cx| {
                    #[cfg(test)]
                    {
                        view.rendered_sidebar_rows = range.len();
                    }
                    range
                        .filter_map(|index| {
                            view.visible_tree_rows
                                .get(index)
                                .copied()
                                .map(|row| view.render_tree_row(row, theme, cx))
                                .or_else(|| {
                                    (index == view.visible_tree_rows.len())
                                        .then(|| view.render_tree_root_drop_row(theme))
                                })
                        })
                        .collect::<Vec<_>>()
                })
            })
            .flex_1()
            .min_h(px(0.0))
            .track_scroll(&self.tree_scroll)
            .px(px(theme.metrics.spacing_1))
            .py(px(TREE_LIST_PADDING_Y))
            .on_drag_move(move |event: &DragMoveEvent<TreeDrag>, window, cx| {
                let _ = drag_view.update(cx, |view, cx| {
                    view.on_tree_drag_move(event, window, cx);
                });
            })
            .on_drop(move |drag: &TreeDrag, window, cx| {
                let _ = drop_view.update(cx, |view, cx| {
                    view.drop_tree_item(drag, window, cx);
                });
            })
            .can_drop(|value, _, _| value.downcast_ref::<TreeDrag>().is_some())
            .into_any_element()
        } else {
            let mut tree = div()
                .id("request-tree")
                .flex_1()
                .overflow_y_scroll()
                .p(px(theme.metrics.spacing_2))
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(theme.metrics.spacing_2))
                        .pt(px(theme.metrics.spacing_1))
                        .pb(px(theme.metrics.spacing_2))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme.metrics.spacing_2))
                        .child(
                            div()
                                .text_color(theme.colors.text.muted)
                                .child("Create or open a collection to browse its requests."),
                        )
                        .child(components::secondary_button(
                            theme,
                            "sidebar-new-collection",
                            "New Collection…",
                            move |_, window, cx| {
                                let _ = new_collection_view.update(cx, |view, cx| {
                                    if !view.loading {
                                        view.choose_new_workspace(window, cx);
                                    }
                                });
                            },
                        ))
                        .child(components::secondary_button(
                            theme,
                            "sidebar-open-collection",
                            "Open Collection…",
                            move |_, window, cx| {
                                let _ = open_collection_view.update(cx, |view, cx| {
                                    if !view.loading {
                                        view.choose_workspace(window, cx);
                                    }
                                });
                            },
                        ))
                        .child(components::secondary_button(
                            theme,
                            "sidebar-import-yaak",
                            "Import from Yaak…",
                            move |_, window, cx| {
                                let _ = import_yaak_view.update(cx, |view, cx| {
                                    if !view.loading {
                                        view.request_import_yaak(window, cx);
                                    }
                                });
                            },
                        )),
                );
            if !self.session.recent_collections.is_empty() {
                tree = tree.child(
                    div()
                        .px(px(theme.metrics.spacing_2))
                        .pt(px(theme.metrics.spacing_2))
                        .pb(px(theme.metrics.spacing_1))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Recent Collections"),
                );
                for (index, path) in self.session.recent_collections.iter().enumerate() {
                    let open_path = path.clone();
                    let label = path
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Collection")
                        .to_owned();
                    let detail = path.display().to_string();
                    let view = cx.weak_entity();
                    let row = Button::new(("recent-collection", index))
                        .focusable(false)
                        .tab_stop(false)
                        .py(px(theme.metrics.spacing_2))
                        .px(px(theme.metrics.spacing_2))
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap(px(theme.metrics.spacing_1))
                        .overflow_hidden()
                        .rounded(px(theme.metrics.radius_small))
                        .cursor_pointer()
                        .hover(move |row| row.bg(theme.colors.surfaces.window))
                        .on_click(move |_, window, cx| {
                            let path = open_path.clone();
                            let _ = view.update(cx, |view, cx| {
                                if !view.loading {
                                    view.request_load_workspace(path, None, window, cx);
                                }
                            });
                        })
                        .child(components::truncated_label(label))
                        .child(
                            components::truncated_label(detail)
                                .text_size(px(theme.typography.caption_size))
                                .text_color(theme.colors.text.muted),
                        );
                    #[cfg(test)]
                    let row = row.debug_selector(move || format!("recent-collection-{index}"));
                    tree = tree.child(row);
                }
            }
            tree.into_any_element()
        };

        div()
            .w(px(self.shell.sidebar_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.sidebar)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(
                        div()
                            .h(px(theme.metrics.tab_bar_height))
                            .px(px(theme.metrics.spacing_2))
                            .flex()
                            .items_center()
                            .gap(px(theme.metrics.spacing_1))
                            .child(div().flex_1().min_w(px(0.0)).child(
                                components::sidebar_search_input(
                                    theme,
                                    self.tree_search.clone(),
                                    self.tree_search_placeholder(),
                                    move |value, _, input_cx| {
                                        let _ = search_view.update(input_cx, |view, cx| {
                                            view.set_tree_search(value.to_string(), cx);
                                        });
                                    },
                                ),
                            ))
                            .child(add_menu),
                    ),
            )
            .child(tree)
    }

    fn render_tabs(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Stateful<gpui::Div> {
        let tab_count = self.shell.tabs().len();
        let mut tab_strip = Tabs::new("request-tabs-scroll")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .px(px(theme.metrics.spacing_1))
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .overflow_x_scroll()
            .track_scroll(&self.tab_bar_scroll);
        let Some(loaded) = &self.loaded_workspace else {
            return div()
                .id("request-tabs")
                .h(px(theme.metrics.tab_bar_height))
                .w_full()
                .bg(theme.colors.surfaces.raised)
                .border_b_1()
                .border_color(theme.colors.borders.subtle);
        };
        for key in self.shell.tabs() {
            let Some(request) = loaded.workspace().request(*key) else {
                continue;
            };
            let active = self.shell.active_tab() == Some(*key);
            let dirty = self.persistence.is_dirty(*key, request);
            let label = request
                .metadata
                .name
                .as_deref()
                .unwrap_or("Untitled request");
            let select_view = cx.weak_entity();
            let close_view = cx.weak_entity();
            let context_menu_view = cx.weak_entity();
            let middle_close_view = close_view.clone();
            let tab_key = *key;
            let tab_index = self
                .shell
                .tabs()
                .iter()
                .position(|open| *open == *key)
                .unwrap_or(0);
            tab_strip = tab_strip.child(
                Tab::new(("request-tab", key.slot()))
                    .selected(active)
                    .set_position(tab_index + 1, tab_count)
                    .h(px(theme.metrics.control_height - 2.0))
                    .min_w(px(96.0))
                    .max_w(px(176.0))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_1))
                    .overflow_hidden()
                    .rounded(px(theme.metrics.radius_small))
                    .when(active, |tab| {
                        tab.bg(theme.colors.surfaces.editor)
                            .font_weight(FontWeight::SEMIBOLD)
                    })
                    .when(!active, |tab| {
                        tab.text_color(theme.colors.text.secondary)
                            .hover(move |tab| tab.bg(theme.colors.surfaces.window))
                    })
                    .cursor_pointer()
                    .on_click(move |_, _, cx| {
                        let _ = select_view.update(cx, |view, cx| view.select_request(tab_key, cx));
                    })
                    .on_mouse_down(MouseButton::Middle, |_, _, cx| cx.stop_propagation())
                    .on_mouse_down(MouseButton::Right, move |event: &MouseDownEvent, _, cx| {
                        cx.stop_propagation();
                        let _ = context_menu_view.update(cx, |view, cx| {
                            view.open_tab_context_menu(tab_key, event.position, cx);
                        });
                    })
                    .on_aux_click(move |event, window, cx| {
                        if event.is_middle_click() {
                            cx.stop_propagation();
                            let _ = middle_close_view
                                .update(cx, |view, cx| view.request_close_tab(tab_key, window, cx));
                        }
                    })
                    .child(
                        components::truncated_label(label.to_owned())
                            .flex_1()
                            .when(active, |label| {
                                label.debug_selector(|| "request-tab-label".into())
                            }),
                    )
                    .when(dirty, |tab| {
                        tab.child(
                            div()
                                .id(("request-dirty", key.slot()))
                                .flex_none()
                                .w(px(6.0))
                                .h(px(6.0))
                                .rounded(px(3.0))
                                .bg(theme.colors.actions.accent),
                        )
                    })
                    .child(
                        Button::new(("close-tab", key.slot()))
                            .focusable(false)
                            .tab_stop(false)
                            .flex_none()
                            .w(px(theme.metrics.icon_standard + 4.0))
                            .h(px(theme.metrics.icon_standard + 4.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(theme.metrics.radius_small))
                            .hover(move |close| close.bg(theme.colors.actions.disabled))
                            .child(components::close_icon(theme))
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                let _ = close_view.update(cx, |view, cx| {
                                    view.request_close_tab(tab_key, window, cx)
                                });
                            }),
                    ),
            );
        }

        let mut tabs = div()
            .id("request-tabs")
            .h(px(theme.metrics.tab_bar_height))
            .w_full()
            .flex()
            .items_center()
            .bg(theme.colors.surfaces.raised)
            .border_b_1()
            .border_color(theme.colors.borders.subtle)
            .child(tab_strip);
        let selected = self.shell.selected_environment().unwrap_or("").to_owned();
        let mut options = vec![(String::new(), "No environment".to_owned())];
        options.extend(
            loaded
                .workspace()
                .environments()
                .iter()
                .map(|environment| (environment.name.clone(), environment.name.clone())),
        );
        let environment_view = cx.weak_entity();
        tabs = tabs.child(div().flex_none().px(px(theme.metrics.spacing_2)).child(
            components::dropdown(
                theme,
                "request-environment",
                "Request environment",
                Some(selected),
                options,
                170.0,
                move |value, _, cx| {
                    let value = value.cloned().unwrap_or_default();
                    let _ = environment_view.update(cx, |view, cx| {
                        view.select_environment((!value.is_empty()).then_some(value), cx);
                    });
                },
            ),
        ));
        tabs
    }

    fn edit_request(
        &mut self,
        key: RequestKey,
        edit: impl FnOnce(&mut HttpRequest),
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        edit(request);
        self.persistence.edited(key);
        cx.notify();
    }

    fn change_body_kind(&mut self, key: RequestKey, kind: BodyEditorKind, cx: &mut Context<Self>) {
        let Some(request) = self
            .loaded_workspace
            .as_mut()
            .and_then(|loaded| loaded.request_mut(key))
        else {
            return;
        };
        self.request_editor.switch_body_kind(key, request, kind);
        self.persistence.edited(key);
        cx.notify();
    }

    fn render_request_editor(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let Some(key) = self.shell.active_tab() else {
            return div()
                .flex_1()
                .min_w(px(0.0))
                .min_h(px(0.0))
                .flex()
                .items_center()
                .justify_center()
                .bg(theme.colors.surfaces.editor)
                .text_color(theme.colors.text.muted)
                .child("Select a request from the collection sidebar.");
        };
        let Some(request) = self.active_request().cloned() else {
            return div().flex_1();
        };
        let method = request.method.as_deref().unwrap_or("GET").to_uppercase();
        let url = url_bar_value(&request);
        let request_dirty = self.persistence.is_dirty(key, &request);
        let mut breadcrumb_labels = self
            .loaded_workspace
            .as_ref()
            .and_then(|loaded| {
                loaded
                    .workspace()
                    .request_ancestor_folders(key)
                    .map(|folders| {
                        folders
                            .iter()
                            .filter_map(|folder_key| loaded.workspace().folder(*folder_key))
                            .map(|folder| {
                                folder
                                    .metadata
                                    .name
                                    .as_deref()
                                    .unwrap_or("Untitled folder")
                                    .to_owned()
                            })
                            .collect::<Vec<_>>()
                    })
            })
            .unwrap_or_default();
        let request_breadcrumb_index = breadcrumb_labels.len();
        breadcrumb_labels.push(
            request
                .metadata
                .name
                .as_deref()
                .unwrap_or("Untitled request")
                .to_owned(),
        );
        let save_view = cx.weak_entity();
        let mut breadcrumb_path = div()
            .id("request-breadcrumb-path")
            .flex_1()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .overflow_x_scroll()
            .text_size(px(theme.typography.caption_size))
            .text_color(theme.colors.text.muted);
        for (index, label) in breadcrumb_labels.into_iter().enumerate() {
            if index > 0 {
                breadcrumb_path = breadcrumb_path.child(div().flex_none().child("›"));
            }
            let segment = components::truncated_label(label)
                .max_w(px(220.0))
                .flex_none();
            let segment = if index == request_breadcrumb_index {
                segment
                    .debug_selector(|| "request-breadcrumb-request".into())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.text.primary)
            } else {
                segment.debug_selector(move || format!("request-breadcrumb-folder-{index}"))
            };
            breadcrumb_path = breadcrumb_path.child(segment);
        }
        let breadcrumb = div()
            .id("request-breadcrumb")
            .debug_selector(|| "request-breadcrumb".into())
            .h(px(theme.metrics.control_height))
            .w_full()
            .flex()
            .items_center()
            .child(breadcrumb_path)
            .child(
                Button::new("request-save")
                    .accessibility_label("Save request")
                    .debug_selector(|| "request-save".into())
                    .disabled(!request_dirty)
                    .ml(px(theme.metrics.spacing_2))
                    .flex_none()
                    .w(px(theme.metrics.control_height))
                    .h(px(theme.metrics.control_height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(theme.metrics.radius_small))
                    .border_1()
                    .border_color(theme.colors.borders.standard)
                    .bg(theme.colors.surfaces.raised)
                    .hover(move |button| button.bg(theme.colors.selection.inactive_background))
                    .focus(move |button| button.border_color(theme.colors.borders.focused))
                    .styles(move |styles| {
                        styles.disabled(move |button| {
                            button
                                .bg(theme.colors.actions.disabled)
                                .border_color(theme.colors.actions.disabled)
                                .text_color(theme.colors.actions.disabled_foreground)
                        })
                    })
                    .child(components::save_icon(theme).when(!request_dirty, |icon| {
                        icon.text_color(theme.colors.actions.disabled_foreground)
                    }))
                    .on_click(move |_, window, cx| {
                        let _ = save_view.update(cx, |view, cx| {
                            view.save_active_request(window, cx);
                        });
                    }),
            );
        let url_view = cx.weak_entity();
        let execution_view = cx.weak_entity();
        let request_running = self
            .execution
            .response(key)
            .is_some_and(ResponseState::is_running);
        let mut section_tabs = Tabs::new("request-editor-sections")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        for (index, section) in EditorSection::ALL.into_iter().enumerate() {
            let section_view = cx.weak_entity();
            section_tabs = section_tabs.child(components::text_tab(
                theme,
                ("request-editor-section", index),
                format!(
                    "{}{}",
                    section.label(),
                    match section {
                        EditorSection::Query => format!("  {}", request.query_parameters.len()),
                        EditorSection::Path => format!("  {}", request.path_parameters.len()),
                        EditorSection::Headers => format!("  {}", request.headers.len()),
                        EditorSection::Body | EditorSection::Authentication => String::new(),
                    }
                ),
                self.request_editor.section == section,
                index + 1,
                EditorSection::ALL.len(),
                move |_, _, cx| {
                    let _ = section_view.update(cx, |view, cx| {
                        view.request_editor.section = section;
                        cx.notify();
                    });
                },
            ));
        }

        let section = match self.request_editor.section {
            EditorSection::Query => self.render_query_editor(key, &request, theme, cx),
            EditorSection::Path => self.render_parameter_editor(key, &request, true, theme, cx),
            EditorSection::Headers => self.render_header_editor(key, &request, theme, cx),
            EditorSection::Body => self.render_body_editor(key, &request, theme, cx),
            EditorSection::Authentication => {
                self.render_authentication_editor(key, &request, theme, cx)
            }
        };

        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(120.0))
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.editor)
            .child(
                div()
                    .p(px(theme.metrics.spacing_2))
                    .pb(px(theme.metrics.spacing_2))
                    .flex()
                    .flex_col()
                    .gap(px(theme.metrics.spacing_2))
                    .child(breadcrumb)
                    .child(
                        div()
                            .id("request-url-bar")
                            .debug_selector(|| "request-url-bar".into())
                            .h(px(theme.metrics.control_height))
                            .w_full()
                            .flex()
                            .items_center()
                            .child(div().w(px(108.0)).mr(px(theme.metrics.spacing_1)).child(
                                components::dropdown_with_option_colors(
                                    theme,
                                    "request-method",
                                    "HTTP method",
                                    Some(method.clone()),
                                    request_method_options(theme, &method),
                                    108.0,
                                    {
                                        let method_view = cx.weak_entity();
                                        move |value, _, cx| {
                                            let Some(value) = value.cloned() else {
                                                return;
                                            };
                                            let _ = method_view.update(cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| request.method = Some(value),
                                                    cx,
                                                );
                                            });
                                        }
                                    },
                                ),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .child(components::url_text_input(
                                        theme,
                                        ("request-url", key.slot()),
                                        url.clone(),
                                        "https://api.example.com/users/:userId",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = url_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| apply_url_bar_value(request, &value),
                                                    cx,
                                                );
                                            });
                                        },
                                    )),
                            )
                            .child(div().ml(px(theme.metrics.spacing_1)).flex_none().child(
                                components::primary_button(
                                    theme,
                                    "request-execution",
                                    if request_running { "Cancel" } else { "Send" },
                                    move |_, _, cx| {
                                        let _ = execution_view.update(cx, |view, cx| {
                                            if view
                                                .execution
                                                .response(key)
                                                .is_some_and(ResponseState::is_running)
                                            {
                                                view.cancel_request(key, cx);
                                            } else {
                                                view.send_request(key, cx);
                                            }
                                        });
                                    },
                                ),
                            )),
                    )
                    .child(section_tabs),
            )
            .child(
                div()
                    .id("request-editor-section-content")
                    .flex_1()
                    .min_h(px(0.0))
                    .px(px(theme.metrics.spacing_2))
                    .pb(px(theme.metrics.spacing_2))
                    .when(
                        self.request_editor.section != EditorSection::Body,
                        |content| content.overflow_y_scroll(),
                    )
                    .child(section),
            )
    }

    fn render_query_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.render_parameter_editor(key, request, false, theme, cx)
    }

    fn render_parameter_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        path: bool,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        let parameters = if path {
            &request.path_parameters
        } else {
            &request.query_parameters
        };
        for (index, parameter) in parameters.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                (if path { "path-name" } else { "query-name" }, index),
                                parameter.name.clone(),
                                "Parameter",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if path {
                                                    rename_path_parameter_at(
                                                        request, index, &value,
                                                    );
                                                } else if let Some(parameter) =
                                                    request.query_parameters.get_mut(index)
                                                {
                                                    parameter.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                (if path { "path-value" } else { "query-value" }, index),
                                parameter.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(parameter) = if path {
                                                    request.path_parameters.get_mut(index)
                                                } else {
                                                    request.query_parameters.get_mut(index)
                                                } {
                                                    parameter.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            (
                                if path {
                                    "path-enabled"
                                } else {
                                    "query-enabled"
                                },
                                index,
                            ),
                            if path {
                                "Enable path parameter"
                            } else {
                                "Enable query parameter"
                            },
                            !parameter.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(parameter) = if path {
                                                request.path_parameters.get_mut(index)
                                            } else {
                                                request.query_parameters.get_mut(index)
                                            } {
                                                parameter.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            (if path { "remove-path" } else { "remove-query" }, index),
                            if path {
                                "Remove path parameter"
                            } else {
                                "Remove query parameter"
                            },
                            move |_, _, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if path {
                                                remove_path_parameter_at(request, index);
                                            } else if index < request.query_parameters.len() {
                                                request.query_parameters.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            if path {
                "add-path-parameter"
            } else {
                "add-query-parameter"
            },
            if path {
                "Add path parameter"
            } else {
                "Add query parameter"
            },
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if path {
                                add_path_parameter(request);
                            } else {
                                request.query_parameters.push(QueryParameter {
                                    name: String::new(),
                                    value: String::new(),
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_header_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, header) in request.headers.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("header-name", index),
                                header.name.clone(),
                                "Header",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(header) = request.headers.get_mut(index)
                                                {
                                                    header.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("header-value", index),
                                header.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(header) = request.headers.get_mut(index)
                                                {
                                                    header.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("header-enabled", index),
                            "Enable header",
                            !header.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(header) = request.headers.get_mut(index) {
                                                header.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-header", index),
                            "Remove header",
                            move |_, _, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if index < request.headers.len() {
                                                request.headers.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-header",
            "Add header",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            request.headers.push(Header {
                                name: String::new(),
                                value: String::new(),
                                disabled: false,
                            })
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_body_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active_kind = body_kind(request);
        let choices = [
            ("None", BodyEditorKind::None),
            ("JSON", BodyEditorKind::Json),
            ("Text", BodyEditorKind::Text),
            ("XML", BodyEditorKind::Xml),
            ("SPARQL", BodyEditorKind::Sparql),
            ("Form", BodyEditorKind::Form),
            ("Multipart", BodyEditorKind::Multipart),
            ("File", BodyEditorKind::File),
        ];
        let mut kind_buttons = div().flex().flex_wrap().gap(px(theme.metrics.spacing_1));
        for (index, (label, kind)) in choices.into_iter().enumerate() {
            let kind_view = cx.weak_entity();
            kind_buttons = kind_buttons.child(components::editor_button(
                theme,
                ("body-kind", index),
                label,
                active_kind == label,
                move |_, _, cx| {
                    let _ = kind_view.update(cx, |view, cx| {
                        view.change_body_kind(key, kind, cx);
                    });
                },
            ));
        }

        let mut editor = div()
            .size_full()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_2))
            .child(kind_buttons);
        match request.body.as_ref() {
            Some(RequestBody::Single(Body::Raw(raw))) => {
                let body_view = cx.weak_entity();
                editor = editor.child(
                    div()
                        .id("request-body-editor")
                        .debug_selector(|| "request-body-editor".into())
                        .flex_1()
                        .min_h(px(0.0))
                        .child(components::body_text_input(
                            theme,
                            ("request-body", key.slot()),
                            raw.data.clone(),
                            match raw.kind {
                                RawBodyKind::Json => components::BodySyntax::Json,
                                RawBodyKind::Xml => components::BodySyntax::Xml,
                                _ => components::BodySyntax::Plain,
                            },
                            self.variable_context(cx),
                            move |value, _, input_cx| {
                                let _ = body_view.update(input_cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(data) = raw_body_mut(request) {
                                                *data = value.to_string();
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
            }
            Some(RequestBody::Single(Body::FormUrlEncoded(fields))) => {
                editor = editor.child(self.render_form_body_editor(key, fields, theme, cx));
            }
            Some(RequestBody::Single(Body::Multipart(parts))) => {
                editor = editor.child(self.render_multipart_body_editor(key, parts, theme, cx));
            }
            Some(RequestBody::Single(Body::File(files))) => {
                editor = editor.child(self.render_file_body_editor(key, files, theme, cx));
            }
            Some(_) => {
                editor = editor.child(
                    div()
                        .p(px(theme.metrics.spacing_3))
                        .rounded(px(theme.metrics.radius_small))
                        .bg(theme.colors.surfaces.window)
                        .text_color(theme.colors.text.secondary)
                        .child(format!(
                            "This request uses a {active_kind} body. Choose a raw body type to replace it."
                        )),
                );
            }
            None => {
                editor = editor.child(
                    div()
                        .text_color(theme.colors.text.muted)
                        .child("This request has no body."),
                );
            }
        }
        editor.into_any_element()
    }

    fn render_form_body_editor(
        &self,
        key: RequestKey,
        fields: &[FormField],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, field) in fields.iter().enumerate() {
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("form-field-name", index),
                                field.name.clone(),
                                "Field",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(
                                                    Body::FormUrlEncoded(fields),
                                                )) = request.body.as_mut()
                                                    && let Some(field) = fields.get_mut(index)
                                                {
                                                    field.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("form-field-value", index),
                                field.value.clone(),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(
                                                    Body::FormUrlEncoded(fields),
                                                )) = request.body.as_mut()
                                                    && let Some(field) = fields.get_mut(index)
                                                {
                                                    field.value = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("form-field-enabled", index),
                            "Enable form field",
                            !field.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::FormUrlEncoded(
                                                fields,
                                            ))) = request.body.as_mut()
                                                && let Some(field) = fields.get_mut(index)
                                            {
                                                field.disabled = !enabled;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-form-field", index),
                            "Remove form field",
                            move |_, _, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::FormUrlEncoded(
                                                fields,
                                            ))) = request.body.as_mut()
                                                && index < fields.len()
                                            {
                                                fields.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-form-field",
            "Add field",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::FormUrlEncoded(fields))) =
                                request.body.as_mut()
                            {
                                fields.push(FormField {
                                    name: String::new(),
                                    value: String::new(),
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_multipart_body_editor(
        &self,
        key: RequestKey,
        parts: &[MultipartPart],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, part) in parts.iter().enumerate() {
            let value = match &part.value {
                MultipartValue::Single(value) => value.clone(),
                MultipartValue::Multiple(values) => values.join(", "),
            };
            let name_view = cx.weak_entity();
            let value_view = cx.weak_entity();
            let kind_view = cx.weak_entity();
            let enabled_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            let browse_view = cx.weak_entity();
            let is_file = part.kind == MultipartPartKind::File;
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(components::editor_button(
                            theme,
                            ("multipart-kind", index),
                            if is_file { "File" } else { "Text" },
                            is_file,
                            move |_, _, cx| {
                                let _ = kind_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && let Some(part) = parts.get_mut(index)
                                        {
                                            part.kind = if part.kind == MultipartPartKind::Text {
                                                MultipartPartKind::File
                                            } else {
                                                MultipartPartKind::Text
                                            };
                                        }
                                    },
                                    cx,
                                );
                                });
                            },
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("multipart-name", index),
                                part.name.clone(),
                                "Part",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                if let Some(RequestBody::Single(Body::Multipart(
                                                    parts,
                                                ))) = request.body.as_mut()
                                                    && let Some(part) = parts.get_mut(index)
                                                {
                                                    part.name = value.to_string();
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(if is_file {
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap(px(theme.metrics.spacing_1))
                                .child(div().flex_1().min_w(px(0.0)).child(
                                    components::variable_text_input(
                                        theme,
                                        ("multipart-value", index),
                                        value,
                                        "File path",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = value_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| {
                                                        if let Some(RequestBody::Single(
                                                            Body::Multipart(parts),
                                                        )) = request.body.as_mut()
                                                            && let Some(part) = parts.get_mut(index)
                                                        {
                                                            part.value = MultipartValue::Single(
                                                                value.to_string(),
                                                            );
                                                        }
                                                    },
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                ))
                                .child(components::browse_file_button(
                                    theme,
                                    ("multipart-file-browse", index),
                                    "Browse for file",
                                    move |_, window, cx| {
                                        let _ = browse_view.update(cx, |view, cx| {
                                            view.choose_multipart_file(key, index, window, cx);
                                        });
                                    },
                                ))
                                .into_any_element()
                        } else {
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .child(components::variable_text_input(
                                    theme,
                                    ("multipart-value", index),
                                    value,
                                    "Value",
                                    self.variable_context(cx),
                                    move |value, _, input_cx| {
                                        let _ = value_view.update(input_cx, |view, cx| {
                                            view.edit_request(
                                                key,
                                                |request| {
                                                    if let Some(RequestBody::Single(
                                                        Body::Multipart(parts),
                                                    )) = request.body.as_mut()
                                                        && let Some(part) = parts.get_mut(index)
                                                    {
                                                        part.value = MultipartValue::Single(
                                                            value.to_string(),
                                                        );
                                                    }
                                                },
                                                cx,
                                            );
                                        });
                                    },
                                ))
                                .into_any_element()
                        })
                        .child(components::switch(
                            theme,
                            ("multipart-enabled", index),
                            "Enable multipart part",
                            !part.disabled,
                            move |enabled, _, cx| {
                                let _ = enabled_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && let Some(part) = parts.get_mut(index)
                                        {
                                            part.disabled = !enabled;
                                        }
                                    },
                                    cx,
                                );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-multipart-part", index),
                            "Remove multipart part",
                            move |_, _, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                            request.body.as_mut()
                                            && index < parts.len()
                                        {
                                            parts.remove(index);
                                        }
                                    },
                                    cx,
                                );
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-multipart-part",
            "Add part",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::Multipart(parts))) =
                                request.body.as_mut()
                            {
                                parts.push(MultipartPart {
                                    name: String::new(),
                                    kind: MultipartPartKind::Text,
                                    value: MultipartValue::Single(String::new()),
                                    content_type: None,
                                    disabled: false,
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_file_body_editor(
        &self,
        key: RequestKey,
        files: &[FileReference],
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let mut rows = div().flex().flex_col().gap(px(theme.metrics.spacing_2));
        for (index, file) in files.iter().enumerate() {
            let path_view = cx.weak_entity();
            let type_view = cx.weak_entity();
            let selected_view = cx.weak_entity();
            let remove_view = cx.weak_entity();
            let browse_view = cx.weak_entity();
            rows =
                rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .flex()
                                .items_center()
                                .gap(px(theme.metrics.spacing_1))
                                .child(div().flex_1().min_w(px(0.0)).child(
                                    components::variable_text_input(
                                        theme,
                                        ("body-file-path", index),
                                        file.file_path.clone(),
                                        "File path",
                                        self.variable_context(cx),
                                        move |value, _, input_cx| {
                                            let _ = path_view.update(input_cx, |view, cx| {
                                                view.edit_request(
                                                    key,
                                                    |request| {
                                                        if let Some(RequestBody::Single(
                                                            Body::File(files),
                                                        )) = request.body.as_mut()
                                                            && let Some(file) = files.get_mut(index)
                                                        {
                                                            file.file_path = value.to_string();
                                                        }
                                                    },
                                                    cx,
                                                );
                                            });
                                        },
                                    ),
                                ))
                                .child(components::browse_file_button(
                                    theme,
                                    ("body-file-browse", index),
                                    "Browse for file",
                                    move |_, window, cx| {
                                        let _ = browse_view.update(cx, |view, cx| {
                                            view.choose_body_file(key, index, window, cx);
                                        });
                                    },
                                )),
                        )
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("body-file-content-type", index),
                                file.content_type.clone(),
                                "Content type",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let _ = type_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                    key,
                                    |request| {
                                        if let Some(RequestBody::Single(Body::File(files))) =
                                            request.body.as_mut()
                                            && let Some(file) = files.get_mut(index)
                                        {
                                            file.content_type = value.to_string();
                                        }
                                    },
                                    cx,
                                );
                                    });
                                },
                            ),
                        ))
                        .child(components::switch(
                            theme,
                            ("body-file-selected", index),
                            "Select body file",
                            file.selected,
                            move |selected, _, cx| {
                                let _ = selected_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::File(files))) =
                                                request.body.as_mut()
                                                && let Some(file) = files.get_mut(index)
                                            {
                                                file.selected = selected;
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-body-file", index),
                            "Remove file",
                            move |_, _, cx| {
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(RequestBody::Single(Body::File(files))) =
                                                request.body.as_mut()
                                                && index < files.len()
                                            {
                                                files.remove(index);
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
        }
        let add_view = cx.weak_entity();
        rows.child(components::editor_add_button(
            theme,
            "add-body-file",
            "Add file",
            move |_, _, cx| {
                let _ = add_view.update(cx, |view, cx| {
                    view.edit_request(
                        key,
                        |request| {
                            if let Some(RequestBody::Single(Body::File(files))) =
                                request.body.as_mut()
                            {
                                files.push(FileReference {
                                    file_path: String::new(),
                                    content_type: "application/octet-stream".to_owned(),
                                    selected: files.is_empty(),
                                });
                            }
                        },
                        cx,
                    );
                });
            },
        ))
        .into_any_element()
    }

    fn render_authentication_editor(
        &self,
        key: RequestKey,
        request: &HttpRequest,
        theme: Theme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let active = request
            .authentication
            .as_ref()
            .map(|auth| auth_label(&auth.kind));
        let choices = [
            ("None", None),
            ("Inherit", Some(AuthenticationKind::Inherit)),
            ("Basic", Some(AuthenticationKind::Basic)),
            ("Bearer", Some(AuthenticationKind::Bearer)),
            ("API Key", Some(AuthenticationKind::ApiKey)),
            ("OAuth 1", Some(AuthenticationKind::OAuth1)),
            ("OAuth 2", Some(AuthenticationKind::OAuth2)),
            ("AWS v4", Some(AuthenticationKind::AwsV4)),
            ("WSSE", Some(AuthenticationKind::Wsse)),
            ("Digest", Some(AuthenticationKind::Digest)),
            ("NTLM", Some(AuthenticationKind::Ntlm)),
        ];
        let mut kind_buttons = div().flex().flex_wrap().gap(px(theme.metrics.spacing_1));
        for (index, (label, kind)) in choices.into_iter().enumerate() {
            let kind_view = cx.weak_entity();
            kind_buttons = kind_buttons.child(components::editor_button(
                theme,
                ("authentication-kind", index),
                label,
                active == Some(label) || (active.is_none() && label == "None"),
                move |_, _, cx| {
                    let kind = kind.clone();
                    let _ = kind_view.update(cx, |view, cx| {
                        view.edit_request(key, |request| set_authentication(request, kind), cx);
                    });
                },
            ));
        }

        let mut editor = div()
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_2))
            .child(kind_buttons);
        if let Some(authentication) = &request.authentication {
            for (index, (property_name, value)) in authentication.properties.iter().enumerate() {
                let old_name = property_name.clone();
                let name_view = cx.weak_entity();
                let value_name = property_name.clone();
                let value_view = cx.weak_entity();
                let remove_name = property_name.clone();
                let remove_view = cx.weak_entity();
                editor = editor.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(theme.metrics.spacing_2))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("authentication-property-name", index),
                                property_name.clone(),
                                "Property",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let old_name = old_name.clone();
                                    let _ = name_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                let Some(authentication) =
                                                    request.authentication.as_mut()
                                                else {
                                                    return;
                                                };
                                                if let Some(old_value) =
                                                    authentication.properties.remove(&old_name)
                                                {
                                                    authentication
                                                        .properties
                                                        .insert(value.to_string(), old_value);
                                                }
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(div().flex_1().min_w(px(0.0)).child(
                            components::variable_text_input(
                                theme,
                                ("authentication-property-value", index),
                                auth_value(value),
                                "Value",
                                self.variable_context(cx),
                                move |value, _, input_cx| {
                                    let value_name = value_name.clone();
                                    let _ = value_view.update(input_cx, |view, cx| {
                                        view.edit_request(
                                            key,
                                            |request| {
                                                set_auth_property(
                                                    request,
                                                    value_name,
                                                    value.to_string(),
                                                )
                                            },
                                            cx,
                                        );
                                    });
                                },
                            ),
                        ))
                        .child(components::remove_row_button(
                            theme,
                            ("remove-authentication-property", index),
                            "Remove authentication property",
                            move |_, _, cx| {
                                let remove_name = remove_name.clone();
                                let _ = remove_view.update(cx, |view, cx| {
                                    view.edit_request(
                                        key,
                                        |request| {
                                            if let Some(authentication) =
                                                request.authentication.as_mut()
                                            {
                                                authentication.properties.remove(&remove_name);
                                            }
                                        },
                                        cx,
                                    );
                                });
                            },
                        )),
                );
            }
            let add_view = cx.weak_entity();
            editor = editor.child(components::editor_add_button(
                theme,
                "add-authentication-property",
                "Add property",
                move |_, _, cx| {
                    let _ = add_view.update(cx, |view, cx| {
                        view.edit_request(
                            key,
                            |request| {
                                let Some(authentication) = request.authentication.as_mut() else {
                                    return;
                                };
                                let mut index = authentication.properties.len() + 1;
                                let mut name = "property".to_owned();
                                while authentication.properties.contains_key(&name) {
                                    name = format!("property{index}");
                                    index += 1;
                                }
                                authentication
                                    .properties
                                    .insert(name, AuthenticationValue::String(String::new()));
                            },
                            cx,
                        );
                    });
                },
            ));
        } else {
            editor = editor.child(
                div()
                    .text_color(theme.colors.text.muted)
                    .child("This request does not use authentication."),
            );
        }
        editor.into_any_element()
    }

    fn render_response_panel(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let active_key = self.shell.active_tab();
        let state = active_key.and_then(|key| self.execution.response(key));
        let (summary, content) = match state {
            Some(ResponseState::Running) => (
                "Sending…".to_owned(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Waiting for the server…")
                    .into_any_element(),
            ),
            Some(ResponseState::Cancelled) => (
                "Cancelled".to_owned(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Request cancelled.")
                    .into_any_element(),
            ),
            Some(ResponseState::Failed(error)) => (
                "Failed".to_owned(),
                div()
                    .id("response-error-scroll")
                    .flex_1()
                    .p(px(theme.metrics.spacing_3))
                    .overflow_y_scroll()
                    .text_color(theme.colors.status.error)
                    .child(error.clone())
                    .into_any_element(),
            ),
            Some(ResponseState::Complete(response)) => {
                let status = format!("{} {}", response.status, response.reason);
                let summary = format!(
                    "{}  •  {}  •  {}",
                    status.trim_end(),
                    format_duration(response.duration),
                    format_size(response.size)
                );
                let document = active_key.and_then(|key| self.response_viewer.document(key));
                (summary, self.render_response_document(theme, document, cx))
            }
            None => (
                String::new(),
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.colors.text.muted)
                    .child("Send a request to see its response.")
                    .into_any_element(),
            ),
        };

        div()
            .when(self.shell.pane_layout == PaneLayout::Vertical, |panel| {
                panel.h(px(self.shell.response_height)).w_full()
            })
            .when(self.shell.pane_layout == PaneLayout::Horizontal, |panel| {
                panel.w(px(self.shell.response_width)).h_full()
            })
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.colors.surfaces.window)
            .child(
                div()
                    .h(px(theme.metrics.tab_bar_height))
                    .px(px(theme.metrics.spacing_2))
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(div().font_weight(FontWeight::SEMIBOLD).child("Response"))
                    .child(
                        components::truncated_label(summary)
                            .id("response-status")
                            .debug_selector(|| "response-status".into())
                            .flex_1()
                            .text_size(px(theme.typography.caption_size))
                            .text_color(theme.colors.text.muted),
                    ),
            )
            .child(content)
    }

    fn render_response_document(
        &self,
        theme: Theme,
        document: Option<&PreparedDocument>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(key) = self.shell.active_tab() else {
            return div().into_any_element();
        };
        let Some(document) = document else {
            return div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme.colors.text.muted)
                .child("Preparing response…")
                .into_any_element();
        };

        let mut tabs = Tabs::new("response-view-tabs")
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1));
        for (index, tab) in ResponseViewerTab::ALL.into_iter().enumerate() {
            let tab_view = cx.weak_entity();
            let selected = self.response_viewer.tab() == tab;
            tabs = tabs.child(
                components::text_tab(
                    theme,
                    ("response-view-tab", index),
                    tab.label(),
                    selected,
                    index + 1,
                    ResponseViewerTab::ALL.len(),
                    move |_, _, cx| {
                        let _ = tab_view.update(cx, |view, cx| {
                            view.response_viewer.set_tab(tab);
                            cx.notify();
                        });
                    },
                )
                .debug_selector(move || {
                    format!("response-tab-{}", tab.label().to_ascii_lowercase())
                }),
            );
        }

        let matches = self.response_viewer.matches(key);
        let match_count = matches.len();
        let search_label = if self.response_viewer.search().is_empty() {
            String::new()
        } else if match_count == 0 {
            "No matches".to_owned()
        } else {
            format!(
                "{} of {match_count}",
                self.response_viewer.active_match() + 1
            )
        };
        let search_view = cx.weak_entity();
        let enter_view = cx.weak_entity();
        let previous_view = cx.weak_entity();
        let next_view = cx.weak_entity();
        let search = div()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .child(components::search_input(
                theme,
                "response-search-input",
                self.response_viewer.search().to_owned(),
                "Search",
                move |value, _, input_cx| {
                    let _ = search_view.update(input_cx, |view, cx| {
                        view.response_viewer.set_search(value.to_string());
                        cx.notify();
                    });
                },
                move |_, _, input_cx| {
                    let _ = enter_view.update(input_cx, |view, cx| {
                        view.step_response_match(key, 1);
                        cx.notify();
                    });
                },
            ))
            .child(
                div()
                    .id("response-search-count")
                    .debug_selector(|| "response-search-count".into())
                    .text_size(px(theme.typography.caption_size))
                    .text_color(theme.colors.text.muted)
                    .child(search_label),
            )
            .child(components::editor_button(
                theme,
                "response-search-previous",
                "↑",
                false,
                move |_, _, cx| {
                    let _ = previous_view.update(cx, |view, cx| {
                        view.step_response_match(key, -1);
                        cx.notify();
                    });
                },
            ))
            .child(components::editor_button(
                theme,
                "response-search-next",
                "↓",
                false,
                move |_, _, cx| {
                    let _ = next_view.update(cx, |view, cx| {
                        view.step_response_match(key, 1);
                        cx.notify();
                    });
                },
            ));

        let mut banners = div()
            .px(px(theme.metrics.spacing_2))
            .pt(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_1));
        let mut has_banner = false;
        if document.truncated {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.status.warning)
                    .text_size(px(theme.typography.caption_size))
                    .child("Response body is truncated at the in-memory limit."),
            );
        }
        if let Some(notice) = &document.pretty_notice
            && self.response_viewer.tab() != ResponseViewerTab::Headers
        {
            has_banner = true;
            banners = banners.child(
                div()
                    .text_color(theme.colors.text.muted)
                    .text_size(px(theme.typography.caption_size))
                    .child(notice.clone()),
            );
        }

        let list = match self.response_viewer.tab() {
            ResponseViewerTab::Headers => self.render_response_headers(theme, key, document, cx),
            ResponseViewerTab::Pretty | ResponseViewerTab::Raw => {
                self.render_response_body(theme, key, document, cx)
            }
        };

        div()
            .flex_1()
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .px(px(theme.metrics.spacing_2))
                    .py(px(theme.metrics.spacing_1))
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(theme.metrics.spacing_2))
                    .border_b_1()
                    .border_color(theme.colors.borders.subtle)
                    .child(tabs)
                    .child(search),
            )
            .when(has_banner, |panel| panel.child(banners))
            .child(list)
            .into_any_element()
    }

    fn step_response_match(&mut self, key: probe_core::RequestKey, delta: isize) {
        self.response_viewer.step_match(key, delta);
    }

    fn render_response_body(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.binary {
            return placeholder_message(theme, "Binary response body cannot be displayed as text.");
        }
        let text = self.response_viewer.visible_text(key);
        if text.is_empty() {
            return placeholder_message(theme, "Empty response body.");
        }
        let matches = self.response_viewer.matches(key);
        let active_match = self.response_viewer.active_match();
        let view = cx.weak_entity();
        div()
            .id("response-body")
            .debug_selector(|| "response-body".into())
            .flex_1()
            .min_h(px(0.0))
            .p(px(theme.metrics.spacing_2))
            .child(components::response_body_input(
                theme,
                "response-body-editor",
                text,
                &matches,
                active_match,
                if self.response_viewer.tab() == ResponseViewerTab::Pretty
                    && document.pretty_notice.is_none()
                {
                    "json"
                } else {
                    ""
                },
                move |range, cx| {
                    #[cfg(test)]
                    {
                        let _ = view.update(cx, |this, _| {
                            this.rendered_response_rows = range.len();
                        });
                    }
                    #[cfg(not(test))]
                    {
                        let _ = (&view, range, cx);
                    }
                },
            ))
            .into_any_element()
    }

    fn render_response_headers(
        &self,
        theme: Theme,
        key: probe_core::RequestKey,
        document: &PreparedDocument,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if document.headers.is_empty() {
            return placeholder_message(theme, "No response headers");
        }
        let matches = self.response_viewer.matches(key);
        let active_match = self.response_viewer.active_match();
        let view = cx.weak_entity();
        div()
            .id("response-headers")
            .debug_selector(|| "response-headers".into())
            .flex_1()
            .min_h(px(0.0))
            .p(px(theme.metrics.spacing_2))
            .child(components::response_headers_input(
                theme,
                "response-headers-editor",
                &document.headers,
                &matches,
                active_match,
                move |range, cx| {
                    #[cfg(test)]
                    {
                        let _ = view.update(cx, |this, _| {
                            this.rendered_response_rows = range.len();
                        });
                    }
                    #[cfg(not(test))]
                    {
                        let _ = (&view, range, cx);
                    }
                },
            ))
            .into_any_element()
    }

    fn render_editor_response(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let response_view = cx.weak_entity();
        let horizontal = self.shell.pane_layout == PaneLayout::Horizontal;
        let handle = div()
            .id("response-resize-handle")
            .flex_none()
            .bg(theme.colors.borders.subtle)
            .when(horizontal, |handle| {
                handle
                    .w(px(5.0))
                    .h_full()
                    .cursor(CursorStyle::ResizeLeftRight)
            })
            .when(!horizontal, |handle| {
                handle.h(px(5.0)).w_full().cursor(CursorStyle::ResizeUpDown)
            })
            .hover(move |handle| handle.bg(theme.colors.borders.focused))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                let _ = response_view.update(cx, |view, cx| {
                    view.shell.resizing = Some(ResizePane::Response);
                    cx.notify();
                });
            });

        div()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .when(horizontal, |work_area| work_area.flex_row())
            .when(!horizontal, |work_area| work_area.flex_col())
            .child(self.render_request_editor(theme, cx))
            .child(handle)
            .child(self.render_response_panel(theme, cx))
    }

    fn render_titlebar(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::Div {
        let switcher_view = cx.weak_entity();
        let sidebar_toggle_view = cx.weak_entity();
        let home_view = cx.weak_entity();
        let new_view = cx.weak_entity();
        let open_view = cx.weak_entity();
        let import_yaak_view = cx.weak_entity();
        let layout_view = cx.weak_entity();
        let collection_open = self.loaded_workspace.is_some();
        let mut popup = div()
            .id("workspace-switcher-popup")
            .aria_label("Workspaces")
            .w(px(300.0))
            .p(px(theme.metrics.spacing_1))
            .flex()
            .flex_col()
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard);

        if !self.session.recent_collections.is_empty() {
            popup = popup.child(
                div()
                    .px(px(theme.metrics.spacing_3))
                    .py(px(theme.metrics.spacing_1))
                    .text_size(px(theme.typography.caption_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.colors.text.muted)
                    .child("RECENT COLLECTIONS"),
            );
            for (index, path) in self.session.recent_collections.iter().enumerate() {
                let open_path = path.clone();
                let label = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("Collection")
                    .to_owned();
                let view = cx.weak_entity();
                popup = popup.child(components::menu_button(
                    theme,
                    ("workspace-switcher-recent", index),
                    label,
                    None,
                    move |window, cx| {
                        let path = open_path.clone();
                        let _ = view.update(cx, |view, cx| {
                            view.workspace_switcher_open = false;
                            if !view.loading {
                                view.request_load_workspace(path, None, window, cx);
                            }
                        });
                    },
                ));
            }
            popup = popup.child(
                div()
                    .my(px(theme.metrics.spacing_1))
                    .mx(px(theme.metrics.spacing_3))
                    .flex_none()
                    .h(px(1.0))
                    .bg(theme.colors.borders.standard),
            );
        }

        popup = popup
            .child(
                div()
                    .w_full()
                    .debug_selector(|| "workspace-switcher-new".into())
                    .child(components::menu_button(
                        theme,
                        "workspace-switcher-new",
                        "New Collection…",
                        None,
                        move |window, cx| {
                            let _ = new_view.update(cx, |view, cx| {
                                view.workspace_switcher_open = false;
                                if !view.loading {
                                    view.choose_new_workspace(window, cx);
                                }
                            });
                        },
                    )),
            )
            .child(
                div()
                    .w_full()
                    .debug_selector(|| "workspace-switcher-open".into())
                    .child(components::menu_button(
                        theme,
                        "workspace-switcher-open",
                        "Open Collection…",
                        None,
                        move |window, cx| {
                            let _ = open_view.update(cx, |view, cx| {
                                view.workspace_switcher_open = false;
                                if !view.loading {
                                    view.choose_workspace(window, cx);
                                }
                            });
                        },
                    )),
            )
            .child(
                div()
                    .w_full()
                    .debug_selector(|| "workspace-switcher-import-yaak".into())
                    .child(components::menu_button(
                        theme,
                        "workspace-switcher-import-yaak",
                        "Import from Yaak…",
                        None,
                        move |window, cx| {
                            let _ = import_yaak_view.update(cx, |view, cx| {
                                view.workspace_switcher_open = false;
                                if !view.loading {
                                    view.request_import_yaak(window, cx);
                                }
                            });
                        },
                    )),
            );

        let switcher = Popover::new("workspace-switcher")
            .open(self.workspace_switcher_open)
            .on_open_change(move |open, _, cx| {
                let _ = switcher_view.update(cx, |view, cx| {
                    view.workspace_switcher_open = *open;
                    cx.notify();
                });
            })
            .trigger(
                Button::new("workspace-switcher-trigger")
                    .accessibility_label("Switch workspace")
                    .selected(self.workspace_switcher_open)
                    .h(px(theme.metrics.control_height))
                    .max_w(px(260.0))
                    .px(px(theme.metrics.spacing_3))
                    .flex()
                    .items_center()
                    .gap(px(theme.metrics.spacing_2))
                    .overflow_hidden()
                    .rounded(px(theme.metrics.radius_small))
                    .border_1()
                    .border_color(theme.colors.borders.subtle)
                    .debug_selector(|| "workspace-switcher-trigger".into())
                    .hover(move |trigger| trigger.bg(theme.colors.surfaces.sidebar))
                    .focus(move |trigger| trigger.border_color(theme.colors.borders.focused))
                    .styles(move |styles| {
                        styles.selected(move |trigger| trigger.bg(theme.colors.surfaces.sidebar))
                    })
                    .child(components::truncated_label(self.workspace_name()).flex_1())
                    .child(components::chevron_icon(
                        theme,
                        self.workspace_switcher_open,
                    )),
            )
            .content(move |_, _, _| popup);

        div()
            .h(px(theme.metrics.tab_bar_height))
            .w_full()
            .pl(px(if cfg!(target_os = "macos") {
                80.0
            } else {
                theme.metrics.spacing_3
            }))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(theme.metrics.spacing_1))
            .bg(theme.colors.surfaces.raised)
            .border_b_1()
            .border_color(theme.colors.borders.subtle)
            .child(components::sidebar_toggle(
                theme,
                self.shell.sidebar_collapsed,
                move |_, cx| {
                    let _ = sidebar_toggle_view.update(cx, |view, cx| {
                        view.shell.toggle_sidebar();
                        view.persist_session(cx);
                        cx.notify();
                    });
                },
            ))
            .child(components::home_button(
                theme,
                collection_open,
                move |window, cx| {
                    let _ = home_view.update(cx, |view, cx| {
                        if view.loaded_workspace.is_some() {
                            view.request_close_workspace(window, cx);
                        }
                    });
                },
            ))
            .child(switcher)
            .child(
                div()
                    .h_full()
                    .flex_1()
                    .window_control_area(WindowControlArea::Drag)
                    .on_mouse_down(MouseButton::Left, |_, window, _| {
                        window.start_window_move();
                    }),
            )
            .child(components::pane_layout_toggle(
                theme,
                self.shell.pane_layout,
                move |layout, _, cx| {
                    let _ = layout_view.update(cx, |view, cx| {
                        view.shell.set_pane_layout(layout);
                        view.persist_session(cx);
                        cx.notify();
                    });
                },
            ))
            .child(render_windows_controls(theme))
    }

    fn render_structure_dialog(&self, theme: Theme, cx: &mut Context<Self>) -> gpui::AnyElement {
        let Some(dialog) = self.structure_dialog.as_ref() else {
            return div().into_any_element();
        };
        let name_view = cx.weak_entity();
        let name_enter_view = cx.weak_entity();
        let parent_view = cx.weak_entity();
        let index_view = cx.weak_entity();
        let index_enter_view = cx.weak_entity();
        let cancel_view = cx.weak_entity();
        let submit_view = cx.weak_entity();
        let mut content = div()
            .w(px(420.0))
            .p(px(theme.metrics.spacing_4))
            .flex()
            .flex_col()
            .gap(px(theme.metrics.spacing_3))
            .rounded(px(theme.metrics.radius_medium))
            .bg(theme.colors.surfaces.overlay)
            .border_1()
            .border_color(theme.colors.borders.standard)
            .child(
                div()
                    .text_size(px(theme.typography.title_size))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(dialog.title()),
            );
        if dialog.edits_name() {
            content = content
                .child(
                    div()
                        .text_size(px(theme.typography.caption_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Name"),
                )
                .child(components::dialog_text_input(
                    theme,
                    "structure-name",
                    dialog.name.clone(),
                    "Name",
                    true,
                    move |value, _, cx| {
                        let _ = name_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.name = value.to_string();
                            }
                            cx.notify();
                        });
                    },
                    move |value, window, cx| {
                        let _ = name_enter_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.name = value.to_string();
                            }
                            view.submit_structure_dialog(window, cx);
                        });
                    },
                ));
        }
        if dialog.edits_destination() {
            let mut options = vec![(ROOT_PARENT.to_owned(), "Collection root".to_owned())];
            if let Some(loaded) = &self.loaded_workspace {
                options.extend(loaded.folders().iter().filter_map(|located| {
                    let name = loaded
                        .workspace()
                        .folder(located.key())?
                        .metadata
                        .name
                        .as_deref()
                        .unwrap_or("Untitled folder");
                    Some((
                        located.selector().to_owned(),
                        format!("{name} — {}", located.selector()),
                    ))
                }));
            }
            content = content
                .child(
                    div()
                        .text_size(px(theme.typography.caption_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Destination"),
                )
                .child(components::dropdown(
                    theme,
                    "structure-parent",
                    "Destination folder",
                    Some(dialog.parent.clone()),
                    options,
                    380.0,
                    move |value, _, cx| {
                        let Some(value) = value else {
                            return;
                        };
                        let value = value.clone();
                        let _ = parent_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.parent = value;
                                dialog.index.clear();
                            }
                            cx.notify();
                        });
                    },
                ))
                .child(
                    div()
                        .text_size(px(theme.typography.caption_size))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Position"),
                )
                .child(components::dialog_text_input(
                    theme,
                    "structure-index",
                    dialog.index.clone(),
                    "Append",
                    false,
                    move |value, _, cx| {
                        let _ = index_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.index = value.to_string();
                            }
                            cx.notify();
                        });
                    },
                    move |value, window, cx| {
                        let _ = index_enter_view.update(cx, |view, cx| {
                            if let Some(dialog) = view.structure_dialog.as_mut() {
                                dialog.index = value.to_string();
                            }
                            view.submit_structure_dialog(window, cx);
                        });
                    },
                ));
        }
        let submit_label = dialog.submit_label();
        content = content.child(
            div()
                .flex()
                .justify_end()
                .gap(px(theme.metrics.spacing_2))
                .child(components::secondary_button(
                    theme,
                    "structure-cancel",
                    "Cancel",
                    move |_, window, cx| {
                        let _ = cancel_view.update(cx, |view, cx| {
                            view.structure_dialog = None;
                            view.focus_handle.focus(window, cx);
                            cx.notify();
                        });
                    },
                ))
                .child(components::primary_button(
                    theme,
                    "structure-submit",
                    submit_label,
                    move |_, window, cx| {
                        let _ = submit_view.update(cx, |view, cx| {
                            view.submit_structure_dialog(window, cx);
                        });
                    },
                )),
        );

        div()
            .absolute()
            .top(px(0.0))
            .right(px(0.0))
            .bottom(px(0.0))
            .left(px(0.0))
            .occlude()
            .track_focus(&self.structure_dialog_focus)
            .tab_stop(true)
            .key_context("StructureDialog")
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .absolute()
                    .top(px(0.0))
                    .right(px(0.0))
                    .bottom(px(0.0))
                    .left(px(0.0))
                    .bg(theme.colors.surfaces.scrim),
            )
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(content)
            .into_any_element()
    }

    fn active_request(&self) -> Option<&HttpRequest> {
        let key = self.shell.active_tab()?;
        self.loaded_workspace.as_ref()?.workspace().request(key)
    }

    fn variable_context(&self, cx: &mut Context<Self>) -> components::VariableContext {
        let Some(selected) = self.shell.selected_environment() else {
            return components::VariableContext {
                values: Default::default(),
                unavailable_message: "Select an environment to resolve this variable".to_owned(),
                on_change: None,
                ..components::VariableContext::default()
            };
        };
        let Some(loaded) = &self.loaded_workspace else {
            return components::VariableContext::default();
        };
        match resolve_environment(loaded.workspace().environments(), selected) {
            Ok(environment) => {
                let view = cx.weak_entity();
                components::VariableContext {
                    values: environment.variables().clone(),
                    secrets: environment.secrets_without_values().clone(),
                    unavailable_message: "Variable value is unavailable".to_owned(),
                    on_change: Some(Rc::new(move |name, value, window, cx| {
                        let name = name.to_owned();
                        let view = view.clone();
                        window.defer(cx, move |window, cx| {
                            let _ = view.update(cx, |view, cx| {
                                view.update_environment_variable(&name, value, window, cx);
                            });
                        });
                    })),
                }
            }
            Err(error) => components::VariableContext {
                values: Default::default(),
                unavailable_message: error.to_string(),
                on_change: None,
                ..components::VariableContext::default()
            },
        }
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

    fn tree_search_placeholder(&self) -> String {
        let count = self
            .loaded_workspace
            .as_ref()
            .map_or(0, |loaded| loaded.workspace().request_count());
        if count == 1 {
            "Search in 1 request".to_owned()
        } else {
            format!("Search in {count} requests")
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

impl Render for ProbeApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_active_drag()
            && (self.tree_drop_target.is_some() || self.tree_drag_source.is_some())
        {
            self.clear_tree_drag();
        }
        if self.pending_tab_reveal {
            self.pending_tab_reveal = false;
            cx.on_next_frame(window, |this, _, cx| {
                this.scroll_active_tab_into_view();
                cx.notify();
            });
        }
        let theme = Theme::for_window_appearance(window.appearance());
        let sidebar_view = cx.weak_entity();
        let status_message = self.message.clone();
        let mut status_message_hover: Hsla = theme.colors.status.error.into();
        status_message_hover.l = (status_message_hover.l * 0.88).max(0.0);

        div()
            .size_full()
            .relative()
            .track_focus(&self.focus_handle)
            .bg(theme.colors.surfaces.window)
            .text_color(theme.colors.text.primary)
            .font_family(theme.typography.interface_family)
            .text_size(px(theme.typography.body_size))
            .line_height(relative(theme.typography.body_line_height))
            .flex()
            .flex_col()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.reset_caret_blink(cx)),
            )
            .on_action(cx.listener(|view, _: &SaveRequest, window, cx| {
                view.save_active_request(window, cx);
            }))
            .on_action(cx.listener(|view, _: &OpenWorkspace, window, cx| {
                view.choose_workspace(window, cx);
            }))
            .on_action(cx.listener(|view, _: &NewCollection, window, cx| {
                if !view.loading {
                    view.choose_new_workspace(window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &CloseActiveTab, window, cx| {
                if let Some(key) = view.shell.active_tab() {
                    view.request_close_tab(key, window, cx);
                }
            }))
            .on_action(cx.listener(|view, _: &QuitApplication, window, cx| {
                view.quit_application(window, cx);
            }))
            .on_action(cx.listener(|_, _: &FocusNextControl, window, cx| {
                window.focus_next(cx);
            }))
            .on_action(cx.listener(|_, _: &FocusPreviousControl, window, cx| {
                window.focus_prev(cx);
            }))
            .on_action(cx.listener(|view, _: &NewRequest, window, cx| {
                view.open_create_request_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &NewFolder, window, cx| {
                view.open_create_folder_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &RenameTreeItem, window, cx| {
                view.open_rename_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &DeleteTreeItem, window, cx| {
                view.request_delete_selected(window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItem, window, cx| {
                view.open_move_dialog(window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItemUp, window, cx| {
                view.reorder_selected(-1, window, cx);
            }))
            .on_action(cx.listener(|view, _: &MoveTreeItemDown, window, cx| {
                view.reorder_selected(1, window, cx);
            }))
            .on_action(cx.listener(|view, _: &SelectPreviousTreeItem, _, cx| {
                view.select_tree_offset(-1, cx);
            }))
            .on_action(cx.listener(|view, _: &SelectNextTreeItem, _, cx| {
                view.select_tree_offset(1, cx);
            }))
            .on_action(cx.listener(|view, _: &CollapseTreeItem, _, cx| {
                view.collapse_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &ExpandTreeItem, _, cx| {
                view.expand_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &ActivateTreeItem, _, cx| {
                view.activate_selected_tree_item(cx);
            }))
            .on_action(cx.listener(|view, _: &CancelStructureDialog, window, cx| {
                view.structure_dialog = None;
                view.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.finish_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|view, _, _, cx| view.finish_resize(cx)),
            )
            .child(self.render_titlebar(theme, cx))
            .when_some(status_message, |root, message| {
                root.child(
                    div()
                        .px(px(theme.metrics.spacing_3))
                        .py(px(theme.metrics.spacing_2))
                        .flex()
                        .items_start()
                        .justify_between()
                        .gap(px(theme.metrics.spacing_2))
                        .bg(theme.colors.status.error)
                        .text_color(theme.colors.text.inverse)
                        .child(div().flex_1().min_w(px(0.0)).child(message))
                        .child(
                            Button::new("status-message-dismiss")
                                .focusable(true)
                                .tab_stop(true)
                                .flex_none()
                                .w(px(theme.metrics.control_height - 4.0))
                                .h(px(theme.metrics.control_height - 4.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(theme.metrics.radius_small))
                                .text_color(theme.colors.text.inverse)
                                .hover(move |button| button.bg(status_message_hover))
                                .on_click({
                                    let dismiss_view = cx.weak_entity();
                                    move |_, _, cx| {
                                        let _ = dismiss_view.update(cx, |view, cx| {
                                            view.message = None;
                                            cx.notify();
                                        });
                                    }
                                })
                                .child(
                                    components::close_icon(theme)
                                        .text_color(theme.colors.text.inverse),
                                ),
                        ),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .flex()
                    .when(!self.shell.sidebar_collapsed, |row| {
                        row.child(self.render_sidebar(theme, cx)).child(
                            div()
                                .id("sidebar-resize-handle")
                                .w(px(5.0))
                                .h_full()
                                .flex_none()
                                .cursor(CursorStyle::ResizeLeftRight)
                                .bg(theme.colors.borders.subtle)
                                .hover(move |handle| handle.bg(theme.colors.borders.focused))
                                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                    let _ = sidebar_view.update(cx, |view, cx| {
                                        view.shell.resizing = Some(ResizePane::Sidebar);
                                        cx.notify();
                                    });
                                }),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .h_full()
                            .flex()
                            .flex_col()
                            .child(self.render_tabs(theme, cx))
                            .child(self.render_editor_response(theme, cx)),
                    ),
            )
            .child(self.render_structure_dialog(theme, cx))
            .child(self.render_tab_context_menu(theme, window, cx))
            .child(self.render_tree_context_menu(theme, window, cx))
    }
}

fn shortcut_label_for_action(window: &Window, action: &dyn gpui::Action) -> Option<String> {
    window
        .highest_precedence_binding_for_action(action)
        .map(|binding| shortcut_label_for_binding(&binding))
}

fn shortcut_label_for_action_in_context(
    window: &Window,
    action: &dyn gpui::Action,
    context: &str,
) -> Option<String> {
    let context = gpui::KeyContext::parse(context).ok()?;
    window
        .highest_precedence_binding_for_action_in_context(action, context)
        .map(|binding| shortcut_label_for_binding(&binding))
}

fn shortcut_label_for_binding(binding: &KeyBinding) -> String {
    binding
        .keystrokes()
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(" ")
}

fn tree_level_indent(theme: Theme, depth: usize) -> f32 {
    theme.metrics.spacing_2 + depth as f32 * theme.metrics.icon_standard
}

fn tree_method_font_size(theme: Theme, method: &str) -> f32 {
    if method.len() > 3 {
        theme.typography.caption_size - 2.0
    } else {
        theme.typography.caption_size - 1.0
    }
}

fn tree_method_label(method: &str) -> &str {
    match method {
        "DELETE" => "DEL",
        "OPTION" | "OPTIONS" => "OPT",
        "PATCH" => "PAT",
        "CONNECT" => "CON",
        method if method.len() <= 4 => method,
        _ => "HTTP",
    }
}

fn tree_hierarchy_guides(theme: Theme, depth: usize, selected: bool) -> gpui::Div {
    let mut guides = div().absolute().top(px(0.0)).bottom(px(0.0)).left(px(0.0));
    let color = if selected {
        theme.colors.selection.active_foreground.opacity(0.22)
    } else {
        theme.colors.borders.standard
    };
    for level in 0..depth {
        guides = guides.child(
            div()
                .absolute()
                .top(px(0.0))
                .bottom(px(0.0))
                .left(px(
                    tree_level_indent(theme, level) + theme.metrics.icon_standard / 2.0
                ))
                .w(px(1.0))
                .bg(color),
        );
    }
    guides
}

fn flatten_visible_tree_rows(
    workspace: &Workspace,
    items: &[WorkspaceItemRef],
    depth: usize,
    shell: &ShellState,
    filter: Option<&TreeSearchMatches>,
    rows: &mut Vec<TreeRow>,
) {
    for item in items {
        if filter.is_some_and(|hits| !hits.contains(*item)) {
            continue;
        }
        rows.push(TreeRow { item: *item, depth });
        if let WorkspaceItemRef::Folder(key) = item
            && shell.folder_is_expanded(*key)
            && let Some(folder) = workspace.folder(*key)
        {
            flatten_visible_tree_rows(workspace, &folder.children, depth + 1, shell, filter, rows);
        }
    }
}

fn placeholder_message(theme: Theme, message: &str) -> gpui::AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p(px(theme.metrics.spacing_3))
        .text_color(theme.colors.text.muted)
        .child(message.to_owned())
        .into_any_element()
}

fn request_method_options(
    theme: Theme,
    active_method: &str,
) -> Vec<(String, String, Option<gpui::Rgba>)> {
    let mut methods = vec!["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS"];
    if !methods.contains(&active_method) {
        methods.push(active_method);
    }
    methods
        .into_iter()
        .map(|method| {
            (
                method.to_owned(),
                method.to_owned(),
                Some(theme.method_color(method)),
            )
        })
        .collect()
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
    gpui_platform::application().run(|cx: &mut App| {
        cx.set_app_identity(APPLICATION_ID, APPLICATION_NAME);
        Theme::init(cx);
        bind_platform_hotkeys(cx);

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

        cx.activate(true);
    });
}

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
        KeyBinding::new("n", NewRequest, Some("RequestTree")),
        KeyBinding::new("shift-n", NewFolder, Some("RequestTree")),
        KeyBinding::new("m", MoveTreeItem, Some("RequestTree")),
        KeyBinding::new("alt-up", MoveTreeItemUp, Some("RequestTree")),
        KeyBinding::new("alt-down", MoveTreeItemDown, Some("RequestTree")),
        KeyBinding::new("escape", CancelStructureDialog, Some("StructureDialog")),
    ]);

    #[cfg(target_os = "macos")]
    cx.bind_keys([
        KeyBinding::new("cmd-o", OpenWorkspace, None),
        KeyBinding::new("cmd-n", NewCollection, None),
        KeyBinding::new("cmd-s", SaveRequest, None),
        KeyBinding::new("cmd-w", CloseActiveTab, None),
        KeyBinding::new("cmd-q", QuitApplication, None),
        KeyBinding::new("cmd-e", RenameTreeItem, Some("RequestTree")),
        KeyBinding::new("backspace", DeleteTreeItem, Some("RequestTree")),
    ]);

    #[cfg(not(target_os = "macos"))]
    cx.bind_keys([
        KeyBinding::new("f2", RenameTreeItem, Some("RequestTree")),
        KeyBinding::new("delete", DeleteTreeItem, Some("RequestTree")),
    ]);

    #[cfg(target_os = "windows")]
    cx.bind_keys([
        KeyBinding::new("ctrl-o", OpenWorkspace, None),
        KeyBinding::new("ctrl-n", NewCollection, None),
        KeyBinding::new("ctrl-s", SaveRequest, None),
        KeyBinding::new("ctrl-w", CloseActiveTab, None),
        KeyBinding::new("alt-f4", QuitApplication, None),
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
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, SystemTime, UNIX_EPOCH},
    };

    use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size};
    use probe_core::WorkspaceItemRef;
    use probe_http::{HttpResponse, ResponseHeader};
    use probe_yaak::{ImportDiagnostic, ImportDiagnosticSeverity};

    use super::{
        IMPORT_DIAGNOSTIC_GROUP_LIMIT, ProbeApp, bind_platform_hotkeys, format_import_diagnostics,
    };
    use crate::{
        request_editor::{BodyEditorKind, EditorSection},
        response_viewer::ResponseViewerTab,
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
    fn discarding_a_dirty_tab_restores_the_workspace_request(cx: &mut TestAppContext) {
        cx.update(Theme::init);
        let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
            ProbeApp::new(window, cx)
        });
        let fixture = bundled_fixture()
            .canonicalize()
            .expect("fixture should exist");
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        visual
            .debug_bounds("workspace-switcher-import-yaak")
            .expect("workspace switcher should include Import from Yaak");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
                view.restore_shell_state();
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
        let request_key = workspace.requests()[0].key();
        window
            .update(cx, |view, _, cx| {
                view.session_store = None;
                view.set_workspace(fixture, workspace);
                view.select_request(request_key, cx);
                let (cancellation, _) = tokio::sync::oneshot::channel();
                let generation = view.execution.begin(request_key, cancellation);
                view.complete_execution(
                    request_key,
                    generation,
                    Ok(HttpResponse {
                        status: 201,
                        reason: "Created".to_owned(),
                        url: "https://api.example.test/users".to_owned(),
                        duration: Duration::from_millis(42),
                        size: 11,
                        headers: vec![ResponseHeader {
                            name: "content-type".to_owned(),
                            value: "application/json".to_owned(),
                        }],
                        body: br#"{"ok":true}"#.to_vec(),
                        body_complete: true,
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
            assert!(visual.debug_bounds("response-tab-pretty").is_some());
            assert!(visual.debug_bounds("response-tab-raw").is_some());
            assert!(visual.debug_bounds("response-tab-headers").is_some());
            assert!(visual.debug_bounds("response-search").is_some());
            assert!(visual.debug_bounds("response-body").is_some());
            assert!(visual.debug_bounds("response-headers").is_none());
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
                view.response_viewer.set_search("ok".to_owned());
                cx.notify();
            })
            .expect("test window should remain open");
        cx.run_until_parked();
        let match_count = window
            .update(cx, |view, _, _| {
                view.response_viewer.matches(request_key).len()
            })
            .expect("test window should remain open");
        assert!(match_count >= 1);
        {
            let mut visual = VisualTestContext::from_window(window.into(), cx);
            assert!(visual.debug_bounds("response-search-count").is_some());
            assert!(visual.debug_bounds("response-body").is_some());
        }
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
    fn environment_selection_is_shared_when_opening_another_request(cx: &mut TestAppContext) {
        cx.update(Theme::init);
        let window = cx.open_window(size(px(1180.0), px(780.0)), |window, cx| {
            ProbeApp::new(window, cx)
        });
        let fixture = environment_fixture()
            .canonicalize()
            .expect("fixture should exist");
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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

        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should reload");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let reloaded =
            probe_opencollection::load_workspace(&fixture).expect("saved env should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let reloaded =
            probe_opencollection::load_workspace(&fixture).expect("saved env should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
                        request.body = Some(probe_core::RequestBody::Single(
                            probe_core::Body::Raw(probe_core::RawBody {
                                kind: probe_core::RawBodyKind::Json,
                                data: "{\n  \"tenant\": \"{{tenant}}\"\n}".to_owned(),
                            }),
                        ));
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
    fn platform_close_tab_hotkey_closes_the_active_request(cx: &mut TestAppContext) {
        cx.update(Theme::init);
        cx.update(bind_platform_hotkeys);
        let window = cx.open_window(size(px(900.0), px(640.0)), |window, cx| {
            ProbeApp::new(window, cx)
        });
        let fixture = bundled_fixture()
            .canonicalize()
            .expect("fixture should exist");
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
        let workspace =
            probe_opencollection::load_workspace(&fixture).expect("fixture should load");
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
                let last_visible =
                    view.tab_bar_scroll
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
}
