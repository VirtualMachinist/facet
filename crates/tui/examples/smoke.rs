//! Headless smoke render for the Probe TUI.
//!
//! Renders the TUI into a virtual 120×32 buffer using `ratatui::backend::TestBackend`,
//! dumps every cell to stdout as a text grid, and prints a one-line summary. The
//! fixture is the bundled Pet Store collection at
//! `tests/fixtures/opencollection/phase1-bundled.yml`.
//!
//! Run with:
//!   cargo run -p probe-tui --example smoke

use std::path::PathBuf;

use probe_tui::{App, Appearance, Depth, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use unicode_width::UnicodeWidthChar;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/opencollection/phase1-bundled.yml");
    let mut app = App::load(Some(fixture.as_path())).await;
    // Default to Graphite Honey (the dark flavor) so the smoke render
    // shows the carbon-on-gold selection and accent.
    app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));

    let backend = TestBackend::new(120, 32);
    let mut terminal = Terminal::new(backend).expect("test backend");
    app.render_to(&mut terminal).expect("render");

    let buffer: Buffer = terminal.backend().buffer().clone();
    print_buffer(&buffer);
    println!(
        "--- rendered: {}x{}  rows={}  selection={}  focus={:?}  appearance={:?}",
        buffer.area.width,
        buffer.area.height,
        app.rows().len(),
        app.selection(),
        app.focus(),
        app.theme().appearance(),
    );
}

fn print_buffer(buffer: &Buffer) {
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let cell = &buffer[(x, y)];
            let symbol = cell.symbol();
            if symbol.is_empty() {
                line.push(' ');
            } else {
                for ch in symbol.chars() {
                    if ch.width().unwrap_or(0) == 0 {
                        continue;
                    }
                    line.push(ch);
                }
            }
        }
        println!("{line}");
    }
}
