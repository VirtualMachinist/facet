use std::{env, io, path::PathBuf, process::ExitCode};

use probe_tui::App;
use ratatui::{Terminal, backend::CrosstermBackend};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let workspace_path = args.next().map(PathBuf::from);

    let mut terminal = match setup_terminal() {
        Ok(terminal) => terminal,
        Err(error) => {
            eprintln!("probe-tui: failed to enter raw mode: {error}");
            return ExitCode::from(1);
        }
    };

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("probe-tui: failed to start tokio runtime: {error}");
            let _ = restore_terminal();
            return ExitCode::from(1);
        }
    };

    let exit_code = runtime.block_on(async {
        let app = App::load(workspace_path.as_deref()).await;
        let mut app = app;
        let result = app.run(&mut terminal).await;
        if let Err(error) = result {
            eprintln!("probe-tui: {error}");
            ExitCode::from(1)
        } else {
            ExitCode::SUCCESS
        }
    });

    if let Err(error) = restore_terminal() {
        eprintln!("probe-tui: failed to restore terminal: {error}");
    }
    exit_code
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
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend)
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
