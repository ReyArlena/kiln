# kiln

Hardware burn-in and diagnostics runner for Linux. Pick tests, set guards such as a temperature cap, run them in sequence, keep the results. Built after a long session hunting a faulty Ryzen I/O die: the tests here are the ones that found it.

- **Interactive TUI** with keyboard and mouse: picker, parameter form, live dashboard (temperature sparkline, output, kernel log), results, history, settings.
- **Headless CLI** for SSH sessions and live ISOs: same engine, plain text output, JSON results.
- **Guards**: temperature cap (reads hwmon directly), per-test time cap, kernel log patterns (MCE, AER, checksum errors), stop on first failure, cooldown between tests.
- **Records everything**: machine snapshot (CPU, board, BIOS, kernel, DIMMs, NVMe), parameters, metrics, sensor samples, per-test logs. `run.json` is written after every test so a crash still leaves a readable run.
- **Safe by default**: destructive tests need an explicit device, refuse mounted devices and the system disk, and ask for the device name to be typed back.

## Tests

| id | What it is good for | Wraps |
| --- | --- | --- |
| `cpu-stress` | Cores that compute wrong under load or heat | `stress-ng --cpu --verify` |
| `ram-stress` | Bad DIMMs, slots, unstable memory clocks (CPU side) | `stress-ng --vm --verify` |
| `ram-hash` | Bit flips in resident memory, no external tools | in-process SHA-256 loop |
| `fio-verify` | Data corrupted on the way to or from a drive (DMA path) | `fio --verify=crc32c --direct=1` |
| `btrfs-harness` | Reproduces OS-install corruption: format, copy, scrub, compare | `mkfs.btrfs`, `cp -a`, `btrfs scrub`, `cmp` |
| `health-snapshot` | SMART, PCIe link speed and kernel errors, before and after | `nvme smart-log`, `smartctl`, sysfs |
| `cpu-bench` | Units/s single and all threads; throttling or a lost core shows as a drop | in-process sieve, matrix, SHA-256 |
| `mem-bandwidth` | GB/s; single vs dual channel, JEDEC vs EXPO show here first | in-process STREAM kernels |
| `mem-latency` | ns per random DRAM access; timings and fabric clock | in-process pointer chase |
| `disk-read` | Sequential MB/s and 4k IOPS, read-only, safe on the system drive | `fio --readonly` |

Benchmarks are for comparing kiln runs against each other (`kiln compare <run> <run>`), on the same machine over time or across machines. They are not comparable to published STREAM or Geekbench numbers, and need a release build.

`kiln info <id>` prints the full breakdown of each one.

## Plans

A plan is a named sequence of tests with their parameters, repeats and guards. Five ship with kiln:

| plan | what | about |
| --- | --- | --- |
| `quick-check` | health snapshot, the benchmarks, short RAM hash | 5 min |
| `new-build-shakedown` | health, CPU and RAM stress with verification, RAM hash, benchmarks, health again | 1 h |
| `overnight-burn-in` | alternating CPU and RAM stress, long RAM hash, stop on first failure, 85°C cap | 8 h |
| `storage-shakedown` | **wipes a device**: fio verify, btrfs harness x2, health before and after | depends |
| `benchmarks` | CPU, memory bandwidth, memory latency, disk read | few min |

In the TUI the Plans pane sits above the test list; Enter opens one in Setup where every value can still be changed. `w` in Setup saves the current form as your own plan (`~/.config/kiln/plans/<name>.toml`, hand-editable). Headless:

```sh
kiln plans
kiln run --plan quick-check
kiln run --plan ~/my-plan.toml
```

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh
```

Downloads the prebuilt static binary for your architecture (x86_64 or aarch64) into `~/.local/bin/kiln`. No compiler, no root. Uninstall with `sh install.sh --uninstall`, or `--purge` to also delete settings and history. To build it yourself instead, see [Develop](#develop).

Most tests need root. Just run `kiln`: it notices it is not root and offers to relaunch itself through `sudo`. Settings and results always go to your own home, never root's.

> Prefer plain `sudo kiln` to work from any shell or script? Install system-wide instead: run the same line with `| sudo sh` and the binary goes to `/usr/local/bin`. On most distros `sudo` searches only system directories, which is why a per-user install needs the relaunch offer (or `sudo ~/.local/bin/kiln` by hand).

Tools the tests call, on Arch:

```sh
sudo pacman -S stress-ng fio nvme-cli smartmontools btrfs-progs
```

`kiln doctor` lists what is missing and how to install it on your distro.

## Use

```sh
kiln            # TUI, offers to relaunch as root
kiln doctor     # tools, sensors, permissions
```

Headless:

```sh
kiln list                                              # tests and one-line summaries
kiln info btrfs-harness                                # full description, parameters, deps
kiln run cpu-stress ram-hash --set duration=20m --temp-max 88
sudo ~/.local/bin/kiln run fio-verify --set device=/dev/nvme1n1 --set size=20G --yes-destroy
kiln devices                                           # disks to pick from
kiln runs                                              # history
kiln report 2026-10-20T17-07-07 > report.md
kiln compare 2026-10-20T17-07-07 2026-10-21T09-12-00     # metrics side by side with % change
kiln log 2026-10-20T17-07-07 1                         # log of test 1
```

Exit codes from `run`: 0 all passed, 1 a test failed, 3 aborted or skipped.

Ctrl-C once stops the current test and moves on, twice stops the run.

### Keys in the TUI

Arrows and `h j k l` move. `Tab` switches pane. `Space` toggles, `Enter` confirms or edits, `Esc` goes back, `?` shows help for the current screen, `q` quits. Click to select or toggle, click again to open, wheel to scroll. The hint bar at the bottom is clickable.

### Files

| Path | Purpose |
| --- | --- |
| `~/.config/kiln/config.toml` | settings (defaults for guards, sensor, status bar) |
| `~/.config/kiln/profiles/<test>/<name>.toml` | saved parameter sets per test |
| `~/.config/kiln/plans/*.toml` | saved sequences |
| `~/.local/share/kiln/runs/<id>/` | `run.json`, one log per test, optional `report.md` |
| `~/.local/share/kiln/kiln.log` | app log (`KILN_LOG=debug` for more) |

Under sudo these resolve to the invoking user's home, not root's.

Profiles: `kiln profile save cpu-stress long --set duration=2h` then `kiln run cpu-stress --profile long`, or cycle them with Space on the setup screen.

## Develop

Install the toolchain:

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh   # or: sudo pacman -S rustup && rustup default stable
rustup component add clippy rustfmt
```

Then:

```sh
git clone https://github.com/ReyArlena/kiln
cd kiln
cargo build                 # target/debug/kiln
cargo run -- doctor         # run it
cargo run -- run ram-hash --set size=256M --set passes=3
cargo build --release       # optimised, stripped binary in target/release/
```

Static binary for a live ISO:

```sh
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

### Tests

```sh
cargo test                  # all
cargo test engine::         # one module
cargo test -- --nocapture   # show output
cargo clippy --all-targets  # lints
cargo fmt                   # format
```

Unit tests live next to the code they cover (parsers, guards, parameter validation, config round-trips). `tests/tui_render.rs` draws each screen into an in-memory terminal to catch layout panics. The aim is a small suite that guards the pure logic, not hundreds of cases.

### Layout

```
src/engine/   test trait, params, guards, sampler, runner, results, plans
src/suite/    one file per test
src/cli.rs    headless commands
src/tui/      ratatui interface (app.rs loop, screens/, widgets.rs, input.rs)
src/config/   XDG paths (sudo-aware) and settings
```

See `CLAUDE.md` for conventions and how to add a test.

## License

MIT
