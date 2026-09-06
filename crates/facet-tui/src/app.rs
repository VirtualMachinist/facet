use std::{
    error::Error,
    fmt, io,
    path::Path,
    time::{Duration, Instant},
};

use facet_record::{ConfigOverrides, RecordRequest, Recording, record};

use crossterm::event::{KeyCode, KeyModifiers};
use probe_core::{
    FolderKey, Header, HttpRequest, QueryParameter, RequestKey, RequestUpdate, resolve_environment,
    resolve_request,
};
use probe_http::{ExecutionOptions, HttpEngine, HttpResponse};
use probe_opencollection::{LoadedWorkspace, SaveError, load_workspace};
use ratatui::Terminal;
use ratatui::backend::Backend;
use tokio::sync::mpsc;
use tokio::sync::watch;

use crate::theme::{Appearance, Theme};
use crate::tree::{Row, TreeView};

/// Top-level TUI error type. Wraps I/O, OpenCollection, and HTTP failures
/// in a single variant family so the main entrypoint can show a clean
/// status message without re-implementing error handling.
#[derive(Debug)]
pub enum TuiError {
    Io(io::Error),
    Workspace(String),
    Http(String),
    Draw(String),
}

impl fmt::Display for TuiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "io error: {error}"),
            Self::Workspace(message) => write!(formatter, "workspace: {message}"),
            Self::Http(message) => write!(formatter, "http: {message}"),
            Self::Draw(message) => write!(formatter, "draw: {message}"),
        }
    }
}

impl Error for TuiError {}

impl From<io::Error> for TuiError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Focusable pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Focus {
    Tree,
    Request,
    Response,
}

/// Which field inside the request pane is active.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestFocus {
    Url,
    Editor,
}

/// App-wide vim mode (Surface 2, option A). `Normal` is the verb state —
/// navigation, pane keys, and the flat arrow/enter fallback all live here.
/// `Insert` is the only place text fields accept input. `Command` is the
/// `:` command line. The footer shows a three-letter indicator
/// (NOR/INS/CMD), helix-style.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
}

impl Mode {
    pub fn indicator(self) -> &'static str {
        match self {
            Mode::Normal => "NOR",
            Mode::Insert => "INS",
            Mode::Command => "CMD",
        }
    }
}

/// Section tab the user is currently inside. Matches the desktop labels
/// (Path / Query / Headers / Body / Authentication) and is rendered as
/// a single stone-pill row in the work area.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Section {
    Path,
    Query,
    Headers,
    Body,
    Auth,
}

impl Section {
    pub const ALL: [Section; 5] = [
        Section::Path,
        Section::Query,
        Section::Headers,
        Section::Body,
        Section::Auth,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Path => "Path",
            Section::Query => "Query",
            Section::Headers => "Headers",
            Section::Body => "Body",
            Section::Auth => "Auth",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Section::Path => Section::Query,
            Section::Query => Section::Headers,
            Section::Headers => Section::Body,
            Section::Body => Section::Auth,
            Section::Auth => Section::Path,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Section::Path => Section::Auth,
            Section::Query => Section::Path,
            Section::Headers => Section::Query,
            Section::Body => Section::Headers,
            Section::Auth => Section::Body,
        }
    }

    pub fn is_kv(self) -> bool {
        matches!(self, Section::Path | Section::Query | Section::Headers)
    }
}

/// Tabs on the response pane. `Pretty` formats the body when it can (JSON
/// today); `Raw` shows the body exactly as decoded; `Headers` lists response
/// headers; `Inspect` is the run summary (status, timing, sizes).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ResponseTab {
    #[default]
    Pretty,
    Raw,
    Headers,
    Inspect,
}

impl ResponseTab {
    pub const ALL: [ResponseTab; 4] = [
        ResponseTab::Pretty,
        ResponseTab::Raw,
        ResponseTab::Headers,
        ResponseTab::Inspect,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ResponseTab::Pretty => "Pretty",
            ResponseTab::Raw => "Raw",
            ResponseTab::Headers => "Headers",
            ResponseTab::Inspect => "Inspect",
        }
    }

    pub fn next(self) -> Self {
        match self {
            ResponseTab::Pretty => ResponseTab::Raw,
            ResponseTab::Raw => ResponseTab::Headers,
            ResponseTab::Headers => ResponseTab::Inspect,
            ResponseTab::Inspect => ResponseTab::Pretty,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            ResponseTab::Pretty => ResponseTab::Inspect,
            ResponseTab::Raw => ResponseTab::Pretty,
            ResponseTab::Headers => ResponseTab::Raw,
            ResponseTab::Inspect => ResponseTab::Headers,
        }
    }
}

/// Status of the most recent send, or the current run.
#[derive(Clone, Debug, Default)]
pub enum RunStatus {
    #[default]
    Idle,
    Running,
    Done {
        status: u16,
        duration: Duration,
    },
    Failed(String),
}

impl RunStatus {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Running => "running…",
            Self::Done { .. } => "done",
            Self::Failed(_) => "failed",
        }
    }
}

/// One response body preview, capped for the response pane.
#[derive(Clone, Debug)]
pub struct ResponseView {
    pub status: u16,
    pub reason: String,
    pub url: String,
    pub duration: Duration,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// Byte length of the body as received, before lossy UTF-8 decoding.
    pub body_len: usize,
}

impl ResponseView {
    pub fn from_response(response: HttpResponse) -> Self {
        let body_len = response.body.len();
        let body = String::from_utf8_lossy(&response.body).into_owned();
        Self {
            status: response.status,
            reason: response.reason,
            url: response.url,
            duration: response.duration,
            headers: response
                .headers
                .into_iter()
                .map(|header| (header.name, header.value))
                .collect(),
            body,
            body_len,
        }
    }

    /// Value of the `Content-Type` response header, if present.
    pub fn content_type(&self) -> Option<&str> {
        self.headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
            .map(|(_, value)| value.as_str())
    }

    /// Body for the Pretty tab: indented JSON when the payload parses as
    /// JSON, otherwise the raw decoded text unchanged.
    pub fn pretty_body(&self) -> String {
        let trimmed = self.body.trim_start();
        let looks_json = trimmed.starts_with('{') || trimmed.starts_with('[');
        if !looks_json {
            return self.body.clone();
        }
        match serde_json::from_str::<serde_json::Value>(&self.body) {
            Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| self.body.clone()),
            Err(_) => self.body.clone(),
        }
    }
}

/// Mutable buffer the user is editing. Mirrors the request field that
/// was focused when insert mode was entered. Save flushes this back
/// into the `LoadedWorkspace` via `RequestUpdate`.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    pub url: String,
    pub path_rows: Vec<(String, String)>,
    pub query_rows: Vec<(String, String)>,
    pub header_rows: Vec<(String, String)>,
    pub body_text: String,
    pub body_kind: BodyKind,
    pub auth_label: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BodyKind {
    #[default]
    None,
    Json,
    Text,
    Xml,
    Sparql,
    Form,
    Multipart,
    File,
}

impl BodyKind {
    pub const ALL: [BodyKind; 8] = [
        BodyKind::None,
        BodyKind::Json,
        BodyKind::Text,
        BodyKind::Xml,
        BodyKind::Sparql,
        BodyKind::Form,
        BodyKind::Multipart,
        BodyKind::File,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BodyKind::None => "None",
            BodyKind::Json => "JSON",
            BodyKind::Text => "Text",
            BodyKind::Xml => "XML",
            BodyKind::Sparql => "SPARQL",
            BodyKind::Form => "Form",
            BodyKind::Multipart => "Multipart",
            BodyKind::File => "File",
        }
    }

    fn next(self) -> Self {
        match self {
            BodyKind::None => BodyKind::Json,
            BodyKind::Json => BodyKind::Text,
            BodyKind::Text => BodyKind::Xml,
            BodyKind::Xml => BodyKind::Sparql,
            BodyKind::Sparql => BodyKind::Form,
            BodyKind::Form => BodyKind::Multipart,
            BodyKind::Multipart => BodyKind::File,
            BodyKind::File => BodyKind::None,
        }
    }
}

/// Snapshot of the loaded request used to detect unsaved edits.
#[derive(Clone, Debug, Default)]
pub struct EditorSnapshot {
    pub url: String,
    pub method: String,
    pub path_rows: Vec<(String, String)>,
    pub header_rows: Vec<(String, String)>,
    pub query_rows: Vec<(String, String)>,
    pub body_text: String,
    pub body_kind: BodyKind,
}

impl EditorSnapshot {
    pub fn from_request(request: &HttpRequest) -> Self {
        Self {
            url: request.url.clone().unwrap_or_default(),
            method: request.method.clone().unwrap_or_else(|| "GET".to_string()),
            path_rows: request
                .path_parameters
                .iter()
                .map(|param| (param.name.clone(), param.value.clone()))
                .collect(),
            header_rows: request
                .headers
                .iter()
                .map(|header| (header.name.clone(), header.value.clone()))
                .collect(),
            query_rows: request
                .query_parameters
                .iter()
                .map(|param| (param.name.clone(), param.value.clone()))
                .collect(),
            body_text: body_text(request),
            body_kind: body_kind(request),
        }
    }

    pub fn matches(&self, method: &str, editor: &Editor) -> bool {
        self.url == editor.url
            && self.method == method
            && self.path_rows == editor.path_rows
            && self.header_rows == editor.header_rows
            && self.query_rows == editor.query_rows
            && self.body_text == editor.body_text
            && self.body_kind == editor.body_kind
    }
}

fn body_text(request: &HttpRequest) -> String {
    match &request.body {
        Some(probe_core::RequestBody::Single(probe_core::Body::Raw(raw))) => raw.data.clone(),
        _ => String::new(),
    }
}

fn body_kind(request: &HttpRequest) -> BodyKind {
    match &request.body {
        Some(probe_core::RequestBody::Single(probe_core::Body::Raw(raw))) => match raw.kind {
            probe_core::RawBodyKind::Json => BodyKind::Json,
            probe_core::RawBodyKind::Text => BodyKind::Text,
            probe_core::RawBodyKind::Xml => BodyKind::Xml,
            probe_core::RawBodyKind::Sparql => BodyKind::Sparql,
        },
        Some(probe_core::RequestBody::Single(probe_core::Body::FormUrlEncoded(_))) => {
            BodyKind::Form
        }
        Some(probe_core::RequestBody::Single(probe_core::Body::Multipart(_))) => {
            BodyKind::Multipart
        }
        Some(probe_core::RequestBody::Single(probe_core::Body::File(_))) => BodyKind::File,
        _ => BodyKind::None,
    }
}

fn auth_label(request: &HttpRequest) -> String {
    match &request.authentication {
        Some(auth) => auth.kind.as_str().to_string(),
        None => "No authentication".to_string(),
    }
}

/// TUI application state.
pub struct App {
    pub(crate) loaded: Option<LoadedWorkspace>,
    tree: TreeView,
    /// Active environments selectable from the dropdown. `None` when no
    /// environment is selected (CLI parity: `--environment <name>`).
    environments: Vec<EnvironmentEntry>,
    active_environment: Option<usize>,
    env_dropdown_open: bool,
    focus: Focus,
    searching: bool,
    request_focus: RequestFocus,
    theme_state: Theme,
    section: Section,
    mode: Mode,
    /// `:` command-line buffer (Command mode).
    command: String,
    /// `?` help overlay.
    help_open: bool,
    /// Awaiting the second key of a `Ctrl-W` focus chord.
    ctrl_w_pending: bool,
    editor: Editor,
    editor_snapshot: EditorSnapshot,
    editor_dirty: bool,
    kv_index: usize,
    kv_on_value: bool,
    method: String,
    status: RunStatus,
    response_scroll: u16,
    response_tab: ResponseTab,
    response: Option<ResponseView>,
    /// Receiver for completion events from background runs.
    pending: Option<mpsc::Receiver<(RunResult, Option<RecordSummary>)>>,
    /// Lattice outcome of the last send.
    last_recording: Option<RecordSummary>,
    /// Cancellation signal for an in-flight HTTP request.
    cancel: Option<watch::Sender<bool>>,
    /// Set by quit keys; the run loop breaks on it so the caller can
    /// restore the terminal instead of dying mid-frame via `process::exit`.
    should_quit: bool,
}

#[derive(Clone, Debug)]
pub struct EnvironmentEntry {
    pub name: String,
    pub inherits_from: Option<String>,
    pub variable_count: usize,
}

#[derive(Debug)]
pub enum RunResult {
    Ok(ResponseView),
    Err(String),
}

/// What Lattice did with the last send, for the footer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecordSummary {
    /// Run ULID when the row landed.
    pub run_id: Option<String>,
    /// `recorded`, a skip reason, or `unrecorded: <why>`.
    pub note: String,
}

impl RecordSummary {
    #[must_use]
    pub fn from_recording(recording: &Recording) -> Self {
        match recording {
            Recording::Recorded { run, .. } => Self {
                run_id: Some(run.id.clone()),
                note: "recorded".to_string(),
            },
            Recording::Skipped(reason) => Self {
                run_id: None,
                note: (*reason).to_string(),
            },
            Recording::Failed(message) => Self {
                run_id: None,
                note: format!("unrecorded: {message}"),
            },
        }
    }
}

impl App {
    /// Loads a workspace from disk (or starts empty when no path is given).
    pub async fn load(path: Option<&Path>) -> Self {
        let mut app = Self {
            loaded: None,
            tree: TreeView::default(),
            environments: Vec::new(),
            active_environment: None,
            env_dropdown_open: false,
            focus: Focus::Tree,
            searching: false,
            request_focus: RequestFocus::Url,
            theme_state: Theme::detect_default(),
            section: Section::Path,
            mode: Mode::Normal,
            command: String::new(),
            help_open: false,
            ctrl_w_pending: false,
            editor: Editor::default(),
            editor_snapshot: EditorSnapshot::default(),
            editor_dirty: false,
            kv_index: 0,
            kv_on_value: true,
            method: "GET".to_string(),
            status: RunStatus::Idle,
            response_scroll: 0,
            response_tab: ResponseTab::Pretty,
            response: None,
            pending: None,
            last_recording: None,
            cancel: None,
            should_quit: false,
        };
        if let Some(path) = path {
            match load_workspace(path) {
                Ok(loaded) => app.adopt(loaded),
                Err(error) => {
                    app.status = RunStatus::Failed(format!("load failed: {error}"));
                }
            }
        }
        app
    }

    fn adopt(&mut self, loaded: LoadedWorkspace) {
        let envs: Vec<EnvironmentEntry> = loaded
            .workspace()
            .environments()
            .iter()
            .map(|env| EnvironmentEntry {
                name: env.name.clone(),
                inherits_from: env.extends.clone(),
                variable_count: env.variables.len(),
            })
            .collect();
        self.environments = envs;
        self.active_environment = None;
        self.env_dropdown_open = false;
        let workspace = loaded.workspace().clone();
        self.tree.reset(workspace);
        self.loaded = Some(loaded);
        self.refresh_editor_from_selection();
    }

    fn refresh_editor_from_selection(&mut self) {
        let extracted = self.selected_request().map(|(_, request)| {
            (
                request.method.clone().unwrap_or_else(|| "GET".to_string()),
                Editor {
                    url: request.url.clone().unwrap_or_default(),
                    path_rows: request
                        .path_parameters
                        .iter()
                        .map(|param| (param.name.clone(), param.value.clone()))
                        .collect(),
                    query_rows: request
                        .query_parameters
                        .iter()
                        .map(|param| (param.name.clone(), param.value.clone()))
                        .collect(),
                    header_rows: request
                        .headers
                        .iter()
                        .map(|header| (header.name.clone(), header.value.clone()))
                        .collect(),
                    body_text: body_text(request),
                    body_kind: body_kind(request),
                    auth_label: auth_label(request),
                },
                EditorSnapshot::from_request(request),
            )
        });
        let Some((method, editor, snapshot)) = extracted else {
            self.method = "GET".to_string();
            self.editor = Editor::default();
            self.editor_snapshot = EditorSnapshot::default();
            self.editor_dirty = false;
            self.response = None;
            self.status = RunStatus::Idle;
            self.kv_index = 0;
            return;
        };
        self.method = method;
        self.editor = editor;
        self.editor_snapshot = snapshot;
        self.editor_dirty = false;
        self.response = None;
        self.status = RunStatus::Idle;
        self.response_scroll = 0;
        self.kv_index = 0;
        self.kv_on_value = true;
        self.request_focus = RequestFocus::Url;
    }

    /// Returns the current request selected in the tree, if any.
    pub fn selected_request(&self) -> Option<(RequestKey, &HttpRequest)> {
        let key = self.tree.selected_request_key()?;
        let loaded = self.loaded.as_ref()?;
        let request = loaded.workspace().request(key)?;
        Some((key, request))
    }

    fn base_directory(&self) -> Option<std::path::PathBuf> {
        self.loaded
            .as_ref()
            .and_then(|loaded| loaded.source_path().map(|path| path.to_path_buf()))
    }

    /// Applies a `RequestUpdate` for the currently selected request, then
    /// persists the document.
    pub fn save_current(&mut self) -> Result<(), SaveError> {
        let Some((key, _)) = self.selected_request() else {
            return Ok(());
        };
        let loaded = self.loaded.as_mut().expect("workspace loaded");
        let selector = loaded
            .request_selector(key)
            .ok_or_else(|| SaveError::RequestNotFound("selected".to_string()))?
            .to_string();
        let update = build_update(&self.method, &self.editor);
        loaded.update_request(&selector, &update)?;
        let new_workspace = loaded.workspace().clone();
        self.tree.set_workspace(new_workspace);
        self.refresh_editor_from_selection();
        self.status = RunStatus::Idle;
        Ok(())
    }

    /// Cancels the in-flight HTTP request, if any. Idempotent.
    pub fn cancel_run(&mut self) {
        if let Some(sender) = self.cancel.take() {
            let _ = sender.send(true);
        }
    }

    /// Runs the event loop until the user quits.
    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<(), TuiError> {
        let tick = tokio::time::Duration::from_millis(100);
        let mut last_draw = std::time::Instant::now();
        loop {
            if self.should_quit {
                return Ok(());
            }
            if let Some(receiver) = self.pending.as_mut()
                && let Ok((result, recording)) = receiver.try_recv()
            {
                self.last_recording = recording;
                self.apply_run_result(result);
                self.pending = None;
                self.cancel = None;
            }

            if last_draw.elapsed() >= tick {
                self.draw(terminal)?;
                last_draw = std::time::Instant::now();
            }

            if crossterm::event::poll(tick)? {
                let event = crossterm::event::read()?;
                if self.handle_event(event).await? {
                    self.draw(terminal)?;
                    last_draw = std::time::Instant::now();
                }
            }
        }
    }

    fn draw<B: Backend>(&self, terminal: &mut Terminal<B>) -> Result<(), TuiError> {
        terminal
            .draw(|frame| crate::ui::draw(frame, self))
            .map(|_| ())
            .map_err(|error| TuiError::Draw(error.to_string()))
    }

    /// Public form of `draw` used by the headless smoke harness.
    pub fn render_to<B: Backend>(&self, terminal: &mut Terminal<B>) -> Result<(), TuiError> {
        self.draw(terminal)
    }

    /// Marks the app as mid-send so headless harnesses can render the
    /// running overlay without a network. Not part of the product API.
    #[doc(hidden)]
    pub fn preview_running(&mut self) {
        self.status = RunStatus::Running;
    }

    /// Injects a Lattice outcome for headless renders. Not product API.
    #[doc(hidden)]
    pub fn preview_recording(&mut self, summary: Option<RecordSummary>) {
        self.last_recording = summary;
    }

    /// Lattice outcome of the last send, if any.
    pub fn last_recording(&self) -> Option<&RecordSummary> {
        self.last_recording.as_ref()
    }

    /// Opens the `?` help overlay for headless harnesses. Not part of the
    /// product API.
    #[doc(hidden)]
    pub fn preview_help(&mut self) {
        self.help_open = true;
    }

    /// Returns `Ok(true)` when the event was consumed and a redraw is wanted.
    async fn handle_event(&mut self, event: crossterm::event::Event) -> Result<bool, TuiError> {
        use crossterm::event::{Event, KeyEvent, KeyEventKind};

        if let Event::Key(KeyEvent {
            code,
            kind,
            modifiers,
            ..
        }) = event
            && kind == KeyEventKind::Press
        {
            return self.handle_key(code, modifiers).await;
        }
        Ok(false)
    }

    async fn handle_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool, TuiError> {
        match self.mode {
            Mode::Insert => return self.handle_insert_key(code, modifiers),
            Mode::Command => return self.handle_command_key(code).await,
            Mode::Normal => {}
        }
        if self.help_open {
            return self.handle_help_key(code);
        }
        if self.env_dropdown_open {
            return self.handle_env_dropdown_key(code);
        }
        if self.searching {
            return self.handle_search_key(code, modifiers);
        }
        if self.ctrl_w_pending {
            self.ctrl_w_pending = false;
            self.handle_focus_chord(code);
            return Ok(true);
        }

        if matches!(code, KeyCode::Esc) && self.cancel.is_some() {
            self.cancel_run();
            return Ok(true);
        }

        match (code, modifiers) {
            (KeyCode::Char('q'), _) => {
                self.should_quit = true;
                Ok(true)
            }
            // Esc in Normal is a no-op. Overlays, search, env, Insert and
            // Command consume it above; an in-flight run is cancelled above
            // that. `q` (or `:q`) quits — never Esc.
            (KeyCode::Esc, _) => Ok(true),
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
                Ok(true)
            }
            (KeyCode::Char(':'), _) => {
                self.mode = Mode::Command;
                self.command.clear();
                Ok(true)
            }
            (KeyCode::Char('?'), _) => {
                self.help_open = true;
                Ok(true)
            }
            (KeyCode::Char('w'), KeyModifiers::CONTROL) => {
                self.ctrl_w_pending = true;
                Ok(true)
            }
            (KeyCode::Char('s'), KeyModifiers::CONTROL) => {
                if let Err(error) = self.save_current() {
                    self.status = RunStatus::Failed(format!("save: {error}"));
                }
                Ok(true)
            }
            (KeyCode::Char('e'), _) => {
                self.env_dropdown_open = true;
                Ok(true)
            }
            (KeyCode::Char('/'), _) if self.focus == Focus::Tree => {
                self.searching = true;
                Ok(true)
            }
            (KeyCode::Tab, _) => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Request,
                    Focus::Request => Focus::Response,
                    Focus::Response => Focus::Tree,
                };
                Ok(true)
            }
            (KeyCode::BackTab, _) => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Response,
                    Focus::Request => Focus::Tree,
                    Focus::Response => Focus::Request,
                };
                Ok(true)
            }
            (KeyCode::Char(']'), _) if self.focus == Focus::Request => {
                self.section = self.section.next();
                self.request_focus = RequestFocus::Editor;
                self.kv_index = 0;
                Ok(true)
            }
            (KeyCode::Char('['), _) if self.focus == Focus::Request => {
                self.section = self.section.prev();
                self.request_focus = RequestFocus::Editor;
                self.kv_index = 0;
                Ok(true)
            }
            (KeyCode::Char(']'), _) if self.focus == Focus::Response => {
                self.response_tab = self.response_tab.next();
                self.response_scroll = 0;
                Ok(true)
            }
            (KeyCode::Char('['), _) if self.focus == Focus::Response => {
                self.response_tab = self.response_tab.prev();
                self.response_scroll = 0;
                Ok(true)
            }
            // `i` and `a` both enter Insert (our fields append at the end,
            // so insert *is* append). Appearance moves to `:theme` only —
            // no appearance verbs leak into Normal mode.
            (KeyCode::Char('i') | KeyCode::Char('a'), _) if self.focus == Focus::Request => {
                self.ensure_kv_row();
                self.mode = Mode::Insert;
                Ok(true)
            }
            (KeyCode::Char('m'), _) if self.focus == Focus::Request => {
                cycle_method(&mut self.method);
                self.editor_dirty = true;
                Ok(true)
            }
            (KeyCode::Char('b'), _)
                if self.focus == Focus::Request && self.section == Section::Body =>
            {
                self.editor.body_kind = self.editor.body_kind.next();
                self.editor_dirty = true;
                Ok(true)
            }
            (KeyCode::Char('n'), _) if self.focus == Focus::Request && self.section.is_kv() => {
                self.add_kv_row();
                Ok(true)
            }
            (KeyCode::Char('d'), _) if self.focus == Focus::Request && self.section.is_kv() => {
                self.delete_kv_row();
                Ok(true)
            }
            (KeyCode::Char('h') | KeyCode::Left, _) if self.focus == Focus::Tree => {
                self.toggle_collapse_selected();
                Ok(true)
            }
            (KeyCode::Char('l') | KeyCode::Right, _) if self.focus == Focus::Tree => {
                self.toggle_collapse_selected();
                Ok(true)
            }
            (KeyCode::Char(' '), _) if self.focus == Focus::Tree => {
                self.toggle_collapse_selected();
                Ok(true)
            }
            (KeyCode::Char('h') | KeyCode::Left, _)
                if self.focus == Focus::Request && self.section.is_kv() =>
            {
                self.kv_on_value = false;
                Ok(true)
            }
            (KeyCode::Char('l') | KeyCode::Right, _)
                if self.focus == Focus::Request && self.section.is_kv() =>
            {
                self.kv_on_value = true;
                Ok(true)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if self.focus == Focus::Tree => {
                self.tree.move_selection(1);
                self.refresh_editor_from_selection();
                Ok(true)
            }
            (KeyCode::Up | KeyCode::Char('k'), _) if self.focus == Focus::Tree => {
                self.tree.move_selection(-1);
                self.refresh_editor_from_selection();
                Ok(true)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if self.focus == Focus::Request => {
                self.move_request_cursor(1);
                Ok(true)
            }
            (KeyCode::Up | KeyCode::Char('k'), _) if self.focus == Focus::Request => {
                self.move_request_cursor(-1);
                Ok(true)
            }
            (KeyCode::Down | KeyCode::Char('j'), _) if self.focus == Focus::Response => {
                self.response_scroll = self.response_scroll.saturating_add(1);
                Ok(true)
            }
            (KeyCode::Up | KeyCode::Char('k'), _) if self.focus == Focus::Response => {
                self.response_scroll = self.response_scroll.saturating_sub(1);
                Ok(true)
            }
            (KeyCode::PageDown, _) => {
                self.response_scroll = self.response_scroll.saturating_add(10);
                Ok(true)
            }
            (KeyCode::PageUp, _) => {
                self.response_scroll = self.response_scroll.saturating_sub(10);
                Ok(true)
            }
            (KeyCode::Enter, _) if self.focus == Focus::Tree => {
                if self.tree.selected_folder_key().is_some() {
                    self.toggle_collapse_selected();
                } else if self.tree.selected_request_key().is_some() {
                    self.focus = Focus::Request;
                    self.request_focus = RequestFocus::Url;
                }
                Ok(true)
            }
            (KeyCode::Enter, _) => {
                self.run_selected().await?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn handle_insert_key(
        &mut self,
        code: KeyCode,
        _modifiers: KeyModifiers,
    ) -> Result<bool, TuiError> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                Ok(true)
            }
            KeyCode::Enter => {
                if let Err(error) = self.save_current() {
                    self.status = RunStatus::Failed(format!("save: {error}"));
                }
                self.mode = Mode::Normal;
                Ok(true)
            }
            KeyCode::Backspace => {
                self.pop_in_active();
                Ok(true)
            }
            KeyCode::Char(ch) => {
                self.push_in_active(ch);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Second key of the `Ctrl-W` focus chord (vim window style).
    /// `h`/`l` left-right, `j`/`k` down-up across the stacked right panes,
    /// `w` cycles like Tab. Any other key just ends the chord.
    fn handle_focus_chord(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('h') | KeyCode::Left => self.focus = Focus::Tree,
            KeyCode::Char('k') | KeyCode::Up | KeyCode::Char('l') | KeyCode::Right => {
                self.focus = Focus::Request;
            }
            KeyCode::Char('j') | KeyCode::Down => self.focus = Focus::Response,
            KeyCode::Char('w') => {
                self.focus = match self.focus {
                    Focus::Tree => Focus::Request,
                    Focus::Request => Focus::Response,
                    Focus::Response => Focus::Tree,
                };
            }
            _ => {}
        }
    }

    /// `:` command line. Esc cancels, Enter executes, printable chars edit.
    async fn handle_command_key(&mut self, code: KeyCode) -> Result<bool, TuiError> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command.clear();
                Ok(true)
            }
            KeyCode::Enter => {
                self.execute_command().await?;
                Ok(true)
            }
            KeyCode::Backspace => {
                self.command.pop();
                Ok(true)
            }
            KeyCode::Char(ch) => {
                self.command.push(ch);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Executes the `:` buffer and returns to Normal mode. Command set is
    /// vim grammar, not aliases: `w`/`q`/`wq`, `send`, `theme`, `env`,
    /// `help`.
    async fn execute_command(&mut self) -> Result<(), TuiError> {
        let input = self.command.trim().to_string();
        self.command.clear();
        self.mode = Mode::Normal;
        let (name, arg) = match input.split_once(char::is_whitespace) {
            Some((name, rest)) => (name, rest.trim()),
            None => (input.as_str(), ""),
        };
        match name {
            "q" | "quit" => self.should_quit = true,
            "w" | "write" => {
                if let Err(error) = self.save_current() {
                    self.status = RunStatus::Failed(format!("save: {error}"));
                }
            }
            "wq" => {
                if let Err(error) = self.save_current() {
                    self.status = RunStatus::Failed(format!("save: {error}"));
                }
                self.should_quit = true;
            }
            "send" => {
                self.run_selected().await?;
            }
            "help" => self.help_open = true,
            "theme" | "appearance" => match arg {
                "graphite" | "dark" => {
                    self.theme_state =
                        Theme::new(Appearance::Dark).with_depth(self.theme_state.depth());
                }
                "porcelain" | "light" => {
                    self.theme_state =
                        Theme::new(Appearance::Light).with_depth(self.theme_state.depth());
                }
                _ => {
                    self.status = RunStatus::Failed("usage: :theme graphite|porcelain".to_string());
                }
            },
            "env" | "environment" => {
                if arg.is_empty() {
                    self.status = RunStatus::Failed("usage: :env <name>|none".to_string());
                } else if arg.eq_ignore_ascii_case("none") {
                    self.active_environment = None;
                } else if let Some(index) = self
                    .environments
                    .iter()
                    .position(|entry| entry.name.eq_ignore_ascii_case(arg))
                {
                    self.active_environment = Some(index);
                } else {
                    self.status = RunStatus::Failed(format!("no environment named {arg}"));
                }
            }
            "" => {}
            _ => {
                self.status = RunStatus::Failed(format!("unknown command :{name}"));
            }
        }
        Ok(())
    }

    /// `?` help overlay: any of Esc/q/?/Enter closes it.
    fn handle_help_key(&mut self, code: KeyCode) -> Result<bool, TuiError> {
        match code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.help_open = false;
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn handle_search_key(
        &mut self,
        code: KeyCode,
        modifiers: KeyModifiers,
    ) -> Result<bool, TuiError> {
        match (code, modifiers) {
            (KeyCode::Esc, _) => {
                self.tree.clear_search();
                self.searching = false;
                self.refresh_editor_from_selection();
                Ok(true)
            }
            (KeyCode::Enter, _) => {
                self.searching = false;
                Ok(true)
            }
            (KeyCode::Backspace, _) => {
                self.tree.pop_search();
                self.refresh_editor_from_selection();
                Ok(true)
            }
            (KeyCode::Char(ch), _) if !ch.is_control() => {
                self.tree.push_search(ch);
                self.refresh_editor_from_selection();
                Ok(true)
            }
            _ => Ok(true),
        }
    }

    fn handle_env_dropdown_key(&mut self, code: KeyCode) -> Result<bool, TuiError> {
        match code {
            KeyCode::Esc | KeyCode::Char('e') | KeyCode::Char('w') => {
                self.env_dropdown_open = false;
                Ok(true)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.active_environment = match self.active_environment {
                    None if self.environments.is_empty() => None,
                    None => Some(0),
                    Some(idx) if idx + 1 < self.environments.len() => Some(idx + 1),
                    Some(_) => None,
                };
                Ok(true)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.active_environment = match self.active_environment {
                    None if self.environments.is_empty() => None,
                    None => Some(self.environments.len() - 1),
                    Some(0) => None,
                    Some(idx) => Some(idx - 1),
                };
                Ok(true)
            }
            KeyCode::Enter => {
                self.env_dropdown_open = false;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn kv_rows_mut(&mut self) -> Option<&mut Vec<(String, String)>> {
        match self.section {
            Section::Path => Some(&mut self.editor.path_rows),
            Section::Query => Some(&mut self.editor.query_rows),
            Section::Headers => Some(&mut self.editor.header_rows),
            Section::Body | Section::Auth => None,
        }
    }

    fn kv_len(&self) -> usize {
        match self.section {
            Section::Path => self.editor.path_rows.len(),
            Section::Query => self.editor.query_rows.len(),
            Section::Headers => self.editor.header_rows.len(),
            Section::Body | Section::Auth => 0,
        }
    }

    fn ensure_kv_row(&mut self) {
        if !self.section.is_kv() {
            return;
        }
        if self.kv_len() == 0 {
            self.add_kv_row();
        }
        let len = self.kv_len();
        if len > 0 {
            self.kv_index = self.kv_index.min(len - 1);
        }
    }

    fn add_kv_row(&mut self) {
        let next_index = {
            let Some(rows) = self.kv_rows_mut() else {
                return;
            };
            rows.push((String::new(), String::new()));
            rows.len() - 1
        };
        self.kv_index = next_index;
        self.kv_on_value = false;
        self.editor_dirty = true;
        self.request_focus = RequestFocus::Editor;
    }

    fn delete_kv_row(&mut self) {
        let index = self.kv_index;
        let next_index = {
            let Some(rows) = self.kv_rows_mut() else {
                return;
            };
            if index >= rows.len() {
                return;
            }
            rows.remove(index);
            if rows.is_empty() {
                0
            } else {
                index.min(rows.len() - 1)
            }
        };
        self.kv_index = next_index;
        self.editor_dirty = true;
    }

    fn move_request_cursor(&mut self, delta: isize) {
        if self.request_focus == RequestFocus::Url {
            if delta > 0 {
                self.request_focus = RequestFocus::Editor;
            }
            return;
        }
        if self.section.is_kv() {
            let len = self.kv_len() as isize;
            if len == 0 {
                if delta < 0 {
                    self.request_focus = RequestFocus::Url;
                }
                return;
            }
            let next = self.kv_index as isize + delta;
            if next < 0 {
                self.request_focus = RequestFocus::Url;
            } else {
                self.kv_index = next.clamp(0, len - 1) as usize;
            }
            return;
        }
        if delta < 0 {
            self.request_focus = RequestFocus::Url;
        }
    }

    fn push_in_active(&mut self, ch: char) {
        if self.request_focus == RequestFocus::Url {
            self.editor.url.push(ch);
            self.editor_dirty = true;
            return;
        }
        match self.section {
            Section::Path | Section::Query | Section::Headers => {
                self.ensure_kv_row();
                let index = self.kv_index;
                let on_value = self.kv_on_value;
                let mutated = {
                    if let Some(rows) = self.kv_rows_mut()
                        && let Some(row) = rows.get_mut(index)
                    {
                        if on_value {
                            row.1.push(ch);
                        } else {
                            row.0.push(ch);
                        }
                        true
                    } else {
                        false
                    }
                };
                if mutated {
                    self.editor_dirty = true;
                }
            }
            Section::Body => {
                self.editor.body_text.push(ch);
                self.editor_dirty = true;
            }
            Section::Auth => {}
        }
    }

    fn pop_in_active(&mut self) {
        if self.request_focus == RequestFocus::Url {
            self.editor.url.pop();
            self.editor_dirty = true;
            return;
        }
        match self.section {
            Section::Path | Section::Query | Section::Headers => {
                let index = self.kv_index;
                let on_value = self.kv_on_value;
                let mutated = {
                    if let Some(rows) = self.kv_rows_mut()
                        && let Some(row) = rows.get_mut(index)
                    {
                        if on_value {
                            row.1.pop();
                        } else {
                            row.0.pop();
                        }
                        true
                    } else {
                        false
                    }
                };
                if mutated {
                    self.editor_dirty = true;
                }
            }
            Section::Body => {
                self.editor.body_text.pop();
                self.editor_dirty = true;
            }
            Section::Auth => {}
        }
    }

    fn toggle_collapse_selected(&mut self) {
        if let Some(key) = self.tree.selected_folder_key() {
            self.tree.toggle_collapsed(key);
        }
    }

    fn prepared_request(&self) -> Result<HttpRequest, String> {
        let Some((_, request)) = self.selected_request() else {
            return Err("no request selected".to_string());
        };
        let mut prepared = request.clone();
        build_update(&self.method, &self.editor).apply(&mut prepared);
        if let Some(index) = self.active_environment {
            let Some(entry) = self.environments.get(index) else {
                return Ok(prepared);
            };
            let Some(loaded) = self.loaded.as_ref() else {
                return Ok(prepared);
            };
            let resolved = resolve_environment(loaded.workspace().environments(), &entry.name)
                .map_err(|error| error.to_string())?;
            prepared = resolve_request(&prepared, &resolved).map_err(|error| error.to_string())?;
        }
        Ok(prepared)
    }

    async fn run_selected(&mut self) -> Result<(), TuiError> {
        if self.pending.is_some() {
            return Ok(());
        }
        let request = match self.prepared_request() {
            Ok(request) => request,
            Err(message) => {
                self.status = RunStatus::Failed(message);
                return Ok(());
            }
        };
        let options = ExecutionOptions {
            base_directory: self.base_directory(),
            response_cache: None,
        };
        // Facts for Lattice, captured before the task takes the request.
        let root = self.base_directory();
        let selector = self
            .tree
            .selected_request_key()
            .and_then(|key| self.loaded.as_ref()?.request_selector(key))
            .map(str::to_string);
        let environment = self.active_environment_name().map(str::to_string);
        let should_record = root.is_some() && !facet_record::recording_disabled();
        let actor = facet_record::actor_from_env();
        let session = facet_record::session_from_env();

        let (sender, receiver) = mpsc::channel(1);
        let (cancel_sender, mut cancel_signal) = watch::channel(false);
        self.cancel = Some(cancel_sender);
        self.pending = Some(receiver);
        self.status = RunStatus::Running;
        self.response = None;
        self.response_scroll = 0;
        self.last_recording = None;

        tokio::spawn(async move {
            let started_at = lattice::now_ms();
            let clock = Instant::now();
            let engine = match HttpEngine::new() {
                Ok(engine) => engine,
                Err(error) => {
                    let _ = sender.send((RunResult::Err(error.to_string()), None)).await;
                    return;
                }
            };
            let outcome = engine
                .execute_cancellable(&request, &options, async move {
                    loop {
                        if *cancel_signal.borrow() {
                            return;
                        }
                        if cancel_signal.changed().await.is_err() {
                            return;
                        }
                    }
                })
                .await;
            let elapsed_ms = i64::try_from(clock.elapsed().as_millis()).unwrap_or(i64::MAX);
            // Same recording path as `facet request run`, so TUI and CLI
            // rows are identical. Store I/O is brief and off the UI loop.
            let summary = if should_record {
                let selector = selector.as_deref().unwrap_or("");
                let recording = record(&RecordRequest {
                    root: root.as_deref(),
                    overrides: &ConfigOverrides::default(),
                    selector,
                    environment: environment.as_deref(),
                    request: &request,
                    started_at,
                    elapsed_ms,
                    result: &outcome,
                    output: None,
                    tags: &[],
                    actor: &actor,
                    session: session.as_deref(),
                });
                Some(RecordSummary::from_recording(&recording))
            } else {
                Some(RecordSummary {
                    run_id: None,
                    note: if root.is_none() {
                        "no workspace".to_string()
                    } else {
                        "disabled".to_string()
                    },
                })
            };
            let result = match outcome {
                Ok(response) => RunResult::Ok(ResponseView::from_response(response)),
                Err(error) => RunResult::Err(error.to_string()),
            };
            let _ = sender.send((result, summary)).await;
        });
        Ok(())
    }

    /// `pub(crate)` so headless render tests and the smoke example can
    /// inject a canned response without a network.
    pub(crate) fn apply_run_result(&mut self, result: RunResult) {
        match result {
            RunResult::Ok(view) => {
                self.status = RunStatus::Done {
                    status: view.status,
                    duration: view.duration,
                };
                self.response = Some(view);
            }
            RunResult::Err(message) => {
                self.status = RunStatus::Failed(message);
            }
        }
    }

    /// Returns a slice of the visible sidebar rows.
    pub fn rows(&self) -> &[Row] {
        self.tree.visible_rows()
    }

    /// Current row selection index.
    pub fn selection(&self) -> usize {
        self.tree.selection()
    }

    /// Current focus pane.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Current section tab.
    pub fn section(&self) -> Section {
        self.section
    }

    /// Current editor mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// The `:` command-line buffer, shown in the footer in Command mode.
    pub fn command_line(&self) -> &str {
        &self.command
    }

    pub fn help_open(&self) -> bool {
        self.help_open
    }

    /// Which request-pane field is active.
    pub fn request_focus(&self) -> RequestFocus {
        self.request_focus
    }

    /// True when `key` is collapsed in the sidebar tree.
    pub fn tree_is_collapsed(&self, key: FolderKey) -> bool {
        self.tree.is_collapsed(key)
    }

    /// Current method (URL bar left side).
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Current editor buffer.
    pub fn editor(&self) -> &Editor {
        &self.editor
    }

    /// Whether the editor has unsaved changes.
    pub fn editor_dirty(&self) -> bool {
        self.editor_dirty || !self.editor_snapshot.matches(&self.method, &self.editor)
    }

    /// Search query for the sidebar.
    pub fn search(&self) -> &str {
        self.tree.search()
    }

    /// True if the sidebar search input is capturing keys.
    pub fn searching(&self) -> bool {
        self.searching
    }

    /// True if the sidebar is currently filtered.
    pub fn has_search(&self) -> bool {
        self.tree.has_search()
    }

    /// Index of the active key/value row in Path/Query/Headers.
    pub fn kv_index(&self) -> usize {
        self.kv_index
    }

    /// True when the value cell (not the name) is selected.
    pub fn kv_on_value(&self) -> bool {
        self.kv_on_value
    }

    /// Current theme.
    pub fn theme(&self) -> Theme {
        self.theme_state
    }

    /// Replaces the active theme (used by the headless smoke render
    /// and `--appearance` overrides).
    pub fn apply_theme(&mut self, theme: Theme) {
        self.theme_state = theme;
    }

    /// Response pane scroll offset.
    pub fn response_tab(&self) -> ResponseTab {
        self.response_tab
    }

    pub fn response_scroll(&self) -> u16 {
        self.response_scroll
    }

    /// Most recent response, if any.
    pub fn response(&self) -> Option<&ResponseView> {
        self.response.as_ref()
    }

    /// Current run status.
    pub fn status(&self) -> &RunStatus {
        &self.status
    }

    /// Available environment entries.
    pub fn environments(&self) -> &[EnvironmentEntry] {
        &self.environments
    }

    /// Active environment index.
    pub fn active_environment(&self) -> Option<usize> {
        self.active_environment
    }

    /// Whether the env dropdown overlay is open.
    pub fn env_dropdown_open(&self) -> bool {
        self.env_dropdown_open
    }

    /// Active environment name, if any.
    pub fn active_environment_name(&self) -> Option<&str> {
        self.active_environment
            .and_then(|index| self.environments.get(index))
            .map(|entry| entry.name.as_str())
    }

    /// The collection metadata, if a workspace is loaded.
    pub fn collection_name(&self) -> Option<&str> {
        self.loaded
            .as_ref()
            .and_then(|loaded| loaded.workspace().metadata().name.as_deref())
    }

    /// Whether a collection is loaded (the splash shows when it is not).
    pub fn has_collection(&self) -> bool {
        self.loaded.is_some()
    }

    /// Whether a Lattice workspace store exists beside the loaded collection
    /// (`.facet/lattice.db` in the collection directory). A filesystem check
    /// only; the TUI does not open the store.
    pub fn lattice_ready(&self) -> bool {
        let Some(source) = self.loaded.as_ref().and_then(|loaded| loaded.source_path()) else {
            return false;
        };
        let root = if source.is_dir() {
            source.to_path_buf()
        } else {
            source.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        root.join(".facet").join("lattice.db").is_file()
    }

    /// Resolves a folder's display name from its session key.
    pub fn folder_name(&self, key: FolderKey) -> Option<String> {
        let loaded = self.loaded.as_ref()?;
        let folder = loaded.workspace().folder(key)?;
        Some(display_name(folder.metadata.name.as_deref(), "folder"))
    }

    /// Resolves a request's display name and HTTP method.
    pub fn request_name_and_method(&self, key: RequestKey) -> Option<(String, String)> {
        let loaded = self.loaded.as_ref()?;
        let request = loaded.workspace().request(key)?;
        let name = display_name(request.metadata.name.as_deref(), "request");
        let method = request.method.clone().unwrap_or_else(|| "GET".to_string());
        Some((name, method))
    }

    /// Returns a `›`-joined chain of folder names for a request's ancestors.
    pub fn ancestor_chain(&self, key: RequestKey) -> String {
        let Some(loaded) = self.loaded.as_ref() else {
            return String::new();
        };
        let Some(chain) = loaded.workspace().request_ancestor_folders(key) else {
            return String::new();
        };
        chain
            .iter()
            .filter_map(|folder_key| self.folder_name(*folder_key))
            .collect::<Vec<_>>()
            .join(" › ")
    }
}

fn display_name(name: Option<&str>, fallback: &str) -> String {
    name.map(str::to_string)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| fallback.to_string())
}

const METHOD_CYCLE: &[&str] = &["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

fn cycle_method(slot: &mut String) {
    let upper = slot.to_ascii_uppercase();
    let position = METHOD_CYCLE.iter().position(|method| *method == upper);
    let next = match position {
        Some(idx) => METHOD_CYCLE[(idx + 1) % METHOD_CYCLE.len()],
        None => "GET",
    };
    *slot = next.to_string();
}

fn build_update(method: &str, editor: &Editor) -> RequestUpdate {
    let headers: Vec<Header> = editor
        .header_rows
        .iter()
        .map(|(name, value)| Header {
            name: name.clone(),
            value: value.clone(),
            disabled: false,
        })
        .collect();
    let query: Vec<QueryParameter> = editor
        .query_rows
        .iter()
        .map(|(name, value)| QueryParameter {
            name: name.clone(),
            value: value.clone(),
            disabled: false,
        })
        .collect();
    let path: Vec<QueryParameter> = editor
        .path_rows
        .iter()
        .map(|(name, value)| QueryParameter {
            name: name.clone(),
            value: value.clone(),
            disabled: false,
        })
        .collect();
    let body: Option<probe_core::RequestBody> = match editor.body_kind {
        BodyKind::None => None,
        BodyKind::Json => Some(probe_core::RequestBody::Single(probe_core::Body::Raw(
            probe_core::RawBody {
                kind: probe_core::RawBodyKind::Json,
                data: editor.body_text.clone(),
            },
        ))),
        BodyKind::Text => Some(probe_core::RequestBody::Single(probe_core::Body::Raw(
            probe_core::RawBody {
                kind: probe_core::RawBodyKind::Text,
                data: editor.body_text.clone(),
            },
        ))),
        BodyKind::Xml => Some(probe_core::RequestBody::Single(probe_core::Body::Raw(
            probe_core::RawBody {
                kind: probe_core::RawBodyKind::Xml,
                data: editor.body_text.clone(),
            },
        ))),
        BodyKind::Sparql => Some(probe_core::RequestBody::Single(probe_core::Body::Raw(
            probe_core::RawBody {
                kind: probe_core::RawBodyKind::Sparql,
                data: editor.body_text.clone(),
            },
        ))),
        BodyKind::Form | BodyKind::Multipart | BodyKind::File => None,
    };
    RequestUpdate {
        name: None,
        method: Some(method.to_string()),
        url: Some(editor.url.clone()),
        headers: Some(headers),
        query_parameters: Some(query),
        path_parameters: Some(path),
        body: Some(body),
        authentication: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/opencollection/phase1-bundled.yml")
    }

    #[tokio::test]
    async fn loads_pet_store_and_selects_a_request() {
        let app = App::load(Some(&fixture())).await;
        assert_eq!(app.collection_name(), Some("Pet Store"));
        assert!(app.selected_request().is_some());
        assert!(app.editor().url.contains("example.com"));
        assert_eq!(app.section(), Section::Path);
        assert_eq!(app.focus(), Focus::Tree);
    }

    #[tokio::test]
    async fn collapse_hides_folder_children() {
        let mut app = App::load(Some(&fixture())).await;
        let before = app.rows().len();
        assert!(before > 1, "pet store should list folder + requests");
        // First visible row is the Pets folder.
        if app.tree.selected_folder_key().is_none() {
            app.tree.move_selection(-1);
            while app.tree.selected_folder_key().is_none() && app.selection() > 0 {
                app.tree.move_selection(-1);
            }
        }
        assert!(
            app.tree.selected_folder_key().is_some()
                || matches!(app.rows().first(), Some(Row::Folder { .. }))
        );
        // Select the folder row explicitly: row 0 is Pets.
        if !matches!(app.rows()[0], Row::Folder { .. }) {
            panic!("expected Pets folder at row 0");
        }
        app.tree.move_selection(-(app.selection() as isize));
        app.toggle_collapse_selected();
        assert!(app.rows().len() < before);
        assert!(app.tree_is_collapsed(match app.rows()[0] {
            Row::Folder { key, .. } => key,
            _ => panic!("folder"),
        }));
    }

    #[tokio::test]
    async fn section_tabs_cycle() {
        let mut app = App::load(Some(&fixture())).await;
        assert_eq!(app.section(), Section::Path);
        app.section = app.section.next();
        assert_eq!(app.section(), Section::Query);
        app.section = app.section.prev();
        assert_eq!(app.section(), Section::Path);
        app.section = Section::Auth.next();
        assert_eq!(app.section(), Section::Path);
    }

    #[tokio::test]
    async fn response_tabs_cycle() {
        let mut app = App::load(Some(&fixture())).await;
        assert_eq!(app.response_tab(), ResponseTab::Pretty);
        app.response_tab = app.response_tab.next();
        assert_eq!(app.response_tab(), ResponseTab::Raw);
        app.response_tab = app.response_tab.next();
        app.response_tab = app.response_tab.next();
        assert_eq!(app.response_tab(), ResponseTab::Inspect);
        app.response_tab = app.response_tab.next();
        assert_eq!(app.response_tab(), ResponseTab::Pretty);
        app.response_tab = app.response_tab.prev();
        assert_eq!(app.response_tab(), ResponseTab::Inspect);
    }

    #[test]
    fn pretty_body_formats_json_and_passes_through_text() {
        let json = ResponseView {
            status: 200,
            reason: "OK".to_string(),
            url: "https://example.com".to_string(),
            duration: Duration::from_millis(1),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: "{\"a\":1}".to_string(),
            body_len: 7,
        };
        assert_eq!(json.pretty_body(), "{\n  \"a\": 1\n}");
        assert_eq!(json.content_type(), Some("application/json"));
        assert_eq!(json.body_len, 7);

        let text = ResponseView {
            status: 200,
            reason: "OK".to_string(),
            url: "https://example.com".to_string(),
            duration: Duration::from_millis(1),
            headers: Vec::new(),
            body: "plain text".to_string(),
            body_len: 10,
        };
        assert_eq!(text.pretty_body(), "plain text");
        assert_eq!(text.content_type(), None);

        // JSON-looking but invalid: fall back to the raw body.
        let broken = ResponseView {
            body: "{not json".to_string(),
            body_len: 9,
            ..json.clone()
        };
        assert_eq!(broken.pretty_body(), "{not json");
    }

    #[tokio::test]
    async fn command_line_runs_vim_verbs() {
        let mut app = App::load(Some(&fixture())).await;
        assert_eq!(app.mode(), Mode::Normal);

        // `:` enters Command mode; typing accumulates; Enter executes.
        app.handle_key(KeyCode::Char(':'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.mode(), Mode::Command);
        for ch in "theme porcelain".chars() {
            app.handle_command_key(KeyCode::Char(ch)).await.unwrap();
        }
        assert_eq!(app.command_line(), "theme porcelain");
        app.handle_command_key(KeyCode::Enter).await.unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.theme().appearance(), Appearance::Light);

        // :env with an unknown name fails loudly; :env none clears.
        app.command = "env nosuch".to_string();
        app.execute_command().await.unwrap();
        assert!(matches!(app.status(), RunStatus::Failed(message) if message.contains("nosuch")));
        app.command = "env none".to_string();
        app.execute_command().await.unwrap();
        assert!(app.active_environment.is_none());

        // Unknown command is an error, not a quit.
        app.command = "frobnicate".to_string();
        app.execute_command().await.unwrap();
        assert!(
            matches!(app.status(), RunStatus::Failed(message) if message.contains("unknown command"))
        );
        assert!(!app.should_quit);

        // :q quits.
        app.command = "q".to_string();
        app.execute_command().await.unwrap();
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn command_mode_esc_cancels_without_running() {
        let mut app = App::load(Some(&fixture())).await;
        app.handle_key(KeyCode::Char(':'), KeyModifiers::NONE)
            .await
            .unwrap();
        app.handle_command_key(KeyCode::Char('q')).await.unwrap();
        app.handle_command_key(KeyCode::Esc).await.unwrap();
        assert_eq!(app.mode(), Mode::Normal);
        assert_eq!(app.command_line(), "");
        assert!(!app.should_quit);
    }

    #[tokio::test]
    async fn command_send_and_help_verbs() {
        let mut app = App::load(Some(&fixture())).await;
        app.command = "help".to_string();
        app.execute_command().await.unwrap();
        assert!(app.help_open());
        app.help_open = false;

        app.command = "send".to_string();
        app.execute_command().await.unwrap();
        assert!(matches!(app.status(), RunStatus::Running));
        assert!(app.pending.is_some());
        app.cancel_run();
    }

    #[tokio::test]
    async fn a_enters_insert_and_appearance_has_no_normal_verb() {
        let mut app = App::load(Some(&fixture())).await;
        let before = app.theme().appearance();
        app.focus = Focus::Request;
        app.handle_key(KeyCode::Char('a'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.mode(), Mode::Insert);
        assert_eq!(app.theme().appearance(), before);
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)
            .await
            .unwrap();
        // `t` is unbound in Normal: appearance only moves via :theme.
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.theme().appearance(), before);
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[tokio::test]
    async fn esc_in_normal_never_quits() {
        let mut app = App::load(Some(&fixture())).await;
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.mode(), Mode::Normal);
        // Overlay first: Esc closes help, still no quit.
        app.handle_key(KeyCode::Char('?'), KeyModifiers::NONE)
            .await
            .unwrap();
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(!app.help_open());
        assert!(!app.should_quit);
        // q quits.
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(app.should_quit);
    }

    #[tokio::test]
    async fn ctrl_w_focus_chord_moves_between_panes() {
        let mut app = App::load(Some(&fixture())).await;
        assert_eq!(app.focus(), Focus::Tree);
        app.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await
            .unwrap();
        app.handle_key(KeyCode::Char('l'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Request);
        app.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await
            .unwrap();
        app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Response);
        app.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await
            .unwrap();
        app.handle_key(KeyCode::Char('h'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Tree);
        // Ctrl-W w cycles like Tab.
        app.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await
            .unwrap();
        app.handle_key(KeyCode::Char('w'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Request);
        // An unknown second key just ends the chord.
        app.handle_key(KeyCode::Char('w'), KeyModifiers::CONTROL)
            .await
            .unwrap();
        app.handle_key(KeyCode::Char('x'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Request);
    }

    #[tokio::test]
    async fn flat_fallback_arrows_tab_enter_drive_the_flow() {
        let mut app = App::load(Some(&fixture())).await;
        assert_eq!(app.mode(), Mode::Normal);
        // Up to the top of the tree, then arrows move the selection.
        for _ in 0..5 {
            app.handle_key(KeyCode::Up, KeyModifiers::NONE)
                .await
                .unwrap();
        }
        assert_eq!(app.selection(), 0);
        app.handle_key(KeyCode::Down, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.selection(), 1);
        app.handle_key(KeyCode::Up, KeyModifiers::NONE)
            .await
            .unwrap();
        // Row 0 is the Pets folder: Enter collapses and re-expands it.
        let rows_before = app.rows().len();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(app.rows().len() < rows_before);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.rows().len(), rows_before);
        // Enter on a request opens the editor.
        app.handle_key(KeyCode::Down, KeyModifiers::NONE)
            .await
            .unwrap();
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Request);
        // Arrows move inside the request pane; Tab cycles focus.
        app.handle_key(KeyCode::Down, KeyModifiers::NONE)
            .await
            .unwrap();
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Response);
        app.handle_key(KeyCode::Down, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.response_scroll(), 1);
        app.handle_key(KeyCode::Tab, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.focus(), Focus::Tree);
        assert_eq!(app.mode(), Mode::Normal);
        assert!(!app.should_quit);
    }

    #[tokio::test]
    async fn help_overlay_opens_and_closes() {
        let mut app = App::load(Some(&fixture())).await;
        assert!(!app.help_open());
        app.handle_key(KeyCode::Char('?'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(app.help_open());
        // Keys are swallowed while help is open (no verbs fire).
        app.handle_key(KeyCode::Char('t'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(app.help_open());
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)
            .await
            .unwrap();
        assert!(!app.help_open());
    }

    #[tokio::test]
    async fn insert_mode_is_the_only_text_mode() {
        let mut app = App::load(Some(&fixture())).await;
        app.focus = Focus::Request;
        app.handle_key(KeyCode::Char('i'), KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.mode(), Mode::Insert);
        app.handle_key(KeyCode::Esc, KeyModifiers::NONE)
            .await
            .unwrap();
        assert_eq!(app.mode(), Mode::Normal);
    }

    #[tokio::test]
    async fn search_filters_pet_store() {
        let mut app = App::load(Some(&fixture())).await;
        app.tree.set_search("health");
        let names: Vec<_> = app
            .rows()
            .iter()
            .filter_map(|row| match row {
                Row::Request { key, .. } => app.request_name_and_method(*key).map(|(name, _)| name),
                _ => None,
            })
            .collect();
        assert_eq!(names, vec!["Health check".to_string()]);
    }
}
