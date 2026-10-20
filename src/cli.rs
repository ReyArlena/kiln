//! Command-line front end. `kiln` with no subcommand opens the TUI; the
//! rest is a headless interface meant for SSH sessions and scripts.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};

use crate::app::AppState;
use crate::engine::params::ParamValue;
use crate::engine::plan::Profile;
use crate::engine::runner::start_run;
use crate::engine::util::{format_bytes, format_duration_secs};
use crate::engine::{DepReport, Event, ParamSet, Plan, PlanEntry, Run};
use crate::report;

#[derive(Parser, Debug)]
#[command(
    name = "kiln",
    version,
    about = "Hardware burn-in and diagnostics runner"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Open the interactive interface (default)
    Tui,
    /// Check which tools and permissions are available
    Doctor,
    /// List tests
    List {
        /// Show the full description of every test
        #[arg(long)]
        full: bool,
    },
    /// Show everything about one test: description, parameters, dependencies, profiles
    Info { test: String },
    /// Run tests headlessly, in order, and write results to the runs directory
    Run(RunArgs),
    /// List past runs, newest first
    Runs,
    /// Print a Markdown report for a run (id, or path to run.json)
    Report { run: String },
    /// Print the log of one test from a past run
    Log {
        run: String,
        /// Test index within the run (1-based). Defaults to the last test
        index: Option<usize>,
    },
    /// Manage saved profiles (parameter sets) for a test
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Show or reset settings
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Test ids to run in order, e.g. `cpu-stress ram-hash`
    pub tests: Vec<String>,
    /// Run a saved plan file instead of listing tests
    #[arg(long, value_name = "FILE", conflicts_with = "tests")]
    pub plan: Option<PathBuf>,
    /// Parameter override, `key=value`. Applies to every listed test that has that key
    #[arg(long = "set", value_name = "KEY=VALUE")]
    pub sets: Vec<String>,
    /// Use a saved profile (by name) for every listed test that has one
    #[arg(long)]
    pub profile: Option<String>,
    /// Temperature cap in Celsius. `off` disables it
    #[arg(long, value_name = "C")]
    pub temp_max: Option<String>,
    /// Per-test time cap, e.g. 30m
    #[arg(long, value_name = "DURATION")]
    pub time_max: Option<String>,
    /// Seconds to wait between tests
    #[arg(long)]
    pub cooldown: Option<u64>,
    /// Keep going after a failed test
    #[arg(long)]
    pub keep_going: bool,
    /// Repeat each listed test this many times
    #[arg(long, default_value_t = 1)]
    pub repeat: u32,
    /// Required to run destructive tests: confirms the device may be wiped
    #[arg(long)]
    pub yes_destroy: bool,
    /// Print every output line rather than a summary every few seconds
    #[arg(short, long)]
    pub verbose: bool,
}

#[derive(Subcommand, Debug)]
pub enum ProfileAction {
    /// List saved profiles for a test
    List { test: String },
    /// Save a profile from `--set` values
    Save {
        test: String,
        name: String,
        #[arg(long = "set", value_name = "KEY=VALUE")]
        sets: Vec<String>,
        #[arg(long, default_value = "")]
        description: String,
    },
    /// Delete a saved profile
    Delete { test: String, name: String },
}

#[derive(Subcommand, Debug)]
pub enum ConfigAction {
    /// Print where files live
    Path,
    /// Print the effective settings as TOML
    Show,
    /// Delete the settings file and return to defaults
    Reset,
}

/// Entry point. Returns the process exit code.
pub async fn run_cli(cli: Cli) -> Result<i32> {
    let mut state = AppState::load()?;
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => {
            crate::tui::run(state).await?;
            Ok(0)
        }
        Command::Doctor => doctor(&state),
        Command::List { full } => list(&state, full),
        Command::Info { test } => info(&state, &test),
        Command::Run(args) => run(&state, args).await,
        Command::Runs => runs(&state),
        Command::Report { run } => {
            let run = load_run(&state, &run)?;
            print!("{}", report::markdown(&run));
            Ok(0)
        }
        Command::Log { run, index } => log(&state, &run, index),
        Command::Profile { action } => profile(&state, action),
        Command::Config { action } => config(&mut state, action),
    }
}

fn doctor(state: &AppState) -> Result<i32> {
    println!("kiln {}\n", env!("CARGO_PKG_VERSION"));
    for line in state.env.summary_lines() {
        println!("{line}");
    }
    println!();
    if !state.is_root() {
        println!(
            "WARNING: not running as root. Tests marked [root] cannot run. Relaunch with: sudo kiln\n"
        );
    }
    let sampler = state.sampler();
    match &sampler.primary {
        Some(s) => println!(
            "Guard sensor: {} ({})",
            s.id(),
            s.read_c()
                .map(|c| format!("{c:.1}C"))
                .unwrap_or_else(|| "unreadable".into())
        ),
        None => println!("Guard sensor: none found (temperature cap will not work)"),
    }
    println!("Sensors found: {}", sampler.sensors.len());
    println!("\nTests:");
    let mut all_ok = true;
    for test in state.registry.iter() {
        let info = test.info();
        let report = DepReport::check(&test.deps());
        let mark = if report.all_ok() { "ok " } else { "-- " };
        all_ok &= report.all_ok();
        println!("  {mark}{:<16} {}", info.id, info.name);
        for missing in report.missing() {
            println!(
                "       missing {:<12} {}",
                missing.dep.label(),
                missing.hint.clone().unwrap_or_default()
            );
        }
    }
    println!("\nConfig: {}", state.paths.config_file.display());
    println!("Runs:   {}", state.paths.runs_dir.display());
    Ok(if all_ok { 0 } else { 1 })
}

fn list(state: &AppState, full: bool) -> Result<i32> {
    for test in state.registry.iter() {
        let info = test.info();
        let flags = if info.destructive {
            " [destructive]"
        } else {
            ""
        };
        println!("{:<16} {}{flags}", info.id, info.name);
        println!("{:<16} {}", "", info.tldr);
        if full {
            println!();
            for line in info.description.lines() {
                println!("    {line}");
            }
        }
        println!();
    }
    Ok(0)
}

fn info(state: &AppState, id: &str) -> Result<i32> {
    let test = state
        .registry
        .get(id)
        .with_context(|| format!("unknown test `{id}`. Try `kiln list`"))?;
    let info = test.info();
    println!("{} ({})", info.name, info.id);
    println!(
        "Category: {}{}\n",
        info.category.label(),
        if info.destructive {
            "   DESTRUCTIVE"
        } else {
            ""
        }
    );
    println!("{}\n", info.tldr);
    println!("{}\n", info.description);
    println!("Uses:");
    for u in info.uses {
        println!("  - {u}");
    }
    println!("\nDependencies:");
    let report = DepReport::check(&test.deps());
    if report.statuses.is_empty() {
        println!("  none");
    }
    for s in &report.statuses {
        println!(
            "  {} {}{}",
            if s.ok { "ok " } else { "-- " },
            s.dep.label(),
            s.hint
                .as_ref()
                .map(|h| format!("  ({h})"))
                .unwrap_or_default()
        );
    }
    println!("\nParameters:");
    let params = test.params();
    if params.is_empty() {
        println!("  none");
    }
    for p in &params {
        let default = p
            .default_value()
            .map(|v| v.to_display())
            .unwrap_or_else(|| "(required)".into());
        println!("  {:<16} {:<12} {}", p.key, default, p.help);
    }
    let profiles = Profile::list(&state.paths.profiles_for(id));
    if !profiles.is_empty() {
        println!("\nProfiles:");
        for p in profiles {
            let values: Vec<String> = p
                .params
                .iter()
                .map(|(k, v)| format!("{k}={}", v.to_display()))
                .collect();
            println!("  {:<16} {}  {}", p.name, values.join(" "), p.description);
        }
    }
    Ok(0)
}

async fn run(state: &AppState, args: RunArgs) -> Result<i32> {
    let plan = match &args.plan {
        Some(path) => Plan::load(path)?,
        None => plan_from_args(state, &args)?,
    };
    if plan.tests.is_empty() {
        bail!("nothing to run. Give test ids, e.g. `kiln run cpu-stress`, or `--plan file.toml`");
    }

    // Refuse early rather than skip halfway through a sequence.
    let mut destructive = Vec::new();
    let mut needs_root = false;
    for entry in &plan.tests {
        let test = state
            .registry
            .get(&entry.test)
            .with_context(|| format!("unknown test `{}`", entry.test))?;
        let report = DepReport::check(&test.deps());
        needs_root |= report.needs_root();
        if test.info().destructive {
            let device = entry
                .params
                .get("device")
                .map(|v| v.to_text())
                .unwrap_or_else(|| "(device not set)".into());
            destructive.push(format!("{} on {device}", entry.test));
        }
    }
    if needs_root {
        bail!("some of these tests need root. Relaunch with: sudo kiln run ...");
    }
    if !destructive.is_empty() && !args.yes_destroy {
        eprintln!("These tests will WIPE a device:");
        for d in &destructive {
            eprintln!("  {d}");
        }
        bail!("add --yes-destroy to confirm");
    }

    println!(
        "kiln {}  plan `{}`  {} test(s)",
        env!("CARGO_PKG_VERSION"),
        plan.name,
        plan.tests.len()
    );
    for line in state.env.summary_lines().iter().take(4) {
        println!("  {line}");
    }
    let g = &plan.guards;
    println!(
        "  guards: temp {}  time {}  cooldown {}s  stop_on_fail {}",
        g.temp_max_c
            .map(|c| format!("{c:.0}C"))
            .unwrap_or_else(|| "off".into()),
        g.time_max_secs
            .map(format_duration_secs)
            .unwrap_or_else(|| "off".into()),
        g.cooldown_secs,
        g.stop_on_fail
    );

    let ctx = state.run_context();
    let run_dir = ctx.run_dir.clone();
    println!("  output: {}\n", run_dir.display());
    let mut handle = start_run(plan, ctx);

    // Ctrl-C stops the current test; a second Ctrl-C stops the whole run.
    let ctrl_c_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut sample_count = 0usize;
    loop {
        tokio::select! {
            event = handle.events.recv() => {
                let Some(event) = event else { break };
                print_event(&event, args.verbose, &mut sample_count);
                if matches!(event, Event::RunFinished(_)) {
                    break;
                }
            }
            _ = tokio::signal::ctrl_c() => {
                let n = ctrl_c_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    eprintln!("\nstopping current test (Ctrl-C again to stop the run)");
                    handle.cancel(crate::engine::runner::CancelScope::CurrentTest);
                } else {
                    eprintln!("\nstopping run");
                    handle.cancel(crate::engine::runner::CancelScope::Run);
                }
            }
        }
    }
    let run = handle.finish().await?;
    Run::prune(&state.paths.runs_dir, state.config.runs_to_keep);

    println!("\n{}", summary_table(&run));
    println!("report: kiln report {}", run.id);
    Ok(run.exit_code())
}

fn print_event(event: &Event, verbose: bool, sample_count: &mut usize) {
    let stamp = chrono::Local::now().format("%H:%M:%S");
    match event {
        Event::TestStarted {
            index,
            name,
            expected_secs,
            ..
        } => {
            let expect = expected_secs
                .map(|s| format!(" (~{})", format_duration_secs(s)))
                .unwrap_or_default();
            println!("[{stamp}] ### {}: {name}{expect}", index + 1);
        }
        Event::Line { text, .. } => {
            if verbose
                || text.starts_with("KILN_METRIC")
                || text.contains("fail")
                || text.contains("error")
                || text.starts_with("kiln:")
            {
                println!("[{stamp}]   {text}");
            }
        }
        Event::Metric { .. } => {}
        Event::Dmesg { text } => println!("[{stamp}]   dmesg: {text}"),
        Event::GuardTripped { trip, .. } => println!("[{stamp}] !!! guard: {}", trip.describe()),
        Event::Cooldown { secs_left } => {
            if *secs_left > 0 && *secs_left % 10 == 0 {
                println!("[{stamp}]   cooldown {secs_left}s");
            }
        }
        Event::Sample(s) => {
            *sample_count += 1;
            if verbose || (*sample_count).is_multiple_of(10) {
                println!(
                    "[{stamp}]   temp {}  cpu {}  freq {}  mem {}",
                    s.primary_temp_c
                        .map(|t| format!("{t:.1}C"))
                        .unwrap_or_else(|| "-".into()),
                    s.cpu_percent
                        .map(|p| format!("{p:.0}%"))
                        .unwrap_or_else(|| "-".into()),
                    s.freq_mhz
                        .map(|f| format!("{f} MHz"))
                        .unwrap_or_else(|| "-".into()),
                    format_bytes(s.mem_used_bytes)
                );
            }
        }
        Event::TestFinished { result, .. } => {
            let detail = result.status.detail();
            println!(
                "[{stamp}] === {}: {} {}{}",
                result.index + 1,
                result.name,
                result.status.label(),
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            );
        }
        Event::RunFinished(_) => {}
    }
}

fn summary_table(run: &Run) -> String {
    let mut lines = vec![format!(
        "{:<3} {:<28} {:<6} {:<9} {:<8} detail",
        "#", "test", "status", "duration", "max temp"
    )];
    for r in &run.results {
        let secs = (r.ended - r.started).num_seconds().max(0) as u64;
        lines.push(format!(
            "{:<3} {:<28} {:<6} {:<9} {:<8} {}",
            r.index + 1,
            r.name,
            r.status.label(),
            format_duration_secs(secs),
            r.max_temp_c
                .map(|t| format!("{t:.0}C"))
                .unwrap_or_else(|| "-".into()),
            r.status.detail()
        ));
    }
    lines.push(format!(
        "overall: {}",
        if run.all_passed() { "PASS" } else { "FAIL" }
    ));
    lines.join("\n")
}

/// Build a plan from `kiln run` flags.
fn plan_from_args(state: &AppState, args: &RunArgs) -> Result<Plan> {
    let mut guards = state.config.guards.clone();
    if let Some(t) = &args.temp_max {
        guards.temp_max_c = if t.eq_ignore_ascii_case("off") {
            None
        } else {
            Some(t.parse().context("--temp-max expects a number or `off`")?)
        };
    }
    if let Some(t) = &args.time_max {
        guards.time_max_secs = Some(
            crate::engine::util::parse_duration_secs(t).context("--time-max expects e.g. 30m")?,
        );
    }
    if let Some(c) = args.cooldown {
        guards.cooldown_secs = c;
    }
    if args.keep_going {
        guards.stop_on_fail = false;
    }
    let overrides = parse_sets(&args.sets)?;

    let mut plan = Plan::new("cli", guards);
    for id in &args.tests {
        let test = state
            .registry
            .get(id)
            .with_context(|| format!("unknown test `{id}`. Try `kiln list`"))?;
        let keys: Vec<&str> = test.params().iter().map(|p| p.key).collect();
        let mut entry = PlanEntry::new(id);
        entry.repeat = args.repeat.max(1);
        // Only hand each test the overrides it understands.
        entry.params = overrides
            .iter()
            .filter(|(k, _)| keys.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some(profile) = &args.profile
            && state
                .paths
                .profiles_for(id)
                .join(format!("{profile}.toml"))
                .exists()
        {
            entry.profile = Some(profile.clone());
        }
        plan.tests.push(entry);
    }
    Ok(plan)
}

/// Parse `key=value` pairs. Values stay as text; the runner validates them
/// against each test's spec.
fn parse_sets(sets: &[String]) -> Result<ParamSet> {
    let mut map = BTreeMap::new();
    for s in sets {
        let (k, v) = s
            .split_once('=')
            .with_context(|| format!("--set expects key=value, got `{s}`"))?;
        map.insert(k.trim().to_string(), ParamValue::Text(v.trim().to_string()));
    }
    Ok(map)
}

fn runs(state: &AppState) -> Result<i32> {
    let runs = Run::list(&state.paths.runs_dir);
    if runs.is_empty() {
        println!("no runs yet in {}", state.paths.runs_dir.display());
        return Ok(0);
    }
    for (_, run) in runs {
        println!("{}", report::one_line(&run));
    }
    Ok(0)
}

fn load_run(state: &AppState, id_or_path: &str) -> Result<Run> {
    let path = PathBuf::from(id_or_path);
    let candidates = [
        path.clone(),
        path.join("run.json"),
        state.paths.runs_dir.join(id_or_path).join("run.json"),
    ];
    let found = candidates
        .iter()
        .find(|p| p.is_file())
        .with_context(|| format!("no run `{id_or_path}`. Try `kiln runs`"))?;
    Run::load(found)
}

fn log(state: &AppState, id: &str, index: Option<usize>) -> Result<i32> {
    let run = load_run(state, id)?;
    let result = match index {
        Some(i) => run
            .results
            .get(i.saturating_sub(1))
            .with_context(|| format!("run has {} test(s)", run.results.len()))?,
        None => run.results.last().context("run has no tests")?,
    };
    if result.log_file.is_empty() {
        println!("(no log: {})", result.status.detail());
        return Ok(0);
    }
    let path = state.paths.runs_dir.join(&run.id).join(&result.log_file);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    print!("{text}");
    Ok(0)
}

fn profile(state: &AppState, action: ProfileAction) -> Result<i32> {
    match action {
        ProfileAction::List { test } => {
            state
                .registry
                .get(&test)
                .with_context(|| format!("unknown test `{test}`"))?;
            let profiles = Profile::list(&state.paths.profiles_for(&test));
            if profiles.is_empty() {
                println!("no saved profiles for {test} (built-in defaults apply)");
            }
            for p in profiles {
                let values: Vec<String> = p
                    .params
                    .iter()
                    .map(|(k, v)| format!("{k}={}", v.to_display()))
                    .collect();
                println!("{:<16} {}  {}", p.name, values.join(" "), p.description);
            }
        }
        ProfileAction::Save {
            test,
            name,
            sets,
            description,
        } => {
            let t = state
                .registry
                .get(&test)
                .with_context(|| format!("unknown test `{test}`"))?;
            let raw = parse_sets(&sets)?;
            // Validate against the spec so a bad profile is never written.
            let resolved = crate::engine::params::resolve_params(&t.params(), &[&raw]).or_else(
                |e| match e {
                    crate::engine::ParamError::Missing { .. } => Ok(raw.clone()),
                    other => Err(other),
                },
            )?;
            let params: ParamSet = resolved
                .into_iter()
                .filter(|(k, _)| raw.contains_key(k))
                .collect();
            let profile = Profile {
                name: name.clone(),
                description,
                params,
            };
            let dir = state.paths.profiles_for(&test);
            profile.save(&dir)?;
            state.paths.restore_owner(&dir);
            state.paths.restore_owner(&dir.join(format!("{name}.toml")));
            println!("saved {}", dir.join(format!("{name}.toml")).display());
        }
        ProfileAction::Delete { test, name } => {
            let path = state.paths.profiles_for(&test).join(format!("{name}.toml"));
            std::fs::remove_file(&path)
                .with_context(|| format!("no profile at {}", path.display()))?;
            println!("deleted {}", path.display());
        }
    }
    Ok(0)
}

fn config(state: &mut AppState, action: ConfigAction) -> Result<i32> {
    match action {
        ConfigAction::Path => {
            println!("config:   {}", state.paths.config_file.display());
            println!("profiles: {}", state.paths.profiles_dir.display());
            println!("plans:    {}", state.paths.plans_dir.display());
            println!("runs:     {}", state.paths.runs_dir.display());
            println!("log:      {}", state.paths.log_file.display());
        }
        ConfigAction::Show => print!("{}", toml::to_string_pretty(&state.config)?),
        ConfigAction::Reset => {
            state.reset_config()?;
            println!("settings reset to defaults");
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_flags_parse_into_text_values() {
        let map = parse_sets(&["duration=5m".into(), "device = /dev/sdz".into()]).unwrap();
        assert_eq!(map["duration"], ParamValue::Text("5m".into()));
        assert_eq!(map["device"], ParamValue::Text("/dev/sdz".into()));
        assert!(parse_sets(&["novalue".into()]).is_err());
    }
}
