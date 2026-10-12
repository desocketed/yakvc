#!/usr/bin/env bash
# Everything CI checks on Linux, locally: see `cargo xtask ci`.
#
#   scripts/ci-local.sh            everything, including the Minecraft gametest
#   scripts/ci-local.sh --fast     stop before the release builds, mod and gametest
set -euo pipefail
if [ -z "${IN_NIX_SHELL:-}" ]; then
	exec nix develop "$(dirname "$0")/.." --command "$0" "$@"
fi
exec cargo xtask ci "$@"
