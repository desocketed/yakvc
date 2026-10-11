#!/usr/bin/env bash
# Runs the Fabric client gametest with no display or GPU: Xvfb plus Mesa's
# software renderers from nixpkgs (see headless-env.sh). When it passes, the
# user guide's screenshots in docs/guide/ are refreshed from it (only those
# that look different; see UpdateGuideScreenshots.java), ready to commit.
#
# Usage: scripts/gametest-headless.sh   (from the repo root, Nix required)
# Screenshots land in mod/build/run/clientGameTest/screenshots/.
set -euo pipefail

cd "$(dirname "$0")/.."
if [[ -z "${YAKVC_GAMETEST_INNER:-}" ]]; then
	export YAKVC_GAMETEST_INNER=1
	exec nix shell --inputs-from . nixpkgs#xorg-server --command nix develop --command scripts/gametest-headless.sh "$@"
fi

source scripts/headless-env.sh
(cd mod && ./gradlew runClientGameTest --console=plain "$@")
java scripts/UpdateGuideScreenshots.java
