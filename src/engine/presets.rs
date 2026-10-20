//! Plans shipped with kiln. Saved plans in the config directory sit beside these.

use crate::engine::params::ParamValue;
use crate::engine::{Guards, Plan, PlanEntry};

fn entry(test: &str, params: &[(&str, ParamValue)], repeat: u32) -> PlanEntry {
    let mut e = PlanEntry::new(test);
    for (k, v) in params {
        e.params.insert((*k).to_string(), v.clone());
    }
    e.repeat = repeat;
    e
}

fn plan(name: &str, description: &str, guards: Guards, tests: Vec<PlanEntry>) -> Plan {
    Plan {
        name: name.into(),
        description: description.into(),
        guards,
        tests,
        builtin: true,
    }
}

/// Every built-in plan, in display order.
pub fn builtin_plans() -> Vec<Plan> {
    let secs = ParamValue::Duration;
    let bytes = ParamValue::Bytes;
    let int = ParamValue::Int;
    let guards = Guards::default();
    let strict = Guards {
        temp_max_c: Some(85.0),
        stop_on_fail: true,
        cooldown_secs: 60,
        ..Guards::default()
    };

    vec![
        plan(
            "Quick Check",
            "About 5 minutes. Health snapshot plus the benchmarks and a short RAM hash. A baseline to compare against later.",
            guards.clone(),
            vec![
                entry("health-snapshot", &[], 1),
                entry("cpu-bench", &[], 1),
                entry("mem-bandwidth", &[], 1),
                entry("mem-latency", &[], 1),
                entry(
                    "ram-hash",
                    &[("size", bytes(2 << 30)), ("passes", int(5))],
                    1,
                ),
            ],
        ),
        plan(
            "New Build Shakedown",
            "About an hour, no spare drive needed. Stress CPU and RAM with verification, hash RAM, benchmark, and snapshot health before and after.",
            guards.clone(),
            vec![
                entry("health-snapshot", &[], 1),
                entry("cpu-stress", &[("duration", secs(20 * 60))], 1),
                entry("ram-stress", &[("duration", secs(20 * 60))], 1),
                entry(
                    "ram-hash",
                    &[("size", bytes(8 << 30)), ("passes", int(20))],
                    1,
                ),
                entry("mem-bandwidth", &[], 1),
                entry("mem-latency", &[], 1),
                entry("cpu-bench", &[], 1),
                entry("health-snapshot", &[], 1),
            ],
        ),
        plan(
            "Overnight Burn-in",
            "About 8 hours. Alternating CPU and RAM stress with verification and long RAM hash loops. Stops at the first failure, 85°C cap.",
            strict.clone(),
            vec![
                entry("health-snapshot", &[], 1),
                entry("cpu-stress", &[("duration", secs(2 * 3600))], 1),
                entry("ram-stress", &[("duration", secs(2 * 3600))], 1),
                entry(
                    "ram-hash",
                    &[("size", bytes(8 << 30)), ("passes", int(200))],
                    1,
                ),
                entry(
                    "cpu-stress",
                    &[
                        ("duration", secs(3600)),
                        ("method", ParamValue::Text("matrixprod".into())),
                    ],
                    1,
                ),
                entry("ram-stress", &[("duration", secs(3600))], 1),
                entry("health-snapshot", &[], 1),
            ],
        ),
        plan(
            "Storage Shakedown",
            "DESTRUCTIVE: wipes the device you choose. fio write and verify, then the btrfs install harness, with health snapshots around them.",
            strict,
            vec![
                entry("health-snapshot", &[], 1),
                entry("fio-verify", &[("size", bytes(20 << 30))], 1),
                entry("btrfs-harness", &[], 2),
                entry("health-snapshot", &[], 1),
            ],
        ),
        plan(
            "Benchmarks",
            "CPU, memory bandwidth, memory latency and a read-only disk benchmark. A few minutes.",
            guards,
            vec![
                entry("cpu-bench", &[], 1),
                entry("mem-bandwidth", &[], 1),
                entry("mem-latency", &[], 1),
                entry("disk-read", &[], 1),
            ],
        ),
    ]
}
