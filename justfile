# Dev sessions on your own machine, in real windows with real audio: a local
# Yak VC rendezvous, a dev Minecraft server and any number of clients that log
# in under their own names and join it by themselves. Run inside the dev
# shell: `nix develop --command just dev Alice Bob`.
#
# Everything lives in mod/run/dev/ (gitignored) and survives between runs:
# each client's settings and the server's world. Ctrl-C stops the session.

set shell := ["bash", "-euo", "pipefail", "-c"]

dir := justfile_directory() / "mod/run/dev"
rendezvous_port := "7843"
server_port := "25565"

# Libraries Minecraft loads from the host at runtime, as on NixOS, where
# /run/opengl-driver holds the GPU driver.
export LD_LIBRARY_PATH := "/run/opengl-driver/lib:" + `nix build --inputs-from . --no-link --print-out-paths nixpkgs#libglvnd nixpkgs#vulkan-loader nixpkgs#wayland nixpkgs#libxkbcommon nixpkgs#libpulseaudio nixpkgs#alsa-lib nixpkgs#xorg.libX11 nixpkgs#xorg.libXcursor nixpkgs#xorg.libXrandr nixpkgs#xorg.libXi nixpkgs#xorg.libXext | sed 's|$|/lib|' | paste -sd:`

# The rendezvous, the server and one client per name, all joining the server.
dev +names="Alice Bob": build
    #!/usr/bin/env bash
    set -euo pipefail
    # Each part runs in its own process group, so stopping the session stops
    # the java processes too, and nothing outside it.
    groups=()
    trap 'kill -- "${groups[@]}" 2>/dev/null || true' EXIT
    setsid just dir={{dir}} rendezvous & groups+=("-$!")
    setsid just dir={{dir}} server & groups+=("-$!")
    # Clients wait for the server, or quick play gives up on the connection.
    until (exec 3<>/dev/tcp/127.0.0.1/{{server_port}}) 2>/dev/null; do sleep 1; done
    for name in {{names}}; do setsid just dir={{dir}} client "$name" & groups+=("-$!"); done
    wait

# Build the mod and write the java command lines of its run configs.
build:
    cargo build --quiet --locked -p xtask
    cd mod && ./gradlew writeDevLaunchArgs --quiet

# A local Yak VC rendezvous; its keys stay in mod/run/dev/rendezvous.
rendezvous:
    {{justfile_directory()}}/target/debug/xtask dev --port {{rendezvous_port}} --dir {{dir}}/rendezvous

# A dev Minecraft server in offline mode, so any name can join.
server:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dir}}/server && cd {{dir}}/server
    if ! grep -qs '^eula=true' eula.txt; then
        echo "Read the Minecraft EULA (https://aka.ms/MinecraftEULA); to accept it, run:" >&2
        echo "  echo eula=true > {{dir}}/server/eula.txt" >&2
        exit 1
    fi
    [[ -f server.properties ]] || printf '%s\n' online-mode=false enforce-secure-profile=false white-list=false enforce-whitelist=false \
        server-ip=127.0.0.1 server-port={{server_port}} difficulty=peaceful spawn-protection=0 > server.properties
    mapfile -t args < {{justfile_directory()}}/mod/build/devlaunch/server.args
    exec java "${args[@]}" nogui

# One client named NAME, with its own game directory, joining the server.
client name:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p {{dir}}/{{name}}/config/yakvc && cd {{dir}}/{{name}}
    # The local rendezvous's address and keys, kept once written so the
    # client's own settings survive.
    [[ -f config/yakvc/client.toml ]] || cp {{dir}}/rendezvous/client.toml config/yakvc/client.toml
    [[ -f options.txt ]] || printf '%s\n' onboardAccessibility:false skipMultiplayerWarning:true \
        joinedFirstServer:true > options.txt
    mapfile -t args < {{justfile_directory()}}/mod/build/devlaunch/client.args
    exec java "${args[@]}" --gameDir . --username {{name}} --quickPlayMultiplayer 127.0.0.1:{{server_port}}
