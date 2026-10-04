#!/usr/bin/env bash
# Runs the same checks as .github/workflows/ci.yml, locally and in order,
# stopping at the first failure. Use it before pushing to main.
#
#   scripts/ci-local.sh            everything, including the Minecraft gametest
#   scripts/ci-local.sh --fast     skip the mod build and gametest
set -euo pipefail

if [ -z "${IN_NIX_SHELL:-}" ]; then
	exec nix develop "$(dirname "$0")/.." --command "$0" "$@"
fi

cd "$(dirname "$0")/.."
step() { printf '\n==> %s\n' "$*"; }

step "rust: fmt"
cargo fmt --all --check
step "rust: clippy"
cargo clippy --workspace --all-targets --all-features -- -D warnings
step "rust: header"
cargo xtask header --check
step "rust: test"
cargo test --workspace --all-features
step "rust: server has no audio dependencies"
if cargo tree -p yakvc-server -e normal --prefix none |
	grep -E '^(yakvc-audio|yakvc-client|cpal|opus|audiopus|alsa)'; then
	echo "yakvc-server must not depend on audio crates" >&2
	exit 1
fi
step "deny"
cargo deny check

if [ "${1:-}" = --fast ]; then
	step "skipped the mod build and gametest (--fast)"
	exit 0
fi

step "mod: build"
(cd mod && ./gradlew build)
step "mod: client gametest"
scripts/gametest-headless.sh

step "all checks passed"
