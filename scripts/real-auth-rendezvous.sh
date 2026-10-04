#!/usr/bin/env bash
# Runs a local yakvc-server with real Mojang authentication, for the manual
# real-account check in docs/DEVELOPMENT.md. Builds the server and the mod jar,
# prints the client.toml to use, then serves in the foreground until Ctrl-C.
#
# Keys and config live in mod/build/real-auth/ and are reused between runs, so
# the IDs in the printed client.toml stay valid.
set -euo pipefail

if [ -z "${IN_NIX_SHELL:-}" ]; then
	exec nix develop "$(dirname "$0")/.." --command "$0" "$@"
fi

cd "$(dirname "$0")/.."
work=$PWD/mod/build/real-auth
rdv_addr=127.0.0.1:7843
metrics_addr=127.0.0.1:9187

cargo build --quiet -p yakvc-server
server_bin=$PWD/target/debug/yakvc-server
(cd mod && ./gradlew build --console=plain --quiet)

mkdir -p "$work"
[ -f "$work/endpoint.key" ] || "$server_bin" keygen --out "$work/endpoint.key"
[ -f "$work/issuer.key" ] || "$server_bin" keygen --issuer --out "$work/issuer.key"
# No insecure_dev_auth: clients must prove their account with Mojang.
cat >"$work/config.toml" <<EOF
endpoint_key = "$work/endpoint.key"
issuer_key = "$work/issuer.key"
bind = "$rdv_addr"
metrics_bind = "$metrics_addr"
EOF

cat <<EOF

Mod jar: $(ls "$PWD"/mod/build/libs/yakvc-*.jar | grep -v sources)

Put this in the client's config/yakvc/client.toml (no dev_mode, so the
client only accepts a real ticket):

trusted_issuers = ["$("$server_bin" issuer-id "$work/issuer.key")"]

[rendezvous]
endpoint_id = "$("$server_bin" endpoint-id "$work/endpoint.key")"
addrs = ["$rdv_addr"]

Server counters: curl -s http://$metrics_addr/metrics | grep -E 'auth|mojang'

EOF
exec "$server_bin" run --config "$work/config.toml"
