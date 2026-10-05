# Yak VC

Proximity voice chat for Minecraft that needs nothing on the server. You hear players near you, from the direction they stand, and quieter the further away they are. Yak VC is a client-only Fabric mod: voice goes straight from player to player, so it works on any online-mode server, including vanilla, Paper and server networks, without a plugin.

Yak VC is in development and has no release yet.

## For players

- **Requirements:** Minecraft 26.3 with Fabric Loader and Fabric API, on Windows, macOS or Linux. Everyone who wants to talk needs the mod.
- **Installing:** put the jar in your `mods` folder and add `--enable-native-access=ALL-UNNAMED` to your launcher's JVM arguments.
- **Using it:** hold `V` to talk. Settings are in the voice menu, or in Mod Menu if you have it.
- **Privacy:** other Yak VC players on the same server learn your IP address, because voice is peer to peer. The **Relay Only** setting hides it by sending everything through the Yak VC relay. Voice is encrypted from player to player.

The [user guide](docs/USER_GUIDE.md) covers all of this with screenshots, plus the settings, troubleshooting and the config file.

## For server owners

You don't need to install anything. Players who have the mod find each other through the Yak VC voice server, which checks their Minecraft accounts the same way your server does.

- **Opting out:** put `[no-yakvc]` anywhere in your server's MOTD, and the mod turns itself off for everyone on your server.
- **Running your own voice server:** `yakvc-server` is the rendezvous and relay service, available as a container image or a NixOS module. See [deploy/README.md](deploy/README.md). Players then point their config at it.

## How it works

The mod's voice engine is written in Rust and loaded by the Fabric mod through Java's Foreign Function & Memory API. Peers connect over QUIC with [Iroh](https://www.iroh.computer/) and fall back to the relay when a direct connection fails, and voice is encoded with Opus. [docs/DESIGN.md](docs/DESIGN.md) has the full design.

## Development

`nix develop` provides the pinned Rust toolchain, JDK 25 and the other build tools. Without Nix you need rustup (which reads `rust-toolchain.toml`), JDK 25, `cargo-deny` and `cmake`.

```sh
cargo test --workspace          # Rust crates
cargo xtask header              # regenerate crates/yakvc-ffi/include/yakvc.h after changing the C ABI
cargo xtask dev                 # a local dev-mode yakvc-server; prints its client.toml (--write-client-config puts it in mod/run/)
cd mod && ./gradlew runClient   # builds the native library for this machine and starts the game
cargo xtask dist                # the mod jar in dist/, with this machine's native library only
```

Release jars hold the native library for every platform; CI builds each on its own runner and packages them with `cargo xtask dist --from <dir>`. Pushing a `v<version>` tag publishes a release. To run your own `yakvc-server`, see [deploy/README.md](deploy/README.md).

## Running the mod

Yak VC loads its native library through Java's Foreign Function & Memory API. Java 25 prints a warning unless the game is started with:

```
--enable-native-access=ALL-UNNAMED
```

Add it to your launcher's JVM arguments. A future Java release will refuse to load the library without it.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
