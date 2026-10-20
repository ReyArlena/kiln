//! One file per screen. Each screen owns its state, draws itself and turns
//! actions into [`Effect`]s that the app applies.

pub mod dashboard;
pub mod history;
pub mod picker;
pub mod results;
pub mod settings;
pub mod setup;

use std::path::PathBuf;

use crate::engine::{Plan, Run};

use super::widgets::Modal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Picker,
    Setup,
    Dashboard,
    Results,
    History,
    Settings,
}

impl Screen {
    pub fn title(self) -> &'static str {
        match self {
            Self::Picker => "Tests",
            Self::Setup => "Setup",
            Self::Dashboard => "Running",
            Self::Results => "Results",
            Self::History => "History",
            Self::Settings => "Settings",
        }
    }
}

/// What a screen asks the app to do after handling an action.
#[derive(Debug)]
pub enum Effect {
    Goto(Screen),
    Modal(Modal),
    Toast(String),
    /// Open setup for these test ids, in order.
    SetupFor(Vec<String>),
    StartRun(Plan),
    CancelTest,
    CancelRun,
    /// Show a finished or past run on the results screen.
    ShowRun {
        run: Box<Run>,
        dir: PathBuf,
        from_history: bool,
    },
    SaveConfig,
    ResetConfig,
    DeleteAllRuns,
    Restart,
    Quit,
}
