# Minecraft P2P Voice Chat — Design

As of 2026-10-03. Living copy: https://claude.ai/code/artifact/3cb80f7d-8b37-49e3-a651-cee86022d28b

## Overview

p2pvc is a client-only Fabric mod that gives players on any online-mode vanilla server proximity voice chat, with audio flowing peer-to-peer over Iroh. A small optional public service (`p2pvc-server`) handles discovery, identity attestation and relay fallback; no Minecraft server mod or plugin is ever required. "p2pvc" is a working name used for all crate and package prefixes; rename once before the first commit.

**Goals**

- Proximity voice between players who both have the mod, on unmodified servers (vanilla, Paper, proxies).
- Voice traffic is peer-to-peer when NAT allows, relayed (still end-to-end encrypted) when it does not.
- Peers are cryptographically bound to their Minecraft UUID; impersonation is not possible without the victim's Mojang session.
- All networking, crypto, codec and audio logic lives in Rust; Java is a thin game-integration layer.
- A headless CLI can do everything the mod can, so the core is testable without launching Minecraft.

**Non-goals (v1)**

- Offline-mode (cracked) servers: no trustworthy identity source.
- Server-side moderation or admin control; there is nothing on the server to control.
- Group/radio channels, recording, video. Designed not to preclude them.
- Bedrock, Forge or NeoForge. The Rust core is loader-agnostic, so ports stay possible.

**Glossary**

| Term | Meaning |
| --- | --- |
| Game server | The Minecraft server players are connected to. Untouched. |
| Rendezvous | `p2pvc-server`: discovery + attestation service, itself an Iroh endpoint. |
| NodeId | Iroh endpoint identity: an ed25519 public key. One per client install. |
| Match set | The p2pvc clients that see each other in their tab lists. Computed by the rendezvous from pair tokens, never named or addressed. |
| Ticket | Short-lived statement signed by the rendezvous: "NodeId X is Minecraft UUID Y". |
| Core | `p2pvc-client`: the Rust voice engine used by both the mod and the CLI. |

## System architecture

Each player runs the Fabric mod with an embedded Rust engine. The unmodified game server already supplies tab lists and positions; `p2pvc-server` handles auth, matching and relay fallback; and voice goes directly between engines.

```
                     ┌──────────────────────────┐
           ┌─────────┤ Game server (unmodified) ├─────────┐
           │         │ vanilla, Paper or proxy  │         │
           │         └──────────────────────────┘         │
           │  unchanged Minecraft protocol: tab list      │
           ▼  and positions to each client                ▼
┌─ Player A's machine ──┐                      ┌─ Player B's machine ──┐
│ ┌───────────────────┐ │                      │ ┌───────────────────┐ │
│ │ Fabric mod (Java) │ │                      │ │ Fabric mod (Java) │ │
│ │ tab list, input,  │ │                      │ │ tab list, input,  │ │
│ │ UI, joinServer    │ │                      │ │ UI, joinServer    │ │
│ └─────────┬─────────┘ │                      │ └─────────┬─────────┘ │
│           │ JNI       │                      │           │ JNI       │
│ ┌─────────▼─────────┐ │  Voice: QUIC dgrams  │ ┌─────────▼─────────┐ │
│ │ Rust engine       │◄├──────────────────────┤►│ Rust engine       │ │
│ │ Iroh, peers, Opus │ │ direct UDP or relay  │ │ Iroh, peers, Opus │ │
│ └─────────┬─────────┘ │                      │ └─────────┬─────────┘ │
└───────────┼───────────┘                      └───────────┼───────────┘
            │ auth, pair tokens   ┌──────────────────┐     │
            └────────────────────►│  p2pvc-server    │◄────┘
                                  │ rendezvous +     │
                                  │ iroh-relay       │
                                  └────────┬─────────┘
                                           │ hasJoined
                                  ┌────────▼─────────┐
                                  │ Mojang session   │
                                  │ server           │
                                  └──────────────────┘
```

The game server is touched only through the normal game protocol. Mojang is called by the rendezvous (`hasJoined`) and by the mod through authlib (`joinServer`), never by the Rust engine with the player's token.

## Repository and workspace layout

The repo root holds a virtual Cargo manifest and no Rust source; every crate lives under `crates/`, the only root-level Rust is the `xtask` build helper, and the Fabric mod is a separate Gradle project under `mod/`.

```
minecraft-p2p-vc/
├── Cargo.toml              # [workspace] only: members, workspace.dependencies, lints
├── Cargo.lock
├── rust-toolchain.toml     # pinned stable toolchain + targets
├── .cargo/config.toml      # `cargo xtask` alias, per-target linker settings
├── xtask/                  # build/packaging utility (the only root-level Rust)
├── crates/
│   ├── p2pvc-shared/       # lib: protocol, tickets, pair tokens, Mojang client
│   ├── p2pvc-audio/        # lib: capture, playback, Opus, jitter buffer, spatial mixer
│   ├── p2pvc-client/       # lib: the voice engine (Iroh endpoint, peers, rendezvous session)
│   ├── p2pvc-jni/          # cdylib: JNI bridge loaded by the mod
│   ├── p2pvc-cli/          # bin: headless client + diagnostics
│   ├── p2pvc-server/       # bin (+lib): rendezvous + embedded relay
│   └── p2pvc-testkit/      # lib, dev-only: in-process server + N clients for tests
├── mod/                    # Fabric mod (Gradle, Fabric Loom, Java)
│   ├── build.gradle
│   └── src/main/{java,resources}/
├── deploy/                 # server Dockerfile, example config, systemd unit
└── docs/
```

**Crates**

| Crate | Kind | Depends on (internal) | Owns |
| --- | --- | --- | --- |
| `p2pvc-shared` | lib | none | Wire messages + versioning, ALPN constants, datagram header, `Ticket` (sign/verify), pair-token derivation, Mojang session-server client (`hasJoined`), common error types |
| `p2pvc-audio` | lib | none | `cpal` devices, Opus encode/decode, resampling to 48 kHz, jitter buffer, per-source gain/pan mixer, VAD. No networking. |
| `p2pvc-client` | lib | shared, audio | `Engine`: Iroh endpoint, rendezvous session, peer manager, voice send/receive loop, world model (positions, tab list), command/event API, `sim` module (feature-gated) |
| `p2pvc-jni` | cdylib | client | `Java_*` exports, handle management, panic barrier, event queue encoding |
| `p2pvc-cli` | bin | client, shared | Dev and diagnostics commands (see p2pvc-cli section) |
| `p2pvc-server` | bin + lib | shared | Rendezvous protocol, Mojang verification, ticket signing, matcher, embedded `iroh-relay`, metrics |
| `p2pvc-testkit` | lib (dev) | server, client | Spawns a server and simulated clients in one process for integration tests |
| `xtask` | bin (root) | none | `cargo xtask natives`, `dist`, `server-image`, `dev`: cross-builds `p2pvc-jni`, stages libraries into the mod, runs Gradle |

**Dependency graph** (arrow = "depends on"; dashed = dev-only crate or runtime load)

```
  mod/ (Java)
      ┆ System.load
      ▼
  p2pvc-jni          p2pvc-cli                         p2pvc-server ──┐
      │                │  ╲                                  ▲        │
      │                ▼   ╲                                 │        │
      └──────────► p2pvc-client ◄──────────── p2pvc-testkit ─┘        │
                     │      ╲     ╲            (dev-only)             │
                     ▼       ╲     ╲                                  │
                p2pvc-audio   ╲     ╲                                 │
                               ▼     ▼                                │
                             p2pvc-shared ◄───────────────────────────┘
```

`p2pvc-shared` and `p2pvc-audio` are the leaves; `xtask` sits outside the graph and only builds `p2pvc-jni` for packaging.

**Dependency rules**

- The graph is a strict DAG: `shared` and `audio` are leaves; `client` is the only crate that combines them.
- `p2pvc-server` never depends on `audio` or `client`, so the server build pulls in no ALSA/CoreAudio or Opus.
- `p2pvc-shared` depends on `iroh-base` (key and NodeId types) rather than full `iroh`, and gates the Mojang HTTP client behind a `mojang` feature.
- Binaries contain no logic beyond argument parsing and wiring; anything worth testing lives in a lib.
- All third-party versions are declared once in root `[workspace.dependencies]`; crates use `dep = { workspace = true }`. Edition, license and `[workspace.lints]` (`unsafe_code = "deny"` everywhere except `p2pvc-jni`) are inherited the same way.

## Identity, discovery and authentication

A client proves its UUID to the rendezvous once with a Mojang session challenge and receives a signed ticket. The rendezvous then matches clients that appear in each other's tab lists, and peers verify each other's tickets directly, with no further Mojang calls.

**Identities**

- **Client key:** one Iroh `SecretKey` per installation, stored in the mod config directory (`config/p2pvc/node.key`, mode 0600). Its public half is the NodeId.
- **Rendezvous key:** the server's Iroh `SecretKey`. It is the server's dial address (NodeId) *and* the ticket-signing key. Clients ship with the default rendezvous NodeId and accept a list of trusted ones in config.
- **Minecraft identity:** UUID + name, proven via Mojang's session server. The Mojang access token never leaves Java (authlib makes the call).

**Authentication (client → rendezvous, ALPN `p2pvc/rdv/1`)**

1. Client dials the rendezvous NodeId. QUIC/TLS gives the server the client's NodeId cryptographically.
2. Client sends `Hello { proto, mod_version, uuid, name, node_addr }`.
3. If a cached, unexpired ticket for this NodeId exists, client sends it instead and skips to step 8.
4. Server sends `Challenge { nonce: [u8; 32] }`.
5. Client derives `server_id = mc_hex_digest(SHA-1("p2pvc-auth-v1" ‖ nonce ‖ client_node_id ‖ rendezvous_node_id))` and asks Java to call `MinecraftSessionService.joinServer(uuid, token, server_id)`.
6. Client sends `Joined`.
7. Server calls `GET https://sessionserver.mojang.com/session/minecraft/hasJoined?username=<name>&serverId=<server_id>` and requires the returned UUID to equal the claimed one.
8. Server replies `Ticket { uuid, name, node_id, issued_at, expires_at, issuer, sig }` (ed25519 by the rendezvous key, 12 h lifetime). Client caches it on disk.

The `joinServer` call must never happen while the game is logging in to a server: it would replace the game's own pending session join and break the login. The client authenticates at title screen / after login completes, and the cached ticket makes this rare.

**Discovery: mutual tab-list matching**

Two clients are on the same server if each one's UUID is in the other's tab list (the vanilla player list packets). This needs no knowledge of server addresses, so SRV records, proxies and anycast IPs don't matter.

- For each tab-list entry `u`, the client computes a pair token `SHA-256("p2pvc-pair-v1" ‖ min(me,u) ‖ max(me,u))` and sends the set to the rendezvous (`SetPairs`, then incremental `AddPairs` / `RemovePairs` as players join/leave).
- The rendezvous matches identical tokens held by two sessions whose tickets name exactly that pair, then sends each side `PeerAvailable { ticket, node_addr }`, and later `PeerGone { node_id }`.
- The rendezvous learns only pairs of p2pvc users who are co-located; tab-list entries without the mod stay hashed.
- A lurker cannot discover anyone without actually being on the server and in the other player's tab list.
- On server networks with a global tab list, players on different backends match but never hear each other: audio needs an entity position (see voice section).

**Peer handshake (ALPN `p2pvc/voice/1`)**

1. The peer with the lower NodeId dials; the other dials only if nothing has arrived after 3 s. Duplicate connections are resolved by keeping the one dialled by the lower NodeId.
2. Both open a control bi-stream and send `PeerHello { proto, ticket, caps }`.
3. Each side accepts only if: the signature verifies against a trusted rendezvous key; `ticket.node_id` equals the connection's remote NodeId; the ticket is unexpired; `ticket.uuid` is in the local tab list and is not our own UUID.
4. On success the connection carries voice datagrams plus control messages (`Talking`, `MuteState`, `ReceiveState`, `Ping`). On any failure, close with an error code and do not retry for 60 s.

**Serverless discovery (post-v1)**

The same pair tokens can work as `iroh-gossip` topics bootstrapped from the BitTorrent Mainline DHT. Peers would then authenticate each other with a direct Mojang challenge (A sends a nonce, B calls `joinServer`, A calls `hasJoined`). This removes the rendezvous but costs one Mojang round trip per new peer and is harder to debug, so it stays a fallback behind a config flag.

## Voice transport and audio pipeline

Voice is 48 kHz mono Opus in 20 ms frames, sent as unreliable QUIC datagrams on each peer's Iroh connection. Each frame goes only to peers within voice range, and the receiver spatializes it from game positions it already knows.

**Audio parameters (defaults, configurable)**

| Parameter | Value |
| --- | --- |
| Sample rate / channels | 48 kHz, mono capture; stereo output |
| Frame | 20 ms (960 samples) |
| Codec | Opus, `VOIP` application, 24 kbps (16–64), in-band FEC on, DTX on |
| Voice range | 48 blocks; full volume within 4 blocks |
| Jitter buffer | adaptive, target 40 ms, bounds 20–200 ms |
| Activation | push-to-talk (default) or VAD with 300 ms hangover |

**Send path** (`p2pvc-audio` → `p2pvc-client`)

1. `cpal` input callback writes samples into a lock-free SPSC ring (`rtrb`). The callback never allocates or locks.
2. A dedicated audio thread resamples to 48 kHz if needed (`rubato`), applies optional noise suppression (`nnnoiseless`), gain and VAD, then encodes 20 ms frames.
3. If activation is on and the player is not muted, the engine picks recipients: verified peers that have a tracked entity within `range + 8` blocks of the local player, that have not sent `ReceiveState { wants_audio: false }` (deafened or muted us), capped at the 32 nearest.
4. Each frame is sent with `Connection::send_datagram`. Header: `ver: u8 | flags: u8 (end_of_talk, fec) | seq: u32 | ts: u32` then the Opus payload (~60–120 B, well under the datagram MTU). The last frame of a talk spurt sets `end_of_talk`.

**Receive path**

1. Datagrams from unverified connections are dropped. Verified ones go into a per-peer jitter buffer ordered by `seq`.
2. A mixer thread pulls one frame per peer every 20 ms: decode, use Opus FEC if the next packet is present, else PLC, and reset the stream after `end_of_talk`.
3. **Spatialization:** Java pushes the listener pose (position, yaw, pitch) and positions of tracked players by UUID every tick (20 Hz), and the mixer interpolates between snapshots. Gain = 1 within 4 blocks, linear to 0 at the voice range. Equal-power stereo pan from azimuth relative to listener yaw, with mild attenuation for sources behind the listener.
4. **Range is enforced by the receiver:** if the speaker has no tracked entity (out of tracking range, other backend, vanished) or is beyond range, the frame is discarded. The sender-side filter only saves bandwidth.
5. Per-peer volume and mute, master volume and a soft limiter are applied, then the result is written to the `cpal` output ring.

**Connection lifecycle**

- On `PeerAvailable` the engine connects eagerly, nearest first, up to 64 open peer connections. Hole-punching takes up to a few seconds, so connecting only on approach would clip the first words.
- Idle connections cost only QUIC keepalives. A connection is closed after the peer has been untracked or beyond 160 blocks for 60 s, or on `PeerGone`.
- Iroh picks the path (direct UDP vs relay) and migrates transparently. The engine exposes the current path per peer for the UI and the CLI.

**Threads**

- A Tokio runtime (2 worker threads) for Iroh, rendezvous and control streams.
- Two `cpal` real-time callbacks (input, output): ring-buffer I/O only.
- One audio worker thread for resample, encode, decode and mix, talking to Tokio through bounded channels.

**Budgets**

- Mouth-to-ear latency target is 120–180 ms: capture 10–20 + framing 20 + network 20–80 + jitter 40 + output 10–20.
- Upload is about 35 kbps per recipient including QUIC/UDP/IP overhead, so 10 listeners nearby is about 350 kbps. Above 16 recipients the encoder steps down to 16 kbps.

## p2pvc-server

`p2pvc-server` is one stateless-on-restart binary that runs the rendezvous protocol on an Iroh endpoint and, optionally, an embedded `iroh-relay` on the same host. It never carries voice except as an encrypted relay hop.

**Responsibilities**

- Authenticate clients (Mojang challenge) and issue signed tickets.
- Hold live sessions and match pair tokens; push `PeerAvailable` / `PeerGone`.
- Relay fallback for peers whose NAT defeats hole-punching (embedded `iroh-relay`, HTTPS + QUIC address discovery).
- Nothing durable: all state is in memory, and clients re-register within seconds of a restart.

**Crate structure**

| Module | Role |
| --- | --- |
| `main.rs` | `clap` CLI: `run --config <path>`, `keygen`, `node-id`. Wiring only. |
| `lib.rs` | `Server::builder(config).spawn()` so `p2pvc-testkit` can run it in-process |
| `config` | TOML config: key paths, bind addresses, relay hostname + TLS (ACME or files), ticket lifetime, limits, metrics bind |
| `rdv` | Per-connection protocol handler for `p2pvc/rdv/1` (state machine: Hello → Challenge → Joined → Registered) |
| `auth` | Uses `p2pvc_shared::mojang` to call `hasJoined`, signs tickets with the server key |
| `matcher` | `HashMap<NodeId, Session>` + `HashMap<PairToken, SmallVec<[NodeId; 2]>>`; O(changed tokens) per update |
| `relay` | Starts `iroh-relay` server when enabled |
| `limits` | Token-bucket rate limits per NodeId and per source IP |
| `metrics` | Prometheus endpoint: sessions, auth ok/fail, matches, Mojang latency, relay bytes |

**Limits (defaults)**

- 5 auth attempts per NodeId per minute; 30 per source IP per minute.
- At most 2,048 pair tokens per session and 10 pair updates per second.
- 5 s timeout on Mojang calls with one retry; auth fails closed on Mojang outage (cached tickets keep working).

**Dev mode**

`--insecure-dev-auth` skips Mojang and accepts the claimed UUID. Tickets are marked `dev: true`, and clients reject them unless their own config sets `dev_mode = true`. This is what the CLI, testkit and local mod runs use.

**Deployment**

- Docker image + example `config.toml` + systemd unit in `deploy/`. A single small VPS with a public IP serves v1.
- Relay bandwidth is the main cost: about 35 kbps per relayed voice stream. Metrics track the relayed share so capacity can be planned.
- Clients ship with the default server's NodeId and relay URL. Self-hosters change two config values and add their key to `trusted_issuers`.
- Single instance in v1. If needed later, shard sessions by NodeId with a shared pub/sub for cross-shard matches.

## p2pvc-cli

`p2pvc-cli` drives the same `p2pvc-client::Engine` the mod uses, with no game attached, so every networking and audio milestone can be built and tested before touching Java. It stands in for the game by supplying fake identity, tab-list and position inputs.

| Command | Purpose |
| --- | --- |
| `keygen [--out]` | Create a client key; print its NodeId |
| `audio devices` | List `cpal` input/output devices |
| `audio loopback` | Mic → encode → jitter buffer → decode → speakers, with stats. Proves the audio crate alone. |
| `call listen` / `call dial <node-addr>` | Two-node direct call by NodeId with no rendezvous or auth. Prints path (direct/relay), RTT and loss. |
| `net report` | Iroh net report: NAT type, IPv4/IPv6 reachability, relay latencies. For user bug reports. |
| `join --server <node-id> --dev-uuid <uuid> --sees <uuid,...> [--pos x,y,z]` | Full client against a dev-mode rendezvous: fake identity, tab list and position. Real mic/speakers, or `--wav` input and `--null-out`. |
| `bot --count N --wav file --spread R` | N simulated peers in one process speaking a WAV at random positions within R blocks. Load and soak testing. |
| `ticket inspect <file>` | Decode and verify a cached ticket |

- `join` reads stdin commands (`pos 10 64 -3`, `ptt on`, `mute <uuid>`) so a test script can move fake players around.
- Output is human-readable by default, with `--json` for scripted assertions in CI.
- `bot` uses `p2pvc-client`'s `sim` module (behind a `sim` feature: fake game inputs, WAV source, null sink). `p2pvc-testkit` builds on the same module, so load tests and integration tests can't drift apart, and the CLI never depends on the server crate.

## Fabric mod and JNI bridge

The mod is a client-only Fabric mod (`"environment": "client"`) that reads game state, owns input and UI, and talks to the Rust engine through a small poll-based JNI API. Rust never calls into the JVM from its own threads.

**Why JNI.** JNI works on every Java version and loader. The FFM (Panama) API is only final from Java 22, and the target Minecraft version's Java baseline must be checked; even where FFM is available, JNI keeps one code path. Calls are coarse (about 20 per second), so JNI overhead is irrelevant.

**Java side (`mod/`, package `dev.p2pvc` as a placeholder)**

| Class / area | Role |
| --- | --- |
| `P2pvcClient` | `ClientModInitializer`: load natives, create engine, register events and keybinds |
| `natives.NativeLoader` | Map `os.name`/`os.arch` to `natives/<os>-<arch>/`, extract the library to `config/p2pvc/natives/<sha256>/`, `System.load`. Unsupported platform: disable the mod and show a toast. |
| `NativeBridge` | `static native` declarations (below), nothing else |
| `VoiceSession` | Per-connection lifecycle on `ClientPlayConnectionEvents` JOIN/DISCONNECT; tracks whether a login is in progress |
| `GameStateFeeder` | On `END_CLIENT_TICK`: listener pose from the camera; tracked players from `world.getPlayers()`; tab-list diff from `getPlayerList()`; push to native; then drain events |
| `SessionJoiner` | Handles `JoinRequest` events on a worker thread via authlib `MinecraftSessionService.joinServer`, refuses while logging in, replies with `completeJoin` |
| `input` | Keybinds via `KeyBindingHelper`: push-to-talk, mute, deafen, open voice menu |
| `ui` | Talking indicator over heads, own mic/connection HUD icon, peer list with volume/mute, settings (devices, PTT/VAD, range, bitrate, relay-only) |
| `config` | TOML in `config/p2pvc/client.toml`, passed to native as a string |

Fabric API events are preferred over mixins; the target is zero mixins in v1.

**JNI surface (`NativeBridge`)**

```java
static native long   create(String configDir, String configToml, int abiVersion); // handle; throws on ABI mismatch
static native void   destroy(long h);
static native void   setIdentity(long h, long uuidMsb, long uuidLsb, String name);
static native void   setTabList(long h, long[] uuidPairs);              // full replace, only when changed
static native void   pushWorld(long h, double[] listener,               // x, y, z, yaw, pitch
                               long[] uuidPairs, double[] positions);   // xyz per tracked player
static native void   setInput(long h, int flags);                       // PTT | MUTED | DEAFENED
static native void   setPeerVolume(long h, long msb, long lsb, float volume, boolean muted);
static native void   completeJoin(long h, int requestId, boolean ok);
static native int    pollEvents(long h, java.nio.ByteBuffer directBuf); // bytes written
static native void   updateConfig(long h, String configToml);
static native String listDevices(long h);                               // JSON
```

- **Events** are little-endian TLV records `u16 type | u16 len | payload` written into a reusable direct `ByteBuffer`: `JoinRequest{id, server_id}`, `RendezvousState`, `PeerState{uuid, connecting|direct|relayed|failed}`, `Talking{uuid, bool}`, `MicLevel{f32}`, `Error{code, msg}`. Both sides check `ABI_VERSION`.
- **Rust side (`p2pvc-jni`):** the handle is `Box<Bridge>` as `jlong`. Every export runs inside an `ffi_guard` that does `catch_unwind` and turns panics or errors into a Java `RuntimeException`. This is the only crate with `unsafe` allowed.
- **`p2pvc-client` API** is JNI-agnostic: `Engine::start(Config) -> (EngineHandle, EventReceiver)`, where `EngineHandle` has the same operations as above with typed arguments. The CLI uses it directly; `p2pvc-jni` is only marshalling.

**Lifecycle**

1. Game start: load natives, `create`. Engine authenticates with the rendezvous at the title screen, using the cached ticket if valid.
2. Join a server: `VoiceSession` starts. Tab list and positions flow every tick; matched peers connect in the background.
3. Leave: empty the tab list. Peers drop and the rendezvous session stays up.
4. Game exit: `destroy` (graceful close, 500 ms cap).

## Build, packaging and CI

`cargo xtask` is the single entry point that builds `p2pvc-jni` for each platform and stages it into the mod's resources. Gradle calls it for host-only dev builds, and CI builds each platform on its native runner and assembles one jar.

**Native targets**

| Target | Built on | Notes |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Linux runner, `cargo-zigbuild` targeting glibc 2.17 | ALSA linked dynamically; PipeWire/Pulse provide ALSA compatibility |
| `aarch64-unknown-linux-gnu` | Linux runner, `cargo-zigbuild` | Raspberry Pi / ARM Linux |
| `x86_64-pc-windows-msvc` | Windows runner | WASAPI via `cpal` |
| `x86_64-apple-darwin` + `aarch64-apple-darwin` | macOS runner | Merged with `lipo` into one universal `.dylib` |

libopus is built from source and linked statically (needs `cmake` in CI), so the jar has no system dependency besides the OS audio stack.

**xtask commands**

- `cargo xtask natives [--target host|all|<triple>]` builds `p2pvc-jni` in release and copies it to `mod/src/main/resources/natives/<os>-<arch>/` (gitignored), with a `natives.sha256` manifest.
- `cargo xtask dist` stages natives from `--from <dir>` (CI artifacts) or builds them, runs `./gradlew build`, and writes the jar to `dist/`.
- `cargo xtask server-image` builds the `p2pvc-server` Docker image.
- `cargo xtask dev` starts a dev-mode server and prints the config snippet for `runClient` / the CLI.

**Gradle integration**

- `processResources` depends on a `cargoNatives` Exec task (`cargo xtask natives --target host`), so `./gradlew runClient` just works on a dev machine.
- `-Pp2pvc.prebuiltNatives=<dir>` skips cargo for CI packaging.
- `NativeLoader` checks the extracted library against `natives.sha256` before `System.load`.

**CI (GitHub Actions)**

1. `rust`: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (includes `p2pvc-testkit` integration tests with dev auth and null audio).
2. `natives` (matrix: ubuntu, windows, macos): `cargo xtask natives --target <list>`, upload artifacts.
3. `mod`: download natives, `cargo xtask dist --from artifacts/`, run Fabric client gametests headless (load mod, create engine, destroy).
4. `release` (on tag): publish the jar to Modrinth/CurseForge and the server image to GHCR.

**Versioning**

- Wire protocols (`rdv/1`, `voice/1`) are versioned in the ALPN; incompatible changes bump the ALPN, and the server serves old and new side by side during rollouts.
- `ABI_VERSION` ties the Java and native halves of one mod build together; a mismatch fails at `create`.
- Mod version and Rust workspace version stay in lockstep: one release = one version number.

## Security, privacy and abuse

The design rule is **you only send voice to, and only play voice from, players your game can currently see as entities within range.** Most privacy properties follow from that one rule plus Mojang-backed tickets.

| Threat | Mitigation |
| --- | --- |
| Impersonating another player | Ticket requires a Mojang `hasJoined` proof for the UUID; peers check `ticket.node_id` against the QUIC-authenticated remote NodeId |
| Eavesdropping in transit | QUIC/TLS 1.3 end to end between peers; relays forward ciphertext only |
| Remote eavesdropper (not on the server) | Discovery needs mutual tab-list presence; the sender only transmits to peers with a tracked entity in range |
| Vanished staff / spectators listening | No tracked entity means no audio is sent to them or played from them. Peers whose tab-list game mode is spectator are excluded both ways. |
| Peers learn your IP address | Inherent to direct P2P. A `relay_only` setting stops advertising direct addresses so all traffic goes through the relay (needs confirming against Iroh's current API; see open questions). |
| Rendezvous learns who plays with whom | Only co-located mod users and their IPs; other tab-list entries stay hashed. No logs of pairs are kept; documented in a privacy note. |
| Harassment | Per-player mute and volume persisted by UUID; "only hear friends" mode; deafen. Mutes also tell the speaker to stop sending (`ReceiveState`). |
| Flooding a peer | Per-peer cap of 100 datagrams/s and 400 B/frame; overflow drops packets, and sustained abuse closes the connection with a 10-minute ban |
| Rendezvous abuse | Rate limits per NodeId and IP, pair-set caps, dev tickets rejected by production clients |
| Tampered native library | SHA-256 manifest checked before load; releases built only in CI |
| Server owner doesn't want voice | No technical control exists. The client honours an opt-out marker `[no-p2pvc]` in the server MOTD and disables itself there. |

**Dependencies on Mojang**

- Auth relies on the public `sessionserver` `hasJoined` endpoint and authlib's `joinServer`, the same mechanism third-party Minecraft login services use. If Mojang changes or rate-limits it, cached tickets keep existing users working for up to 12 h while a fix ships.
- Mojang access tokens never cross into Rust or leave the machine except to Mojang through authlib.

## Milestones

Build order runs Rust-first: every networking and audio milestone is provable with the CLI before any Java exists, and the mod arrives at M4 as a thin integration layer.

1. **M0 Skeleton.** Virtual workspace, all seven crates as stubs with the final dependency graph, `xtask` stub, CI running fmt/clippy/test.
   - Exit: `cargo test --workspace` is green; `cargo tree -p p2pvc-server` shows no `cpal`/`opus`.
2. **M1 Audio.** `p2pvc-audio` (devices, Opus, jitter buffer, mixer) and `cli audio loopback`.
   - Exit: loopback runs 10 min with no underruns; added latency ≤ 60 ms.
3. **M2 Direct call.** Datagram header in `shared`, voice send/receive in `client`, `cli call listen/dial`, `cli net report`.
   - Exit: two machines on different home networks talk; with UDP blocked the call continues over the relay.
4. **M3 Rendezvous (dev auth).** Server protocol, matcher, tickets; client rendezvous session; `cli join` and `cli bot`; `p2pvc-testkit` integration tests.
   - Exit: three CLI clients with overlapping fake tab lists connect only to mutual matches, and gain/pan follow `pos` commands.
5. **M4 Mod integration.** Fabric project, `NativeLoader`, `p2pvc-jni`, `GameStateFeeder`, push-to-talk, dev mode.
   - Exit: two dev clients on a local vanilla server hear each other spatially.
6. **M5 Real auth.** Mojang challenge on server and client, `SessionJoiner`, ticket cache.
   - Exit: two real accounts on a public online-mode server, no dev flags.
7. **M6 Beta.** All-platform natives, settings and peer UI, talking indicators, relay-only mode, MOTD opt-out, production server deployed.
   - Exit: the same jar works on Windows, macOS and Linux; public beta on Modrinth.
8. **Post-v1.** Serverless discovery, OpenAL/HRTF output, groups, NeoForge port.

## Open questions

Each item below needs an answer before the milestone named in brackets; none of them blocks M0.

- [ ] **Project name** to replace the `p2pvc` placeholder in crates, packages and ALPNs. [M0]
- [ ] **License** for the mod, crates and server. [M0]
- [ ] **Crate split:** keep `audio`, `client`, `jni` and `testkit` as separate crates beyond `server` / `cli` / `shared`, or fold some together? [M0]
- [ ] **Iroh version:** pin the current stable release and confirm the datagram API, the relay-only option (no direct-address advertising) and the `iroh-relay` server feature set. [M2]
- [ ] **Separate ticket-signing key** from the server's endpoint key, so keys can rotate without changing the dial address? v1 assumes one key. [M3]
- [ ] **Target Minecraft version** and its Java baseline. This decides whether FFM is even an option, and which Fabric API and authlib signatures to use. [M4]
- [ ] **Output path:** keep `cpal` + our own panning, or hand PCM to Minecraft's OpenAL for HRTF and the game's sound sliders? At minimum, scale by the game's Voice/Speech volume. [M4]
- [ ] **Spectator tracking:** confirm whether vanilla clients receive spectator player entities, to validate the exclusion rule. [M4]
- [ ] **Default keybinds** that don't clash with vanilla or popular mods. [M4]
- [ ] **Mojang `hasJoined` limits** as seen from one server IP at expected auth volume. [M5]
- [ ] **Opt-out mechanism:** is a MOTD marker enough, or should the rendezvous also keep an admin-requested denylist (which needs a way to identify servers)? [M6]
- [ ] **Hosting:** who runs the default rendezvous and relay, in which region(s), with what bandwidth budget. [M6]
