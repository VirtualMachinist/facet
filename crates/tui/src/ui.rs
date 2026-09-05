//! TUI rendering: title, collection tree, request method/url, response
//! pane, and footer status line. All colors come from `Theme`/`Palette`
//! so a single switch on `Appearance` restyles every pane.
//!
//! Layout follows the desktop analog: sidebar (collection tree) on the
//! left; work area on the right splits **vertically** (request over
//! response) by default. Chrome is quiet — pane borders are stone, the
//! only accent chrome is the title bar fill, the tree selection pill,
//! and the running-status pill. No double-line boxes, no orange splitters.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::{App, Focus, ResponseView, RunStatus};
use crate::theme::{Appearance, PaletteToken, Styles, resolve};
use crate::tree::Row;

const SIDEBAR_WIDTH: Constraint = Constraint::Length(36);
const REQUEST_PANE_HEIGHT: Constraint = Constraint::Length(7);

/// Top-level entry: paint the whole frame for `app`.
pub fn draw(frame: &mut Frame, app: &App) {
    let styles = Styles::for_theme(app.theme());
    let area = frame.area();

    let shell = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

    render_title(frame, shell[0], app, styles);
    render_body(frame, shell[1], app, styles);
    render_footer(frame, shell[2], app, styles);

    if matches!(app.status(), RunStatus::Running) {
        render_running_overlay(frame, shell[1], styles);
    }
}

fn render_title(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let title_text = match app.collection_name() {
        Some(name) => format!(" probe-tui · {name} "),
        None => " probe-tui ".to_string(),
    };
    let appearance = app.theme().appearance().label();
    let line = Line::from(vec![
        Span::styled(title_text, styles.title),
        Span::raw(" "),
        Span::styled(appearance, styles.muted),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([SIDEBAR_WIDTH, Constraint::Min(20)])
        .split(area);

    render_sidebar(frame, columns[0], app, styles);
    render_work_area(frame, columns[1], app, styles);
}

fn render_sidebar(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let focused = app.focus() == Focus::Tree;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_pane_title(frame, area, " Collection ", focused, styles);
    render_tree(frame, inner, app, styles);
}

fn render_work_area(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let focused = app.focus() == Focus::Response;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_pane_title(frame, area, " Request / Response ", focused, styles);

    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([REQUEST_PANE_HEIGHT, Constraint::Min(5)])
        .split(inner);

    render_request_pane(frame, split[0], app, styles);
    render_response_pane(frame, split[1], app, styles);
}

fn render_pane_title(frame: &mut Frame, area: Rect, label: &str, focused: bool, styles: Styles) {
    let marker = if focused { "▸ " } else { "  " };
    let mut spans = vec![Span::raw(marker)];
    spans.push(Span::styled(label, styles.muted));
    let pad_x = area.x + 2;
    let paragraph = Paragraph::new(Line::from(spans));
    let title_area = Rect {
        x: pad_x,
        y: area.y,
        width: area.width.saturating_sub(2),
        height: 1,
    };
    frame.render_widget(paragraph, title_area);
}

fn render_tree(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    if area.height == 0 {
        return;
    }
    let rows = app.rows();
    if rows.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled("  no workspace loaded", styles.muted)),
            Line::from(Span::styled(
                "  pass a path: probe-tui <collection>",
                styles.muted,
            )),
        ]);
        frame.render_widget(empty, area);
        return;
    }
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| row_item(*row, index, app.selection(), app, styles))
        .collect();
    let list = List::new(items)
        .highlight_style(styles.selected_row)
        .highlight_symbol("  ");
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.selection()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn row_item(row: Row, index: usize, selection: usize, app: &App, styles: Styles) -> ListItem<'_> {
    let selected = index == selection;
    let (depth, prefix, method_label, name) = match row {
        Row::Folder { depth, key } => {
            let name = app
                .folder_name(key)
                .unwrap_or_else(|| "<folder>".to_string());
            (depth, "▾ ", String::new(), name)
        }
        Row::Request { depth, key } => {
            let (name, method) = app
                .request_name_and_method(key)
                .unwrap_or_else(|| ("<request>".to_string(), "GET".to_string()));
            (depth, "  ", method_label(&method), name)
        }
    };
    let indent = "  ".repeat(depth as usize);
    // Tree rows are 4-char method column for requests, blank for folders.
    let method_cell = if method_label.is_empty() {
        "    ".to_string()
    } else {
        format!("{:<4}", method_label)
    };
    let line = if selected {
        // Filled honey pill: every cell on the row inverts to the
        // accent foreground + inverse background, including the method
        // cell — desktop hides the method color on selection.
        Line::from(vec![
            Span::styled(indent, styles.selected_row),
            Span::styled(prefix, styles.selected_row),
            Span::styled(method_cell, styles.selected_row),
            Span::styled(" ", styles.selected_row),
            Span::styled(name, styles.selected_row),
        ])
    } else {
        let method_style = if method_label.is_empty() {
            styles.base
        } else {
            styles
                .method
                .fg(resolve(app.theme(), method_token(&method_label)))
        };
        Line::from(vec![
            Span::raw(indent),
            Span::raw(prefix),
            Span::styled(method_cell, method_style),
            Span::raw(" "),
            Span::styled(name, styles.base),
        ])
    };
    ListItem::new(line)
}

/// Returns the four-character method label shown in the tree column.
fn method_label(method: &str) -> String {
    let upper = method.to_ascii_uppercase();
    match upper.as_str() {
        "DELETE" => "DEL".to_string(),
        "PATCH" => "PAT".to_string(),
        "OPTIONS" => "OPT".to_string(),
        "CONNECT" => "CON".to_string(),
        "HEAD" => "HEAD".to_string(),
        other => {
            if other.len() <= 4 {
                other.to_string()
            } else {
                "HTTP".to_string()
            }
        }
    }
}

/// Maps a tree-column method label back to a `PaletteToken` for the
/// unselected row color.
fn method_token(label: &str) -> PaletteToken {
    match label {
        "GET" => PaletteToken::MethodGet,
        "POST" => PaletteToken::MethodPost,
        "PUT" => PaletteToken::MethodPut,
        "PAT" => PaletteToken::MethodPatch,
        "DEL" => PaletteToken::MethodDelete,
        _ => PaletteToken::MethodOther,
    }
}

fn render_request_pane(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let breadcrumb = match app.selected_request() {
        Some((key, _)) => {
            let ancestors = app.ancestor_chain(key);
            if ancestors.is_empty() {
                "  Collection".to_string()
            } else {
                format!("  Collection › {ancestors}")
            }
        }
        None => "  no request selected".to_string(),
    };

    let url_line = match app.selected_request() {
        Some((_, request)) => {
            let method = request.method.as_deref().unwrap_or("GET");
            let url = request.url.as_deref().unwrap_or("");
            let method_color = app.theme().palette().method(method);
            Line::from(vec![
                Span::styled(format!(" {:<4} ", method), styles.method.fg(method_color)),
                Span::styled("▾ ".to_string(), styles.muted),
                Span::styled(url.to_string(), styles.url),
            ])
        }
        None => Line::from(Span::styled(
            "  press Enter on a request to run it",
            styles.muted,
        )),
    };

    let send_label = match app.status() {
        RunStatus::Running => "  Sending… ",
        _ => "  Send ",
    };
    let send_line = Line::from(Span::styled(send_label, styles.title));

    let lines = vec![
        Line::from(Span::styled(breadcrumb, styles.muted)),
        url_line,
        send_line,
    ];
    let block = Block::default().borders(Borders::NONE);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_response_pane(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    if area.height == 0 {
        return;
    }
    let block = Block::default().borders(Borders::NONE);
    let lines: Vec<Line> = match app.response() {
        Some(response) => response_lines(response, app, styles),
        None => match app.status() {
            RunStatus::Running => vec![Line::from(Span::styled(
                "  Sending… / waiting for the server",
                styles.status_running,
            ))],
            RunStatus::Failed(message) => vec![
                Line::from(Span::styled("  error", styles.status_error)),
                Line::from(Span::styled(format!("  {message}"), styles.muted)),
            ],
            _ => vec![Line::from(Span::styled(
                "  Send a request to see its response.",
                styles.muted,
            ))],
        },
    };
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((app.response_scroll(), 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn response_lines<'a>(response: &'a ResponseView, app: &'a App, styles: Styles) -> Vec<Line<'a>> {
    let palette = app.theme().palette();
    let status_color = palette.response_status(response.status);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  Response  ", styles.muted),
        Span::styled(
            format!("{} {}", response.status, response.reason),
            styles.base.fg(status_color),
        ),
        Span::styled(
            format!("   {} ms", response.duration.as_millis()),
            styles.muted,
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!("  url     {}", response.url),
        styles.url,
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  headers", styles.muted)));
    for (name, value) in &response.headers {
        lines.push(Line::from(vec![
            Span::styled(format!("    {name}: "), styles.muted),
            Span::styled(value.clone(), styles.base),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  body", styles.muted)));
    for line in response.body.lines() {
        lines.push(Line::from(Span::styled(format!("    {line}"), styles.base)));
    }
    let _ = palette;
    lines
}
fn render_footer(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let status_style = match app.status() {
        RunStatus::Idle => styles.status_idle,
        RunStatus::Running => styles.status_running,
        RunStatus::Done { status, .. } => {
            let color = app.theme().palette().response_status(*status);
            styles.base.fg(color)
        }
        RunStatus::Failed(_) => styles.status_error,
    };
    let label = match app.status() {
        RunStatus::Done { status, duration } => {
            format!(" done · {} · {} ms", status, duration.as_millis())
        }
        other => format!(" {}", other.label()),
    };
    let hint = " ↑/↓ move · Enter run · t theme · Tab focus · q quit ";
    let spans = vec![
        Span::styled(label, status_style),
        Span::raw(" "),
        Span::styled(hint, styles.muted),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_running_overlay(frame: &mut Frame, area: Rect, styles: Styles) {
    let overlay = centered(area, 30.min(area.width), 3.min(area.height));
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border);
    let text =
        Paragraph::new(Span::styled("  Sending request…", styles.status_running)).block(block);
    frame.render_widget(text, overlay);
    let _ = Appearance::Dark;
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}
