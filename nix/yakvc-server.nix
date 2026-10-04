# yakvc-server built with the toolchain pinned in rust-toolchain.toml.
{ lib, rust-bin, makeRustPlatform }:
let
  channel = (lib.importTOML ../rust-toolchain.toml).toolchain.channel;
  rust = rust-bin.stable.${channel}.minimal;
  rustPlatform = makeRustPlatform { cargo = rust; rustc = rust; };
in
rustPlatform.buildRustPackage {
  pname = "yakvc-server";
  version = (lib.importTOML ../Cargo.toml).workspace.package.version;

  # Cargo needs every workspace member to resolve the workspace, even though
  # only the server and its dependencies are built. A test parses the example
  # deploy config.
  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../crates
      ../xtask
      ../deploy/config.toml
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  # Unsandboxed Nix builds (such as in the dev container) otherwise leave a
  # cargo cache in /homeless-shelter, which makes every later build fail.
  preUnpack = ''export HOME="$(mktemp -d)"'';

  cargoBuildFlags = [ "--package" "yakvc-server" ];
  cargoTestFlags = [ "--package" "yakvc-server" ];

  meta = {
    description = "Yak VC rendezvous server with embedded relay";
    license = with lib.licenses; [ mit asl20 ];
    mainProgram = "yakvc-server";
    # The server is Linux only by design.
    platforms = lib.platforms.linux;
  };
}
