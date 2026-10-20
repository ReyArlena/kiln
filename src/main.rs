use clap::Parser;
use kiln::cli::{Cli, run_cli};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    match run_cli(cli).await {
        Ok(code) => std::process::exit(code),
        Err(err) => {
            eprintln!("kiln: {err:#}");
            std::process::exit(2);
        }
    }
}
