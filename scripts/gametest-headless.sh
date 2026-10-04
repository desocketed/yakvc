#!/usr/bin/env bash
# Runs the Fabric client gametest with no display or GPU: Xvfb plus Mesa's
# software renderers from nixpkgs (see headless-env.sh).
#
# Usage: scripts/gametest-headless.sh   (from the repo root, Nix required)
# Screenshots land in mod/build/run/clientGameTest/screenshots/; then
# `java scripts/UpdateGuideScreenshots.java` copies changed ones into docs/guide/.
set -euo pipefail

if [[ -z "${YAKVC_GAMETEST_INNER:-}" ]]; then
	export YAKVC_GAMETEST_INNER=1
	exec nix shell nixpkgs#xorg-server --command nix develop --command "$0" "$@"
fi

source "$(dirname "$0")/headless-env.sh"
cd "$(dirname "$0")/../mod"
./gradlew runClientGameTest --console=plain "$@"
