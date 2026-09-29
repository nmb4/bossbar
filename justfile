# Common tasks for the bossbar workspace.

default:
    @just --list

build:
    cargo build --workspace

release:
    cargo build --release -p bossbar-daemon -p bossbar-cli

test:
    cargo test --workspace

lint:
    cargo clippy --workspace --all-targets -- -D warnings

fmt:
    cargo fmt --all

snapshots:
    UPDATE_SNAPSHOTS=1 cargo test -p bossbar-daemon

daemon:
    cargo run -p bossbar-daemon

probe:
    cargo run -p bossbar-daemon --example window_probe

install:
    install -m 755 target/release/bossbar target/release/bossbar-daemon ~/.local/bin/

# Visual demos use the installed bossbar on PATH. Default duration: 30 seconds.
# Pass a duration, e.g. `just demo-multi 60`. Minimum: 10 seconds.

# One percent bar with a detail line, held at 47% for screenshots.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-single seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario single -DurationSeconds {{seconds}}

# One segmented bar, held at 7/20.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-ticks seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario ticks -DurationSeconds {{seconds}}

# One indeterminate bar with its animation running.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-spinner seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario spinner -DurationSeconds {{seconds}}

# Percent, ticks, and indeterminate bars together, expanded.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-multi seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario multi -DurationSeconds {{seconds}}

# All three types in the normal collapsed layout.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-normal seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario normal -DurationSeconds {{seconds}}

# All three types in the compact collapsed layout.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-compact seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario compact -DurationSeconds {{seconds}}

# Remove demo bars left behind by an interrupted run.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-clean:
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario clean

# Animate only the label of one bar, updating every 3 seconds.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-update-label seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario update-label -DurationSeconds {{seconds}}

# Animate only the detail line of one bar, updating every 3 seconds.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-update-detail seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario update-detail -DurationSeconds {{seconds}}

# Animate a single bar's label, detail, and progress every 3 seconds.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-update seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario update -DurationSeconds {{seconds}}

# Animate text and progress across all three bar types every 3 seconds.
[script("powershell.exe", "-NoLogo", "-NoProfile", "-File")]
demo-update-multi seconds="30":
    & "{{justfile_directory()}}/scripts/demo.ps1" -Scenario update-multi -DurationSeconds {{seconds}}
