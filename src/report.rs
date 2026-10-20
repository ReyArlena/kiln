//! Turns a saved run into a Markdown report.

use std::fmt::Write;

use crate::engine::util::{format_bytes, format_duration_secs};
use crate::engine::{Run, Status};

pub fn markdown(run: &Run) -> String {
    let mut out = String::new();
    let overall = if run.in_progress {
        "IN PROGRESS (kiln did not finish this run)"
    } else if run.all_passed() {
        "PASS"
    } else {
        "FAIL"
    };
    let _ = writeln!(out, "# kiln run {}\n", run.id);
    let _ = writeln!(out, "Result: **{overall}**  ");
    let _ = writeln!(out, "Plan: {}  ", run.plan.name);
    let _ = writeln!(
        out,
        "Started: {}  ",
        run.started.format("%Y-%m-%d %H:%M:%S")
    );
    let _ = writeln!(
        out,
        "Duration: {}  ",
        format_duration_secs((run.ended - run.started).num_seconds().max(0) as u64)
    );
    let _ = writeln!(out, "kiln: {}\n", run.kiln_version);

    let _ = writeln!(out, "## Machine\n");
    for line in run.env.summary_lines() {
        let _ = writeln!(out, "- {}", line.trim());
    }

    let _ = writeln!(out, "\n## Guards\n");
    let g = &run.plan.guards;
    let _ = writeln!(
        out,
        "- Temperature cap: {}",
        g.temp_max_c
            .map(|c| format!("{c:.0}°C"))
            .unwrap_or_else(|| "none".into())
    );
    let _ = writeln!(
        out,
        "- Time cap: {}",
        g.time_max_secs
            .map(format_duration_secs)
            .unwrap_or_else(|| "none".into())
    );
    let _ = writeln!(out, "- Stop on fail: {}", g.stop_on_fail);
    let _ = writeln!(out, "- Cooldown: {}s", g.cooldown_secs);

    let _ = writeln!(out, "\n## Results\n");
    let _ = writeln!(out, "| # | Test | Status | Duration | Max temp | Detail |");
    let _ = writeln!(out, "| - | ---- | ------ | -------- | -------- | ------ |");
    for r in &run.results {
        let secs = (r.ended - r.started).num_seconds().max(0) as u64;
        let temp = r
            .max_temp_c
            .map(|t| format!("{t:.1}°C"))
            .unwrap_or_else(|| "-".into());
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} | {} | {} |",
            r.index + 1,
            r.name,
            r.status.label(),
            format_duration_secs(secs),
            temp,
            r.status.detail().replace('|', "/")
        );
    }

    for r in &run.results {
        let _ = writeln!(
            out,
            "\n### {}. {} ({})\n",
            r.index + 1,
            r.name,
            r.status.label()
        );
        if !r.params.is_empty() {
            let params: Vec<String> = r
                .params
                .iter()
                .map(|(k, v)| format!("{k}={}", v.to_display()))
                .collect();
            let _ = writeln!(out, "Parameters: {}  ", params.join(", "));
        }
        if let Some(code) = r.exit_code {
            let _ = writeln!(out, "Exit code: {code}  ");
        }
        if !r.metrics.is_empty() {
            let _ = writeln!(out, "\nMetrics:\n");
            for (k, v) in &r.metrics {
                let _ = writeln!(out, "- {k}: {}", trim_float(*v));
            }
        }
        if let Status::Aborted(trip) = &r.status {
            let _ = writeln!(out, "\nAborted: {}", trip.describe());
        }
        if !r.log_file.is_empty() {
            let _ = writeln!(out, "\nLog: `{}`", r.log_file);
        }
        for snap in &r.snapshots {
            if snap.before.trim() == snap.after.trim() {
                let _ = writeln!(out, "\nSnapshot `{}`: unchanged", snap.name);
            } else {
                let _ = writeln!(
                    out,
                    "\nSnapshot `{}` before:\n\n```\n{}\n```\n\nafter:\n\n```\n{}\n```",
                    snap.name,
                    snap.before.trim(),
                    snap.after.trim()
                );
            }
        }
    }

    if let Some(max) = run
        .samples
        .iter()
        .filter_map(|s| s.primary_temp_c)
        .fold(None, |m: Option<f32>, t| Some(m.map_or(t, |m| m.max(t))))
    {
        let _ = writeln!(out, "\n## Sensors\n");
        let _ = writeln!(out, "- Samples: {}", run.samples.len());
        let _ = writeln!(out, "- Peak CPU temperature: {max:.1}°C");
        if let Some(peak_mem) = run.samples.iter().map(|s| s.mem_used_bytes).max() {
            let _ = writeln!(out, "- Peak memory used: {}", format_bytes(peak_mem));
        }
    }
    out
}

fn trim_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v:.2}")
    }
}

/// Two runs side by side. Tests are matched by id in order, so the same
/// plan run twice lines up test for test.
pub fn compare(a: &Run, b: &Run) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "A: {}  ({})", a.id, a.env.cpu_model);
    let _ = writeln!(out, "B: {}  ({})", b.id, b.env.cpu_model);
    if a.env.board != b.env.board || a.env.bios != b.env.bios || a.env.kernel != b.env.kernel {
        let _ = writeln!(out, "note: board, BIOS or kernel differ between runs");
    }
    let mut used = vec![false; b.results.len()];
    for ra in &a.results {
        let Some((bi, rb)) = b
            .results
            .iter()
            .enumerate()
            .find(|(i, r)| !used[*i] && r.test_id == ra.test_id)
        else {
            let _ = writeln!(out, "\n{}: only in A", ra.name);
            continue;
        };
        used[bi] = true;
        let _ = writeln!(
            out,
            "\n{}  ({} / {})",
            ra.name,
            ra.status.label(),
            rb.status.label()
        );
        let _ = writeln!(
            out,
            "  {:<24} {:>12} {:>12} {:>9}",
            "metric", "A", "B", "change"
        );
        let secs =
            |r: &crate::engine::TestResult| (r.ended - r.started).num_seconds().max(0) as f64;
        let mut rows: Vec<(String, Option<f64>, Option<f64>)> = vec![
            ("duration_s".into(), Some(secs(ra)), Some(secs(rb))),
            (
                "max_temp_c".into(),
                ra.max_temp_c.map(|t| t as f64),
                rb.max_temp_c.map(|t| t as f64),
            ),
        ];
        let mut keys: Vec<&String> = ra.metrics.keys().chain(rb.metrics.keys()).collect();
        keys.sort();
        keys.dedup();
        for k in keys {
            rows.push((
                k.clone(),
                ra.metrics.get(k).copied(),
                rb.metrics.get(k).copied(),
            ));
        }
        for (key, va, vb) in rows {
            let change = match (va, vb) {
                (Some(x), Some(y)) if x != 0.0 => format!("{:+.1}%", (y - x) / x * 100.0),
                _ => "-".into(),
            };
            let _ = writeln!(
                out,
                "  {:<24} {:>12} {:>12} {:>9}",
                key,
                va.map(trim_float).unwrap_or_else(|| "-".into()),
                vb.map(trim_float).unwrap_or_else(|| "-".into()),
                change
            );
        }
    }
    for (i, rb) in b.results.iter().enumerate() {
        if !used[i] {
            let _ = writeln!(out, "\n{}: only in B", rb.name);
        }
    }
    out
}

/// One-line summary used by `kiln runs` and the history screen.
pub fn one_line(run: &Run) -> String {
    let state = if run.in_progress {
        "UNFINISHED"
    } else if run.all_passed() {
        "PASS"
    } else {
        "FAIL"
    };
    let tests: Vec<String> = run
        .results
        .iter()
        .map(|r| format!("{}:{}", r.test_id, r.status.label()))
        .collect();
    format!("{}  {:<10} {}", run.id, state, tests.join(" "))
}
