# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `kiln --version` prints the repository URL under the version so an installed binary can be traced back to its source.

## [0.1.0] - 2026-10-20

First release.

### Added

- Engine: test registry, parameter specs with validation, guards (temperature cap, time cap, kernel log patterns, stop on first failure, cooldown), sysfs sensor sampler, sequential runner with SIGINT then SIGKILL escalation, per-test logs, and `run.json` written after every test so a crash still leaves a readable run.
- Tests never outlive kiln: children get SIGTERM from the kernel if kiln dies (including SIGKILL); quitting or losing the terminal stops the running test and still writes `run.json`.
- Tests: `cpu-stress`, `ram-stress`, `ram-hash`, `fio-verify`, `btrfs-harness`, `health-snapshot`. Each has a name, a one-line summary, a full description of what runs and how pass/fail is decided, and a list of what it uses.
- Benchmarks: `cpu-bench` (single and all-thread units/s), `mem-bandwidth` (STREAM-style GB/s), `mem-latency` (pointer-chase ns), `disk-read` (fio read-only MB/s and IOPS). `kiln compare <run> <run>` puts two runs side by side with percentage change.
- Plans: five built in (Quick Check, New Build Shakedown, Overnight Burn-in, Storage Shakedown, Benchmarks), saved plans as TOML, `kiln plans`, `kiln run --plan <name>`.
- Safety: destructive tests need an explicit device, refuse mounted devices and the system disk, and require `--yes-destroy` (CLI) or typing the device name (TUI). The device picker greys out disks that cannot be wiped and shows a red banner; read-only benchmarks get a green one. `kiln devices` prints the same list.
- Headless CLI: `doctor`, `list`, `info`, `run`, `plans`, `devices`, `runs`, `report`, `compare`, `log`, `profile`, `config`. Exit codes 0 pass, 1 fail, 3 aborted or skipped. Ctrl-C stops the current test, twice stops the run.
- TUI: Plans and Tests picker, Setup form (parameters, repeats, guards, profiles, save as plan), live dashboard with braille graphs per component (CPU temperature, usage, frequency, memory, CPU and GPU watts, every NVMe, GPU) plus test metrics, kernel log, results with log viewer, history, settings with developer tools. Arrows and h/j/k/l are navigation only; mouse click and wheel supported.
- Persistent status bar with fixed-width coloured values, the active temperature cap while running, CPU and GPU power, NVMe temperatures per controller, and a red badge when not root.
- Launch popups: offer to relaunch through sudo when not root; list of tools with tick or cross and one distro-specific install command (`c` copies it).
- Settings in `~/.config/kiln`, runs in `~/.local/share/kiln`, resolved from the invoking user under sudo.
- `install.sh`: downloads the prebuilt binary for the current architecture, to `~/.local/bin` or, with sudo, `/usr/local/bin`; `--uninstall` and `--purge`.
- GitHub Actions: CI (fmt, clippy, tests) and a release workflow that attaches static `kiln-linux-x86_64` and `kiln-linux-aarch64` binaries to `v*` tags.

[Unreleased]: https://github.com/ReyArlena/kiln/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ReyArlena/kiln/releases/tag/v0.1.0
