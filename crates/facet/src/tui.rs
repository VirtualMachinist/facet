//! `facet tui`: the ratatui interface from `facet-tui` (Surface 2).
//! Interactive, so it bypasses the captured `RunOutput` path and owns the
//! terminal directly.

use std::{io, path::PathBuf};

use facet_tui::{App, Appearance, Depth, Theme};
use ratatui::{Terminal, backend::CrosstermBackend};

const HELP: &str = "Usage: facet tui [--appearance graphite|porcelain] [<path>]\n\
\n\
Graphite Honey is the default. Porcelain Honey is the light appearance.\n\
Keys: j/k move · Enter send · i edit · / search · e env · t theme · q quit\n";

struct Cli {
    appearance: Option<Appearance>,
    path: Option<PathBuf>,
}

/// Runs the terminal UI and returns the process exit code.
#[must_use]
pub fn run_tui(args: &[String]) -> u8 {
    let cli = match parse_args(args) {
        Ok(cli) => cli,
        Err(message) if message == "help" => {
            print!("{HELP}");
            return 0;
        }
        Err(message) => {
            eprintln!("error[invalid_arguments]: {message}");
            eprint!("{HELP}");
            return crate::INVALID_ARGUMENTS_EXIT_CODE;
        }
    };

    let mut terminal = match setup_terminal() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("error[terminal_error]: failed to enter raw mode: {error}");
            return 1;
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = restore_terminal();
            eprintln!("error[runtime_error]: failed to start tokio runtime: {error}");
            return 1;
        }
    };

    let exit_code = runtime.block_on(async {
        let mut app = App::load(cli.path.as_deref()).await;
        if let Some(appearance) = cli.appearance {
            app.apply_theme(Theme::new(appearance).with_depth(Depth::from_env()));
        }
        match app.run(&mut terminal).await {
            Ok(()) => 0,
            Err(error) => {
                eprintln!("error[tui_error]: {error}");
                1
            }
        }
    });

    if let Err(error) = restore_terminal() {
        eprintln!("warning: failed to restore terminal: {error}");
    }
    exit_code
}

fn parse_args(args: &[String]) -> Result<Cli, String> {
    let mut appearance = None;
    let mut path = None;
    let mut iter = args.iter();
    while let Some(argument) = iter.next() {
        match argument.as_str() {
            "-h" | "--help" => return Err("help".to_owned()),
            "--appearance" => {
                let value = iter
                    .next()
                    .ok_or_else(|| "--appearance requires graphite or porcelain".to_owned())?;
                appearance = Some(parse_appearance(value)?);
            }
            other => {
                if let Some(value) = other.strip_prefix("--appearance=") {
                    appearance = Some(parse_appearance(value)?);
                } else if path.is_none() && !other.starts_with('-') {
                    path = Some(PathBuf::from(other));
                } else {
                    return Err(format!("unexpected argument: {other}"));
                }
            }
        }
    }
    Ok(Cli { appearance, path })
}

fn parse_appearance(value: &str) -> Result<Appearance, String> {
    Appearance::from_flag(value).ok_or_else(|| format!("unknown appearance {value:?}"))
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<io::Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal() -> io::Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture,
        crossterm::cursor::Show
    )?;
    Ok(())
}
