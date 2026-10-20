use clap::Parser;
use kiln::cli::{Cli, run_cli};

#[tokio::main]
async fn main() {
    reset_sigpipe();
    let cli = Cli::parse();
    match run_cli(cli).await {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("kiln: {err:#}");
            std::process::exit(2);
        }
    }
}

/// Rust ignores SIGPIPE, so `kiln log | head` would panic on a closed pipe.
/// Restore the default so the process just exits quietly like other CLI tools.
fn reset_sigpipe() {
    use nix::sys::signal::{SigHandler, Signal, signal};
    // SAFETY: setting a signal disposition before any other thread exists.
    unsafe {
        let _ = signal(Signal::SIGPIPE, SigHandler::SigDfl);
    }
}
