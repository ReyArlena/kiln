//! Shared helpers for the stress-ng based tests.

use crate::engine::test::{Outcome, Verdict};

/// Metrics from `--metrics-brief` lines, e.g.
/// `stress-ng: metrc: [1234] cpu  123456  60.00  ...  2057.60  ...`
/// where the first number after the stressor name is bogo ops.
pub fn parse_metrics_line(line: &str) -> Vec<(String, f64)> {
    let Some(rest) = line.split("metrc: [").nth(1) else {
        return Vec::new();
    };
    let Some(after_pid) = rest.split("] ").nth(1) else {
        return Vec::new();
    };
    let mut parts = after_pid.split_whitespace();
    let Some(stressor) = parts.next() else {
        return Vec::new();
    };
    if stressor == "stressor" {
        return Vec::new(); // header row
    }
    let Some(bogo_ops) = parts.next().and_then(|v| v.parse::<f64>().ok()) else {
        return Vec::new();
    };
    // Columns: bogo ops, real time, usr time, sys time, bogo ops/s (real), bogo ops/s (usr+sys)
    let ops_per_sec = parts.nth(3).and_then(|v| v.parse::<f64>().ok());
    let mut out = vec![(format!("{stressor}_bogo_ops"), bogo_ops)];
    if let Some(rate) = ops_per_sec {
        out.push((format!("{stressor}_bogo_ops_per_sec"), rate));
    }
    out
}

/// stress-ng exits 0 on success and prints `fail:` lines for verification errors.
pub fn verdict(outcome: &Outcome) -> Verdict {
    let fails: Vec<&String> = outcome
        .output
        .iter()
        .filter(|l| l.contains("stress-ng: fail:"))
        .collect();
    if let Some(first) = fails.first() {
        return Verdict::Fail(format!(
            "{} verification failure(s): {}",
            fails.len(),
            first.trim()
        ));
    }
    match outcome.exit_code {
        Some(0) => Verdict::Pass,
        Some(code) => Verdict::Fail(format!("stress-ng exited with code {code}")),
        None => Verdict::Fail("stress-ng was killed".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn metrics_line_yields_bogo_ops() {
        let line = "stress-ng: metrc: [4242] cpu             1234567     60.00    470.12      0.35     20576.12       2622.43";
        let m = parse_metrics_line(line);
        assert_eq!(m[0], ("cpu_bogo_ops".into(), 1234567.0));
        assert_eq!(m[1], ("cpu_bogo_ops_per_sec".into(), 20576.12));
        assert!(
            parse_metrics_line("stress-ng: metrc: [4242] stressor       bogo ops real time")
                .is_empty()
        );
        assert!(parse_metrics_line("stress-ng: info: [4242] dispatching hogs").is_empty());
    }

    #[test]
    fn fail_lines_beat_exit_code() {
        let metrics = BTreeMap::new();
        let output = vec!["stress-ng: fail: [1] vm: data mismatch at 0x1000".to_string()];
        assert!(matches!(
            verdict(&Outcome {
                exit_code: Some(0),
                metrics: &metrics,
                output: &output
            }),
            Verdict::Fail(_)
        ));
        assert_eq!(
            verdict(&Outcome {
                exit_code: Some(0),
                metrics: &metrics,
                output: &[]
            }),
            Verdict::Pass
        );
        assert!(matches!(
            verdict(&Outcome {
                exit_code: Some(2),
                metrics: &metrics,
                output: &[]
            }),
            Verdict::Fail(_)
        ));
    }
}
