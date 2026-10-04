#!/usr/bin/env bash
# Runs the Fabric client gametest with no display or GPU: Xvfb plus Mesa's
# software renderers from nixpkgs. Minecraft 26.3 finds no sRGB GLX config on
# Xvfb and falls back to Vulkan, which Mesa's lavapipe provides.
#
# Usage: scripts/gametest-headless.sh   (from the repo root, Nix required)
# Screenshots land in mod/build/run/clientGameTest/screenshots/.
set -euo pipefail

if [[ -z "${YAKVC_GAMETEST_INNER:-}" ]]; then
	export YAKVC_GAMETEST_INNER=1
	exec nix shell nixpkgs#xorg-server --command nix develop --command "$0" "$@"
fi

store() { nix build --no-link --print-out-paths "$@"; }
mesa=$(store nixpkgs#mesa | head -1)
libs=$(store nixpkgs#libglvnd nixpkgs#vulkan-loader nixpkgs#xorg.libX11 nixpkgs#xorg.libXext \
	nixpkgs#xorg.libXcursor nixpkgs#xorg.libXrandr nixpkgs#xorg.libXxf86vm nixpkgs#xorg.libXi \
	nixpkgs#xorg.libXrender nixpkgs#xorg.libxcb nixpkgs#alsa-lib nixpkgs#libpulseaudio \
	| sed 's|$|/lib|' | paste -sd:)
export LD_LIBRARY_PATH="$mesa/lib:$libs"
export LIBGL_DRIVERS_PATH="$mesa/lib/dri"
export __GLX_VENDOR_LIBRARY_NAME=mesa
export VK_ICD_FILENAMES="$mesa/share/vulkan/icd.d/lvp_icd.x86_64.json"

display=:$((90 + RANDOM % 100))
Xvfb "$display" -screen 0 1280x720x24 >/dev/null 2>&1 &
xvfb=$!
trap 'kill $xvfb' EXIT
sleep 2

cd "$(dirname "$0")/../mod"
DISPLAY=$display ./gradlew runClientGameTest --console=plain "$@"
