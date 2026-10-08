# Deploying yakvc-server

`yakvc-server` is the rendezvous (sign-in and matching) plus the relay that clients fall back to when hole-punching fails. One small Linux VPS with a public IP runs it. It keeps no state worth backing up apart from its two keys and the Let's Encrypt certificate cache.

Files here:

- `config.toml`: the production config, for `relay01.purplg.com`. Set the contact address; self-hosters replace the domain too.
- `yakvc-server-docker.service`: the recommended setup. systemd manages the service and Docker isolates it.
- `yakvc-server.service`: the static binary as a hardened systemd service, for hosts without Docker.
- `Dockerfile`: used by `cargo xtask server-image`, which builds the image locally.
- The repository's `flake.nix` also packages the server (`nix build github:desocketed/yakvc#yakvc-server`) and has a NixOS module; see "On NixOS" below.

## On Linode, with systemd and Docker

The image is `ghcr.io/desocketed/yakvc-server`, published by the release workflow. Until the first release, build it with `cargo xtask server-image` and copy it over with `docker save` and `docker load`.

1. **Create the Linode.** Debian 13 in a US East region (Newark), starting with the smallest shared plan; resize later from the relay metrics.
2. **Add a Cloud Firewall** (Linodes → Firewalls) with the default inbound policy set to drop, and these inbound rules. The server listens on IPv4 and IPv6, so give the public rules both "All IPv4" and "All IPv6" as sources:
   - TCP 22, from your own IP only (SSH)
   - TCP 80 and 443 (relay, and Let's Encrypt's certificate check)
   - UDP 7842 (QUIC address discovery) and UDP 7843 (rendezvous)
3. **Point DNS at it.** An `A` record for the relay hostname to the Linode's IPv4 address, and an `AAAA` record to its IPv6 address (the Linode's Network tab, the SLAAC address). Let's Encrypt needs them before the first start, and checks over IPv6 too once the `AAAA` record exists, so open the firewall for IPv6 first. The config binds `[::]`, which also serves IPv4 as long as `net.ipv6.bindv6only` keeps its Linux default of 0.
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

The flake has a NixOS module. It runs the server as a hardened systemd service, like `yakvc-server.service`, and the server creates its two keys on first start in `/var/lib/yakvc`; back them up. Your system needs flakes enabled (`nix.settings.experimental-features = [ "nix-command" "flakes" ];`).

**1. Add the flake** to your system's `flake.nix` and import the module:

```nix
{
  inputs.yakvc.url = "github:desocketed/yakvc";

  outputs = { nixpkgs, yakvc, ... }: {
    nixosConfigurations.<hostname> = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [
        ./configuration.nix
        yakvc.nixosModules.default
      ];
    };
  };
}
```

If the system has no `flake.nix` yet, add the module to `configuration.nix` directly instead:

```nix
imports = [ (builtins.getFlake "github:desocketed/yakvc").nixosModules.default ];
```

Nothing pins this reference, so pin a commit (`github:desocketed/yakvc/<commit>`) to upgrade deliberately.

**2. Enable it** in `configuration.nix`. Without a relay (a private test: no domain or certificate needed, opens UDP 7843):

```nix
services.yakvc-server = {
  enable = true;
  openFirewall = true;
};
```

With the relay (production: DNS for the domain must point at this machine; opens TCP 80 and 443 and UDP 7842 and 7843):

```nix
services.yakvc-server = {
  enable = true;
  domain = "relay.example.com";
  acmeContact = "admin@example.com";
  openFirewall = true;
  # Anything else from config.toml, merged over the defaults:
  # settings.ticket_lifetime_hours = 24;
};
```

**3. Rebuild** (`sudo nixos-rebuild switch`), then `journalctl -u yakvc-server` prints the endpoint id, issuer id and (with a relay) the relay URL, for clients' `client.toml` (see "Pointing clients at the server" below).

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
addrs = ["<server public IPv4>:7843", "[<server public IPv6>]:7843"]
relay = "https://relay.example.com/"
```

List both addresses so that clients on IPv6-only networks can reach the rendezvous directly; one alone works too.

Keep the keys: a new endpoint key changes the endpoint id, and a new issuer key invalidates every ticket and every client's `trusted_issuers`.
