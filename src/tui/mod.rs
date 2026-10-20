//! Terminal user interface. Placeholder until the screens land.

use anyhow::Result;

use crate::app::AppState;

pub async fn run(_state: AppState) -> Result<()> {
    println!("TUI not available yet. Try `kiln doctor`, `kiln list`, `kiln run <test>`.");
    Ok(())
}
