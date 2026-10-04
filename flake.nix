{
  description = "Yak VC development shell";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, rust-overlay, ... }:
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
    in
    {
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
          ] ++ lib.optionals stdenv.hostPlatform.isLinux [ alsa-lib ];
          JAVA_HOME = pkgs.jdk25.home;
          # pkg-config's per-target search path, read by alsa-sys when
          # cross-building for ARM Linux.
          PKG_CONFIG_PATH_aarch64_unknown_linux_gnu = "${aarch64Alsa.dev}/lib/pkgconfig";
        };
      });
    };
}
