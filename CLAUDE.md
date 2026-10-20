# kiln

Hardware burn-in and diagnostics runner: a Rust binary with a headless CLI and a ratatui TUI. It wraps external tools (stress-ng, fio, btrfs) and in-process tests, guards them (temperature cap etc.), and records results.

## Layout

- `src/engine/` the core. `test.rs` (Test trait, Job, parsers/verdicts), `params.rs` (ParamSpec/ParamSet, validation, layering defaults < profile < overrides), `runner.rs` (executes a Plan, emits `Event`s), `sampler.rs` (hwmon/cpufreq/proc readers), `guards.rs`, `result.rs` (Run/TestResult JSON), `plan.rs` (Plan + Profile TOML), `env.rs` (machine snapshot), `deps.rs` (dependency checks and install hints).
- `src/suite/` one file per test. `device.rs` holds the destructive-target safety check. `mod.rs::all()` is the display order.
- `src/cli.rs` clap subcommands. `src/tui/` the interface: `app.rs` event loop and `Effect` application, `screens/` one file per screen, `widgets.rs` shared pieces, `input.rs` key/mouse mapping and the click `HitMap`.
- `src/app.rs` shared bootstrap (`AppState`): paths, config, registry, env snapshot, logging.
- `src/config/` `paths.rs` (XDG dirs, sudo-aware) and `settings.rs` (AppConfig TOML).

## Rules

- Arrow keys and h/j/k/l are navigation only. Never bind them to actions.
- No default device for destructive tests. Keep `check_destructive_target` in the path of every test that writes to a block device.
- Tests must have `name`, `tldr`, full `description` (what runs, what is measured, how pass/fail is decided) and `uses`.
- Engine must not know about terminals. New UI needs go through `Event`/`Effect`.
- Config lives in `~/.config/kiln`, data in `~/.local/share/kiln`. When run via sudo, `Paths` uses `SUDO_USER`'s home and chowns created files back.
- Before every commit, grep for personal data (passwords, IPs, hostnames, serials, emails). Only the git author email is acceptable.
- Commits: short imperative subject, no trailer lines, no co-author lines. Atomic, and the tree must build and launch at every commit. Never push.
- Update `CHANGELOG.md` (Keep a Changelog) with user-visible changes.
- Comment concisely for humans; prefer readable code over clever shorthand.

## Commands

```
cargo build            # debug build
cargo run -- doctor    # check tools and sensors
cargo test             # unit tests + tests/tui_render.rs
cargo clippy --all-targets
cargo fmt
```

TUI cannot be driven in this environment without a TTY. Use `tests/tui_render.rs` (ratatui TestBackend) for layout checks, and the headless CLI for behaviour.

## Adding a test

1. New file in `src/suite/`, struct implementing `Test`. Build a `Job` in `plan()`: `JobKind::Command` for external tools (output streamed and parsed line by line) or `JobKind::Builtin` for Rust code.
2. Emit metrics as `KILN_METRIC key=value` lines or via a custom `parser`.
3. Register in `suite::all()`. Add a unit test for the parser and verdict.
