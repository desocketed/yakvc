# Yak VC

Peer-to-peer proximity voice chat for Minecraft, as a client-only Fabric mod. Works on online-mode servers without any server-side mod. See [docs/DESIGN.md](docs/DESIGN.md).

## Development

`nix develop` provides the pinned Rust toolchain, JDK 25 and the other build tools. Without Nix you need rustup (which reads `rust-toolchain.toml`), JDK 25, `cargo-deny` and `cmake`.

```sh
cargo test --workspace          # Rust crates
cargo xtask header              # regenerate crates/yakvc-ffi/include/yakvc.h after changing the C ABI
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
