# Deploying yakvc-server

`yakvc-server` is the rendezvous (sign-in and matching) plus the relay that clients fall back to when hole-punching fails. One small Linux VPS with a public IP runs it. It keeps no state worth backing up apart from its two keys and the Let's Encrypt certificate cache.

Files here:

- `config.toml`: example production config. Replace `relay.example.com` and the contact address.
- `yakvc-server-docker.service`: the recommended setup. systemd manages the service and Docker isolates it.
- `yakvc-server.service`: the static binary as a hardened systemd service, for hosts without Docker.
- `Dockerfile`: used by `cargo xtask server-image`, which builds the image locally.
- The repository's `flake.nix` also packages the server (`nix build github:desocketed/yakvc#yakvc-server`) and has a NixOS module; see "On NixOS" below.

## On Linode, with systemd and Docker

The image is `ghcr.io/desocketed/yakvc-server`, published by the release workflow. Until the first release, build it with `cargo xtask server-image` and copy it over with `docker save` and `docker load`.

1. **Create the Linode.** Debian 13 in a US East region (Newark), starting with the smallest shared plan; resize later from the relay metrics.
2. **Add a Cloud Firewall** (Linodes → Firewalls) with the default inbound policy set to drop, and these inbound rules:
   - TCP 22, from your own IP only (SSH)
   - TCP 80 and 443 (relay, and Let's Encrypt's certificate check)
   - UDP 7842 (QUIC address discovery) and UDP 7843 (rendezvous)
3. **Point DNS at it.** An `A` record for the relay hostname to the Linode's IPv4 address; Let's Encrypt needs it before the first start. Leave out `AAAA` for now: the server listens on IPv4 only until #38.
4. **Install Docker:**

   ```sh
   sudo apt update && sudo apt install -y docker.io
   ```

5. **Add the config:**

   ```sh
   sudo install -d -m 755 /etc/yakvc
   sudo install -m 644 config.toml /etc/yakvc/   # then edit the hostname and contact
   ```

6. **Start it:**

   ```sh
   sudo install -m 644 yakvc-server-docker.service /etc/systemd/system/yakvc-server.service
   sudo systemctl daemon-reload && sudo systemctl enable --now yakvc-server
   journalctl -u yakvc-server   # prints the endpoint id, issuer id and relay URL
   ```

   On first start the server creates its two keys in the `yakvc-state` volume and says so.

7. **Check it:** `curl -I https://<relay hostname>/` answers over HTTPS once the certificate is issued, and `curl -s 127.0.0.1:9100/metrics` on the Linode shows the metrics.
8. **Back up the keys** (`/var/lib/docker/volumes/yakvc-state/_data/*.key`) somewhere off the server (see below).

To upgrade, change the image tag in `/etc/systemd/system/yakvc-server.service`, then `sudo systemctl daemon-reload && sudo systemctl restart yakvc-server`.

The container uses the host's network so that the relay and address discovery see players' real addresses, which hole-punching depends on. It runs as root inside the container because Docker only grants added capabilities to root, but with every capability dropped except binding ports 80 and 443, no new privileges and a read-only filesystem. The metrics endpoint listens on 127.0.0.1, so it stays local to the Linode.

## On NixOS

Add the flake as an input and enable the module. It runs the server as a hardened systemd service, like `yakvc-server.service`, and the server creates its two keys on first start in `/var/lib/yakvc`; back them up.

```nix
{
  inputs.yakvc.url = "github:desocketed/yakvc";

  outputs = { nixpkgs, yakvc, ... }: {
    nixosConfigurations.relay = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        yakvc.nixosModules.default
        {
          services.yakvc-server = {
            enable = true;
            domain = "relay.example.com";
            acmeContact = "admin@example.com";
            openFirewall = true;
            # Anything else from config.toml, merged over the defaults:
            # settings.ticket_lifetime_hours = 24;
          };
        }
      ];
    };
  };
}
```

`journalctl -u yakvc-server` prints the endpoint id, issuer id and relay URL. The repository is private for now, so fetching the flake needs GitHub access (`git+ssh://git@github.com/desocketed/yakvc` works with a deploy key).

## Without Docker

Download the static `yakvc-server` binary from the GitHub release, then:

```sh
sudo install -m 755 yakvc-server /usr/local/bin/
sudo install -d -m 755 /etc/yakvc
sudo install -m 644 config.toml /etc/yakvc/   # then edit it
sudo install -m 644 yakvc-server.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now yakvc-server
journalctl -u yakvc-server   # prints the endpoint id, issuer id and relay URL
```

On first start the server creates its two keys in `/var/lib/yakvc` (really `/var/lib/private/yakvc`, readable only by the service and root); back them up.

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
