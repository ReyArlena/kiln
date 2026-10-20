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
- `install.sh` with `--uninstall` and `--purge`.
- Dashboard banner with spinner, test position and progress.
- Dashboard graphs one row per series: CPU temperature always, plus what the test declares (core frequency, NVMe temperature of the target drive, hash throughput).
- Launch popup explaining which tests need root.
- GitHub Actions: CI (fmt, clippy, tests) and a release workflow that attaches a static musl binary to `v*` tags.

### Changed

- Status bar: fixed-width coloured values, red `not root` badge, NVMe drives named per controller.
- Log panes wrap long lines; temperature graph fills from the right and spans 30C to the cap.
- Screen title line removed; toasts now appear in the top-right corner.
- Temperatures show a degree symbol; the header shows the active cap as `CPU 40°C/70°C` while a run is going; the dashboard clock stops when a test ends.
