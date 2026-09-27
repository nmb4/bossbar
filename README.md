# bossbar

System-wide progress bars in a floating pill, driven by a GUI-less background
daemon you (or an agent) talk to from the CLI.

```
┌──────────────────────────────────┐
│  ● 3 TASKS                  36% ⌃│
│  ● Building wire-app        38%  │
│  ▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░ │
│  cargo build --release · 214 crates
│  ● Compiling bossbar         7/20│
│  ▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░░░░░░░░░ │
│  ● Waiting for device            │
│  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░░░░░░ │
└──────────────────────────────────┘
```

- A frameless, always-on-top **pill** anchored top-center or top-right of the
  **active screen** (the monitor your cursor is on). Dark Alcove-style surface,
  all bars and labels in one pill.
- **Minecraft-style boss bars**: `--ticks 20` splits the bar into segments and
  `--tick N` advances them.
- **Collapse toggle**: with more than one bar, squeeze the pill into a single
  line with a progress ring (aggregate percentage for determinate bars, a
  spinning arc otherwise).
- **GUI-less daemon**: no Dock icon, no main window, a tray icon only. 0% CPU
  while idle or when the pill is static; the window is created/hidden on
  demand.
- **CLI-first**: every bar is created, updated, finished or removed with one
  command, and `--json` output makes it easy to script from agents.

## Install

```bash
cargo build --release
# put both binaries on PATH, e.g.:
install -m 755 target/release/bossbar target/release/bossbar-daemon ~/.local/bin/
```

## Quick start

```bash
# The daemon starts on demand; no setup required.
bossbar create "Building wire-app" --id build --percent 5 --detail "cargo build --release"

bossbar update build --percent 47
bossbar finish build            # fills, flashes success, disappears

bossbar create "Compiling bossbar" --id compile --ticks 20 --tick 0
bossbar tick compile --by 3

bossbar create "Waiting for device" --id wait --indeterminate --color violet

bossbar list                    # human table; --json for machines
bossbar wait compile            # blocks until the bar is gone
bossbar run --ticks 8 -- make -j8   # drive a bar from a command's output
bossbar daemon stop
```

Run `bossbar --help` and `bossbar <command> --help` for everything.

## How it works

```
bossbar (CLI) ──loopback TCP + token──▶ bossbar-daemon (eframe/wgpu)
                                             │
                     tray icon ◀─────────────┤
                     pill viewport ◀─────────┘
```

- The daemon listens on a random loopback port and publishes
  `daemon.json` (pid, port, token) under the per-user data directory:
  - macOS: `~/Library/Application Support/bossbar/`
  - Linux: `~/.local/share/bossbar/`
  - Windows: `%APPDATA%\bossbar\`
- The CLI auto-spawns the daemon (`BOSSBAR_DAEMON` overrides the binary path,
  `--no-spawn` disables auto-start).
- Bars are ephemeral by design: the daemon keeps them in memory and removes
  them when finished/failed or on request.
- On macOS the daemon runs as an accessory app (no Dock icon) and its window is
  created non-activating, so it never steals focus. On Windows the window is
  trimmed to the pill with a native region, so no per-pixel alpha is needed.
- Tray menu: show/hide bars, collapse/expand, position, clear all, quit.
  Left-clicking the tray icon toggles visibility.

## CLI reference (short)

| Command | What it does |
| --- | --- |
| `bossbar create <label> [--id ID] [--percent N \| --ticks N [--tick K] \| --indeterminate] [--detail D] [--color C]` | New bar; prints its id |
| `bossbar update <id> [--label L] [--percent N] [--tick K] [--detail D] [--color C] [--status running\|done\|failed]` | Patch a bar |
| `bossbar tick <id> [--by N \| --to N]` | Advance a ticks bar |
| `bossbar finish <id> [--label L] [--hold 1600ms]` | Complete + retire a bar |
| `bossbar fail <id> [--message M] [--hold 4200ms]` | Fail + retire a bar |
| `bossbar remove <id>` / `bossbar clear` | Remove one / all |
| `bossbar list [--json]` | Current state |
| `bossbar wait <id> [--timeout 30m]` | Block until the bar disappears (exit 1 if it failed, 2 on timeout) |
| `bossbar collapse [on\|off\|toggle]` | Collapse the pill |
| `bossbar position <top-center\|top-right>` | Anchor on the active screen |
| `bossbar visible [on\|off\|toggle]` | Show/hide without touching bars |
| `bossbar run [--label L] [--ticks N] [--color C] -- <cmd…>` | Run a command; one tick (or a label update) per output line |
| `bossbar daemon <start\|stop\|restart\|status>` | Manage the daemon |

Colors: `white`, `blue`, `cyan`, `green`, `amber`, `red`, `violet`, `pink`,
or `#rrggbb`.

## Agent usage notes

- Use stable ids (`--id build`) so updates are one command.
- `bossbar list --json` returns `{bars, collapsed, anchor, visible}`; each bar
  has `kind` (`{"type":"ticks","total":N}`), `percent`, `tick`, `status`,
  `detail`, and `color`.
- `bossbar wait <id>` is the cheap way to block until a task is finished.
- `bossbar run --ticks N -- <cmd>` wraps a build/deploy without extra
  bookkeeping: the exit code is the command's, and the bar is finished or
  failed automatically.
- Bars are not persisted; a daemon restart clears them.

## Development

```bash
cargo test --workspace         # unit + snapshot tests
cargo run -p bossbar-daemon    # run the daemon in the foreground (logs to stderr and file)
cargo run -p bossbar-cli -- list
```

Snapshot tests render the real pill code offscreen (`egui_kittest`) and compare
against `crates/bossbar-daemon/tests/snapshots/*.png`. After intentional visual
changes run `UPDATE_SNAPSHOTS=1 cargo test -p bossbar-daemon` and review the
PNGs.

`cargo run -p bossbar-daemon --example window_probe` prints the pill window's
geometry and the monitor layout — handy for verifying placement without
screenshots.

## License

MIT. Bundled Inter font: SIL Open Font License (see
`crates/bossbar-daemon/fonts/OFL.txt`).
