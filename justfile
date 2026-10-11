# Shortcuts for common tasks; `just` alone lists them. Every recipe runs in
# the dev shell, so the host's own Java and Rust never get in the way.

set shell := ["nix", "develop", "--quiet", "--command", "bash", "-euc"]

default:
    @just --list

# Dev session: a local rendezvous, a dev server and one client window per name.
session *names:
    cargo xtask session {{names}}

# Add clients to a running session.
join +names:
    cargo xtask session --join {{names}}

# Build the mod jar (host natives) into mod/build/libs.
build:
    cd mod && ./gradlew build -x test

# Rust tests, clippy and cargo-deny.
check:
    cargo test --workspace --all-features
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    cargo deny check

# Everything CI runs on Linux; --fast skips the mod build and gametest.
ci *flags:
    scripts/ci-local.sh {{flags}}

# The headless client gametest, which also refreshes the guide screenshots.
gametest:
    scripts/gametest-headless.sh
