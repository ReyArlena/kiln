//! Memory latency: random pointer chasing through a buffer larger than cache.

use std::hint::black_box;
use std::time::Instant;

use anyhow::{Result, bail};

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{BuiltinCtx, Category, GraphSpec, Job, JobKind, Test, TestInfo};
use crate::engine::util::format_bytes;
use crate::engine::{Dep, EnvSnapshot};

use super::bench::{self, Rng};

pub struct MemLatency {
    info: TestInfo,
}

impl MemLatency {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "mem-latency",
                name: "Memory Latency",
                tldr: "Nanoseconds per random memory access. Sensitive to memory timings and the CPU's fabric clock.",
                description: "\
Builds a single random cycle through a buffer much larger than the CPU \
caches (one pointer per cache line, Sattolo's shuffle), then follows it \
for millions of steps on one thread. Every step depends on the previous \
one, so the CPU cannot prefetch or overlap them: the time per step is the \
real round trip to DRAM.

Typical DDR5 desktop figures are 70 to 100 ns. A jump against a previous \
run on the same machine points at looser timings, a lower fabric clock, or \
memory that fell back to a slower profile.

Also reports the same chase over a small buffer that fits in cache, as a \
sanity figure for the CPU itself.

Pure Rust. Compare only against other kiln runs from a release build.

Pass: always, unless the buffer could not be allocated.",
                category: Category::Benchmark,
                uses: &["in-process pointer chase, single thread"],
                destructive: false,
            },
        }
    }
}

impl Default for MemLatency {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for MemLatency {
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
                label: "Buffer size",
                help: "Must dwarf the L3 cache; 512M is safe on any desktop",
                kind: ParamKind::Bytes { default: 512 << 20 },
            },
            ParamSpec {
                key: "steps",
                label: "Steps (millions)",
                help: "Chase length; more steps, steadier number",
                kind: ParamKind::Int {
                    default: 50,
                    min: 1,
                    max: 10_000,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let size = params["size"].as_u64().unwrap_or(512 << 20);
        let steps = params["steps"].as_u64().unwrap_or(50) * 1_000_000;
        if env.mem_total_bytes > 0 && size > env.mem_total_bytes / 2 {
            bail!("{} is more than half of RAM", format_bytes(size));
        }
        Ok(Job {
            kind: JobKind::Builtin(Box::new(move |ctx| {
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || run(&ctx, size, steps)).await?
                })
            })),
            graphs: vec![GraphSpec::metric("ns", "dram_ns")],
            expected_secs: None,
            snapshots: vec![],
            verdict: Box::new(bench::verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

/// One cache line per node so each step is a fresh line.
const LINE: usize = 64;

fn run(ctx: &BuiltinCtx, size: u64, steps: u64) -> Result<i32> {
    bench::warn_if_debug_build(ctx);
    for (label, bytes, key) in [
        ("cache", 64 << 10, "cache_ns"),
        ("dram", size as usize, "dram_ns"),
    ] {
        if ctx.cancelled() {
            return Ok(130);
        }
        let Some(ns) = chase(bytes, steps) else {
            ctx.say("ALLOC_FAIL could not allocate buffer");
            return Ok(2);
        };
        ctx.say(format!(
            "{label:<6} {:>8}: {ns:6.1} ns per access",
            format_bytes(bytes as u64)
        ));
        ctx.say(format!("KILN_METRIC {key}={ns:.1}"));
    }
    Ok(0)
}

/// Build a random single cycle over `bytes / LINE` nodes and time `steps` hops.
fn chase(bytes: usize, steps: u64) -> Option<f64> {
    let nodes = (bytes / LINE).max(2);
    let mut next: Vec<usize> = Vec::new();
    next.try_reserve_exact(nodes * (LINE / size_of::<usize>()))
        .ok()?;
    next.resize(nodes * (LINE / size_of::<usize>()), 0);
    let stride = LINE / size_of::<usize>();

    // Sattolo: a random permutation that is one single cycle.
    let mut order: Vec<usize> = (0..nodes).collect();
    let mut rng = Rng::new(0x5EED);
    for i in (1..nodes).rev() {
        let j = (rng.next() % i as u64) as usize;
        order.swap(i, j);
    }
    for i in 0..nodes {
        next[order[i] * stride] = order[(i + 1) % nodes] * stride;
    }

    let mut pos = 0usize;
    let started = Instant::now();
    for _ in 0..steps {
        pos = next[pos];
    }
    let elapsed = started.elapsed().as_secs_f64();
    black_box(pos);
    Some(elapsed * 1e9 / steps as f64)
}
