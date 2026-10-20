//! STREAM-style memory bandwidth: copy, scale, add and triad over large arrays.

use std::hint::black_box;
use std::time::Instant;

use anyhow::{Result, bail};

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{BuiltinCtx, Category, GraphSpec, Job, JobKind, Test, TestInfo};
use crate::engine::util::format_bytes;
use crate::engine::{Dep, EnvSnapshot};

use super::bench;

pub struct MemBandwidth {
    info: TestInfo,
}

impl MemBandwidth {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "mem-bandwidth",
                name: "Memory Bandwidth",
                tldr: "How fast RAM moves data, in GB/s. Single vs dual channel and wrong memory clocks show up here first.",
                description: "\
The four STREAM kernels over three large double-precision arrays: copy \
(c = a), scale (b = k*c), add (c = a + b) and triad (a = b + k*c). Each \
runs on every thread at once with the arrays split between them, several \
passes, and the best pass is reported in GB/s of data moved. Triad is the \
headline figure.

What the numbers tell you: one DIMM or a stick in the wrong slot gives \
roughly half the bandwidth of a matched pair; memory running at JEDEC \
instead of its EXPO/XMP profile is visible as a 20 to 30 percent drop; \
a run far below a previous one on the same machine means something \
changed in the BIOS or the hardware.

Pure Rust, no external tools. Compare only against other kiln runs from a \
release build, not against published STREAM results.

Pass: always, unless the arrays could not be allocated.",
                category: Category::Benchmark,
                uses: &["in-process STREAM-style kernels on all threads"],
                destructive: false,
            },
        }
    }
}

impl Default for MemBandwidth {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for MemBandwidth {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "size",
                label: "Total array size",
                help: "Memory for all three arrays together, e.g. 1G. Should far exceed the CPU cache",
                kind: ParamKind::Bytes { default: 1 << 30 },
            },
            ParamSpec {
                key: "threads",
                label: "Threads",
                help: "0 uses every thread",
                kind: ParamKind::Int {
                    default: 0,
                    min: 0,
                    max: 1024,
                    zero_means_auto: true,
                },
            },
            ParamSpec {
                key: "passes",
                label: "Passes",
                help: "Repeats per kernel; the best is kept",
                kind: ParamKind::Int {
                    default: 5,
                    min: 1,
                    max: 100,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let size = params["size"].as_u64().unwrap_or(1 << 30);
        let threads = bench::thread_count(params["threads"].as_i64().unwrap_or(0), env.cpu_count);
        let passes = params["passes"].as_u64().unwrap_or(5) as usize;
        if env.mem_total_bytes > 0 && size > env.mem_total_bytes / 2 {
            bail!(
                "{} is more than half of RAM ({})",
                format_bytes(size),
                format_bytes(env.mem_total_bytes)
            );
        }
        Ok(Job {
            kind: JobKind::Builtin(Box::new(move |ctx| {
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || run(&ctx, size, threads, passes)).await?
                })
            })),
            graphs: vec![GraphSpec::metric("GB/s", "triad_gb_s")],
            expected_secs: None,
            snapshots: vec![],
            verdict: Box::new(bench::verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

/// One STREAM kernel over one thread's slices of a, b and c.
type Kernel = fn(&mut [f64], &mut [f64], &mut [f64], f64);

fn run(ctx: &BuiltinCtx, size: u64, threads: usize, passes: usize) -> Result<i32> {
    bench::warn_if_debug_build(ctx);
    let n = (size / 3 / 8) as usize;
    ctx.say(format!(
        "three arrays of {} doubles ({} total), {threads} threads, {passes} passes",
        n,
        format_bytes(n as u64 * 24)
    ));
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    if a.try_reserve_exact(n).is_err()
        || b.try_reserve_exact(n).is_err()
        || c.try_reserve_exact(n).is_err()
    {
        ctx.say("ALLOC_FAIL could not allocate arrays");
        return Ok(2);
    }
    a.resize(n, 1.0f64);
    b.resize(n, 2.0f64);
    c.resize(n, 0.0f64);
    let scalar = 3.0f64;

    // Each kernel: name, bytes moved per element, closure over one thread's slices.
    let kernels: [(&str, u64, Kernel); 4] = [
        ("copy", 16, |a, _b, c, _k| c.copy_from_slice(a)),
        ("scale", 16, |_a, b, c, k| {
            b.iter_mut().zip(c.iter()).for_each(|(b, c)| *b = k * c)
        }),
        ("add", 24, |a, b, c, _k| {
            c.iter_mut()
                .zip(a.iter().zip(b.iter()))
                .for_each(|(c, (a, b))| *c = a + b)
        }),
        ("triad", 24, |a, b, c, k| {
            a.iter_mut()
                .zip(b.iter().zip(c.iter()))
                .for_each(|(a, (b, c))| *a = b + k * c)
        }),
    ];

    let chunk = n.div_ceil(threads).max(1);
    let mut results = Vec::new();
    for (name, bytes_per_elem, kernel) in kernels {
        let mut best = f64::MAX;
        for _ in 0..passes {
            if ctx.cancelled() {
                ctx.say("cancelled");
                return Ok(130);
            }
            let started = Instant::now();
            std::thread::scope(|scope| {
                for ((ca, cb), cc) in a
                    .chunks_mut(chunk)
                    .zip(b.chunks_mut(chunk))
                    .zip(c.chunks_mut(chunk))
                {
                    scope.spawn(move || kernel(ca, cb, cc, scalar));
                }
            });
            best = best.min(started.elapsed().as_secs_f64());
        }
        black_box((&a, &b, &c));
        let gb_s = n as f64 * bytes_per_elem as f64 / best / 1e9;
        ctx.say(format!(
            "{name:<6} {gb_s:7.1} GB/s  (best of {passes}, {:.3}s)",
            best
        ));
        ctx.say(format!("KILN_METRIC {name}_gb_s={gb_s:.1}"));
        results.push((name, gb_s));
    }
    let triad = results
        .iter()
        .find(|(n, _)| *n == "triad")
        .map(|(_, v)| *v)
        .unwrap_or(0.0);
    ctx.say(format!("memory bandwidth (triad): {triad:.1} GB/s"));
    Ok(0)
}
