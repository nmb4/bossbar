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
