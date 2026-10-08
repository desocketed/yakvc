#!/usr/bin/env bash
# The M4 exit check without sound hardware or a display: two dev clients on a
# local server hear each other, and a spectator neither hears nor is heard.
#
# It starts a dev-mode yakvc-server (insecure_dev_auth), a local Fabric
# dedicated server with online-mode=false (the mod is client-only, so the
# server is vanilla apart from the loader), and two headless clients, Alice
# and Bob. The clients use dev test audio (a 440 Hz tone for the microphone,
# no speakers) and hold push-to-talk. A client logs `Talking <uuid> <name>
# true` (at DEBUG, so only in its logs/debug.log) once it is actually playing
# that player's voice. The script checks that:
#   1. each client hears the other;
#   2. after `/gamemode spectator Bob`, each stops hearing the other;
#   3. after `/gamemode survival Bob`, each hears the other again.
#
# Usage: scripts/two-clients.sh --accept-eula [--pipewire]
#   (or YAKVC_ACCEPT_MINECRAFT_EULA=1). Running the Minecraft server means
#   accepting the Minecraft EULA (https://aka.ms/MinecraftEULA); the script
#   writes eula=true into its throwaway server directory only when told to.
#   --pipewire uses real devices instead of the dev test audio: a private
#   PipeWire session per client (`cargo xtask with-pipewire`) with a test
#   microphone playing a tone, reached through ALSA as on a desktop. It also checks that each
#   microphone delivered audio and that no device failed to open.
# Logs and game directories are in mod/build/two-clients/.
set -euo pipefail

pipewire=
for arg in "$@"; do
	case $arg in
	--accept-eula) export YAKVC_ACCEPT_MINECRAFT_EULA=1 ;;
	--pipewire) pipewire=1 ;;
	*) echo "unknown argument $arg" >&2; exit 2 ;;
	esac
done
if [[ "${YAKVC_ACCEPT_MINECRAFT_EULA:-}" != 1 ]]; then
	echo "This runs a Minecraft server, which needs the Minecraft EULA accepted (https://aka.ms/MinecraftEULA)." >&2
	echo "Pass --accept-eula or set YAKVC_ACCEPT_MINECRAFT_EULA=1 to accept it." >&2
	exit 2
fi

if [[ -z "${YAKVC_TWO_CLIENTS_INNER:-}" ]]; then
	export YAKVC_TWO_CLIENTS_INNER=1
	exec nix shell nixpkgs#xorg-server --command nix develop --command "$0" "$@"
fi

cd "$(dirname "$0")/.."
root=$PWD
work=$root/mod/build/two-clients
rdv_port=$((20000 + RANDOM % 10000))
mc_port=$((30000 + RANDOM % 10000))
step() { printf '\n==> %s\n' "$*"; }

step "build xtask and the mod's launch arguments"
cargo build --quiet -p xtask
(cd mod && ./gradlew writeDevLaunchArgs --console=plain --quiet)
mapfile -t server_args < mod/build/devlaunch/server.args
mapfile -t client_args < mod/build/devlaunch/client.args

source scripts/headless-env.sh
pids=()
cleanup() {
	# A negative pid is a process group: a client with its PipeWire session.
	kill -- "${pids[@]}" "$xvfb" 2>/dev/null || true
	wait "${pids[@]}" 2>/dev/null || true
}
trap cleanup EXIT

rm -rf "$work"
mkdir -p "$work/rdv" "$work/server"

step "start the dev rendezvous on 127.0.0.1:$rdv_port"
# xtask replaces itself with the server, so this pid is the server's.
"$root/target/debug/xtask" dev --port "$rdv_port" --dir "$work/rdv" >"$work/rdv.log" 2>&1 &
pids+=($!)

step "start the Minecraft server on 127.0.0.1:$mc_port"
echo "eula=true" >"$work/server/eula.txt"
cat >"$work/server/server.properties" <<EOF
online-mode=false
enforce-secure-profile=false
white-list=false
enforce-whitelist=false
server-ip=127.0.0.1
server-port=$mc_port
level-type=minecraft\:flat
difficulty=peaceful
spawn-protection=0
view-distance=4
simulation-distance=4
motd=Yak VC two-client check
EOF
mkfifo "$work/server.in"
exec 3<>"$work/server.in"
(cd "$work/server" && exec java "${server_args[@]}" nogui <"$work/server.in" >"$work/server.log" 2>&1) &
pids+=($!)
console() { echo "$*" >&3; }

# wait_for SECONDS WHAT COMMAND...: runs COMMAND every second until it succeeds.
wait_for() {
	local deadline=$((SECONDS + $1)) what=$2
	shift 2
	until "$@"; do
		if ((SECONDS > deadline)); then
			echo "FAILED: timed out waiting for $what (logs in $work)" >&2
			exit 1
		fi
		sleep 1
	done
}
# logged FILE SINCE PATTERN: FILE has a line matching PATTERN after line SINCE.
logged() { tail -n +$(($2 + 1)) "$1" 2>/dev/null | grep -qE "$3"; }
lines() { wc -l <"$1"; }

wait_for 120 "the dev rendezvous" logged "$work/rdv.log" 0 'Dev yakvc-server'
wait_for 180 "the Minecraft server" logged "$work/server.log" 0 'Done \('

for name in Alice Bob; do
	dir=$work/$name
	mkdir -p "$dir/config/yakvc"
	# The dev rendezvous's client config, plus dev test audio unless real
	# devices are under test.
	cp "$work/rdv/client.toml" "$dir/config/yakvc/client.toml"
	if [[ -z $pipewire ]]; then
		printf '\n[dev]\ntone_hz = 440.0\nnull_output = true\n' >>"$dir/config/yakvc/client.toml"
	fi
	printf 'onboardAccessibility:false\nskipMultiplayerWarning:true\njoinedFirstServer:true\nrenderDistance:2\nmaxFps:30\n' \
		>"$dir/options.txt"
	step "start $name"
	# With --pipewire each client runs in its own PipeWire session, in a
	# process group of its own so that cleanup stops the daemons too.
	launcher=()
	[[ -z $pipewire ]] || launcher=(setsid "$root/target/debug/xtask" with-pipewire --)
	(cd "$dir" && exec "${launcher[@]}" java -Dyakvc.dev.holdPushToTalk=true "${client_args[@]}" \
		--gameDir . --username "$name" --quickPlayMultiplayer "127.0.0.1:$mc_port" >"$work/$name.log" 2>&1) &
	if [[ -n $pipewire ]]; then pids+=("-$!"); else pids+=($!); fi
done

# Talking is logged at DEBUG, which only the dev log config's debug.log keeps
# (along with every INFO line), so read that rather than the console output.
alice=$work/Alice/logs/debug.log
bob=$work/Bob/logs/debug.log
hears() { logged "$1" "$2" "\(yakvc\) Talking [^ ]+ $3 true"; }
stops_hearing() { logged "$1" "$2" "\(yakvc\) Talking [^ ]+ $3 false"; }

step "1. each client hears the other"
wait_for 300 "Alice to hear Bob" hears "$alice" 0 Bob
wait_for 60 "Bob to hear Alice" hears "$bob" 0 Alice
echo "ok"

step "2. Bob becomes a spectator: neither hears the other"
alice_mark=$(lines "$alice")
bob_mark=$(lines "$bob")
console "gamemode spectator Bob"
wait_for 30 "Alice to stop hearing Bob" stops_hearing "$alice" "$alice_mark" Bob
wait_for 30 "Bob to stop hearing Alice" stops_hearing "$bob" "$bob_mark" Alice
alice_mark=$(lines "$alice")
bob_mark=$(lines "$bob")
sleep 10
if hears "$alice" "$alice_mark" Bob || hears "$bob" "$bob_mark" Alice; then
	echo "FAILED: a spectator was heard, or heard someone (logs in $work)" >&2
	exit 1
fi
echo "ok: nothing heard in 10 s"

step "3. Bob leaves spectator mode: both hear each other again"
alice_mark=$(lines "$alice")
bob_mark=$(lines "$bob")
console "gamemode survival Bob"
wait_for 30 "Alice to hear Bob again" hears "$alice" "$alice_mark" Bob
wait_for 30 "Bob to hear Alice again" hears "$bob" "$bob_mark" Alice
echo "ok"

if [[ -n $pipewire ]]; then
	step "real devices"
	for log in "$alice" "$bob"; do
		grep -q "(yakvc) Microphone is delivering audio" "$log" ||
			{ echo "FAILED: no microphone audio in $log" >&2; exit 1; }
		if grep -E "\(yakvc\) Engine: .*(microphone|speakers)" "$log"; then
			echo "FAILED: a device failed in $log" >&2
			exit 1
		fi
	done
	echo "ok: both microphones delivered audio through PipeWire"
fi

step "voice events"
grep -hE '\(yakvc\) (Rendezvous|Peer|Talking)' "$alice" | sed 's/^/Alice: /'
grep -hE '\(yakvc\) (Rendezvous|Peer|Talking)' "$bob" | sed 's/^/Bob:   /'
console stop
step "passed"
