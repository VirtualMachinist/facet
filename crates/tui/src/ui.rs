//! TUI rendering: title, collection tree, request editor, response
//! pane, and footer status line. All colors come from `Theme`/`Palette`
//! so a single switch on `Appearance` restyles every pane.
//!
//! Layout follows the desktop analog: sidebar on the left; work area on
//! the right splits **vertically** (request over response). Chrome is
//! quiet — pane borders are stone. Accent fill is reserved for the title
//! chip, the tree selection pill, and Send.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};

use crate::app::{
    App, BodyKind, EditorMode, Focus, RequestFocus, ResponseView, RunStatus, Section,
};
use crate::theme::{PaletteToken, Styles, resolve};
use crate::tree::Row;

const SIDEBAR_WIDTH: Constraint = Constraint::Length(36);

/// Top-level entry: paint the whole frame for `app`.
pub fn draw(frame: &mut Frame, app: &App) {
    let styles = Styles::for_theme(app.theme());
    let area = frame.area();
    frame.render_widget(Block::default().style(styles.base), area);

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

    if app.env_dropdown_open() {
        render_env_dropdown(frame, shell[1], app, styles);
    }
    if matches!(app.status(), RunStatus::Running) {
        render_running_overlay(frame, shell[1], styles);
    }
}

fn render_title(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let dirty = if app.editor_dirty() { " •" } else { "" };
    let title_text = match app.collection_name() {
        Some(name) => format!(" probe-tui · {name}{dirty} "),
        None => format!(" probe-tui{dirty} "),
    };
    let appearance = app.theme().appearance().label();
    let line = Line::from(vec![
        Span::styled(title_text, styles.title),
        Span::raw(" "),
        Span::styled(appearance, styles.muted),
    ]);
    frame.render_widget(Paragraph::new(line).style(styles.base), area);
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
        .border_style(if focused {
            styles.title_focused
        } else {
            styles.border
        })
        .style(styles.sidebar);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_pane_title(frame, area, " Collection ", focused, styles);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    render_search(frame, rows[0], app, styles);
    render_tree(frame, rows[1], app, styles);
}

fn render_search(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    if area.width == 0 {
        return;
    }
    let query = app.search();
    let line = if app.searching() {
        Line::from(vec![
            Span::styled(" /", styles.accent_text),
            Span::styled(query.to_string(), styles.sidebar),
            Span::styled("▌", styles.accent_text),
        ])
    } else if query.is_empty() {
        Line::from(Span::styled(" Search requests…", styles.placeholder))
    } else {
        Line::from(vec![
            Span::styled(" /", styles.muted),
            Span::styled(query.to_string(), styles.sidebar),
        ])
    };
    frame.render_widget(Paragraph::new(line).style(styles.sidebar), area);
}

fn render_work_area(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let focused = matches!(app.focus(), Focus::Request | Focus::Response);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(if focused {
            styles.title_focused
        } else {
            styles.border
        })
        .style(styles.editor);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    render_pane_title(frame, area, " Request / Response ", focused, styles);

    let request_height = if inner.height >= 22 { 12 } else { 8 };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(request_height), Constraint::Min(5)])
        .split(inner);

    render_request_pane(frame, split[0], app, styles);
    render_response_pane(frame, split[1], app, styles);
}

fn render_pane_title(frame: &mut Frame, area: Rect, label: &str, focused: bool, styles: Styles) {
    let marker = if focused { "▸ " } else { "  " };
    let style = if focused {
        styles.title_focused
    } else {
        styles.muted
    };
    let paragraph = Paragraph::new(Line::from(vec![
        Span::raw(marker),
        Span::styled(label, style),
    ]));
    let title_area = Rect {
        x: area.x + 2,
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
        let empty = if app.has_search() {
            "  no matching requests"
        } else {
            "  Create or open a collection…"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(empty, styles.muted))).style(styles.sidebar),
            area,
        );
        return;
    }
    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| row_item(*row, index, app.selection(), app, styles))
        .collect();
    let list = List::new(items)
        .style(styles.sidebar)
        .highlight_style(styles.selected_row)
        .highlight_symbol("  ");
    let mut state = ratatui::widgets::ListState::default();
    state.select(Some(app.selection()));
    frame.render_stateful_widget(list, area, &mut state);
}

fn row_item(row: Row, index: usize, selection: usize, app: &App, styles: Styles) -> ListItem<'_> {
    let selected = index == selection;
    let (depth, prefix, method_label, name) = match row {
        Row::Folder {
            depth,
            key,
            request_count,
        } => {
            let name = app
                .folder_name(key)
                .unwrap_or_else(|| "<folder>".to_string());
            let collapsed = app.tree_is_collapsed(key);
            let prefix = if collapsed { "▸ " } else { "▾ " };
            (
                depth,
                prefix,
                String::new(),
                format!("{name} ({request_count})"),
            )
        }
        Row::Request { depth, key } => {
            let (name, method) = app
                .request_name_and_method(key)
                .unwrap_or_else(|| ("<request>".to_string(), "GET".to_string()));
            (depth, "  ", method_label(&method), name)
        }
    };
    let indent = "  ".repeat(depth as usize);
    let method_cell = if method_label.is_empty() {
        "    ".to_string()
    } else {
        format!("{:<4}", method_label)
    };
    let line = if selected {
        // Filled honey pill: method cell inverts with the row — desktop
        // suppresses domain color on the selected tree row.
        Line::from(vec![
            Span::styled(indent, styles.selected_row),
            Span::styled(prefix, styles.selected_row),
            Span::styled(method_cell, styles.selected_row),
            Span::styled(" ", styles.selected_row),
            Span::styled(name, styles.selected_row),
        ])
    } else {
        let method_style = if method_label.is_empty() {
            styles.sidebar
        } else {
            styles
                .method
                .fg(resolve(app.theme(), method_token(&method_label)))
        };
        Line::from(vec![
            Span::styled(indent, styles.sidebar),
            Span::styled(prefix, styles.sidebar),
            Span::styled(method_cell, method_style),
            Span::styled(" ", styles.sidebar),
            Span::styled(name, styles.sidebar),
        ])
    };
    ListItem::new(line)
}

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
    if area.height == 0 {
        return;
    }
    let show_kinds = app.section() == Section::Body && area.height >= 6;
    let mut constraints = vec![
        Constraint::Length(1), // breadcrumb
        Constraint::Length(1), // url + send
        Constraint::Length(1), // section tabs + env
    ];
    if show_kinds {
        constraints.push(Constraint::Length(1));
    }
    constraints.push(Constraint::Min(1));
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    render_breadcrumb(frame, rows[0], app, styles);
    render_url_bar(frame, rows[1], app, styles);
    render_section_tabs(frame, rows[2], app, styles);
    let editor_index = if show_kinds {
        render_body_kinds(frame, rows[3], app, styles);
        4
    } else {
        3
    };
    render_section_editor(frame, rows[editor_index], app, styles);
}

fn render_breadcrumb(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let text = match app.selected_request() {
        Some((key, _)) => {
            let ancestors = app.ancestor_chain(key);
            if ancestors.is_empty() {
                " Collection".to_string()
            } else {
                format!(" Collection › {ancestors}")
            }
        }
        None => " no request selected".to_string(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, styles.muted))).style(styles.editor),
        area,
    );
}

fn render_url_bar(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let send_width = 8;
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(8),
            Constraint::Length(send_width.min(area.width)),
        ])
        .split(area);

    let method = app.method();
    let method_color = app.theme().palette().method(method);
    let url_focused = app.focus() == Focus::Request && app.request_focus() == RequestFocus::Url;
    let insert = app.editor_mode() == EditorMode::Insert && url_focused;
    let mut url = app.editor().url.clone();
    if insert {
        url.push('▌');
    }
    let url_style = if url_focused {
        styles
            .url
            .add_modifier(ratatui::style::Modifier::UNDERLINED)
    } else {
        styles.url
    };
    let url_line = Line::from(vec![
        Span::styled(format!(" {:<4} ", method), styles.method.fg(method_color)),
        Span::styled("▾ ", styles.muted),
        Span::styled(url, url_style),
    ]);
    frame.render_widget(Paragraph::new(url_line).style(styles.editor), split[0]);

    let send_label = match app.status() {
        RunStatus::Running => " Cancel ",
        _ => "  Send  ",
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(send_label, styles.title))),
        split[1],
    );
}

fn render_section_tabs(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let env_label = match app.active_environment_name() {
        Some(name) => format!(" {name} ▾ "),
        None => " No env ▾ ".to_string(),
    };
    let env_width = env_label.chars().count() as u16;
    let split = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Min(10),
            Constraint::Length(env_width.min(area.width)),
        ])
        .split(area);

    let editor = app.editor();
    let mut spans = vec![Span::raw(" ")];
    for section in Section::ALL {
        let count = match section {
            Section::Path => editor.path_rows.len(),
            Section::Query => editor.query_rows.len(),
            Section::Headers => editor.header_rows.len(),
            Section::Body => {
                if editor.body_kind == BodyKind::None && editor.body_text.is_empty() {
                    0
                } else {
                    1
                }
            }
            Section::Auth => usize::from(editor.auth_label != "No authentication"),
        };
        let label = if matches!(section, Section::Body | Section::Auth) && count == 0 {
            format!(" {} ", section.label())
        } else {
            format!(" {} {count} ", section.label())
        };
        let selected = app.section() == section;
        let style = if selected {
            styles.stone_pill
        } else {
            styles.muted
        };
        spans.push(Span::styled(label, style));
        spans.push(Span::raw(" "));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(styles.editor),
        split[0],
    );

    let env_style = if app.env_dropdown_open() {
        styles.accent_text
    } else {
        styles.muted
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(env_label, env_style))).style(styles.editor),
        split[1],
    );
}

fn render_body_kinds(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let mut spans = vec![Span::raw(" ")];
    for kind in BodyKind::ALL {
        let label = format!(" {} ", kind.label());
        let selected = app.editor().body_kind == kind;
        let style = if selected {
            styles
                .accent_text
                .add_modifier(ratatui::style::Modifier::UNDERLINED)
        } else {
            styles.muted
        };
        spans.push(Span::styled(label, style));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(styles.editor), area);
}

fn render_section_editor(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    if area.height == 0 {
        return;
    }
    let editor_focused =
        app.focus() == Focus::Request && app.request_focus() == RequestFocus::Editor;
    let insert = app.editor_mode() == EditorMode::Insert && editor_focused;
    let lines = match app.section() {
        Section::Path => kv_lines(
            &app.editor().path_rows,
            app.kv_index(),
            app.kv_on_value(),
            editor_focused,
            insert,
            styles,
        ),
        Section::Query => kv_lines(
            &app.editor().query_rows,
            app.kv_index(),
            app.kv_on_value(),
            editor_focused,
            insert,
            styles,
        ),
        Section::Headers => kv_lines(
            &app.editor().header_rows,
            app.kv_index(),
            app.kv_on_value(),
            editor_focused,
            insert,
            styles,
        ),
        Section::Body => body_lines(app, insert, styles),
        Section::Auth => vec![Line::from(Span::styled(
            format!("  {}", app.editor().auth_label),
            styles.muted,
        ))],
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(styles.editor)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn kv_lines(
    rows: &[(String, String)],
    index: usize,
    on_value: bool,
    focused: bool,
    insert: bool,
    styles: Styles,
) -> Vec<Line<'static>> {
    if rows.is_empty() {
        return vec![Line::from(Span::styled(
            "  empty · n add row · i edit",
            styles.placeholder,
        ))];
    }
    rows.iter()
        .enumerate()
        .map(|(row_index, (name, value))| {
            let active = focused && row_index == index;
            let mut name = name.clone();
            let mut value = value.clone();
            if active && insert {
                if on_value {
                    value.push('▌');
                } else {
                    name.push('▌');
                }
            }
            let name_style = if active && !on_value {
                styles.stone_pill
            } else {
                styles.muted
            };
            let value_style = if active && on_value {
                styles.stone_pill
            } else {
                styles.editor
            };
            Line::from(vec![
                Span::styled("  ", styles.editor),
                Span::styled(format!("{name:<16}"), name_style),
                Span::styled("  ", styles.editor),
                Span::styled(value, value_style),
            ])
        })
        .collect()
}

fn body_lines(app: &App, insert: bool, styles: Styles) -> Vec<Line<'static>> {
    match app.editor().body_kind {
        BodyKind::Form | BodyKind::Multipart | BodyKind::File => {
            vec![Line::from(Span::styled(
                "  this body kind is not editable in the TUI yet",
                styles.placeholder,
            ))]
        }
        BodyKind::None if app.editor().body_text.is_empty() => {
            vec![Line::from(Span::styled(
                "  no body · b cycle kind · i edit",
                styles.placeholder,
            ))]
        }
        _ => {
            let mut text = app.editor().body_text.clone();
            if insert {
                text.push('▌');
            }
            if text.is_empty() {
                vec![Line::from(Span::styled("  ", styles.editor))]
            } else {
                text.lines()
                    .map(|line| Line::from(Span::styled(format!("  {line}"), styles.editor)))
                    .collect()
            }
        }
    }
}

fn render_response_pane(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    if area.height == 0 {
        return;
    }
    let focused = app.focus() == Focus::Response;
    let header_style = if focused {
        styles.title_focused
    } else {
        styles.muted
    };
    let lines: Vec<Line> = match app.response() {
        Some(response) => response_lines(response, app, styles, header_style),
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
    frame.render_widget(
        Paragraph::new(lines)
            .style(styles.editor)
            .scroll((app.response_scroll(), 0))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn response_lines<'a>(
    response: &'a ResponseView,
    app: &'a App,
    styles: Styles,
    header_style: ratatui::style::Style,
) -> Vec<Line<'a>> {
    let palette = app.theme().palette();
    let status_color = palette.response_status(response.status);
    let mut lines = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  Response  ", header_style),
        Span::styled(
            format!("{} {}", response.status, response.reason),
            styles.base.fg(status_color),
        ),
        Span::styled(
            format!("  · {} ms", response.duration.as_millis()),
            styles.muted,
        ),
    ]));
    lines.push(Line::from(Span::styled(
        format!("  {}", response.url),
        styles.url,
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  headers", styles.muted)));
    for (name, value) in &response.headers {
        lines.push(Line::from(vec![
            Span::styled(format!("    {name}: "), styles.muted),
            Span::styled(value.clone(), styles.editor),
        ]));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled("  body", styles.muted)));
    for line in response.body.lines() {
        lines.push(Line::from(Span::styled(
            format!("    {line}"),
            styles.editor,
        )));
    }
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
    let hint = if app.editor_mode() == EditorMode::Insert {
        " insert · Esc normal · Enter save "
    } else if app.searching() {
        " search · Enter apply · Esc clear "
    } else if app.env_dropdown_open() {
        " env · j/k select · Enter close "
    } else {
        " j/k move · Enter send · i edit · / search · e env · [ ] tabs · t theme · q quit "
    };
    let spans = vec![
        Span::styled(label, status_style),
        Span::raw(" "),
        Span::styled(hint, styles.muted),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)).style(styles.base), area);
}

fn render_env_dropdown(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let names: Vec<String> = std::iter::once("No environment".to_string())
        .chain(app.environments().iter().map(|entry| {
            let inherit = entry
                .inherits_from
                .as_deref()
                .map(|parent| format!(" ← {parent}"))
                .unwrap_or_default();
            format!("{}{inherit}  ({})", entry.name, entry.variable_count)
        }))
        .collect();
    let width = names
        .iter()
        .map(|name| name.chars().count() as u16)
        .max()
        .unwrap_or(16)
        .saturating_add(4)
        .min(area.width.saturating_sub(2))
        .max(18);
    let height = (names.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let overlay = Rect {
        x: area.x + area.width.saturating_sub(width).saturating_sub(2),
        y: area.y.saturating_add(2),
        width,
        height,
    };
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border)
        .style(styles.editor)
        .title(" env ");
    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    let selected = app.active_environment().map(|index| index + 1).unwrap_or(0);
    let items: Vec<ListItem> = names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            let active = index == selected;
            let style = if active {
                styles.accent_text
            } else {
                styles.editor
            };
            let prefix = if active { " › " } else { "   " };
            ListItem::new(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(name, style),
            ]))
        })
        .collect();
    frame.render_widget(List::new(items).style(styles.editor), inner);
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
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Appearance, Depth, Theme};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/fixtures/opencollection/phase1-bundled.yml")
    }

    fn dump(buffer: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[tokio::test]
    async fn graphite_render_includes_ship_chrome() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("Search requests"), "search row: {text}");
        assert!(text.contains("Path"), "section tabs: {text}");
        assert!(text.contains("Query"), "{text}");
        assert!(text.contains("Headers"), "{text}");
        assert!(text.contains("Body"), "{text}");
        assert!(text.contains("Auth"), "{text}");
        assert!(text.contains("Send"), "{text}");
        assert!(text.contains("No env"), "{text}");
        assert!(text.contains("Graphite Honey"), "{text}");
        assert!(
            !text.contains('╔') && !text.contains('╗'),
            "no double-line boxes"
        );
        assert!(
            text.contains("List pets") || text.contains("Pets"),
            "{text}"
        );
    }

    #[tokio::test]
    async fn porcelain_render_uses_light_label() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Light).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        assert!(text.contains("Porcelain Honey"), "{text}");
        assert!(text.contains("Send"), "{text}");
    }
}
