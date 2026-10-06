{
  description = "Yak VC: development shell, and yakvc-server as a package and NixOS module";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { self, nixpkgs, rust-overlay, ... }:
    let
      systems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      }));
      # ARM Linux's libasound, which `cargo xtask natives` links the aarch64
      # Linux build against. Nix substitutes it from the binary cache on any
      # host, no ARM machine needed.
      aarch64Alsa = nixpkgs.legacyPackages.aarch64-linux.alsa-lib;
      # yakvc-server is Linux only.
      linuxSystems = [ "x86_64-linux" "aarch64-linux" ];
      forLinux = f: nixpkgs.lib.genAttrs linuxSystems (system: f (import nixpkgs {
        inherit system;
        overlays = [ rust-overlay.overlays.default ];
      }));
    in
    {
      packages = forLinux (pkgs: rec {
        yakvc-server = pkgs.callPackage ./nix/yakvc-server.nix { };
        default = yakvc-server;
      });

      nixosModules.default = import ./nix/module.nix self;

      # `nix flake check` builds the server (running its tests) and evaluates
      # the module's systemd unit.
      checks = forLinux (pkgs: {
        yakvc-server = self.packages.${pkgs.stdenv.hostPlatform.system}.yakvc-server;
        module = (nixpkgs.lib.nixosSystem {
          inherit (pkgs.stdenv.hostPlatform) system;
          modules = [
            self.nixosModules.default
            {
              services.yakvc-server = {
                enable = true;
                domain = "relay.example.com";
                acmeContact = "admin@example.com";
                openFirewall = true;
              };
              boot.loader.grub.enable = false;
              fileSystems."/" = { device = "/dev/null"; fsType = "ext4"; };
              system.stateVersion = "26.05";
            }
          ];
        }).config.systemd.units."yakvc-server.service".unit;
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            jdk25
            cargo-deny
            cargo-about
            cargo-zigbuild
            zig
            cmake
            pkg-config
            # GitHub CLI and JSON tooling, from the Nix store so they survive
            # container restarts.
            gh
            jq
            # git pushes over SSH with the key in $HOME/.claude/ssh.
            openssh
          ] ++ lib.optionals stdenv.hostPlatform.isLinux [ alsa-lib ];
          JAVA_HOME = pkgs.jdk25.home;
          # pkg-config's per-target search path, read by alsa-sys when
          # cross-building for ARM Linux.
          PKG_CONFIG_PATH_aarch64_unknown_linux_gnu = "${aarch64Alsa.dev}/lib/pkgconfig";
        };
      });
    };
}
