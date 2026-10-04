# Deploying yakvc-server

`yakvc-server` is the rendezvous (sign-in and matching) plus the relay that clients fall back to when hole-punching fails. One small Linux VPS with a public IP runs it. It keeps no state worth backing up apart from its two keys and the Let's Encrypt certificate cache.

Files here:

- `config.toml`: example production config. Replace `relay.example.com` and the contact address.
- `compose.yaml`: run the Docker image (`ghcr.io/desocketed/yakvc-server`) with Docker Compose.
- `yakvc-server.service`: run the static binary as a hardened systemd service instead.
- `Dockerfile`: used by `cargo xtask server-image`, which builds the image locally.

## Before either setup

- A DNS name for the relay (the `relay.example.com` placeholder) pointing at the server.
- These ports open in the firewall: TCP 80 and 443 (relay, and Let's Encrypt's certificate check), UDP 7842 (QUIC address discovery) and UDP 7843 (rendezvous).

## With Docker Compose

```sh
mkdir config && cp config.toml config/   # then edit config/config.toml
# The image runs as uid 65532, which must own the keys it writes and reads.
sudo chown 65532:65532 config
docker run --rm -v "$PWD/config:/etc/yakvc" ghcr.io/desocketed/yakvc-server keygen --out endpoint.key
docker run --rm -v "$PWD/config:/etc/yakvc" ghcr.io/desocketed/yakvc-server keygen --issuer --out issuer.key
docker compose up -d
docker compose logs   # prints the endpoint id, issuer id and relay URL
```

The metrics endpoint listens on 127.0.0.1 inside the container, so it is not published. To scrape it from the host, set `metrics_bind = "0.0.0.0:9100"` and add `- "127.0.0.1:9100:9100"` to the ports.

## With systemd

Download the static `yakvc-server` binary from the GitHub release, then:

```sh
sudo install -m 755 yakvc-server /usr/local/bin/
sudo install -d -m 755 /etc/yakvc
sudo install -m 644 config.toml /etc/yakvc/   # then edit it
sudo yakvc-server keygen --out /etc/yakvc/endpoint.key
sudo yakvc-server keygen --issuer --out /etc/yakvc/issuer.key
sudo install -m 644 yakvc-server.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now yakvc-server
journalctl -u yakvc-server   # prints the endpoint id, issuer id and relay URL
```

The keys stay readable by root only; systemd passes them to the service as credentials.

## Pointing clients at the server

Clients use the project's default server unless their `config/yakvc/client.toml` names another. With the ids the server printed:

```toml
trusted_issuers = ["<issuer id>"]

[rendezvous]
endpoint_id = "<endpoint id>"
addrs = ["<server public IP>:7843"]
relay = "https://relay.example.com/"
```

Keep the keys: a new endpoint key changes the endpoint id, and a new issuer key invalidates every ticket and every client's `trusted_issuers`.
