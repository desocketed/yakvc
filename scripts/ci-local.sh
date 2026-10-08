#!/usr/bin/env bash
# Runs the same checks as .github/workflows/ci.yml, locally and in order,
# stopping at the first failure. Use it before pushing to main.
#
#   scripts/ci-local.sh            everything, including the Minecraft gametest,
#                                  and refreshes the user guide screenshots in docs/guide/
#   scripts/ci-local.sh --fast     skip the release builds, mod build and gametest
#
# Windows and macOS natives build only on their own CI runners, and the
# server image only where Docker is installed.
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
# cargo tree on its own line: inside the `if` its failure would pass.
deps=$(cargo tree -p yakvc-server -e normal --prefix none)
if grep -E '^(yakvc-audio|yakvc-client|cpal|opus|audiopus|alsa)' <<<"$deps"; then
	echo "yakvc-server must not depend on audio crates" >&2
	exit 1
fi
step "deny"
cargo deny check

if [ "${1:-}" = --fast ]; then
	step "skipped the release builds, mod build and gametest (--fast)"
	exit 0
fi

step "natives: Linux release targets"
cargo xtask natives --target all --out target/ci-natives
step "server: image"
if command -v docker >/dev/null; then
	cargo xtask server-image
else
	echo "no docker here; skipped"
fi
step "mod: build (cargo xtask dist, host natives)"
cargo xtask dist
step "mod: client gametest, and the user guide's screenshots"
scripts/gametest-headless.sh

step "all checks passed"
