//! Built-in tests. Each file defines one test type; `all()` lists them in the
//! order the picker shows them.
//!
//! Adding a test: create a module with a struct implementing
//! [`crate::engine::Test`], then push it in `all()`. Give it a clear `tldr`
//! and a `description` that says what runs, what is measured and how the
//! verdict is decided. Mark it `destructive` if it writes to a block device.

mod bench;
mod btrfs_harness;
mod cpu_bench;
mod cpu_stress;
mod device;
mod disk_read;
mod fio_verify;
mod health_snapshot;
mod mem_bandwidth;
mod mem_latency;
mod ram_hash;
mod ram_stress;
mod stress_ng;

use std::sync::Arc;

use crate::engine::Test;

pub fn all() -> Vec<Arc<dyn Test>> {
    vec![
        Arc::new(cpu_stress::CpuStress::new()),
        Arc::new(ram_stress::RamStress::new()),
        Arc::new(ram_hash::RamHash::new()),
        Arc::new(fio_verify::FioVerify::new()),
        Arc::new(btrfs_harness::BtrfsHarness::new()),
        Arc::new(health_snapshot::HealthSnapshot::new()),
        Arc::new(cpu_bench::CpuBench::new()),
        Arc::new(mem_bandwidth::MemBandwidth::new()),
        Arc::new(mem_latency::MemLatency::new()),
        Arc::new(disk_read::DiskRead::new()),
    ]
}
