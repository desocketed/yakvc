# NixOS module: `services.yakvc-server.enable = true;` runs the rendezvous and
# relay as a hardened systemd service, like deploy/yakvc-server.service.
# The server creates its keys on first start in /var/lib/yakvc; back them up.
self:
{ config, lib, pkgs, ... }:
let
  cfg = config.services.yakvc-server;

  settings = lib.recursiveUpdate {
    endpoint_key = "/var/lib/yakvc/endpoint.key";
    issuer_key = "/var/lib/yakvc/issuer.key";
    bind = "0.0.0.0:7843";
    metrics_bind = "127.0.0.1:9100";
    relay = {
      http_bind = "0.0.0.0:80";
      https_bind = "0.0.0.0:443";
      quic_bind = "0.0.0.0:7842";
      domain = cfg.domain;
      acme_contact = cfg.acmeContact;
      acme_cache_dir = "/var/lib/yakvc/acme";
    };
  } cfg.settings;

  configFile = (pkgs.formats.toml { }).generate "yakvc-server.toml" settings;
  server = lib.getExe cfg.package;
in
{
  options.services.yakvc-server = {
    enable = lib.mkEnableOption "the Yak VC rendezvous server and relay";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.yakvc-server;
      description = "The yakvc-server package.";
    };

    domain = lib.mkOption {
      type = lib.types.str;
      example = "relay.example.com";
      description = "Public hostname of the relay. Let's Encrypt issues its certificate, so DNS must point here.";
    };

    acmeContact = lib.mkOption {
      type = lib.types.str;
      example = "admin@example.com";
      description = "Contact address for the Let's Encrypt account.";
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open TCP 80 and 443 and UDP 7842 and 7843.";
    };

    settings = lib.mkOption {
      type = (pkgs.formats.toml { }).type;
      default = { };
      description = "Overrides for config.toml (see deploy/config.toml), merged over the module's defaults.";
    };
  };

  config = lib.mkIf cfg.enable {
    networking.firewall = lib.mkIf cfg.openFirewall {
      allowedTCPPorts = [ 80 443 ];
      allowedUDPPorts = [ 7842 7843 ];
    };

    systemd.services.yakvc-server = {
      description = "Yak VC rendezvous server and relay";
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      wantedBy = [ "multi-user.target" ];

      serviceConfig = {
        # The server reports READY=1 once it is listening.
        Type = "notify";
        ExecStart = "${server} run --config ${configFile}";
        Restart = "on-failure";

        # A throwaway user, which may bind ports 80 and 443 and write only
        # /var/lib/yakvc (the keys and the ACME cache).
        DynamicUser = true;
        StateDirectory = "yakvc";
        StateDirectoryMode = "0700";
        AmbientCapabilities = [ "CAP_NET_BIND_SERVICE" ];
        CapabilityBoundingSet = [ "CAP_NET_BIND_SERVICE" ];
        NoNewPrivileges = true;
        ProtectSystem = "strict";
        ProtectHome = true;
        PrivateTmp = true;
        PrivateDevices = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectKernelLogs = true;
        ProtectControlGroups = true;
        ProtectClock = true;
        ProtectHostname = true;
        RestrictNamespaces = true;
        RestrictRealtime = true;
        LockPersonality = true;
        MemoryDenyWriteExecute = true;
        SystemCallArchitectures = "native";
        # Netlink: iroh watches the network interfaces for changes.
        RestrictAddressFamilies = "AF_INET AF_INET6 AF_UNIX AF_NETLINK";
      };
    };
  };
}
