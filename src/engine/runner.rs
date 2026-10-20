//! Executes a plan: one test after another, with sampling, guards, logging
//! and cancellation. Progress is reported through [`Event`]s so the caller
//! decides how to show it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Local;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use super::deps::DepReport;
use super::env::EnvSnapshot;
use super::event::Event;
use super::guards::{Guards, Trip};
use super::params::{ParamSet, resolve_params};
use super::plan::{Plan, Profile};
use super::registry::Registry;
use super::result::{Run, SnapshotCapture, Status, TestResult};
use super::sampler::{Sample, Sampler};
use super::test::{BuiltinCtx, Job, JobKind, Outcome, Snapshot, Verdict, parse_kiln_metric};

/// How long a job gets to exit cleanly after SIGINT before SIGKILL.
const GRACE_PERIOD: Duration = Duration::from_secs(20);
/// Snapshot commands that hang are abandoned after this.
const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(30);

/// What the user asked to cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelScope {
    /// Stop the running test and carry on with the next one.
    CurrentTest,
    /// Stop everything.
    Run,
}

/// Everything the runner needs that is not part of the plan itself.
pub struct RunContext {
    pub registry: Registry,
    pub env: EnvSnapshot,
    pub sampler: Sampler,
    pub sample_interval: Duration,
    /// Directory where this run's `run.json` and logs are written.
    pub run_dir: PathBuf,
    /// Root of saved profiles (`profiles/<test>/<name>.toml`).
    pub profiles_dir: PathBuf,
    /// Called on every file created, so files can be handed back to the sudo user.
    pub on_file_created: Arc<dyn Fn(&Path) + Send + Sync>,
}

/// Handle to a run in progress.
pub struct RunHandle {
    pub events: mpsc::UnboundedReceiver<Event>,
    cancel: watch::Sender<Option<CancelScope>>,
    task: JoinHandle<Result<Run>>,
}

impl RunHandle {
    pub fn cancel(&self, scope: CancelScope) {
        let _ = self.cancel.send(Some(scope));
    }

    /// Wait for the run to finish and get the final result.
    pub async fn finish(self) -> Result<Run> {
        self.task.await.context("runner task panicked")?
    }
}

/// Start running `plan` in the background.
pub fn start_run(plan: Plan, ctx: RunContext) -> RunHandle {
    let (event_tx, event_rx) = mpsc::unbounded_channel();
    let (cancel_tx, cancel_rx) = watch::channel(None);
    let task = tokio::spawn(run_plan(plan, ctx, event_tx, cancel_rx));
    RunHandle {
        events: event_rx,
        cancel: cancel_tx,
        task,
    }
}

/// Internal signals multiplexed into one channel inside the runner.
enum Signal_ {
    Sample(Sample),
    Dmesg(String),
}

async fn run_plan(
    plan: Plan,
    ctx: RunContext,
    events: mpsc::UnboundedSender<Event>,
    mut cancel: watch::Receiver<Option<CancelScope>>,
) -> Result<Run> {
    let started_at = Local::now();
    let started = Instant::now();
    let run_id = Run::make_id(started_at);
    std::fs::create_dir_all(&ctx.run_dir)
        .with_context(|| format!("creating {}", ctx.run_dir.display()))?;
    (ctx.on_file_created)(&ctx.run_dir);

    // One sampler and one dmesg follower live for the whole run.
    let (sig_tx, mut sig_rx) = mpsc::unbounded_channel::<Signal_>();
    let (stop_tx, stop_rx) = watch::channel(false);
    let sampler_task = {
        let (sample_tx, mut sample_rx) = mpsc::unbounded_channel::<Event>();
        let forward = sig_tx.clone();
        tokio::spawn(async move {
            while let Some(Event::Sample(s)) = sample_rx.recv().await {
                if forward.send(Signal_::Sample(s)).is_err() {
                    break;
                }
            }
        });
        tokio::spawn(ctx.sampler.clone().run(
            ctx.sample_interval,
            started,
            sample_tx,
            stop_rx.clone(),
        ))
    };
    let mut dmesg_child = spawn_dmesg_follower(sig_tx.clone());

    let mut results = Vec::new();
    let mut samples = Vec::new();
    let mut index = 0usize;
    let mut stop_everything = false;

    'entries: for entry in &plan.tests {
        for _ in 0..entry.repeat.max(1) {
            // Reset any cancellation left over from the previous test.
            let _ = cancel.borrow_and_update();
            let test_started = Local::now();
            let test_id = entry.test.clone();

            let prepared = prepare(&ctx, &test_id, entry.profile.as_deref(), &entry.params);
            let (name, params, job) = match prepared {
                Ok(p) => p,
                Err(why) => {
                    let result = TestResult {
                        index,
                        test_id: test_id.clone(),
                        name: test_id.clone(),
                        params: entry.params.clone(),
                        started: test_started,
                        ended: Local::now(),
                        status: Status::Skipped(why),
                        exit_code: None,
                        metrics: BTreeMap::new(),
                        max_temp_c: None,
                        log_file: String::new(),
                        snapshots: Vec::new(),
                    };
                    let _ = events.send(Event::TestFinished {
                        index,
                        result: result.clone(),
                    });
                    results.push(result);
                    index += 1;
                    if plan.guards.stop_on_fail {
                        break 'entries;
                    }
                    continue;
                }
            };

            let _ = events.send(Event::TestStarted {
                index,
                test_id: test_id.clone(),
                name: name.clone(),
                expected_secs: job.expected_secs,
                graphs: job.graphs.clone(),
            });

            let log_name = format!("{index:02}-{test_id}.log");
            let log_path = ctx.run_dir.join(&log_name);
            let mut log = tokio::fs::File::create(&log_path)
                .await
                .with_context(|| format!("creating {}", log_path.display()))?;
            (ctx.on_file_created)(&log_path);

            let before = capture_snapshots(&job.snapshots).await;
            let exec = execute_job(
                job,
                index,
                &plan.guards,
                &events,
                &mut sig_rx,
                &mut cancel,
                &mut log,
                &mut samples,
            )
            .await;
            let after = capture_snapshots(&exec.snapshots).await;
            let snapshots = before
                .into_iter()
                .zip(after)
                .map(|((name, before), (_, after))| SnapshotCapture {
                    name,
                    before,
                    after,
                })
                .collect();

            let result = TestResult {
                index,
                test_id: test_id.clone(),
                name,
                params,
                started: test_started,
                ended: Local::now(),
                status: exec.status,
                exit_code: exec.exit_code,
                metrics: exec.metrics,
                max_temp_c: exec.max_temp_c,
                log_file: log_name,
                snapshots,
            };
            let _ = log.flush().await;
            let _ = events.send(Event::TestFinished {
                index,
                result: result.clone(),
            });
            let failed = !result.status.is_pass();
            results.push(result);
            index += 1;

            // Persist progress so a crash or power cut still leaves a readable run.
            save_partial(&ctx, &run_id, started_at, &plan, &results, &samples);

            if exec.cancel_run || (failed && plan.guards.stop_on_fail) {
                stop_everything = true;
                break 'entries;
            }

            // Let temperatures settle before the next test.
            let is_last = std::ptr::eq(entry, plan.tests.last().unwrap()) && entry.repeat <= 1;
            if !is_last && plan.guards.cooldown_secs > 0 {
                let cancelled = cooldown(
                    plan.guards.cooldown_secs,
                    &events,
                    &mut sig_rx,
                    &mut cancel,
                    &mut samples,
                )
                .await;
                if cancelled == Some(CancelScope::Run) {
                    stop_everything = true;
                    break 'entries;
                }
            }
        }
    }
    let _ = stop_everything;

    let _ = stop_tx.send(true);
    sampler_task.abort();
    if let Some(child) = dmesg_child.as_mut() {
        let _ = child.start_kill();
    }

    let run = Run {
        id: run_id,
        kiln_version: env!("CARGO_PKG_VERSION").to_string(),
        started: started_at,
        ended: Local::now(),
        in_progress: false,
        plan,
        env: ctx.env.clone(),
        results,
        samples,
    };
    let path = run.save(&ctx.run_dir)?;
    (ctx.on_file_created)(&path);
    let _ = events.send(Event::RunFinished(Box::new(run.clone())));
    Ok(run)
}

/// Write `run.json` with what has finished so far, marked as in progress.
fn save_partial(
    ctx: &RunContext,
    run_id: &str,
    started_at: chrono::DateTime<Local>,
    plan: &Plan,
    results: &[TestResult],
    samples: &[Sample],
) {
    let run = Run {
        id: run_id.to_string(),
        kiln_version: env!("CARGO_PKG_VERSION").to_string(),
        started: started_at,
        ended: Local::now(),
        in_progress: true,
        plan: plan.clone(),
        env: ctx.env.clone(),
        results: results.to_vec(),
        samples: samples.to_vec(),
    };
    if let Ok(path) = run.save(&ctx.run_dir) {
        (ctx.on_file_created)(&path);
    }
}

/// Resolve the test, its parameters and dependencies, then build the job.
fn prepare(
    ctx: &RunContext,
    test_id: &str,
    profile: Option<&str>,
    overrides: &ParamSet,
) -> Result<(String, ParamSet, Job), String> {
    let test = ctx
        .registry
        .get(test_id)
        .ok_or_else(|| format!("unknown test `{test_id}`"))?;
    let info = test.info();

    let report = DepReport::check(&test.deps());
    if !report.all_ok() {
        let missing: Vec<String> = report
            .missing()
            .map(|s| format!("{} ({})", s.dep.label(), s.hint.clone().unwrap_or_default()))
            .collect();
        return Err(format!("missing: {}", missing.join(", ")));
    }

    let profile_params = match profile {
        Some(name) => {
            let path = ctx.profiles_dir.join(test_id).join(format!("{name}.toml"));
            Profile::load(&path)
                .map_err(|e| format!("profile `{name}`: {e}"))?
                .params
        }
        None => ParamSet::new(),
    };
    let params =
        resolve_params(&test.params(), &[&profile_params, overrides]).map_err(|e| e.to_string())?;
    let job = test.plan(&params, &ctx.env).map_err(|e| format!("{e:#}"))?;
    Ok((info.name.to_string(), params, job))
}

struct Executed {
    status: Status,
    exit_code: Option<i32>,
    metrics: BTreeMap<String, f64>,
    max_temp_c: Option<f32>,
    snapshots: Vec<Snapshot>,
    cancel_run: bool,
}

/// Run one job to completion or until a guard or the user stops it.
#[allow(clippy::too_many_arguments)]
async fn execute_job(
    job: Job,
    index: usize,
    guards: &Guards,
    events: &mpsc::UnboundedSender<Event>,
    sig_rx: &mut mpsc::UnboundedReceiver<Signal_>,
    cancel: &mut watch::Receiver<Option<CancelScope>>,
    log: &mut tokio::fs::File,
    samples: &mut Vec<Sample>,
) -> Executed {
    let started = Instant::now();
    let mut metrics = BTreeMap::new();
    let mut output: Vec<String> = Vec::new();
    let mut max_temp_c: Option<f32> = None;
    let mut trip: Option<Trip> = None;
    let mut cancel_run = false;

    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (job_cancel_tx, job_cancel_rx) = watch::channel(false);

    let mut running = match spawn_job(job.kind, line_tx, job_cancel_rx) {
        Ok(r) => r,
        Err(e) => {
            return Executed {
                status: Status::Skipped(format!("could not start: {e:#}")),
                exit_code: None,
                metrics,
                max_temp_c,
                snapshots: job.snapshots,
                cancel_run: false,
            };
        }
    };

    let mut exit_code: Option<i32> = None;
    let mut lines_open = true;
    let mut kill_deadline: Option<Instant> = None;
    let mut finished = false;

    while !finished {
        tokio::select! {
            line = line_rx.recv(), if lines_open => {
                match line {
                    Some(text) => {
                        let stamp = format!("[{:>8.1}] {text}\n", started.elapsed().as_secs_f64());
                        let _ = log.write_all(stamp.as_bytes()).await;
                        for (key, value) in (job.parser)(&text).into_iter().chain(parse_kiln_metric(&text)) {
                            metrics.insert(key.clone(), value);
                            let _ = events.send(Event::Metric { index, key, value });
                        }
                        let _ = events.send(Event::Line { index, text: text.clone() });
                        output.push(text);
                    }
                    None => lines_open = false,
                }
            }
            sig = sig_rx.recv() => {
                match sig {
                    Some(Signal_::Sample(sample)) => {
                        if let Some(t) = sample.primary_temp_c {
                            max_temp_c = Some(max_temp_c.map_or(t, |m| m.max(t)));
                        }
                        if trip.is_none() && let Some(t) = guards.check_sample(&sample) {
                            trip = Some(t);
                        }
                        let _ = events.send(Event::Sample(sample.clone()));
                        samples.push(sample);
                    }
                    Some(Signal_::Dmesg(text)) => {
                        let _ = log.write_all(format!("[{:>8.1}] dmesg: {text}\n", started.elapsed().as_secs_f64()).as_bytes()).await;
                        if trip.is_none() && let Some(t) = guards.check_dmesg(&text) {
                            trip = Some(t);
                        }
                        let _ = events.send(Event::Dmesg { text });
                    }
                    None => {}
                }
            }
            _ = cancel.changed() => {
                let scope = *cancel.borrow_and_update();
                if let Some(scope) = scope && trip.is_none() {
                    trip = Some(Trip::UserCancelled);
                    cancel_run = scope == CancelScope::Run;
                }
            }
            code = running.wait() => {
                exit_code = code;
                finished = true;
            }
            _ = tokio::time::sleep(Duration::from_millis(250)) => {
                if trip.is_none() && let Some(t) = guards.check_elapsed(started.elapsed()) {
                    trip = Some(t);
                }
            }
        }

        // A guard or the user asked us to stop: interrupt once, then escalate.
        if let Some(t) = &trip
            && !finished
        {
            match kill_deadline {
                None => {
                    let _ = events.send(Event::GuardTripped {
                        index,
                        trip: t.clone(),
                    });
                    let _ = log
                        .write_all(
                            format!(
                                "[{:>8.1}] kiln: stopping job: {}\n",
                                started.elapsed().as_secs_f64(),
                                t.describe()
                            )
                            .as_bytes(),
                        )
                        .await;
                    running.interrupt(&job_cancel_tx);
                    kill_deadline = Some(Instant::now() + GRACE_PERIOD);
                }
                Some(deadline) if Instant::now() >= deadline => {
                    let _ = log
                        .write_all(b"kiln: grace period over, killing job\n")
                        .await;
                    running.kill().await;
                    kill_deadline = Some(Instant::now() + GRACE_PERIOD);
                }
                Some(_) => {}
            }
        }
    }

    // Drain any output that arrived after the process exited.
    while let Ok(text) = line_rx.try_recv() {
        let _ = log
            .write_all(format!("[{:>8.1}] {text}\n", started.elapsed().as_secs_f64()).as_bytes())
            .await;
        for (key, value) in (job.parser)(&text)
            .into_iter()
            .chain(parse_kiln_metric(&text))
        {
            metrics.insert(key, value);
        }
        output.push(text);
    }

    let status = match trip {
        Some(t) => Status::Aborted(t),
        None => match (job.verdict)(&Outcome {
            exit_code,
            metrics: &metrics,
            output: &output,
        }) {
            Verdict::Pass => Status::Pass,
            Verdict::Fail(why) => Status::Fail(why),
        },
    };
    let _ = log
        .write_all(format!("kiln: result {} {}\n", status.label(), status.detail()).as_bytes())
        .await;

    Executed {
        status,
        exit_code,
        metrics,
        max_temp_c,
        snapshots: job.snapshots,
        cancel_run,
    }
}

/// A job in flight: either a child process group or an in-process task.
enum Running {
    Process { child: Child, pgid: Pid },
    Builtin { task: JoinHandle<Result<i32>> },
}

impl Running {
    async fn wait(&mut self) -> Option<i32> {
        match self {
            Self::Process { child, .. } => child.wait().await.ok().and_then(|s| s.code()),
            Self::Builtin { task } => match task.await {
                Ok(Ok(code)) => Some(code),
                _ => None,
            },
        }
    }

    fn interrupt(&mut self, builtin_cancel: &watch::Sender<bool>) {
        match self {
            Self::Process { pgid, .. } => {
                let _ = killpg(*pgid, Signal::SIGINT);
            }
            Self::Builtin { .. } => {
                let _ = builtin_cancel.send(true);
            }
        }
    }

    async fn kill(&mut self) {
        match self {
            Self::Process { child, pgid } => {
                let _ = killpg(*pgid, Signal::SIGKILL);
                let _ = child.kill().await;
            }
            Self::Builtin { task } => task.abort(),
        }
    }
}

fn spawn_job(
    kind: JobKind,
    line_tx: mpsc::UnboundedSender<String>,
    cancel: watch::Receiver<bool>,
) -> Result<Running> {
    match kind {
        JobKind::Command { program, args } => {
            let mut cmd = Command::new(&program);
            cmd.args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);
            // Own process group so SIGINT reaches every worker the tool forks.
            cmd.process_group(0);
            let mut child = cmd.spawn().with_context(|| format!("spawning {program}"))?;
            let pgid = Pid::from_raw(child.id().map(|id| id as i32).unwrap_or(0));
            let _ = line_tx.send(format!("kiln: exec {program} {}", args.join(" ")));
            if let Some(stdout) = child.stdout.take() {
                tokio::spawn(forward_lines(stdout, line_tx.clone()));
            }
            if let Some(stderr) = child.stderr.take() {
                tokio::spawn(forward_lines(stderr, line_tx));
            }
            Ok(Running::Process { child, pgid })
        }
        JobKind::Builtin(func) => {
            let ctx = BuiltinCtx {
                cancel,
                output: line_tx,
            };
            let task = tokio::spawn(func(ctx));
            Ok(Running::Builtin { task })
        }
    }
}

async fn forward_lines<R: tokio::io::AsyncRead + Unpin>(
    reader: R,
    tx: mpsc::UnboundedSender<String>,
) {
    let mut lines = BufReader::new(reader).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if tx.send(line).is_err() {
            break;
        }
    }
}

/// Follow new kernel messages. Best effort: silently absent when `dmesg -W`
/// is unavailable or not permitted.
fn spawn_dmesg_follower(tx: mpsc::UnboundedSender<Signal_>) -> Option<Child> {
    let mut child = Command::new("dmesg")
        .args(["--follow-new", "--notime"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .ok()?;
    let stdout = child.stdout.take()?;
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            if tx.send(Signal_::Dmesg(line)).is_err() {
                break;
            }
        }
    });
    Some(child)
}

/// Run each snapshot command and keep its output as text.
async fn capture_snapshots(snapshots: &[Snapshot]) -> Vec<(String, String)> {
    let mut captured = Vec::new();
    for snap in snapshots {
        let fut = Command::new(&snap.program)
            .args(&snap.args)
            .stdin(Stdio::null())
            .output();
        let text = match tokio::time::timeout(SNAPSHOT_TIMEOUT, fut).await {
            Ok(Ok(out)) => {
                let mut text = String::from_utf8_lossy(&out.stdout).to_string();
                if !out.stderr.is_empty() {
                    text.push_str(&String::from_utf8_lossy(&out.stderr));
                }
                text
            }
            Ok(Err(e)) => format!("(could not run {}: {e})", snap.program),
            Err(_) => "(timed out)".to_string(),
        };
        captured.push((snap.name.clone(), text));
    }
    captured
}

/// Wait between tests, still forwarding samples. Returns the cancel scope if interrupted.
async fn cooldown(
    secs: u64,
    events: &mpsc::UnboundedSender<Event>,
    sig_rx: &mut mpsc::UnboundedReceiver<Signal_>,
    cancel: &mut watch::Receiver<Option<CancelScope>>,
    samples: &mut Vec<Sample>,
) -> Option<CancelScope> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                let left = deadline.saturating_duration_since(Instant::now()).as_secs();
                let _ = events.send(Event::Cooldown { secs_left: left });
                if Instant::now() >= deadline {
                    return None;
                }
            }
            sig = sig_rx.recv() => {
                match sig {
                    Some(Signal_::Sample(sample)) => {
                        let _ = events.send(Event::Sample(sample.clone()));
                        samples.push(sample);
                    }
                    Some(Signal_::Dmesg(text)) => {
                        let _ = events.send(Event::Dmesg { text });
                    }
                    _ => {}
                }
            }
            _ = cancel.changed() => {
                let scope = *cancel.borrow_and_update();
                if scope.is_some() {
                    return scope;
                }
            }
        }
    }
}
