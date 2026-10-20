//! Application configuration: where files live and what the user has set.

mod paths;
mod settings;

pub use paths::Paths;
pub use settings::{AppConfig, SensorChoice, StatusBarConfig};
