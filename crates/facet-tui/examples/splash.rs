//! Headless render of the `facet tui` splash (no collection loaded) at
//! 100×30, dumped as a text grid. Pair with `smoke` for the loaded state.
//!
//! Run with:
//!   cargo run -p facet-tui --example splash
//!   cargo run -p facet-tui --example splash -- --appearance porcelain
//!   cargo run -p facet-tui --example splash -- --running --styles
//!
//! `--running` overlays the send-in-flight lattice glyph; `--styles`
//! prints the resolved brand styles so color checks work without a
//! color terminal.

use facet_tui::{App, Appearance, Depth, Styles, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use unicode_width::UnicodeWidthChar;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let mut appearance = Appearance::Dark;
    let mut running = false;
    let mut show_styles = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--appearance" => {
                let value = args.next().expect("--appearance graphite|porcelain");
                appearance =
                    Appearance::from_flag(&value).expect("unknown appearance (graphite|porcelain)");
            }
            "--running" => running = true,
            "--styles" => show_styles = true,
            other => panic!("unexpected argument: {other}"),
        }
    }

    let theme = Theme::new(appearance).with_depth(Depth::Truecolor);
    let mut app = App::load(None).await;
    app.apply_theme(theme);
    if running {
        app.preview_running();
    }

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

    if show_styles {
        let styles = Styles::for_theme(theme);
        println!("--- appearance={:?}", theme.appearance());
        println!("brand        {:?}", styles.brand);
        println!("brand_bright {:?}", styles.brand_bright);
        println!("brand_line   {:?}", styles.brand_line);
        println!("brand_fill   {:?}", styles.brand_fill);
    }
}
