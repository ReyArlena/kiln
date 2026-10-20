//! kiln: a hardware burn-in and diagnostics runner.
//!
//! The crate is split into an engine (test definitions, guards, sampling,
//! running, results) and two front ends over it: a headless CLI and a TUI.
//! Nothing in the engine knows about terminals, so both front ends consume
//! the same stream of [`engine::Event`]s.

pub mod app;
pub mod cli;
pub mod config;
pub mod engine;
pub mod report;
pub mod suite;
pub mod tui;
