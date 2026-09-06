//! Headless render of the `facet tui` splash (no collection loaded) at
//! 100×30, dumped as a text grid. Pair with `smoke` for the loaded state.
//!
//! Run with:
//!   cargo run -p facet-tui --example splash

use facet_tui::{App, Appearance, Depth, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use unicode_width::UnicodeWidthChar;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut app = App::load(None).await;
    app.apply_theme(Theme::new(Appearance::Dark).with_depth(Depth::Truecolor));

    let backend = TestBackend::new(100, 30);
    let mut terminal = Terminal::new(backend).expect("test backend");
    app.render_to(&mut terminal).expect("render");

    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let mut line = String::new();
        for x in 0..buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            if symbol.is_empty() {
                line.push(' ');
            } else {
                line.extend(symbol.chars().filter(|ch| ch.width().unwrap_or(0) > 0));
            }
        }
        println!("{}", line.trim_end());
    }
}
