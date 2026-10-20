//! The app: owns every screen, routes input, applies effects, drives the runner.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;
use crossterm::event::{Event as TermEvent, KeyCode};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use tokio::sync::{mpsc, watch};

use crate::app::AppState;
use crate::engine::runner::{CancelScope, RunHandle, start_run};
use crate::engine::{Event, Plan, Run, Sample};

use super::input::{Action, HitMap, map_event};
use super::screens::dashboard::DashboardScreen;
use super::screens::history::HistoryScreen;
use super::screens::picker::PickerScreen;
use super::screens::results::ResultsScreen;
use super::screens::settings::SettingsScreen;
use super::screens::setup::SetupScreen;
use super::screens::{Effect, Screen};
use super::theme;
use super::widgets::{Modal, draw_help, draw_status_bar};

/// How the TUI ended.
pub enum Exit {
    Quit,
    Restart,
}

pub struct App {
    state: AppState,
    screen: Screen,
    picker: PickerScreen,
    setup: SetupScreen,
    dashboard: DashboardScreen,
    results: ResultsScreen,
    history: HistoryScreen,
    settings: SettingsScreen,
    modal: Option<Modal>,
    help: bool,
    hits: HitMap,
    latest_sample: Option<Sample>,
    run: Option<RunHandle>,
    run_dir: PathBuf,
    toast: Option<(String, Instant)>,
    exit: Option<Exit>,
}

impl App {
    pub fn new(state: AppState) -> Self {
        let picker = PickerScreen::new(&state);
        let modal = (!state.is_root()).then(|| {
            Modal::message(
                "Not running as root",
                &format!("Storage tests and some health captures need root.\nThey are greyed out in the list.\n\nTo enable them, quit and relaunch with:\n  {}", state.sudo_hint()),
            )
        });
        Self {
            screen: Screen::Picker,
            picker,
            setup: SetupScreen::empty(),
            dashboard: DashboardScreen::empty(),
            results: ResultsScreen::empty(),
            history: HistoryScreen::new(),
            settings: SettingsScreen::new(),
            modal,
            help: false,
            hits: HitMap::default(),
            latest_sample: None,
            run: None,
            run_dir: PathBuf::new(),
            toast: None,
            exit: None,
            state,
        }
    }

    pub async fn run(
        &mut self,
        terminal: &mut DefaultTerminal,
        mut input: mpsc::UnboundedReceiver<TermEvent>,
    ) -> Result<Exit> {
        // A sampler for the status bar, independent of any run.
        let (sample_tx, mut sample_rx) = mpsc::unbounded_channel::<Event>();
        let (_stop_tx, stop_rx) = watch::channel(false);
        let sampler = self.state.sampler();
        let interval = Duration::from_millis(self.state.config.sample_interval_ms.max(200));
        tokio::spawn(sampler.run(interval, Instant::now(), sample_tx, stop_rx));

        let mut redraw = tokio::time::interval(Duration::from_millis(250));
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if let Some(exit) = self.exit.take() {
                return Ok(exit);
            }
            tokio::select! {
                Some(event) = input.recv() => self.on_terminal_event(event),
                Some(Event::Sample(sample)) = sample_rx.recv() => self.latest_sample = Some(sample),
                event = recv_run_event(&mut self.run) => {
                    match event {
                        Some(event) => self.on_run_event(event).await,
                        None => self.run = None,
                    }
                }
                _ = redraw.tick() => {}
            }
        }
    }

    fn on_terminal_event(&mut self, event: TermEvent) {
        // Text editing wants raw keys, so route those before the action mapping.
        if let TermEvent::Key(key) = &event {
            if let Some(modal) = &mut self.modal {
                if let Some(confirmed) = modal.handle_key(*key) {
                    let tag = modal.tag.clone();
                    self.modal = None;
                    if confirmed {
                        self.on_modal_confirmed(&tag);
                    }
                }
                return;
            }
            if self.help {
                self.help = false;
                return;
            }
            if self.screen == Screen::Setup && self.setup.is_editing() {
                match key.code {
                    KeyCode::Enter => self.setup.commit_edit(),
                    KeyCode::Esc => self.setup.cancel_edit(),
                    _ => self
                        .setup
                        .handle(Action::Key(*key), &self.state, &self.hits)
                        .into_iter()
                        .for_each(|_| {}),
                }
                return;
            }
            if self.screen == Screen::Settings && self.settings.is_editing() {
                let effects = match key.code {
                    KeyCode::Enter => self.settings.commit_edit(&mut self.state),
                    KeyCode::Esc => {
                        self.settings.cancel_edit();
                        Vec::new()
                    }
                    _ => self
                        .settings
                        .handle(Action::Key(*key), &mut self.state, &self.hits),
                };
                self.apply(effects);
                return;
            }
        }
        if self.modal.is_some() || self.help {
            // Mouse and resize while a dialog is open: ignore clicks, keep drawing.
            return;
        }
        let Some(action) = map_event(event) else {
            return;
        };
        self.on_action(action);
    }

    fn on_action(&mut self, action: Action) {
        // Clicking a key hint sends the same action the key would.
        let action = match action {
            Action::Click { x, y } => match self.hits.find(x, y) {
                Some(super::input::Hit::Button(c)) => match c {
                    '\n' => Action::Confirm,
                    ' ' => Action::Toggle,
                    '\x1b' => Action::Back,
                    '\t' => Action::NextPane,
                    'q' => Action::Quit,
                    '?' => Action::Help,
                    c => Action::Char(*c),
                },
                _ => action,
            },
            other => other,
        };
        if action == Action::Help {
            self.help = true;
            return;
        }
        if action == Action::Quit && !(self.screen == Screen::Dashboard && self.run.is_some()) {
            self.exit = Some(Exit::Quit);
            return;
        }
        let effects = match self.screen {
            Screen::Picker => self.picker.handle(action, &self.state, &self.hits),
            Screen::Setup => self.setup.handle(action, &self.state, &self.hits),
            Screen::Dashboard => {
                if self.run.is_none() && matches!(action, Action::Confirm | Action::Back) {
                    vec![Effect::Goto(Screen::Results)]
                } else {
                    self.dashboard.handle(action, &self.hits)
                }
            }
            Screen::Results => self.results.handle(action, &self.hits),
            Screen::History => self.history.handle(action, &self.hits),
            Screen::Settings => self.settings.handle(action, &mut self.state, &self.hits),
        };
        self.apply(effects);
    }

    fn on_modal_confirmed(&mut self, tag: &str) {
        let effects = match tag {
            "start" => vec![self.setup.confirmed_start()],
            "cancel-test" => vec![Effect::CancelTest],
            "cancel-run" => vec![Effect::CancelRun],
            "quit" => {
                if let Some(run) = &self.run {
                    run.cancel(CancelScope::Run);
                }
                vec![Effect::Quit]
            }
            "delete-run" => self.history.delete_current(&self.state),
            "reset-config" => vec![Effect::ResetConfig],
            "delete-runs" => vec![Effect::DeleteAllRuns],
            _ => Vec::new(),
        };
        self.apply(effects);
    }

    async fn on_run_event(&mut self, event: Event) {
        self.dashboard.apply(&event);
        if let Event::RunFinished(run) = event {
            if let Some(handle) = self.run.take() {
                let _ = handle.finish().await;
            }
            Run::prune(&self.state.paths.runs_dir, self.state.config.runs_to_keep);
            let dir = self.run_dir.clone();
            self.toast(if run.all_passed() {
                "run finished: PASS"
            } else {
                "run finished: see results"
            });
            self.results.show(*run, dir, false);
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Goto(screen) => self.goto(screen),
                Effect::Modal(modal) => self.modal = Some(modal),
                Effect::Toast(text) => self.toast(&text),
                Effect::SetupFor(ids) => {
                    self.setup = SetupScreen::new(&self.state, &ids);
                    self.goto(Screen::Setup);
                }
                Effect::StartRun(plan) => self.start_run(plan),
                Effect::CancelTest => {
                    if let Some(run) = &self.run {
                        run.cancel(CancelScope::CurrentTest);
                    }
                }
                Effect::CancelRun => {
                    if let Some(run) = &self.run {
                        run.cancel(CancelScope::Run);
                    }
                }
                Effect::ShowRun {
                    run,
                    dir,
                    from_history,
                } => {
                    self.results.show(*run, dir, from_history);
                    self.goto(Screen::Results);
                }
                Effect::SaveConfig => {
                    if let Err(e) = self.state.save_config() {
                        self.toast(&format!("could not save settings: {e}"));
                    }
                }
                Effect::ResetConfig => match self.state.reset_config() {
                    Ok(()) => self.toast("settings cleared"),
                    Err(e) => self.toast(&format!("could not clear settings: {e}")),
                },
                Effect::DeleteAllRuns => {
                    let dir = self.state.paths.runs_dir.clone();
                    let _ = std::fs::remove_dir_all(&dir);
                    let _ = std::fs::create_dir_all(&dir);
                    self.state.paths.restore_owner(&dir);
                    self.toast("run history deleted");
                }
                Effect::Restart => self.exit = Some(Exit::Restart),
                Effect::Quit => self.exit = Some(Exit::Quit),
            }
        }
    }

    fn goto(&mut self, screen: Screen) {
        match screen {
            Screen::Picker => self.picker.refresh(&self.state),
            Screen::History => self.history.refresh(&self.state),
            _ => {}
        }
        self.screen = screen;
    }

    fn start_run(&mut self, plan: Plan) {
        let ctx = self.state.run_context();
        self.run_dir = ctx.run_dir.clone();
        self.dashboard = DashboardScreen::new(&plan);
        self.run = Some(start_run(plan, ctx));
        self.screen = Screen::Dashboard;
    }

    fn toast(&mut self, text: &str) {
        self.toast = Some((text.to_string(), Instant::now()));
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        self.hits.clear();
        let area = frame.area();
        let show_bar = self.state.config.status_bar.enabled;
        let [bar, body] = Layout::vertical([
            Constraint::Length(if show_bar { 1 } else { 0 }),
            Constraint::Min(3),
        ])
        .areas(area);

        if show_bar {
            // Show the cap next to the CPU reading only while a run is going.
            let cap = self.run.as_ref().and_then(|_| self.dashboard.temp_cap());
            draw_status_bar(
                frame,
                bar,
                self.latest_sample.as_ref(),
                &self.state.config.status_bar,
                cap,
                self.state.is_root(),
                self.state.env.mem_total_bytes,
            );
        }

        match self.screen {
            Screen::Picker => self.picker.draw(frame, body, &self.state, &mut self.hits),
            Screen::Setup => self.setup.draw(frame, body, &mut self.hits),
            Screen::Dashboard => self.dashboard.draw(frame, body, &mut self.hits),
            Screen::Results => self.results.draw(frame, body, &mut self.hits),
            Screen::History => self.history.draw(frame, body, &self.state, &mut self.hits),
            Screen::Settings => self.settings.draw(frame, body, &self.state, &mut self.hits),
        }

        if let Some((text, when)) = &self.toast {
            if when.elapsed() < Duration::from_secs(5) {
                let width = (text.chars().count() as u16 + 2).min(area.width);
                let rect = Rect::new(area.x + area.width - width, body.y, width, 1);
                frame.render_widget(Clear, rect);
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        format!(" {text} "),
                        theme::toast(),
                    ))),
                    rect,
                );
            } else {
                self.toast = None;
            }
        }
        if self.help {
            let keys = match self.screen {
                Screen::Picker => self.picker.key_hints(),
                Screen::Setup => self.setup.key_hints(),
                Screen::Dashboard => self.dashboard.key_hints(),
                Screen::Results => self.results.key_hints(),
                Screen::History => self.history.key_hints(),
                Screen::Settings => self.settings.key_hints(),
            };
            draw_help(frame, area, &keys);
        }
        if let Some(modal) = &self.modal {
            modal.draw(frame, area);
        }
    }
}

/// Await the next runner event, or pend forever when no run is active so the
/// select loop ignores this branch.
async fn recv_run_event(run: &mut Option<RunHandle>) -> Option<Event> {
    match run {
        Some(handle) => handle.events.recv().await,
        None => std::future::pending().await,
    }
}

/// Hooks used by the render tests in `tests/tui_render.rs`.
impl App {
    pub fn draw_for_test(&mut self, frame: &mut ratatui::Frame) {
        self.draw(frame);
    }

    pub fn open_setup_for_test(&mut self, ids: &[String]) {
        self.apply(vec![Effect::SetupFor(ids.to_vec())]);
    }

    pub fn show_screen_for_test(&mut self, name: &str) {
        let screen = match name {
            "setup" => Screen::Setup,
            "dashboard" => Screen::Dashboard,
            "results" => Screen::Results,
            "history" => Screen::History,
            "settings" => Screen::Settings,
            _ => Screen::Picker,
        };
        self.goto(screen);
    }
}
