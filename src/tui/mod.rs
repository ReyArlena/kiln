//! Terminal user interface built on ratatui.
//!
//! Layout of this module:
//! - `app.rs`      the `App` state machine and event loop glue
//! - `input.rs`    key and mouse mapping into `Action`s
//! - `widgets.rs`  pieces shared by screens: status bar, key hints, modals, text input
//! - `screens/`    one file per screen
//!
//! Keys: arrows and h/j/k/l only ever move focus or scroll. Everything else
//! is a letter or Space/Enter/Esc. Mouse: click to focus or toggle, wheel to scroll.

mod app;
mod input;
mod screens;
mod theme;
mod widgets;

use std::io::stdout;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use tokio::sync::mpsc;

use crate::app::AppState;

pub use app::App;

/// Run the TUI until the user quits. Restores the terminal on every exit path.
pub async fn run(state: AppState) -> Result<()> {
    let mouse = state.config.mouse;
    let mut terminal = ratatui::init();
    if mouse {
        let _ = execute!(stdout(), EnableMouseCapture);
    }

    let result = {
        let (input_tx, input_rx) = mpsc::unbounded_channel();
        spawn_input_thread(input_tx);
        let mut app = App::new(state);
        app.run(&mut terminal, input_rx).await
    };

    if mouse {
        let _ = execute!(stdout(), DisableMouseCapture);
    }
    ratatui::restore();

    match result {
        Ok(app::Exit::Quit) => Ok(()),
        Ok(app::Exit::Restart) => restart(),
        Err(e) => Err(e),
    }
}

/// crossterm's reader blocks, so it gets its own OS thread.
fn spawn_input_thread(tx: mpsc::UnboundedSender<crossterm::event::Event>) {
    std::thread::spawn(move || {
        loop {
            if crossterm::event::poll(Duration::from_millis(100)).unwrap_or(false)
                && let Ok(event) = crossterm::event::read()
                && tx.send(event).is_err()
            {
                return;
            }
            if tx.is_closed() {
                return;
            }
        }
    });
}

/// Replace this process with a fresh copy of itself, same arguments.
fn restart() -> Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    let err = std::process::Command::new(exe).args(args).exec();
    Err(anyhow::anyhow!("restart failed: {err}"))
}
