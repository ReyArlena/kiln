//! Events the runner emits. Both the TUI and the headless logger consume these.

use super::guards::Trip;
use super::result::{Run, TestResult};
use super::sampler::Sample;

#[derive(Debug, Clone)]
pub enum Event {
    /// Test `index` in the plan is about to start.
    TestStarted {
        index: usize,
        test_id: String,
        name: String,
        expected_secs: Option<u64>,
    },
    /// A line of stdout/stderr from the job.
    Line {
        index: usize,
        text: String,
    },
    /// A metric parsed from the output.
    Metric {
        index: usize,
        key: String,
        value: f64,
    },
    /// A kernel log line observed while running.
    Dmesg {
        text: String,
    },
    /// A guard decided to stop the current test.
    GuardTripped {
        index: usize,
        trip: Trip,
    },
    /// Waiting between tests.
    Cooldown {
        secs_left: u64,
    },
    TestFinished {
        index: usize,
        result: TestResult,
    },
    /// Periodic sensor reading.
    Sample(Sample),
    /// Everything is done and written to disk.
    RunFinished(Run),
}
