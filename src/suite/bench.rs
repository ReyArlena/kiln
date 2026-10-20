//! Shared pieces for the in-process benchmarks.

use crate::engine::test::{BuiltinCtx, Outcome, Verdict};

/// Benchmarks compare kiln runs against each other, so they are only
/// meaningful from an optimised build. Say so loudly in a debug build.
pub fn warn_if_debug_build(ctx: &BuiltinCtx) {
    if cfg!(debug_assertions) {
        ctx.say(
            "WARNING: debug build, numbers are far lower than a release build and not comparable",
        );
        ctx.say("KILN_METRIC debug_build=1");
    }
}

/// Benchmarks pass unless they could not run at all.
pub fn verdict(outcome: &Outcome) -> Verdict {
    match outcome.exit_code {
        Some(0) => Verdict::Pass,
        Some(code) => Verdict::Fail(format!("benchmark exited with code {code}")),
        None => Verdict::Fail("benchmark did not finish".into()),
    }
}

/// Threads to use for an all-core workload: the parameter, or every thread.
pub fn thread_count(param: i64, cpu_count: usize) -> usize {
    if param <= 0 {
        cpu_count.max(1)
    } else {
        param as usize
    }
}

/// Deterministic xorshift64* generator, enough for filling buffers.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}
