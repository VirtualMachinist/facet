//! Application state and event loop for the Probe TUI.

use std::{error::Error, fmt, io, path::Path, time::Duration};

use probe_core::{FolderKey, RequestKey};
use probe_http::{ExecutionOptions, HttpEngine, HttpResponse};
use probe_opencollection::{LoadedWorkspace, load_workspace};
use ratatui::Terminal;
use ratatui::backend::Backend;
use tokio::sync::mpsc;

use crate::theme::{Appearance, Theme};
use crate::tree::{Flattened, Row};

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
    Response,
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
}

impl ResponseView {
    pub fn from_response(response: HttpResponse) -> Self {
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
        }
    }
}

/// TUI application state.
pub struct App {
    pub(crate) loaded: Option<LoadedWorkspace>,
    flattened: Vec<Row>,
    selection: usize,
    focus: Focus,
    theme: Theme,
    engine: HttpEngine,
    status: RunStatus,
    response_scroll: u16,
    response: Option<ResponseView>,
    /// Receiver for completion events from background runs.
    pending: Option<mpsc::Receiver<RunResult>>,
}

#[derive(Debug)]
pub enum RunResult {
    Ok(ResponseView),
    Err(String),
}

impl App {
    /// Loads a workspace from disk (or starts empty when no path is given).
    pub async fn load(path: Option<&Path>) -> Self {
        let mut app = Self {
            loaded: None,
            flattened: Vec::new(),
            selection: 0,
            focus: Focus::Tree,
            theme: Theme::detect_default(),
            engine: HttpEngine::new().expect("default HttpEngine init"),
            status: RunStatus::Idle,
            response_scroll: 0,
            response: None,
            pending: None,
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
        let flattened = Flattened::new(loaded.workspace()).collect();
        self.flattened = flattened;
        self.selection = self.first_request_index().unwrap_or(0);
        self.response = None;
        self.status = RunStatus::Idle;
        self.loaded = Some(loaded);
    }

    /// Returns the current request selected in the tree, if any.
    pub fn selected_request(&self) -> Option<(RequestKey, &probe_core::HttpRequest)> {
        let key = self.selected_request_key()?;
        let loaded = self.loaded.as_ref()?;
        let request = loaded.workspace().request(key)?;
        Some((key, request))
    }

    fn selected_request_key(&self) -> Option<RequestKey> {
        self.flattened
            .get(self.selection)
            .and_then(|row| match row {
                Row::Request { key, .. } => Some(*key),
                Row::Folder { .. } => None,
            })
    }

    fn first_request_index(&self) -> Option<usize> {
        self.flattened
            .iter()
            .position(|row| matches!(row, Row::Request { .. }))
    }

    /// Resolves the active workspace source path for relative body files.
    fn base_directory(&self) -> Option<std::path::PathBuf> {
        self.loaded
            .as_ref()
            .and_then(|loaded| loaded.source_path().map(|path| path.to_path_buf()))
    }

    /// Runs the event loop until the user quits.
    pub async fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<(), TuiError> {
        let tick = tokio::time::Duration::from_millis(100);
        let mut last_draw = std::time::Instant::now();
        loop {
            // Drain any completed background run.
            if let Some(receiver) = self.pending.as_mut()
                && let Ok(result) = receiver.try_recv()
            {
                self.apply_run_result(result);
                self.pending = None;
            }

            // Redraw on tick or when something changes.
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
    async fn handle_event(&mut self, event: crossterm::event::Event) -> Result<bool, TuiError> {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

        match event {
            Event::Key(KeyEvent {
                code,
                kind,
                modifiers,
                ..
            }) if kind == KeyEventKind::Press => match (code, modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                    std::process::exit(0);
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    std::process::exit(0);
                }
                (KeyCode::Char('t'), _) => {
                    self.theme = self.theme.toggle();
                    return Ok(true);
                }
                (KeyCode::Char('a'), _) if self.appearance() == Appearance::Light => {
                    self.theme = Theme::new(Appearance::Dark);
                    return Ok(true);
                }
                (KeyCode::Char('a'), _) => {
                    self.theme = Theme::new(Appearance::Light);
                    return Ok(true);
                }
                (KeyCode::Tab, _) => {
                    self.focus = match self.focus {
                        Focus::Tree => Focus::Response,
                        Focus::Response => Focus::Tree,
                    };
                    return Ok(true);
                }
                (KeyCode::Down | KeyCode::Char('j'), _) => {
                    self.move_selection(1);
                    return Ok(true);
                }
                (KeyCode::Up | KeyCode::Char('k'), _) => {
                    self.move_selection(-1);
                    return Ok(true);
                }
                (KeyCode::PageDown, _) => {
                    self.response_scroll = self.response_scroll.saturating_add(10);
                    return Ok(true);
                }
                (KeyCode::PageUp, _) => {
                    self.response_scroll = self.response_scroll.saturating_sub(10);
                    return Ok(true);
                }
                (KeyCode::Enter, _) => {
                    self.run_selected().await?;
                    return Ok(true);
                }
                _ => {}
            },
            _ => {}
        }
        Ok(false)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.flattened.is_empty() {
            return;
        }
        let len = self.flattened.len() as isize;
        let current = self.selection as isize;
        let mut next = current + delta;
        if next < 0 {
            next = 0;
        } else if next >= len {
            next = len - 1;
        }
        self.selection = next as usize;
        self.response = None;
        self.status = RunStatus::Idle;
    }

    async fn run_selected(&mut self) -> Result<(), TuiError> {
        if self.pending.is_some() {
            return Ok(());
        }
        let Some((_, request)) = self.selected_request() else {
            self.status = RunStatus::Failed("no request selected".to_string());
            return Ok(());
        };
        let request = request.clone();
        let options = ExecutionOptions {
            base_directory: self.base_directory(),
            response_cache: None,
        };
        let (sender, receiver) = mpsc::channel(1);
        self.pending = Some(receiver);
        self.status = RunStatus::Running;
        self.response = None;
        self.response_scroll = 0;

        tokio::spawn(async move {
            let result = match HttpEngine::new() {
                Ok(engine) => match engine.execute(&request, &options).await {
                    Ok(response) => RunResult::Ok(ResponseView::from_response(response)),
                    Err(error) => RunResult::Err(error.to_string()),
                },
                Err(error) => RunResult::Err(error.to_string()),
            };
            let _ = sender.send(result).await;
        });
        Ok(())
    }

    fn apply_run_result(&mut self, result: RunResult) {
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

    /// Returns a slice of the flattened sidebar rows.
    pub fn rows(&self) -> &[Row] {
        &self.flattened
    }

    /// Current row selection index.
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// Current focused pane.
    pub fn focus(&self) -> Focus {
        self.focus
    }

    /// Current theme.
    pub fn theme(&self) -> Theme {
        self.theme
    }

    /// Replaces the active theme (used by the headless smoke render
    /// and `--appearance` overrides).
    pub fn apply_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }
    /// Response pane scroll offset.
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
    fn appearance(&self) -> Appearance {
        self.theme.appearance()
    }

    /// The collection metadata, if a workspace is loaded.
    pub fn collection_name(&self) -> Option<&str> {
        self.loaded
            .as_ref()
            .and_then(|loaded| loaded.workspace().metadata().name.as_deref())
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

    /// Returns a `›`-joined chain of folder names for a request's
    /// ancestors, suitable for a breadcrumb. Empty when the request
    /// lives at the collection root.
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
