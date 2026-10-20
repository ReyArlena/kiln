# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Engine: test registry, parameter specs with validation, guards (temperature cap, time cap, kernel log patterns, stop on fail, cooldown), sysfs sensor sampler, sequential runner with SIGINT then SIGKILL escalation, per-test logs and `run.json` written after every test.
- Tests: `cpu-stress`, `ram-stress`, `ram-hash`, `fio-verify`, `btrfs-harness`, `health-snapshot`.
- Safety: destructive tests refuse mounted devices and the system disk, need an explicit device, and require `--yes-destroy` (CLI) or a typed confirmation (TUI).
- Headless CLI: `doctor`, `list`, `info`, `run`, `runs`, `report`, `log`, `profile`, `config`.
- TUI: test picker, setup form, live dashboard with temperature sparkline and kernel log, results, history with log viewer, settings with developer tools. Keyboard (arrows and h/j/k/l reserved for navigation) and mouse.
- Persistent sensor status bar, configurable per item.
- Saved profiles per test and saved plans, as TOML under the config directory.
- Settings in `~/.config/kiln`, runs in `~/.local/share/kiln`, resolved from the invoking user when run through sudo.
- `install.sh`: downloads the prebuilt binary for the current architecture, to `/usr/local/bin` when run with sudo or `~/.local/bin` otherwise; `--uninstall` and `--purge`.
- Dashboard banner with spinner, test position and progress.
- Dashboard graphs: one braille row per component (CPU temperature, usage, frequency, memory, CPU and GPU watts, every NVMe, GPU), colour sliding green to red through each range, plus metrics the running test reports (e.g. MB/s). Each graph has a toggle in settings.
- CPU package power (RAPL, root only) and GPU power in the header.
- Launch popup explaining which tests need root, with an offer to relaunch through sudo.
- GitHub Actions: CI (fmt, clippy, tests) and a release workflow that attaches static `kiln-linux-x86_64` and `kiln-linux-aarch64` binaries to `v*` tags.

### Changed

- Tests never outlive kiln: children get SIGTERM from the kernel if kiln dies (including SIGKILL), quitting or losing the terminal (SIGHUP/SIGTERM) stops the running test and still writes `run.json`, and the btrfs harness interrupts its copy and scrub immediately.

- Status bar: fixed-width coloured values, red `not root` badge, NVMe drives named per controller.
- Log panes wrap long lines; temperature graph fills from the right and spans 30C to the cap.
- Screen title line removed; toasts now appear in the top-right corner.
- Temperatures show a degree symbol; the header shows the active cap as `CPU 40°C/70°C` while a run is going; the dashboard clock stops when a test ends.
