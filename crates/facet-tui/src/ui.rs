//! TUI rendering: title, collection tree, request editor, response
//! pane, and footer status line. All colors come from `Theme`/`Palette`
//! so a single switch on `Appearance` restyles every pane.
//!
//! Layout follows the desktop analog: sidebar on the left; work area on
//! the right splits **vertically** (request over response). Chrome is
//! quiet — pane borders are stone. Accent fill is reserved for the title
//! chip, the tree selection pill, and Send.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::app::{
    App, BodyKind, Focus, Mode, RequestFocus, ResponseTab, ResponseView, RunStatus, Section,
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
    if app.help_open() {
        render_help_overlay(frame, shell[1], styles);
    }
}

fn render_title(frame: &mut Frame, area: Rect, app: &App, styles: Styles) {
    let dirty = if app.editor_dirty() { " •" } else { "" };
    let title_text = match app.collection_name() {
        Some(name) => format!(" facet · {name}{dirty} "),
        None => format!(" facet{dirty} "),
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
    if !app.has_collection() {
        render_pane_title(frame, area, " facet ", focused, styles);
        render_splash(frame, inner, styles);
        return;
    }
    render_pane_title(frame, area, " Request / Response ", focused, styles);

    let request_height = if inner.height >= 22 { 12 } else { 8 };
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(request_height), Constraint::Min(5)])
        .split(inner);

    render_request_pane(frame, split[0], app, styles);
    render_response_pane(frame, split[1], app, styles);
}

/// Release-train codename shown in the footer's first segment.
pub const CODENAME: &str = "G38";
/// Subtitle under the wordmark on the splash.
pub const TAGLINE: &str = "local-first api client";

/// Seven-node lattice glyph from the design alpha: a hexagon of six nodes
/// around a center, spokes to every vertex. Nodes take the bright brand
/// color, lines the accent.
const LATTICE_GLYPH: [&str; 11] = [
    "     ●     ",
    "   ╱ │ ╲   ",
    " ●   │   ● ",
    " │╲  │  ╱│ ",
    " │ ╲ │ ╱ │ ",
    " │   ●   │ ",
    " │ ╱ │ ╲ │ ",
    " │╱  │  ╲│ ",
    " ●   │   ● ",
    "   ╲ │ ╱   ",
    "     ●     ",
];

/// Box-drawing wordmark, three rows, honey.
const WORDMARK: [&str; 3] = [
    "┌─  ┌─┐ ┌─  ┌─┐ ─┬─",
    "├─  ├─┤ │   ├─┘  │ ",
    "│   ┴ ┴ └─  └─┘  ┴ ",
];

/// The lattice glyph inside an outer hexagon outline, 14×23. The hexagon
/// is drawn with ASCII `/`, `\`, `-` so the renderer can style it apart
/// from the glyph: edges become `╱ ╲ ─` in `brand_fill` (the one filled
/// brand frame). Glyph chars keep their node/line styles. Concentric
/// with the glyph: every glyph row clears the outline by at least one
/// cell.
const SPLASH_HEXAGON: [&str; 14] = [
    "         /---\\         ",
    "        /  ●  \\        ",
    "       / ╱ │ ╲ \\       ",
    "      /●   │   ●\\      ",
    "     / │╲  │  ╱│ \\     ",
    "    /  │ ╲ │ ╱ │  \\    ",
    "   /   │   ●   │   \\   ",
    "   \\   │ ╱ │ ╲ │   /   ",
    "    \\  │╱  │  ╲│  /    ",
    "     \\ ●   │   ● /     ",
    "      \\  ╲ │ ╱  /      ",
    "       \\   ●   /       ",
    "        \\     /        ",
    "         \\---/         ",
];

/// Splash for `facet tui` without a collection: lattice glyph, wordmark,
/// tagline, and how to open something. Tall and wide enough panes get the
/// glyph inside the outer hexagon outline. Degrades by height: hexagon
/// first, then the bare glyph, then the wordmark, then a single line.
fn render_splash(frame: &mut Frame, area: Rect, styles: Styles) {
    if area.width < 24 || area.height < 3 {
        let line = Line::from(Span::styled("facet · local-first api client", styles.brand));
        frame.render_widget(
            Paragraph::new(line)
                .alignment(Alignment::Center)
                .style(styles.editor),
            area,
        );
        return;
    }
    let wordmark_height = WORDMARK.len() as u16 + 5;
    let show_hexagon = area.height >= SPLASH_HEXAGON.len() as u16 + wordmark_height
        && area.width >= SPLASH_HEXAGON[0].chars().count() as u16 + 2;
    let show_glyph = area.height >= LATTICE_GLYPH.len() as u16 + wordmark_height;
    let mut lines: Vec<Line> = Vec::new();
    if show_hexagon {
        for row in SPLASH_HEXAGON {
            lines.push(Line::from(splash_art_spans(row, styles)));
        }
        lines.push(Line::raw(""));
    } else if show_glyph {
        for row in LATTICE_GLYPH {
            lines.push(Line::from(splash_art_spans(row, styles)));
        }
        lines.push(Line::raw(""));
    }
    for row in WORDMARK {
        lines.push(Line::from(Span::styled(row, styles.brand)));
    }
    lines.push(Line::from(Span::styled(TAGLINE, styles.muted)));
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "open a collection · facet tui <path>",
        styles.placeholder,
    )));
    let height = (lines.len() as u16).min(area.height);
    let target = centered(area, area.width, height);
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(styles.editor),
        target,
    );
}

/// Styles one row of splash art. ASCII `/ \ -` are stand-ins for the
/// hexagon outline (rendered as `╱ ╲ ─` in the filled brand band); `●`
/// is a lattice node; any other non-space char is a lattice edge.
fn splash_art_spans(row: &str, styles: Styles) -> Vec<Span<'static>> {
    row.chars()
        .map(|ch| match ch {
            '●' => Span::styled(ch.to_string(), styles.brand_bright),
            '/' => Span::styled("╱", styles.brand_fill),
            '\\' => Span::styled("╲", styles.brand_fill),
            '-' => Span::styled("─", styles.brand_fill),
            ' ' => Span::raw(" "),
            _ => Span::styled(ch.to_string(), styles.brand_line),
        })
        .collect()
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
    let insert = app.mode() == Mode::Insert && url_focused;
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
    let insert = app.mode() == Mode::Insert && editor_focused;
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

    // Stone-pill tab row, mirroring the request section tabs.
    let mut tabs = vec![Span::raw("  ")];
    for tab in ResponseTab::ALL {
        let style = if app.response_tab() == tab {
            styles.stone_pill
        } else {
            styles.muted
        };
        tabs.push(Span::styled(format!(" {} ", tab.label()), style));
        tabs.push(Span::raw(" "));
    }
    lines.push(Line::from(tabs));
    lines.push(Line::from(""));

    match app.response_tab() {
        ResponseTab::Pretty => push_body_lines(&mut lines, &response.pretty_body(), styles),
        ResponseTab::Raw => push_body_lines(&mut lines, &response.body, styles),
        ResponseTab::Headers => {
            for (name, value) in &response.headers {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {name}: "), styles.muted),
                    Span::styled(value.clone(), styles.editor),
                ]));
            }
        }
        ResponseTab::Inspect => {
            let content_type = response.content_type().unwrap_or("—");
            let rows = [
                ("status", format!("{} {}", response.status, response.reason)),
                ("url", response.url.clone()),
                ("duration", format!("{} ms", response.duration.as_millis())),
                ("body", format!("{} bytes", response.body_len)),
                ("content-type", content_type.to_string()),
                ("headers", format!("{}", response.headers.len())),
            ];
            for (name, value) in rows {
                lines.push(Line::from(vec![
                    Span::styled(format!("    {name}: "), styles.muted),
                    Span::styled(value, styles.editor),
                ]));
            }
        }
    }
    lines
}

fn push_body_lines<'a>(lines: &mut Vec<Line<'a>>, body: &str, styles: Styles) {
    for line in body.lines() {
        lines.push(Line::from(Span::styled(
            format!("    {line}"),
            styles.editor,
        )));
    }
}

/// Footer from the design alpha: `[ G38 │ lattice ready │ hints │ graphite honey ]`.
/// Brackets in honey, separators in stone. Segments drop from the middle
/// outward when the terminal is narrow.
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
    let status_label = match app.status() {
        RunStatus::Idle if app.lattice_ready() => "lattice ready".to_string(),
        RunStatus::Idle => "lattice idle".to_string(),
        RunStatus::Done { status, duration } => {
            let lattice = match app.last_recording() {
                Some(summary) if summary.run_id.is_some() => " · recorded",
                Some(_) => " · unrecorded",
                None => "",
            };
            format!("{} · {} ms{lattice}", status, duration.as_millis())
        }
        other => other.label().to_string(),
    };
    let mode = app.mode();
    let mode_style = match mode {
        Mode::Normal => styles.muted,
        Mode::Insert | Mode::Command => styles.accent_text,
    };
    let hint = if mode == Mode::Command {
        format!(":{}", app.command_line())
    } else if mode == Mode::Insert {
        "insert · Esc normal · Enter save".to_string()
    } else if app.help_open() {
        "help · Esc close".to_string()
    } else if app.searching() {
        "search · Enter apply · Esc clear".to_string()
    } else if app.env_dropdown_open() {
        "env · j/k select · Enter close".to_string()
    } else {
        "j/k · Enter send · i insert · : command · ? help · q quit".to_string()
    };
    let appearance = app.theme().appearance().label().to_lowercase();

    let mut segments: Vec<(String, Style)> = vec![
        (CODENAME.to_string(), styles.muted),
        (mode.indicator().to_string(), mode_style),
        (status_label, status_style),
        (hint, styles.muted),
        (appearance, styles.muted),
    ];
    let width = |segments: &[(String, Style)]| -> usize {
        // "[ " + segments joined by " │ " + " ]"
        4 + segments.iter().map(|(text, _)| text.width()).sum::<usize>()
            + 3 * segments.len().saturating_sub(1)
    };
    let available = area.width as usize;
    if width(&segments) > available {
        segments.remove(3);
    }
    if width(&segments) > available {
        segments.remove(0);
    }

    let mut spans = vec![Span::styled("[ ", styles.brand)];
    for (index, (text, style)) in segments.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" │ ", styles.border));
        }
        spans.push(Span::styled(text.clone(), *style));
    }
    let used = width(&segments);
    let pad = available.saturating_sub(used);
    spans.push(Span::raw(" ".repeat(pad)));
    spans.push(Span::styled(" ]", styles.brand));
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

/// Send-in-flight overlay: the seven-node lattice glyph over a caption.
/// Falls back to the caption-only box when the pane is too small for the
/// glyph.
fn render_running_overlay(frame: &mut Frame, area: Rect, styles: Styles) {
    // Box fits the glyph (11 wide) and the caption (16 cells) with margin.
    let overlay_width = 20u16;
    let overlay_height = LATTICE_GLYPH.len() as u16 + 4;
    if area.width < overlay_width || area.height < overlay_height {
        let overlay = centered(area, 30.min(area.width), 3.min(area.height));
        frame.render_widget(Clear, overlay);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(styles.border);
        let text =
            Paragraph::new(Span::styled("  Sending request…", styles.status_running)).block(block);
        frame.render_widget(text, overlay);
        return;
    }
    let overlay = centered(area, overlay_width, overlay_height);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border)
        .style(styles.editor);
    let mut lines: Vec<Line> = Vec::new();
    for row in LATTICE_GLYPH {
        lines.push(Line::from(splash_art_spans(row, styles)));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Sending request…",
        styles.status_running,
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        overlay,
    );
}

/// `?` help overlay: the whole keymap in one stone box. Keys in accent,
/// actions in editor text. This is option A's discoverability device —
/// one overlay, no which-key.
fn render_help_overlay(frame: &mut Frame, area: Rect, styles: Styles) {
    const KEYS: [(&str, &str); 14] = [
        ("j/k · arrows", "move (tree, rows, response scroll)"),
        ("Enter", "folder toggle · open · send"),
        ("i", "insert (URL or section) · Esc back"),
        ("/", "search · Enter apply · Esc clear"),
        ("e", "environment dropdown · :env <name>"),
        ("[ ]", "section / response tabs"),
        ("h/l · Space", "collapse folder · name/value"),
        ("m / b", "method · body kind"),
        ("n / d", "add / delete a row"),
        ("t / a", "toggle / force appearance"),
        ("Ctrl-S", "save to disk · :w"),
        (":", "command line (:w :q :wq :theme :env)"),
        ("?", "this help"),
        ("q · Esc", "quit · cancel run"),
    ];
    let width = 60u16.min(area.width);
    let height = (KEYS.len() as u16 + 4).min(area.height);
    let overlay = centered(area, width, height);
    frame.render_widget(Clear, overlay);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(styles.border)
        .style(styles.editor)
        .title(Span::styled(" facet keys ", styles.brand));
    let mut lines = Vec::new();
    for (key, action) in KEYS {
        lines.push(Line::from(vec![
            Span::styled(format!(" {key:<14}"), styles.accent_text),
            Span::styled(action, styles.editor),
        ]));
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        " normal · i inserts · : commands · arrows always work",
        styles.muted,
    )));
    frame.render_widget(Paragraph::new(lines).block(block), overlay);
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
    async fn response_pane_shows_tab_row_and_pretty_json() {
        use crate::app::{ResponseView, RunResult};
        use std::time::Duration;

        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        app.apply_run_result(RunResult::Ok(ResponseView {
            status: 200,
            reason: "OK".to_string(),
            url: "https://example.com/pets".to_string(),
            duration: Duration::from_millis(42),
            headers: vec![("Content-Type".to_string(), "application/json".to_string())],
            body: "{\"id\":1}".to_string(),
            body_len: 8,
        }));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("200 OK"), "status line: {text}");
        assert!(text.contains("Pretty"), "tab row: {text}");
        assert!(text.contains("Raw"), "{text}");
        assert!(text.contains("Headers"), "{text}");
        assert!(text.contains("Inspect"), "{text}");
        // Pretty tab is the default: JSON is indented, not one line.
        assert!(text.contains("\"id\": 1"), "pretty body: {text}");
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

    #[tokio::test]
    async fn splash_renders_when_no_collection_is_loaded() {
        let mut app = App::load(None).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("local-first api client"), "tagline: {text}");
        assert!(text.contains("┌─┐ ┌─  ┌─┐ ─┬─"), "wordmark: {text}");
        assert_eq!(text.matches('●').count(), 7, "seven lattice nodes: {text}");
        assert!(text.contains("facet tui <path>"), "open hint: {text}");
        assert!(!text.contains("Request / Response"), "{text}");
    }

    #[tokio::test]
    async fn splash_wraps_the_glyph_in_the_hexagon_when_tall() {
        let mut app = App::load(None).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("╱───╲"), "hexagon top edge: {text}");
        assert!(text.contains("╲───╱"), "hexagon bottom edge: {text}");
        assert_eq!(
            text.matches('●').count(),
            7,
            "outline adds no nodes: {text}"
        );
        assert!(text.contains("local-first api client"), "{text}");
    }

    #[tokio::test]
    async fn porcelain_splash_renders_the_hexagon() {
        let mut app = App::load(None).await;
        app.apply_theme(Theme::new(Appearance::Light).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("Porcelain Honey"), "{text}");
        assert!(text.contains("╱───╲"), "hexagon in porcelain: {text}");
        assert!(text.contains("local-first api client"), "{text}");
    }

    #[tokio::test]
    async fn running_overlay_shows_the_lattice_glyph() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        app.preview_running();
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("Sending request…"), "caption: {text}");
        assert_eq!(
            text.matches('●').count(),
            7,
            "overlay glyph has seven nodes: {text}"
        );
    }

    #[tokio::test]
    async fn footer_is_the_bracketed_status_bar() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        let footer = text.lines().last().expect("footer row");

        assert!(footer.starts_with("[ G38 │ "), "{footer}");
        assert!(
            footer.contains("lattice idle"),
            "no store beside the fixture: {footer}"
        );
        assert!(footer.contains("│ graphite honey"), "{footer}");
        assert!(footer.trim_end().ends_with(']'), "{footer}");
        assert!(
            footer.contains("q quit"),
            "hints fit at 120 columns: {footer}"
        );
    }

    #[tokio::test]
    async fn footer_shows_the_mode_indicator() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        let footer = text.lines().last().expect("footer row");

        assert!(footer.starts_with("[ G38 │ NOR │ "), "{footer}");
        assert!(
            footer.contains("? help"),
            "normal hints mention help: {footer}"
        );
    }

    #[tokio::test]
    async fn help_overlay_lists_the_keymap() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        app.preview_help();
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());

        assert!(text.contains("facet keys"), "title: {text}");
        assert!(text.contains("command line"), ": row: {text}");
        assert!(
            text.contains("arrows always work"),
            "flat fallback note: {text}"
        );
    }

    #[tokio::test]
    async fn footer_drops_hints_before_codename_when_narrow() {
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        let backend = TestBackend::new(60, 20);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        let footer = text.lines().last().expect("footer row");

        assert!(footer.starts_with("[ G38 │ "), "{footer}");
        assert!(!footer.contains("q quit"), "hints dropped: {footer}");
        assert!(footer.contains("graphite honey"), "{footer}");
    }

    #[tokio::test]
    async fn footer_marks_a_recorded_send() {
        use crate::app::{RecordSummary, ResponseView, RunResult};
        use std::time::Duration;
        let mut app = App::load(Some(&fixture())).await;
        app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));
        app.apply_run_result(RunResult::Ok(ResponseView {
            status: 200,
            reason: "OK".to_string(),
            url: "http://127.0.0.1/".to_string(),
            duration: Duration::from_millis(12),
            headers: Vec::new(),
            body: "{}".to_string(),
            body_len: 2,
        }));
        app.preview_recording(Some(RecordSummary {
            run_id: Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string()),
            note: "recorded".to_string(),
        }));
        let backend = TestBackend::new(120, 32);
        let mut terminal = Terminal::new(backend).expect("backend");
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        let footer = text.lines().last().expect("footer row");
        assert!(footer.contains("200 · 12 ms · recorded"), "{footer}");

        app.preview_recording(Some(RecordSummary {
            run_id: None,
            note: "unrecorded: boom".to_string(),
        }));
        app.render_to(&mut terminal).expect("render");
        let text = dump(terminal.backend().buffer());
        assert!(
            text.lines().last().unwrap().contains("· unrecorded"),
            "{text}"
        );
    }
}
