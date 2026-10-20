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

`kiln info <id>` prints the full breakdown of each one.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh
```

Builds with cargo if you have it, otherwise downloads a release binary. Installs to `~/.local/bin/kiln`. Uninstall with `sh install.sh --uninstall` (or `--purge` to also delete settings and history).

Tools the tests call, on Arch:

```sh
sudo pacman -S stress-ng fio nvme-cli smartmontools btrfs-progs
```

`kiln doctor` lists what is missing and how to install it on your distro.

## Use

Most tests need root. `sudo` resets `PATH`, so give the full path:

```sh
sudo ~/.local/bin/kiln            # TUI
sudo ~/.local/bin/kiln doctor     # tools, sensors, permissions
```

Headless:

```sh
kiln list                                              # tests and one-line summaries
kiln info btrfs-harness                                # full description, parameters, deps
kiln run cpu-stress ram-hash --set duration=20m --temp-max 88
sudo kiln run fio-verify --set device=/dev/nvme1n1 --set size=20G --yes-destroy
kiln runs                                              # history
kiln report 2026-10-20T17-07-07 > report.md
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

### Release

Tag and push; GitHub Actions builds static binaries for x86_64 and aarch64 (`kiln-linux-<arch>`) and attaches them to the release, which is what `install.sh` downloads on machines without cargo:

```sh
git tag v0.1.0 && git push origin v0.1.0
```

CI runs `cargo fmt --check`, clippy and the tests on every push.

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
