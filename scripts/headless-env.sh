# Sourced by the headless scripts, inside `nix shell nixpkgs#xorg-server` and
# `nix develop`: points the game at Mesa's software renderers from nixpkgs and
# starts an Xvfb display, exported as DISPLAY and killed on exit. Minecraft
# 26.3 finds no sRGB GLX config on Xvfb and falls back to Vulkan, which Mesa's
# lavapipe provides.

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

export DISPLAY=:$((90 + RANDOM % 100))
Xvfb "$DISPLAY" -screen 0 1280x720x24 >/dev/null 2>&1 &
xvfb=$!
trap 'kill $xvfb' EXIT
sleep 2
