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
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = with pkgs; [
            (rust-bin.fromRustupToolchainFile ./rust-toolchain.toml)
            jdk25
            cargo-deny
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
        };
      });
    };
}
