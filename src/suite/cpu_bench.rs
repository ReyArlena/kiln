//! CPU throughput: a fixed mix of integer, floating point and hashing work,
//! single-threaded and then on every thread.

use std::hint::black_box;
use std::time::Instant;

use anyhow::Result;
use sha2::{Digest, Sha256};

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{BuiltinCtx, Category, GraphSpec, Job, JobKind, Test, TestInfo};
use crate::engine::{Dep, EnvSnapshot};

use super::bench::{self, Rng};

pub struct CpuBench {
    info: TestInfo,
}

impl CpuBench {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "cpu-bench",
                name: "CPU Throughput",
                tldr: "Work units per second, one thread and all threads. Throttling, a lost core or a wrong boost clock show up as a drop.",
                description: "\
Runs a fixed unit of work made of three parts: a prime sieve (integer and \
branching), a 128x128 double-precision matrix multiply (floating point) and \
SHA-256 over 64 MB (streaming integer). Each part works on a buffer small \
enough to stay in the core's own cache, so the score measures the cores \
rather than memory bandwidth. The unit runs repeatedly on one thread for \
the single-core score, then on every thread in parallel for the all-core \
score. Both are reported as units per second, plus the time of each part.

Use it as a before/after figure on the same machine: a lower all-core \
score with an unchanged single-core score usually means thermal or power \
throttling under sustained load (watch the frequency graph); both lower \
means a clock or BIOS change.

Pure Rust. Compare only against other kiln runs from a release build.

Pass: always.",
                category: Category::Benchmark,
                uses: &["in-process sieve, matrix multiply and SHA-256"],
                destructive: false,
            },
        }
    }
}

impl Default for CpuBench {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for CpuBench {
    fn info(&self) -> &TestInfo {
        &self.info
    }

    fn deps(&self) -> Vec<Dep> {
        vec![]
    }

    fn params(&self) -> Vec<ParamSpec> {
        vec![
            ParamSpec {
                key: "units",
                label: "Units per thread",
                help: "Work units each thread runs; more takes longer and steadies the number",
                kind: ParamKind::Int {
                    default: 10,
                    min: 1,
                    max: 1000,
                    zero_means_auto: false,
                },
            },
            ParamSpec {
                key: "threads",
                label: "Threads",
                help: "For the all-core score. 0 uses every thread",
                kind: ParamKind::Int {
                    default: 0,
                    min: 0,
                    max: 1024,
                    zero_means_auto: true,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let units = params["units"].as_i64().unwrap_or(10).max(1) as usize;
        let threads = bench::thread_count(params["threads"].as_i64().unwrap_or(0), env.cpu_count);
        Ok(Job {
            kind: JobKind::Builtin(Box::new(move |ctx| {
                Box::pin(async move {
                    tokio::task::spawn_blocking(move || run(&ctx, units, threads)).await?
                })
            })),
            graphs: vec![GraphSpec::metric("units/s", "multi_units_per_s")],
            expected_secs: None,
            snapshots: vec![],
            verdict: Box::new(bench::verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

fn run(ctx: &BuiltinCtx, units: usize, threads: usize) -> Result<i32> {
    bench::warn_if_debug_build(ctx);

    // Single thread: time each part once for the breakdown, then whole units.
    let (sieve_s, matrix_s, hash_s) = (time(sieve), time(matrix), time(hash));
    ctx.say(format!(
        "parts: sieve {sieve_s:.3}s  matrix {matrix_s:.3}s  sha256 {hash_s:.3}s"
    ));
    ctx.say(format!(
        "KILN_METRIC sieve_s={sieve_s:.3} matrix_s={matrix_s:.3} sha256_s={hash_s:.3}"
    ));

    let started = Instant::now();
    for _ in 0..units {
        if ctx.cancelled() {
            return Ok(130);
        }
        unit();
    }
    let single = units as f64 / started.elapsed().as_secs_f64();
    ctx.say(format!("single thread: {single:.2} units/s"));
    ctx.say(format!("KILN_METRIC single_units_per_s={single:.2}"));

    if ctx.cancelled() {
        return Ok(130);
    }
    let started = Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                for _ in 0..units {
                    unit();
                }
            });
        }
    });
    let multi = (units * threads) as f64 / started.elapsed().as_secs_f64();
    ctx.say(format!(
        "all threads ({threads}): {multi:.2} units/s  ({:.1}x single)",
        multi / single.max(1e-9)
    ));
    ctx.say(format!(
        "KILN_METRIC multi_units_per_s={multi:.2} threads={threads} scaling={:.2}",
        multi / single.max(1e-9)
    ));
    Ok(0)
}

fn time(work: fn()) -> f64 {
    let started = Instant::now();
    work();
    started.elapsed().as_secs_f64()
}

fn unit() {
    sieve();
    matrix();
    hash();
}

/// Primes below 2 million (a 2 MB array, cache resident), sieved 12 times over.
fn sieve() {
    const N: usize = 2_000_000;
    let mut composite = vec![false; N + 1];
    let mut count = 0usize;
    for _ in 0..12 {
        composite.fill(false);
        for i in 2..=N {
            if composite[i] {
                continue;
            }
            count += 1;
            let mut j = i * i;
            while j <= N {
                composite[j] = true;
                j += i;
            }
        }
    }
    black_box(count);
}

/// Multiply two 128x128 matrices 60 times (about 125 million multiply-adds).
fn matrix() {
    const N: usize = 128;
    let mut rng = Rng::new(7);
    let a: Vec<f64> = (0..N * N)
        .map(|_| (rng.next() % 1000) as f64 / 1000.0)
        .collect();
    let b: Vec<f64> = (0..N * N)
        .map(|_| (rng.next() % 1000) as f64 / 1000.0)
        .collect();
    let mut c = vec![0.0f64; N * N];
    for _ in 0..60 {
        for i in 0..N {
            for k in 0..N {
                let aik = a[i * N + k];
                for j in 0..N {
                    c[i * N + j] += aik * b[k * N + j];
                }
            }
        }
    }
    black_box(&c);
}

/// SHA-256 over 64 MB, fed from a 1 MB cache-resident buffer.
fn hash() {
    let mut rng = Rng::new(11);
    let data: Vec<u8> = (0..(1 << 20) / 8)
        .flat_map(|_| rng.next().to_le_bytes())
        .collect();
    let mut hasher = Sha256::new();
    for _ in 0..64 {
        hasher.update(&data);
    }
    black_box(hasher.finalize());
}
