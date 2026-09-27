---
name: bossbar
description: Show and update system-wide progress bars in a floating pill so the
  user can watch long-running work from any app. Use when starting, tracking, or
  finishing multi-step or long-running tasks (builds, installs, deploys,
  downloads, test suites, migrations, agent runs), when the user asks for a
  progress/status display, or when work completes and the user should see what
  was done.
---

# bossbar

`bossbar` puts progress bars in a frameless, always-on-top pill at the top of
the user's active screen. A GUI-less daemon (`bossbar-daemon`) draws it; the
`bossbar` CLI creates and updates bars over a local socket. The daemon starts
on demand with the first command — there is nothing to set up.

Use it for work that takes more than a few seconds, or whenever the user asked
to be kept informed. Bars are for the user's eyes: keep them short, current and
honest.

## The core workflow

```bash
# 1. Announce the work and get an id (or pass --id to choose one).
bossbar create "Building wire-app" --id build --percent 0

# 2. Move it as work proceeds. Updates are cheap; a few per second is fine.
bossbar update build --percent 45 --detail "linking 214 crates"

# 3. Finish cleanly. `finish` fills the bar, flashes green, then removes it.
bossbar finish build
# On failure, keep the reason visible:
bossbar fail build --message "linker error: undefined symbol"
```

Always end a bar with `finish` or `fail`; a forgotten bar spins forever and
the user has to clean it up. If you abort a task, `bossbar remove <id>`.

## Command reference

| Command | Notes |
| --- | --- |
| `bossbar create <label> [--id ID] [--percent N \| --ticks N [--tick K] \| --indeterminate] [--detail D] [--color C]` | Prints the id. Use a stable `--id` so later updates are one command. |
| `bossbar update <id> [--label L] [--percent N] [--tick K] [--detail D] [--color C] [--status running\|done\|failed]` | Patch any field. `--status done/failed` fills and retires the bar. |
| `bossbar tick <id> [--by N \| --to N]` | Advance a ticks bar (default +1). |
| `bossbar finish <id> [--hold 1600ms]` | Success flash, then removal. |
| `bossbar fail <id> [--message M] [--hold 4200ms]` | Failure flash with a reason. |
| `bossbar remove <id>` / `bossbar clear` | Remove one / all. |
| `bossbar list [--json]` | Human table, or `{bars, collapsed, collapse_mode, anchor, visible, padding}`. |
| `bossbar wait <id> [--timeout 30m]` | Block until the bar disappears. Exit 0 finished, 1 failed, 1 timeout (message says). |
| `bossbar run [--label L] [--ticks N] [--color C] -- <cmd…>` | Runs the command, streams its output, and drives the bar: one tick per line with `--ticks`, last line as the detail otherwise. Exits with the command's code and finishes/fails the bar. |
| `bossbar collapse on\|off\|toggle` | Expand so the user can see per-task labels and percentages, or collapse to save space. |
| `bossbar collapse-mode normal\|compact` | Collapsed style: `normal` = one ring per task, `compact` = one shared ring with the aggregate. |
| `bossbar position top-center\|top-right` | Anchor on the user's active screen. |
| `bossbar padding on\|off\|toggle` | `off` (default) = flush with the screen edge; `on` = a little breathing room. |
| `bossbar visible on\|off\|toggle` | Hide the pill without touching the bars. |
| `bossbar daemon start\|stop\|restart\|status` | Lifecycle; rarely needed. |

Global flags: `--json` (machine-readable output for every command) and
`--no-spawn` (never auto-start the daemon).

Bar modes:

- `--percent N` — smooth 0-100 bar for measurable progress.
- `--ticks N [--tick K]` — Minecraft-style segmented bar (1-60 ticks) for a
  known number of steps; `bossbar tick` advances it.
- `--indeterminate` — moving sheen when the duration is unknown.

Colors: `white`, `blue`, `cyan`, `green`, `amber`, `red`, `violet`, `pink`, or
`#rrggbb`. Omit `--color` for the default white; done/failed bars recolor
themselves green/red automatically.

## Recipes

**Wrap a long command** (preferred when the work is a shell command — the bar
tracks output and the exit code automatically):

```bash
bossbar run --label "Test suite" --ticks 24 -- cargo test
```

**Track parallel jobs** with one bar each:

```bash
bossbar create "Fetch deps" --id deps --percent 0
bossbar create "Compile" --id compile --percent 0 --detail "waiting for deps"
# ...update each as it progresses, finish each when done...
```

For a non-blocking check, read `bossbar list --json` and inspect each bar's
`status`, `percent` and `tick`. `bossbar wait <id>` blocks until that bar is
gone: exit `0` finished, `1` failed, `2` timed out.

**Signal completion**: when a task the user cared about finishes, expand the
pill so they see which one it was, then retire it:

```bash
bossbar collapse off        # user can now read labels/percentages
bossbar update build --status done   # fills + flashes, then disappears
```

**Keep the label current**, not just the number: agents often know the current
step, so put it in `--detail` (`--detail "running migrations 4/12"`). The pill
truncates gracefully; keep labels under ~40 characters.

## Pitfalls

- Bars live in the daemon's memory only: a daemon restart clears them, and
  `kill -9` on your own process can leave a bar behind — clean up with
  `bossbar remove <id>` or `bossbar clear`.
- Throttle updates. The pill animates towards new values, so 5-10 updates per
  second are plenty; a tight loop of `bossbar update` is wasteful.
- Don't create a bar for work under a couple of seconds — it flashes and
  distracts.
- `bossbar wait` uses polling (250 ms default); it is safe for long waits but
  do not spawn dozens of them.
- The daemon's data dir (`~/Library/Application Support/bossbar` on macOS,
  `~/.local/share/bossbar` elsewhere) holds `daemon.json` and logs; if a
  command reports the daemon unreachable, `bossbar daemon restart` fixes it.
