//! Repeated hashing of a large in-memory buffer. Pure Rust, no external tools.

use std::time::Instant;

use anyhow::{Result, bail};
use sha2::{Digest, Sha256};

use crate::engine::params::{ParamKind, ParamSet, ParamSpec};
use crate::engine::test::{
    BuiltinCtx, Category, GraphSpec, Job, JobKind, Outcome, Test, TestInfo, Verdict,
};
use crate::engine::util::format_bytes;
use crate::engine::{Dep, EnvSnapshot};

pub struct RamHash {
    info: TestInfo,
}

impl RamHash {
    pub fn new() -> Self {
        Self {
            info: TestInfo {
                id: "ram-hash",
                name: "RAM hash loop",
                tldr: "Hashes the same block of RAM over and over. A changing hash means data in memory is not stable.",
                description: "\
Allocates a buffer of the requested size, fills it with a deterministic \
pseudo-random pattern, then computes its SHA-256 repeatedly. Every pass must \
produce the same digest as the first. Unlike the stress-ng vm test this \
keeps the data resident and untouched between reads, so it catches bit \
flips that happen while memory simply sits there, as well as read-path \
errors.

Runs in-process, so it needs no external tools and works on any live ISO.

Measures: passes completed, throughput in MB/s, max temperature.

Pass: every digest identical. Fail: any digest differs, or the buffer \
could not be allocated. Abort: guard cap.",
                category: Category::Memory,
                uses: &[
                    "in-process SHA-256 (sha2 crate)",
                    "hwmon temperature sensors",
                ],
                destructive: false,
            },
        }
    }
}

impl Default for RamHash {
    fn default() -> Self {
        Self::new()
    }
}

impl Test for RamHash {
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
                help: "Memory to hold, e.g. 8G. Leave room for the OS",
                kind: ParamKind::Bytes { default: 4 << 30 },
            },
            ParamSpec {
                key: "passes",
                label: "Passes",
                help: "How many times to hash the buffer",
                kind: ParamKind::Int {
                    default: 20,
                    min: 1,
                    max: 100_000,
                    zero_means_auto: false,
                },
            },
        ]
    }

    fn plan(&self, params: &ParamSet, env: &EnvSnapshot) -> Result<Job> {
        let size = params["size"].as_u64().unwrap_or(4 << 30);
        let passes = params["passes"].as_u64().unwrap_or(20);
        if env.mem_total_bytes > 0 && size > env.mem_total_bytes * 9 / 10 {
            bail!(
                "buffer of {} exceeds 90% of RAM ({})",
                format_bytes(size),
                format_bytes(env.mem_total_bytes)
            );
        }
        Ok(Job {
            kind: JobKind::Builtin(Box::new(move |ctx| Box::pin(hash_loop(ctx, size, passes)))),
            graphs: vec![GraphSpec::metric("MB/s", "mb_per_sec")],
            expected_secs: None,
            snapshots: vec![],
            verdict: Box::new(verdict),
            parser: Box::new(|_| Vec::new()),
        })
    }
}

fn verdict(outcome: &Outcome) -> Verdict {
    match outcome.exit_code {
        Some(0) => Verdict::Pass,
        Some(1) => Verdict::Fail(
            outcome
                .output
                .iter()
                .rev()
                .find(|l| l.starts_with("MISMATCH"))
                .cloned()
                .unwrap_or_else(|| "digest mismatch".into()),
        ),
        Some(code) => Verdict::Fail(format!("hash loop exited with code {code}")),
        None => Verdict::Fail("hash loop did not finish".into()),
    }
}

/// The builtin body. Runs on a blocking thread because hashing is CPU bound.
async fn hash_loop(ctx: BuiltinCtx, size: u64, passes: u64) -> Result<i32> {
    tokio::task::spawn_blocking(move || hash_loop_blocking(&ctx, size, passes)).await?
}

fn hash_loop_blocking(ctx: &BuiltinCtx, size: u64, passes: u64) -> Result<i32> {
    ctx.say(format!("allocating {}", format_bytes(size)));
    let mut buffer: Vec<u8> = Vec::new();
    if buffer.try_reserve_exact(size as usize).is_err() {
        ctx.say("ALLOC_FAIL could not reserve buffer");
        return Ok(2);
    }
    fill_pattern(&mut buffer, size as usize);
    ctx.say("filled, hashing");

    let mut reference: Option<[u8; 32]> = None;
    for pass in 1..=passes {
        if ctx.cancelled() {
            ctx.say(format!("cancelled after {} passes", pass - 1));
            return Ok(130);
        }
        let started = Instant::now();
        let digest = sha256(&buffer);
        let secs = started.elapsed().as_secs_f64().max(1e-9);
        let mb_per_sec = size as f64 / (1024.0 * 1024.0) / secs;
        match reference {
            None => {
                reference = Some(digest);
                ctx.say(format!(
                    "pass {pass}/{passes} {} ({mb_per_sec:.0} MB/s) reference",
                    hex(&digest)
                ));
            }
            Some(expected) if expected == digest => {
                ctx.say(format!("pass {pass}/{passes} ok ({mb_per_sec:.0} MB/s)"));
            }
            Some(expected) => {
                ctx.say(format!(
                    "MISMATCH on pass {pass}: expected {} got {}",
                    hex(&expected),
                    hex(&digest)
                ));
                ctx.say(format!("KILN_METRIC passes={pass} mismatches=1"));
                return Ok(1);
            }
        }
        ctx.say(format!(
            "KILN_METRIC passes={pass} mb_per_sec={mb_per_sec:.0} mismatches=0"
        ));
    }
    Ok(0)
}

/// Fast deterministic fill (xorshift64*), chunked so it is cache friendly.
fn fill_pattern(buffer: &mut Vec<u8>, size: usize) {
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    buffer.resize(size, 0);
    for chunk in buffer.chunks_mut(8) {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let bytes = state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_is_deterministic_and_hash_is_stable() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        fill_pattern(&mut a, 1 << 16);
        fill_pattern(&mut b, 1 << 16);
        assert_eq!(a, b);
        assert_eq!(sha256(&a), sha256(&b));
        assert_ne!(a[..16], [0u8; 16]);
    }
}
