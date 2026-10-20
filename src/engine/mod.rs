//! The engine: everything needed to describe, run and record tests without
//! any knowledge of how results are displayed.

pub mod deps;
pub mod env;
pub mod event;
pub mod guards;
pub mod params;
pub mod plan;
pub mod presets;
pub mod registry;
pub mod result;
pub mod runner;
pub mod sampler;
pub mod test;
pub mod util;

pub use deps::{Dep, DepReport, DepStatus};
pub use env::EnvSnapshot;
pub use event::Event;
pub use guards::{Guards, Trip};
pub use params::{ParamError, ParamSet, ParamSpec, ParamValue};
pub use plan::{Plan, PlanEntry};
pub use registry::Registry;
pub use result::{Run, Status, TestResult};
pub use runner::{RunHandle, start_run};
pub use sampler::{Sample, SensorInfo};
pub use test::{GraphSource, GraphSpec, Job, JobKind, Snapshot, Test, TestInfo};
