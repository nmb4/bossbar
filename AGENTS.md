# AGENTS.md

## What this is

`bossbar` is a system-wide progress-bar pill: a GUI-less daemon (`bossbar-daemon`)
draws a frameless always-on-top window and exposes a loopback JSON API; a CLI
(`bossbar`) creates and updates bars. No main window, no Dock icon; the tray
icon is the only permanent UI.

## Layout

```
crates/bossbar-proto/   wire types shared by CLI and daemon (serde, no GUI deps)
crates/bossbar-daemon/  eframe/wgpu daemon
  src/main.rs           logging, single-instance lock, startup/shutdown
  src/bars.rs           bar model + store; pure logic, unit-tested
  src/state.rs          request -> store mutation + UI events; unit-tested
  src/ipc.rs            loopback TCP server, daemon.json publish/cleanup
  src/platform.rs       monitor/cursor lookup, window placement, Windows region
  src/view.rs           pill layout + painting; snapshot-tested
  src/app.rs            window lifecycle (show/hide/animate) and tray wiring
  src/tray.rs           tray icon + menu
  src/event_loop.rs     winit bootstrap (macOS accessory policy, ControlFlow::Wait)
  src/waker.rs          cross-thread repaint wakeups
  examples/window_probe.rs  prints pill window geometry (no screenshots needed)
crates/bossbar-cli/     CLI client, auto-spawn, wait/run helpers
```

## Build, test, verify

```bash
cargo test --workspace                 # unit + snapshot tests
cargo clippy --workspace --all-targets
cargo run -p bossbar-daemon --example window_probe   # geometry check
```

- Visual changes: the pill is rendered offscreen in tests. Run
  `UPDATE_SNAPSHOTS=1 cargo test -p bossbar-daemon`, then look at the PNGs in
  `crates/bossbar-daemon/tests/snapshots/` before committing. Snapshots must be
  regenerated deliberately, not blindly.
- Manual smoke test (macOS/Linux): `bossbar create "X" --percent 10`, verify
  the pill appears top-center of the monitor with the cursor, `bossbar
  collapse on` with two bars, then `bossbar daemon stop`.
- Idle behavior is part of the contract: with no bars (or a static percent
  bar) the daemon must sit at ~0% CPU. Check with `top -l 2 -pid <pid>
  -stats cpu` (the second sample). A busy loop usually means a repaint was
  requested without being cleared or the run loop was left in a past
  `WaitUntil` deadline — see `src/event_loop.rs`.

## Architecture invariants

- `bars.rs` and `view.rs` know nothing about windows or IPC. Keep new
  behavior there when possible; both are testable without a display.
- The daemon owns **one** window (the root viewport). Bars are drawn into it;
  it is hidden when nothing is shown. Do not add deferred viewports unless the
  pill must outlive the root window.
- Request handling is synchronous under one mutex (`Daemon`). The UI thread
  drains the same store each frame. Keep handlers O(bars).
- `Waker::wake()` is the only supported way to prod the UI from another
  thread. It must run after the state mutation so the repaint sees it.
- macOS: accessory activation policy, `with_activate_ignoring_other_apps(false)`
  and `platform::macos_prevent_activation` (the private
  `_setPreventsActivation:` AppKit switch) are load bearing — without them the
  daemon grabs focus from the user's active app every time a bar appears.
  Don't switch back to `eframe::run_native` without re-checking the Dock icon
  and focus behavior.
- Windows: transparency comes from a native rounded region
  (`platform::apply_window_region`), not per-pixel alpha. `SHADOW_MARGIN` is 0
  there; keep the pill rect equal to the window rect.
- The pill must never steal focus or activate the app. Clickable controls are
  small and local (chevron, per-row close); nothing opens windows.

## Conventions

- Fit the existing style: small focused modules, `tracing` for diagnostics,
  errors surfaced as user-readable strings in `WireResponse`.
- Protocol changes: adjust `bossbar-proto`, bump `PROTOCOL_VERSION` only for
  breaking changes, and keep `--json` output stable enough for scripts.
- The CLI must stay dependency-light (clap + serde only); the daemon may pull
  platform crates under `cfg`.
