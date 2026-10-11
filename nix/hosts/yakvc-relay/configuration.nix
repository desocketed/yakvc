# The production rendezvous and relay: relay.yak.dsktd.net, a Linode Nanode
# in Newark. Installed with nixos-anywhere and updated with
# `nixos-rebuild switch --target-host` (the CI `deploy` job on every release,
# or by hand; see deploy/README.md), so nothing is built on the box itself.
{ config, lib, modulesPath, pkgs, ... }:
let
  # The maintainer's contact address and alert URLs stay out of the public
  # repository: private.nix is gitignored, kept in the CI secret
  # RELAY_PRIVATE_NIX and in Claude's $HOME/.claude/yakvc-relay-private.nix.
  private =
    if builtins.pathExists ./private.nix then import ./private.nix
    else throw "nix/hosts/yakvc-relay/private.nix is missing; see deploy/README.md";

  # Alerting, each off while null. Healthchecks.io gets a ping after every
  # good status check and alerts (through ntfy) when pings stop, which covers
  # the whole box being down. ntfy also gets a message whenever the status
  # flips; its publish token lives only on the server, in
  # /etc/yakvc-alerts/ntfy-token (root-only), handed to the check by systemd.
  alerts = {
    inherit (private) healthcheckUrl ntfyUrl;
  };
in
{
  imports = [ (modulesPath + "/profiles/qemu-guest.nix") ];

  # Linode's own layout: the filesystem on the whole disk, no partition table.
  # Linode's "GRUB 2" kernel setting boots it by reading /boot/grub/grub.cfg,
  # so GRUB is configured but never installed to a disk.
  disko.devices.disk.main = {
    device = "/dev/sda";
    type = "disk";
    content = {
      type = "filesystem";
      format = "ext4";
      mountpoint = "/";
    };
  };
  # Linode's swap disk, left as Linode made it.
  swapDevices = [ { device = "/dev/sdb"; } ];

  boot.loader.grub = {
    enable = true;
    device = "nodev";
    configurationLimit = 5;
  };
  # Lish, Linode's web console, is a serial console.
  boot.kernelParams = [ "console=ttyS0,19200n8" ];

  networking.hostName = "yakvc-relay";
  # eth0, as Linode's network helper and docs expect.
  networking.usePredictableInterfaceNames = false;
  networking.useDHCP = true;

  services.yakvc-server = {
    enable = true;
    domain = "relay.yak.dsktd.net";
    acmeContact = private.acmeContact;
    openFirewall = true;
  };

  services.openssh = {
    enable = true;
    settings = {
      PasswordAuthentication = false;
      KbdInteractiveAuthentication = false;
      PermitRootLogin = "no";
      AllowUsers = [ "dsktd" "claude" ];
      X11Forwarding = false;
      AllowAgentForwarding = false;
      AllowTcpForwarding = false;
      MaxAuthTries = 3;
    };
  };
  # Blocks addresses that keep failing SSH logins.
  services.sshguard.enable = true;

  # Kernel network hardening: no redirects or source routing, reverse-path
  # filtering, SYN cookies; and keep kernel pointers and logs from users.
  boot.kernel.sysctl = {
    "net.ipv4.conf.all.rp_filter" = 1;
    "net.ipv4.conf.default.rp_filter" = 1;
    "net.ipv4.conf.all.accept_redirects" = 0;
    "net.ipv4.conf.default.accept_redirects" = 0;
    "net.ipv6.conf.all.accept_redirects" = 0;
    "net.ipv6.conf.default.accept_redirects" = 0;
    "net.ipv4.conf.all.send_redirects" = 0;
    "net.ipv4.conf.all.accept_source_route" = 0;
    "net.ipv6.conf.all.accept_source_route" = 0;
    "net.ipv4.tcp_syncookies" = 1;
    "kernel.kptr_restrict" = 2;
    "kernel.dmesg_restrict" = 1;
  };

  # SSH, and the status page.
  networking.firewall.allowedTCPPorts = [ 22 8080 ];

  # Personal admin accounts, logging in by key only. dsktd sets its own
  # password with passwd, which doas asks for. claude_keys holds Claude's
  # admin key and the CI deploy key (the RELAY_DEPLOY_KEY secret).
  users.users.dsktd = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keyFiles = [ ./maintainer_keys ];
  };
  users.users.claude = {
    isNormalUser = true;
    extraGroups = [ "wheel" ];
    openssh.authorizedKeys.keyFiles = [ ./claude_keys ];
  };
  # doas instead of sudo, with `sudo` kept as a command that runs doas, so
  # habits and tools that call sudo (nixos-rebuild --use-remote-sudo) work.
  security.sudo.enable = false;
  security.doas = {
    enable = true;
    extraRules = [
      # The maintainer types their password, remembered for a few minutes.
      { users = [ "dsktd" ]; persist = true; keepEnv = true; }
      # Claude deploys unattended, so it can't answer a prompt.
      { users = [ "claude" ]; noPass = true; keepEnv = true; }
    ];
  };
  environment.systemPackages = [
    (pkgs.writeShellScriptBin "sudo" ''exec doas "$@"'')
  ];

  # A 25 GB disk: keep the Nix store and logs small. The system is built
  # elsewhere and copied over, so no compilers or channels live here.
  nix.gc = {
    automatic = true;
    dates = "weekly";
    options = "--delete-older-than 14d";
  };
  nix.settings.auto-optimise-store = true;
  # Lets the wheel accounts copy new systems here for nixos-rebuild.
  nix.settings.trusted-users = [ "@wheel" ];
  nix.settings.experimental-features = [ "nix-command" "flakes" ];
  nix.channel.enable = false;
  documentation.enable = false;
  services.journald.settings.Journal.SystemMaxUse = "500M";

  # Status page: http://relay.yak.dsktd.net:8080/ is a plain-text 🐃👍 or 🐃👎.
  # The relay owns 80 and 443 and can't serve extra pages, so nginx serves
  # a file that a timer rewrites every 30 s: 👍 when `yakvc-server probe`
  # signs in through the relay like a real client (as an offline player, so
  # no Mojang call).
  systemd.services.yakvc-status = {
    path = [ config.services.yakvc-server.package pkgs.curl ];
    # Unprivileged: signing in needs no root.
    serviceConfig = {
      Type = "oneshot";
      LoadCredential = lib.optional (alerts.ntfyUrl != null) "ntfy-token:/etc/yakvc-alerts/ntfy-token";
      User = "yakvc-status";
      Group = "yakvc-status";
      StateDirectory = "yakvc-status";
      StateDirectoryMode = "0755";
      NoNewPrivileges = true;
      ProtectSystem = "strict";
      ProtectHome = true;
      PrivateTmp = true;
    };
    script = ''
      if yakvc-server probe \
        --endpoint-id 5c9578695b7b258c587bebc826f1091a95da2e75ef93a4b7aeff45f28e2cde60 \
        --relay https://relay.yak.dsktd.net >/dev/null; then
        s="🐃👍"
      else
        s="🐃👎"
      fi
      file=/var/lib/yakvc-status/status.txt
      old=$(cat "$file" 2>/dev/null || true)
      echo "$s" > "$file.tmp"
      mv "$file.tmp" "$file"
      ${lib.optionalString (alerts.ntfyUrl != null) ''
        if [ -n "$old" ] && [ "$old" != "$s" ]; then
          curl -fsS -m 10 -H "Title: relay.yak.dsktd.net" \
            -H "Authorization: Bearer $(cat "$CREDENTIALS_DIRECTORY/ntfy-token")" \
            -d "$s" ${alerts.ntfyUrl} >/dev/null || true
        fi
      ''}
      ${lib.optionalString (alerts.healthcheckUrl != null) ''
        if [ "$s" = "🐃👍" ]; then
          curl -fsS -m 10 --retry 3 ${alerts.healthcheckUrl} >/dev/null || true
        fi
      ''}
    '';
  };
  systemd.timers.yakvc-status = {
    wantedBy = [ "timers.target" ];
    timerConfig = { OnBootSec = "30s"; OnUnitActiveSec = "30s"; };
  };
  users.users.yakvc-status = { isSystemUser = true; group = "yakvc-status"; };
  users.groups.yakvc-status = { };
  services.nginx = {
    enable = true;
    serverTokens = false;
    virtualHosts.status = {
      default = true;
      listen = [ { addr = "0.0.0.0"; port = 8080; } { addr = "[::]"; port = 8080; } ];
      locations."/".extraConfig = ''
        root /var/lib/yakvc-status;
        try_files /status.txt =503;
        types { }
        default_type text/plain;
        charset utf-8;
        add_header Cache-Control "no-store";
      '';
    };
  };

  time.timeZone = "UTC";
  system.stateVersion = "26.11";
}
