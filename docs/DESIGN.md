# Yak VC — Design

As of 2026-10-03. This file is the source of truth; the earlier hosted copy (https://claude.ai/code/artifact/3cb80f7d-8b37-49e3-a651-cee86022d28b) predates both design reviews and is out of date. Checks that can only be done later are listed as tasks under the milestone that needs them.

## Overview

Yak VC is a client-only Fabric mod that gives players on any online-mode vanilla server proximity voice chat, with audio flowing peer-to-peer over Iroh. A small optional public service (`yakvc-server`) handles discovery, identity attestation and relay fallback; no Minecraft server mod or plugin is ever required. `yakvc` is the identifier form of the name, used for crate prefixes, the Fabric mod ID, the Java package, protocol IDs and the container image.

**Baseline**

| Area | Decision |
| --- | --- |
| Game | Minecraft 26.3 on Java 25, Fabric Loader + Fabric API, client only. Older versions are not supported. |
| Networking | `iroh` 1.3 (see dependency rules) |
| Bridge | Java's FFM API over a C ABI; no JNI |
| License | `MIT OR Apache-2.0` for the mod, crates and server (see licensing) |
| Distribution | Mod jar on Modrinth (slugs `yakvc` and `yak-vc` were free on 2026-10-03) and CurseForge; server image on GHCR; nothing on crates.io |
| Hosting | One maintainer-run VPS in US East for the default rendezvous and relay |
| Platforms | Mod: every platform Minecraft Java runs on (Windows, macOS, Linux). `yakvc-server`: Linux only, permanently, so it may rely on Linux-specific features (see deployment) |

**Goals**

- Proximity voice between players who both have the mod, on unmodified servers (vanilla, Paper, proxies).
- Voice traffic is peer-to-peer when NAT allows, relayed (still end-to-end encrypted) when it does not.
- Peers are cryptographically bound to their Minecraft UUID; impersonation is not possible without the victim's Mojang session.
- All networking, crypto, codec and audio logic lives in Rust; Java is a thin game-integration layer.
- The core is testable without launching Minecraft: `yakvc-testkit` runs whole networks in-process, and a small CLI covers what automated tests can't (real audio hardware, real NATs).

**Non-goals (v1)**

- Offline-mode (cracked) servers: no trustworthy identity source.
- Server-side moderation or admin control; there is nothing on the server to control.
- Group/radio channels, recording, video. Designed not to preclude them.
- Bedrock, Forge or NeoForge. The Rust core is loader-agnostic, so ports stay possible.

**Glossary**

| Term | Meaning |
| --- | --- |
| Game server | The Minecraft server players are connected to. Untouched. |
| Rendezvous | `yakvc-server`: discovery + attestation service, itself an Iroh endpoint. |
| EndpointId | Iroh endpoint identity: an ed25519 public key. One per client install. (Called `NodeId` in pre-1.0 Iroh releases.) |
| Match set | The Yak VC clients that see each other in their tab lists. Computed by the rendezvous from pair tokens, never named or addressed. |
| Ticket | Short-lived statement signed by a rendezvous issuer key: "EndpointId X is Minecraft UUID Y". |
| Issuer | The ed25519 key that signs tickets. Separate from any rendezvous EndpointId; clients trust a list of issuers. |
| Core | `yakvc-client`: the Rust voice engine used by both the mod and the CLI. |

## System architecture

Each player runs the Fabric mod with an embedded Rust engine. The unmodified game server already supplies tab lists and positions; `yakvc-server` handles auth, matching and relay fallback; and voice goes directly between engines.

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
│           │ FFM       │                      │           │ FFM       │
│ ┌─────────▼─────────┐ │  Voice: QUIC dgrams  │ ┌─────────▼─────────┐ │
│ │ Rust engine       │◄├──────────────────────┤►│ Rust engine       │ │
│ │ Iroh, peers, Opus │ │ direct UDP or relay  │ │ Iroh, peers, Opus │ │
│ └─────────┬─────────┘ │                      │ └─────────┬─────────┘ │
└───────────┼───────────┘                      └───────────┼───────────┘
            │ auth, pair tokens   ┌──────────────────┐     │
            └────────────────────►│  yakvc-server    │◄────┘
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
├── flake.nix               # Nix dev shell: toolchain from rust-toolchain.toml, JDK 25, cargo-deny, zig
├── .cargo/config.toml      # `cargo xtask` alias, per-target linker settings
├── xtask/                  # build/packaging utility (the only root-level Rust)
├── crates/
│   ├── yakvc-shared/       # lib: protocol, tickets, pair tokens, Mojang client
│   ├── yakvc-audio/        # lib: capture, playback, Opus, jitter buffer, spatial mixer
│   ├── yakvc-client/       # lib: the voice engine (Iroh endpoint, peers, rendezvous session)
│   ├── yakvc-ffi/          # cdylib: C ABI bridge called by the mod through FFM
│   ├── yakvc-cli/          # bin: audio and network diagnostics
│   ├── yakvc-server/       # bin (+lib): rendezvous + embedded relay
│   └── yakvc-testkit/      # lib, dev-only: in-process server + N clients for tests
├── mod/                    # Fabric mod (Gradle, Fabric Loom, Java)
│   ├── build.gradle
│   └── src/main/{java,resources}/
├── deploy/                 # server Dockerfile, example config, systemd unit
├── docs/
├── deny.toml               # cargo-deny: allowed licenses, advisories, duplicate deps
├── LICENSE-MIT
└── LICENSE-APACHE
```

**Crates**

| Crate | Kind | Depends on (internal) | Owns |
| --- | --- | --- | --- |
| `yakvc-shared` | lib | none | Wire messages + versioning, ALPN constants, datagram header, `SignedTicket` signing and `TicketVerifier`, pair-token derivation, the auth challenge digest, control-stream framing |
| `yakvc-audio` | lib | none | `cpal` devices, Opus encode/decode, resampling to 48 kHz, jitter buffer, per-source gain/pan mixer, VAD. No networking. Also hardware-free I/O for tests: WAV and tone sources, and a recording null sink. |
| `yakvc-client` | lib | shared, audio | `Engine`, in two layers: `net` (Iroh endpoint, rendezvous session, peer manager, protocol routing) and `voice` (send/receive loop, recipient selection, spatial input). Also the world model (positions, tab list), command/event API, and a `sim` feature for network impairment |
| `yakvc-ffi` | cdylib | client | `extern "C"` exports + `cbindgen` header, handle management, panic barrier, error codes, event queue encoding |
| `yakvc-cli` | bin | client, audio | Manual audio and network diagnostics (see yakvc-cli section) |
| `yakvc-server` | bin + lib | shared | Rendezvous protocol, Mojang verification, ticket signing, matcher, embedded `iroh-relay`, metrics |
| `yakvc-testkit` | lib (dev) | server, client, audio | Spawns a server and simulated clients in one process for integration tests, with optional per-peer network impairment |
| `xtask` | bin (root) | none | `cargo xtask natives`, `header`, `dist`, `server-image`, `dev`: cross-builds `yakvc-ffi`, regenerates its C header, stages libraries into the mod, runs Gradle |

Each of the seven crates is needed: `yakvc-ffi` must be its own cdylib, `yakvc-testkit` depends on both `server` and `client` so it can't live in either, and a separate `yakvc-audio` crate lets the compiler enforce that networking code never touches audio, at little cost.

**Dependency graph** (arrow = "depends on"; dashed = dev-only crate or runtime load)

```
  mod/ (Java)
      ┆ FFM downcalls
      ▼
  yakvc-ffi          yakvc-cli                         yakvc-server ──┐
      │                │                                     ▲        │
      │                ▼                                     │        │
      └──────────► yakvc-client ◄──────────── yakvc-testkit ─┘        │
                     │      ╲                  (dev-only)             │
                     ▼       ╲                                        │
                yakvc-audio   ╲                                       │
                               ▼                                      │
                             yakvc-shared ◄───────────────────────────┘
```

`yakvc-shared` and `yakvc-audio` are the leaves; `xtask` sits outside the graph and only builds `yakvc-ffi` for packaging. Not drawn: `yakvc-cli` and `yakvc-testkit` also depend on `yakvc-audio` directly, for its devices and test I/O.

**Dependency rules**

- The graph is a strict DAG: `shared` and `audio` are leaves; `client` is the only crate that combines them.
- `yakvc-server` never depends on `audio` or `client`, so the server build pulls in no ALSA/CoreAudio or Opus.
- `yakvc-shared` depends on `iroh-base` (key and EndpointId types) rather than full `iroh`. The Mojang `hasJoined` client lives in `yakvc-server`, its only user.
- Binaries contain no logic beyond argument parsing and wiring; anything worth testing lives in a lib.
- Inside `yakvc-client`, `net` knows nothing about audio. It provides verified peers (EndpointId ↔ UUID), per-peer connections, and a `Protocol` trait that receives a peer's control stream and datagrams for one protocol ID. `voice` is the only `Protocol` in v1 and uses `net` only through that trait. This keeps groups/radio, and a possible generic API for other mods (see milestones), as new protocols rather than changes to `net`. `net` could later move into its own crate without touching `voice`.
- The workspace declares `iroh` without its default `portmapper` feature (UPnP/NAT-PMP/PCP port forwarding); `yakvc-client` turns it on, since it raises the share of direct peer connections, and the server has no use for it. `deny.toml` checks only the shipped targets (iroh's wasm-only dependencies are Unlicense), allows CDLA-Permissive-2.0 for `webpki-roots`, and ignores RUSTSEC-2024-0436 (`paste`, an unmaintained build-time macro deep under iroh's `netwatch`).
- `iroh` is pinned at 1.3 (current stable as of 2026-09-28; 1.0 shipped June 2026). The design relies on these 1.x APIs, checked against its docs: `Connection::send_datagram` / `read_datagram` / `max_datagram_size`, `remote_id()`, `paths()` for path reporting, `Builder::clear_ip_transports()` for relay-only, and `Builder::empty()` / `clear_address_lookup()` to drop n0's default address lookup. The `iroh-relay` 1.3 server (feature `server`) was checked against its source on 2026-10-04: `Server::spawn(ServerConfig)` embeds it as a library; `QuicConfig` serves QUIC address discovery; `CertConfig::LetsEncrypt` provides ACME (or `Manual` for certificate files); `RelayConfig::access` takes an async `AccessControl` with `on_connect` / `on_disconnect`; `Limits::client_rx` rate-limits bytes read from each client; and `relay_service().clients().disconnect(endpoint_id, None)` drops a client. Clients retry a refused relay connection with exponential backoff capped at 16 s.
- All third-party versions are declared once in root `[workspace.dependencies]`; crates use `dep = { workspace = true }`. Edition, license (`MIT OR Apache-2.0`) and `[workspace.lints]` (`unsafe_code = "deny"` everywhere except `yakvc-ffi`) are inherited the same way.
- Nothing is published to crates.io: `[workspace.package] publish = false`, inherited by every crate. The crates are internal support code for the shipped artifacts (mod jar, server image, CLI binary), so their APIs can change freely between commits.

## Wire format

All network messages are Rust-to-Rust (Java only talks to the engine through the C ABI), so the encoding is chosen for Rust alone.

- **Control messages** (`rdv/1`, the peer control stream, and each application protocol's control stream) use `postcard`. Each direction of each protocol is one enum (for example `RdvClientMsg` and `RdvServerMsg`). Each message is sent on its QUIC stream as a varint length prefix plus the postcard bytes, capped at 128 KiB, which fits a full `SetPairs` of 2,048 tokens. A message that fails to decode closes the connection with a protocol error.
- **No in-place schema evolution.** Postcard cannot add a field that old peers ignore, so any wire change is a new protocol version (a new ALPN or application-protocol version; see versioning). Each version lives in its own module, so the server can keep serving old ones during rollouts. Protobuf was considered for additive evolution and rejected: a schema language, code generation and all-optional fields cost more than occasional version bumps with one maintainer-run server.
- **Tickets are verified over the bytes received.** A ticket travels as `SignedTicket { body: Vec<u8>, sig: [u8; 64] }`, where `body` is a postcard-encoded `TicketBody` and `sig` is ed25519 over `b"yakvc-ticket-v1" ‖ body`. Verifiers check the signature over `body` exactly as received, then decode it, so nothing is ever re-encoded and encoder changes cannot invalidate tickets. The domain tag keeps the signature from being valid for anything else. The disk cache stores the same bytes.
- **Voice datagrams** use the fixed 10-byte header described in the voice section, parsed by hand, with no serde.
- **Field conventions:** UUIDs are `[u8; 16]`; EndpointIds and issuer keys are 32-byte ed25519 public keys; times are Unix seconds as `u64`; sets travel as `Vec`s, and wire types contain no `HashMap`s.
- The FFI event buffer keeps its own TLV layout (see the FFM bridge section); it is a C ABI, not a network format.

## Identity, discovery and authentication

A client proves its UUID to the rendezvous once with a Mojang session challenge and receives a signed ticket. The rendezvous then matches clients that appear in each other's tab lists, and peers verify each other's tickets directly, with no further Mojang calls.

**Identities**

- **Client key:** one Iroh `SecretKey` per installation, stored in the mod config directory (`config/yakvc/node.key`, mode 0600). Its public half is the EndpointId.
- **Rendezvous endpoint key:** the server's Iroh `SecretKey`, used only as its dial address (EndpointId). Clients ship with the default rendezvous EndpointId and accept others in config.
- **Issuer key:** a separate ed25519 key that signs tickets. Clients trust tickets from any key in `trusted_issuers`. Keeping it apart from the endpoint key lets several rendezvous instances (regions, shards) share one issuer, and lets either key rotate without changing the other.
- **Minecraft identity:** UUID + name, proven via Mojang's session server. The Mojang access token never leaves Java (authlib makes the call).

**Authentication (client → rendezvous, ALPN `yakvc/rdv/1`)**

1. Client dials the rendezvous EndpointId. QUIC/TLS gives the server the client's EndpointId cryptographically.
2. Client sends `Hello { mod_version, uuid, name, addr, cached_ticket: Option<SignedTicket> }`. The protocol version is the ALPN's, so messages carry none.
3. If `cached_ticket` was signed by the issuer key this server holds, names this EndpointId and UUID, and has more than 2 h left, the server registers the session and skips to step 8, returning the same ticket. Otherwise it continues with step 4.
4. Server sends `Challenge { nonce: [u8; 32] }`.
5. Client derives `server_id = mc_hex_digest(SHA-1("yakvc-auth-v1" ‖ nonce ‖ client_endpoint_id ‖ rendezvous_endpoint_id))` and asks Java to call `MinecraftSessionService.joinServer(uuid, token, server_id)`.
6. Client sends `Joined`.
7. Server calls `GET https://sessionserver.mojang.com/session/minecraft/hasJoined?username=<name>&serverId=<server_id>` and requires the returned UUID to equal the claimed one.
8. Server replies with a `SignedTicket` whose body is `TicketBody { uuid, name, endpoint_id, issued_at, expires_at, issuer, dev }`, signed by the issuer key (see wire format), with a 24 h lifetime. Client caches it on disk and renews it once less than 2 h remains, at a random point in that window so renewals don't bunch up.

The `joinServer` call must never happen while the game is logging in to a server: it would replace the game's own pending session join and break the login. The client authenticates at title screen / after login completes, and the cached ticket makes this rare.

The challenge digest cannot be confused with a game login, in either direction. A game server's login hash covers a shared secret the client picks at random, so a malicious game server cannot steer a player's `joinServer` into a valid rendezvous proof. The rendezvous digest covers both EndpointIds and a domain tag, so it is useless as a game login.

**Discovery: mutual tab-list matching**

Two clients are on the same server if each one's UUID is in the other's tab list (the vanilla player-info packets). The client uses every player-info entry (`getOnlinePlayers()`), not only listed ones: tab-layout plugins often hide real players with `listed = false` and show fake entries instead, and fake entries only add harmless unmatched tokens. This needs no knowledge of server addresses, so SRV records, proxies and anycast IPs don't matter.

- For each tab-list entry `u`, the client computes a pair token `SHA-256("yakvc-pair-v1" ‖ min(me,u) ‖ max(me,u))` and sends the set to the rendezvous (`SetPairs`, then incremental `AddPairs` / `RemovePairs` as players join/leave).
- The rendezvous matches identical tokens held by two sessions whose tickets name exactly that pair, then sends each side `PeerAvailable { ticket, endpoint_addr }`, and later `PeerGone { endpoint_id }`.
- The rendezvous is only told about pairs of Yak VC users who are co-located. Tab-list entries without the mod are sent as hashes, but this is **not** strong privacy: UUIDs are public, so the rendezvous can test whether any guessed UUID is in a client's tab list, and it could brute-force against a scraped UUID list. Treat the rendezvous as trusted with tab-list contents and say so in the privacy note.
- A lurker cannot discover anyone without actually being on the server and in the other player's tab list.
- On server networks with a global tab list, players on different backends match but never hear each other: audio needs an entity position (see voice section).

**Peer handshake (ALPN `yakvc/peer/1`)**

1. The peer with the lower EndpointId dials; the other dials only if nothing has arrived after 3 s. Duplicate connections are resolved by keeping the one dialled by the lower EndpointId.
2. The dialer opens one peer control bi-stream, and both sides send `PeerHello { ticket, protocols: [(id, version)] }` on it.
3. Each side accepts only if: the signature verifies against a key in `trusted_issuers`; `ticket.endpoint_id` equals the connection's remote EndpointId; the ticket is unexpired; `ticket.uuid` is in the local tab list and is not our own UUID.
4. On success the connection is shared by every application protocol both sides list (v1 has one: `voice`, version 1). Each protocol gets its own control stream, opened by the dialer and starting with its protocol ID byte, and its datagrams start with a one-byte protocol ID. The peer control stream itself carries only `Hello` and `TicketUpdate`; QUIC handles keepalive and RTT, and closes use QUIC application close codes (`peer::CloseCode`). On any failure, including being refused by the peer during the handshake, close with an error code and do not retry for 60 s.
5. When a client renews its ticket it sends `TicketUpdate { ticket }` to every open peer, which re-runs the step 3 checks. A peer whose ticket expires without an update is closed.

One connection per peer, multiplexed by protocol ID, means one handshake and one hole-punch per peer however many protocols are added later.

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
| Voice range | 48 blocks; full volume within 4 blocks. Per player; between two players the shorter range applies |
| Jitter buffer | adaptive, target 40 ms (a floor), bounds 20–200 ms. Delay is the 95th-percentile jitter over the last 100 packets plus 20 ms; it grows at once and shrinks only between talk spurts. Clock drift is corrected by stretching or skipping one frame |
| Activation | push-to-talk (default) or VAD with 300 ms hangover |

**Send path** (`yakvc-audio` → `yakvc-client`)

1. `cpal` input callback writes samples into a lock-free SPSC ring (`rtrb`). The callback never allocates or locks.
2. A dedicated audio thread resamples to 48 kHz if needed (`rubato`), applies optional noise suppression (`nnnoiseless`, RNNoise: strong on room and low-frequency noise, weak on broadband hiss, adds 10 ms), gain and VAD, then encodes 20 ms frames.
3. If activation is on and the player is not muted, the engine picks recipients: verified peers that have a tracked entity within the local voice range, that have not sent `ReceiveState { wants_audio: false }` (deafened or muted us), capped at the 32 nearest.
4. Each frame is sent with `Connection::send_datagram`. Header: `proto: u8 (voice) | flags: u8 (end_of_talk) | seq: u32 | ts: u32` then the Opus payload (60 B at the default 24 kbps, up to 160 B at 64 kbps; well under `Connection::max_datagram_size()`). The first byte is the peer layer's protocol ID; the voice version is agreed in `PeerHello`, so there is no per-packet version byte. The last frame of a talk spurt sets `end_of_talk`. Opus in-band FEC travels inside the payload, so it needs no flag.

**Receive path**

1. Datagrams from unverified connections are dropped. Verified ones go into a per-peer jitter buffer. Playout is timed by `ts`, which advances 960 per captured frame; `seq` advances only per packet sent, so a `seq` gap means loss and a `ts`-only gap means DTX silence.
2. A mixer thread pulls one frame per peer every 20 ms: decode, use Opus FEC if the next packet is present, else PLC, and reset the stream after `end_of_talk`.
3. **Spatialization:** Java pushes the listener pose (position, yaw, pitch) and positions of tracked players by UUID every tick (20 Hz). The mixer plays one snapshot behind and interpolates positions and yaw between the last two (yaw the short way round), which adds 50 ms of position latency; after a gap of more than 250 ms it snaps instead. Players whose tab-list game mode is spectator are left out of this set. Entity tracking alone can't be used for this, because vanilla does send spectator player entities to other clients and only hides them client-side (unless the spectator is viewing through another entity). Filtering by game mode means the "no tracked entity" rule below covers them in both directions. While the local player is a spectator, the engine neither sends nor plays audio. Gain = 1 within 4 blocks, linear to 0 at the voice range. Equal-power stereo pan from azimuth relative to listener yaw, with mild attenuation for sources behind the listener.
4. **Range is enforced by the receiver:** if the speaker has no tracked entity (out of tracking range, other backend, vanished) or is beyond range, the frame is discarded. The sender-side filter only saves bandwidth. Range is a per-player setting, and the sender filters by its own range while the receiver filters by its own, so between two players the shorter range applies. A listener can never hear further than the speaker allows. The settings screen says so next to the range slider.
5. Per-peer volume and mute, master volume, the game's Voice/Speech slider and a soft limiter are applied, then the result is written to the `cpal` output ring. Output stays on `cpal` with our own panning in v1; routing PCM into Minecraft's OpenAL (for HRTF) would mean a 50 Hz per-peer stream across FFI and is post-v1. The default output device is the one whose name best matches Minecraft's selected sound device, falling back to the system default.

**Connection lifecycle**

- On `PeerAvailable` the engine connects eagerly, tracked peers nearest first, then untracked ones, up to 64 open peer connections. Hole-punching takes up to a few seconds, so connecting only on approach would clip the first words. This matters because Spigot/Paper default to a 48-block player tracking range, so a peer often becomes tracked only at the edge of voice range.
- Idle connections cost only QUIC keepalives, so matched peers stay connected whether or not they are tracked; closing untracked peers would undo the eager connect. Connections close on `PeerGone`.
- "Nearest first" is a ranking, not a dial queue: tracked peers by distance, then untracked peers, most recently tracked first (a peer never tracked counts as untracked since we learned of it). The best 64 are dialed in parallel; at the cap an incoming connection is accepted only from a peer in the best 64; once a second, connections beyond 64 close worst-ranked first. An evicted peer is dialed again once it ranks high enough, for example when it becomes tracked. `net` gets only a distance per UUID from the engine, never world or voice types.
- Iroh picks the path (direct UDP vs relay) and migrates transparently (multipath since 1.0). The engine reports the selected path per peer from `Connection::paths()` for the UI and the CLI.
- Relayed sends share a per-client budget of 512 kbps (estimated wire cost, relay framing included). Recipients on a relay-only path are served nearest-first. If they don't all fit at 24 kbps (9 peers, modelling 76 B of overhead per packet), the encoder steps down to 16 kbps (11 peers), using the same step-down as above 16 recipients. Because there is one encoder, direct peers get the lower bitrate too. Relayed recipients that still don't fit get no datagrams and show as `relay_full` until budget frees up or a direct path appears. Direct peers never count against the budget. This keeps `relay_only` users and players behind strict NATs usable in a crowd while keeping relay cost per user bounded.
- The endpoint is built from `presets::Minimal` plus our own `RelayMode::Custom` relay map. Iroh's default n0 DNS/pkarr address lookup and n0 relays are not used, because peer addresses come from the rendezvous and publishing them to a third party would leak IPs. `relay_only` uses `Builder::clear_ip_transports()`.

**Threads**

- A Tokio runtime (2 worker threads) for Iroh, rendezvous and control streams.
- Two `cpal` real-time callbacks (input, output): ring-buffer I/O only.
- One audio worker thread for resample, encode, decode and mix, talking to Tokio through bounded channels.
- A missing or failing audio device is reported as one `Error` event and never fails `Engine::start`, so networking and the game keep working without a microphone or speakers. The engine retries the device every 2 s and reports again only after it has worked in between. It never reopens a source or sink the caller supplied. `Speakers` counts underruns only after the first audio has been written. Only cpal's `DeviceNotAvailable`, `HostUnavailable` and `StreamInvalidated` count as failure; `Xrun` counts as an overrun or underrun, and other errors are ignored.
- The output device follows the game's sound device (`Engine::set_game_device`, `DeviceChoice::ClosestTo`) unless `audio.output_device` overrides it. The audio thread checks each tick which devices it should be using and reopens only the one that changed.

**Budgets**

- Mouth-to-ear latency target is about 105–185 ms: capture 10–20 + framing 20 + Opus lookahead ~5 + network 20–80 + jitter 40 + output 10–20.
- Upload is about 50 kbps per recipient while talking. Each 20 ms packet carries 60 B of Opus, a 10 B header and about 56 B of QUIC (short header ~12 B, AEAD tag 16 B) + UDP + IPv4 overhead, at 50 packets/s. IPv6 adds about 3 kbps, and the relay's TCP/TLS framing adds more. So 10 listeners nearby is about 500 kbps. Above 16 recipients the encoder steps down to 16 kbps (about 42 kbps on the wire), so the 32-recipient cap is about 1.4 Mbps. DTX and push-to-talk mean nothing is sent while silent.

## yakvc-server

`yakvc-server` is one stateless-on-restart binary that runs the rendezvous protocol on an Iroh endpoint and, optionally, an embedded `iroh-relay` on the same host. It never carries voice except as an encrypted relay hop.

**Responsibilities**

- Authenticate clients (Mojang challenge) and issue signed tickets.
- Hold live sessions and match pair tokens; push `PeerAvailable` / `PeerGone`.
- Relay fallback for peers whose NAT defeats hole-punching (embedded `iroh-relay`, HTTPS + QUIC address discovery). The relay is not an open relay for the wider Iroh ecosystem: it serves only EndpointIds that hold a live rendezvous session, after a short grace period for new connections (see `relay` below).
- Nothing durable apart from its endpoint key, issuer key and relay TLS certificate: all session state is in memory, and clients re-register within seconds of a restart (cached tickets mean no Mojang burst).

**Crate structure**

| Module | Role |
| --- | --- |
| `main.rs` | `clap` CLI: `run --config <path>`, `keygen [--issuer]`, `endpoint-id`, `issuer-id`. Wiring only. |
| `lib.rs` | `Server::builder(config).spawn()` so `yakvc-testkit` can run it in-process |
| `config` | TOML config: endpoint and issuer key paths, bind addresses, relay hostname + TLS (ACME or files), ticket lifetime, limits, metrics bind |
| `rdv` | Per-connection protocol handler for `yakvc/rdv/1` (state machine: Hello → Challenge → Joined → Registered) |
| `mojang` | `SessionServer::has_joined`; the base URL is configurable so tests can use a fake session server |
| `auth` | Runs the challenge with `mojang`, signs tickets with the issuer key |
| `matcher` | `HashMap<EndpointId, Session>` + `HashMap<PairToken, Vec<EndpointId>>`; O(changed tokens) per update |
| `relay` | Starts the `iroh-relay` server when enabled. Its `AccessControl` admits an EndpointId that has a live session. It also admits one without a session for a 30 s grace period, because a client's relay connection can arrive before its rendezvous session, and a `relay_only` client can only reach the rendezvous through the relay. If no session is registered within the grace period, or when a session ends, the server disconnects that EndpointId from the relay. Grace admissions are rate-limited per EndpointId, 3 per 10 minutes. Per source IP is impossible: checked at M3, `iroh-relay`'s `ClientRequest` does not expose the client address. When the server itself uses a relay, `spawn()` waits (up to 10 s) until it is connected to that relay, so early relay-only clients don't outlast their grace. A newer session for the same EndpointId replaces and closes the older one. |
| `limits` | Token-bucket rate limits per EndpointId and per source IP |
| `metrics` | Prometheus endpoint: sessions, auth ok/fail, matches, Mojang latency, relay bytes |

**Limits (defaults)**

- 5 auth attempts per EndpointId per minute; 30 per source IP per minute. Only challenges count (cached-ticket hellos don't), and connections arriving through the relay have no source IP, so they get only the per-EndpointId limit.
- At most 2,048 pair tokens per session and 10 pair updates per second.
- 5 s timeout on Mojang calls with one retry; auth fails closed on Mojang outage, answered with `RetryAfter { secs: 10 }`. Cached tickets keep working, and while Mojang rate-limits us a cached ticket with under 2 h left is still accepted.
- On a Mojang HTTP 429 the server stops calling `hasJoined` until Mojang's `Retry-After` (or 10 s) has passed and answers pending auths with `RetryAfter { secs }`. Clients back off exponentially (10 s doubling to 10 min, with jitter) and keep using any unexpired cached ticket meanwhile.
- Relay: each client keeps its relayed sends within a 512 kbps budget (see connection lifecycle). The server backs this with `iroh-relay`'s per-client receive limit (`Limits::client_rx`, which caps what each client sends into the relay) of 80 KB/s (640 kbps, leaving headroom for control traffic and bursts). The server exports relayed bytes as metrics (`yakvc_relay_bytes_{recv,sent}_total`); comparing them to a monthly budget is left to the metrics system.

**Dev mode**

`--insecure-dev-auth` skips Mojang and accepts the claimed UUID: the server answers `Hello` with `Registered` directly, with no `Challenge`. Tickets are marked `dev: true`, and clients reject them unless their own config sets `dev_mode = true`. This is what the CLI, testkit and local mod runs use.

**Deployment**

- Linux only. Linux-specific choices are welcome where they make the server simpler to run or safer: for example a static musl binary in a `scratch` image, systemd `Type=notify` readiness and watchdog, keys passed as systemd credentials (`LoadCredential=`), and a hardened unit (`DynamicUser=`, `AmbientCapabilities=CAP_NET_BIND_SERVICE` for ports 80/443, `ProtectSystem=strict`). Keep such code in `main.rs` and `deploy/`, so the library that `yakvc-testkit` runs in-process stays portable.
- Docker image + example `config.toml` + systemd unit in `deploy/`. For v1 the project maintainer hosts the default rendezvous and relay on a single small VPS with a public IP in US East, which gives the best average latency to both North American and European players.
- Relay bandwidth is the main cost: about 55–60 kbps in and the same out per relayed voice stream while talking. v1 is planned around the 1–5 TB/month included with a typical VPS. 1 TB/month of outbound transfer covers about 500 relayed streams around the clock if each talks 10% of the time. Metrics track the relayed share and the monthly total so capacity can be planned. The per-client relay budget (see limits) keeps any one user's cost bounded.
- Clients ship with the default server's EndpointId and relay URL. Self-hosters change two config values and add their issuer key to `trusted_issuers`. The client verifies its own ticket from the rendezvous against that list too, so a missing issuer fails at registration.
- Key files hold the 32 raw secret-key bytes. `keygen` writes them with mode 0600 and never overwrites an existing file. `run` stops cleanly on SIGTERM.
- Single instance in v1. If needed later, add regions or shards that share the issuer key, with a shared pub/sub for cross-instance matches.

## yakvc-cli

`yakvc-cli` is a developer and diagnostics tool; players never use it. It drives the same `yakvc-client::Engine` and `yakvc-audio` the mod uses, with no game attached, for the checks automated tests can't make: real sound hardware, and real networks between real machines. The rendezvous, matching and multi-player behaviour are tested in-process by `yakvc-testkit` instead.

| Command | Purpose |
| --- | --- |
| `audio devices` | List `cpal` input/output devices |
| `audio loopback [--wav file] [--null-out]` | Mic → encode → jitter buffer → decode → speakers, with stats including device overruns and underruns. Proves the audio crate alone. |
| `call listen` / `call dial <endpoint-addr>` | Two-node direct call by EndpointId with no rendezvous or auth. `listen` prints a ready-to-paste `dial` command; the address is Iroh's `EndpointAddr` as JSON. Prints path (direct/relay), RTT and loss every second. |
| `net report` | Iroh net report: NAT type, IPv4/IPv6 reachability, relay latencies. For user bug reports. Needs a relay; without one the report is empty. |
- Output is human-readable by default, with `--json` for scripted use.
- `--config <client.toml>` gives `call` and `net` the engine config; its `[rendezvous].relay` is where they get a relay. Direct-call mode never starts a rendezvous session but keeps that relay for fallback.
- Key generation lives in `yakvc-server keygen`; clients create their own key on first start.
- **Network impairment:** in-process tests run over perfect loopback, which would never exercise the jitter buffer, FEC or PLC. `yakvc-client`'s `sim` feature can wrap a peer's datagram path with seeded, deterministic loss (uniform or bursty), added delay, jitter (which reorders packets once it exceeds the frame interval) and duplication. These are applied to voice datagrams after receipt, before the jitter buffer, so they need no hook into Iroh's transport. Testkit tests set them per client and use `yakvc-audio`'s WAV source and null sink. The null sink records the decoded output, so tests can assert on concealed-frame counts and on playout delay staying within the jitter-buffer bounds. Load and soak tests are testkit tests too.

## Fabric mod and FFM bridge

The mod is a client-only Fabric mod (`"environment": "client"`) that reads game state, owns input and UI, and talks to the Rust engine through a small poll-based C ABI called with Java's Foreign Function & Memory (FFM) API. Rust never calls into the JVM: there are no upcalls.

**Target.** Minecraft 26.3 on Java 25 (every 26.3 snapshot requires Java 25), with the matching Fabric Loader and Fabric API. Older versions are not supported.

**Why FFM.** Java 25 has FFM as a final API, so JNI's one advantage, working on old Java versions, does not apply. With FFM:

- Rust exports a plain `extern "C"` API, with no `jni` crate or `JNIEnv` handling.
- The same API can be exercised from Rust tests without a JVM.
- Java arrays can be passed to native code without copying.
- The Java side has no `native` methods.

Calls are coarse (about 20 per second), so performance doesn't matter either way. Loading a library and creating downcall handles are restricted methods, so Java 24+ prints JEP 472's warning unless the launcher passes `--enable-native-access=ALL-UNNAMED`. It is only a warning today, but a future JDK will deny it by default, so the README documents the flag.

**Java side (`mod/`, package `io.github.desocketed.yakvc`, Maven group `io.github.desocketed`)**

Names below use Mojang's official mappings, which Fabric uses from 26.1 on (Minecraft is no longer obfuscated). Fabric API renamed some classes to match, for example `KeyBindingHelper` → `KeyMappingHelper`.

| Class / area | Role |
| --- | --- |
| `YakVcClient` | `ClientModInitializer`: load natives, create engine, register events and keybinds |
| `natives.NativeLoader` | Map `os.name`/`os.arch` to `natives/<os>-<arch>/` (falling back to `natives/<os>-universal/`, used for the macOS universal library), extract the library to `config/yakvc/natives/<sha256>/`, open it with `SymbolLookup.libraryLookup(path, Arena.global())`. The library is never unloaded, because it owns live Tokio and audio threads. Unsupported platform: disable the mod and show a toast. |
| `NativeBridge` | One `final MethodHandle` field per C function (below), built with `Linker.nativeLinker().downcallHandle`, plus thin typed wrappers that turn error codes into `YakVcException`. Nothing else. |
| `VoiceSession` | Per-connection lifecycle on `ClientPlayConnectionEvents` JOIN/DISCONNECT; tracks whether a login is in progress. Voice stays off until the server-data packet arrives (5 s timeout), so the MOTD opt-out is checked first. It also stays off while `getChatStatus()` is `DISABLED_BY_PROFILE` or `DISABLED_BY_LAUNCHER` (`respect_chat_restrictions`) |
| `GameStateFeeder` | On `END_CLIENT_TICK`: listener pose from the camera; tracked players from `level.players()`, minus spectators; tab-list diff from `getOnlinePlayers()`; local spectator state as an input flag; players blocked in Social Interactions as mutes (`mute_blocked_players`); the Voice/Speech slider; push to native; then drain events |
| `SessionJoiner` | Handles `JoinRequest` events on a worker thread via authlib `MinecraftSessionService.joinServer`, refuses while logging in, replies with `completeJoin` |
| `input` | Keybinds via `KeyMappingHelper`: push-to-talk (default `V`, unused by vanilla); mute, deafen and open voice menu are unbound by default to avoid clashing with minimap and utility mods |
| `ui` | Talking indicator over heads, own mic/connection HUD icon, peer list with volume/mute, settings (devices, PTT/VAD, range, bitrate, relay-only) |
| `config` | TOML in `config/yakvc/client.toml`, passed to native as a string. Includes `respect_chat_restrictions` and `mute_blocked_players`, both default `true` and both can be turned off |

Fabric API events are preferred over mixins; the target is zero mixins in v1.

**C ABI (`yakvc-ffi`, header `yakvc.h`)**

```c
typedef struct YakVcEngine YakVcEngine;  // opaque
// Every int32_t return is 0 on success or a negative YAKVC_ERR_* code.
// Strings are UTF-8 (ptr, len), not NUL-terminated. UUIDs are 16 bytes, big-endian (msb then lsb).

uint32_t yakvc_abi_version(void);
int32_t  yakvc_create(const uint8_t *config_dir, size_t dir_len,
                      const uint8_t *config_toml, size_t toml_len,
                      uint32_t abi_version, YakVcEngine **out);
void     yakvc_destroy(YakVcEngine *e);                      // graceful close, 500 ms cap
int32_t  yakvc_set_identity(YakVcEngine *e, const uint8_t uuid[16],
                            const uint8_t *name, size_t name_len);
int32_t  yakvc_set_tab_list(YakVcEngine *e, const uint8_t *uuids, size_t count);   // full replace, only when changed
int32_t  yakvc_push_world(YakVcEngine *e, const double listener[5],               // x, y, z, yaw, pitch
                          const uint8_t *uuids, const double *xyz, size_t count); // tracked players
int32_t  yakvc_set_input(YakVcEngine *e, uint32_t flags);                         // PTT | MUTED | DEAFENED | SPECTATOR
int32_t  yakvc_set_game_volume(YakVcEngine *e, float voice);                     // game's Voice/Speech slider, 0..1
int32_t  yakvc_set_peer_volume(YakVcEngine *e, const uint8_t uuid[16], float volume, bool muted);
int32_t  yakvc_complete_join(YakVcEngine *e, uint32_t request_id, bool ok);
int32_t  yakvc_poll_events(YakVcEngine *e, uint8_t *buf, size_t cap, size_t *written);
int32_t  yakvc_update_config(YakVcEngine *e, const uint8_t *toml, size_t len);
int32_t  yakvc_list_devices(YakVcEngine *e, uint8_t *buf, size_t cap, size_t *needed); // JSON; retry if needed > cap
int32_t  yakvc_set_game_device(YakVcEngine *e, const uint8_t *name, size_t len);   // game's sound device; null/empty = system default
size_t   yakvc_last_error(uint8_t *buf, size_t cap);         // message for this thread's last failed call
```

- **Header:** `cbindgen` generates `crates/yakvc-ffi/include/yakvc.h`, which is checked in. `cargo xtask header --check` fails CI when it is stale, so changes to the ABI show up in review. Java bindings are hand-written (15 functions) rather than generated with `jextract`, which is not part of the JDK.
- **ABI check:** Java calls `yakvc_abi_version()` before anything else and refuses to continue on a mismatch; `yakvc_create` checks again.
- **Argument checks:** every call rejects with `YAKVC_ERR_INVALID_ARGUMENT` rather than guessing: null or misaligned pointers with a non-zero length, invalid UTF-8, non-finite coordinates, unknown input flag bits, game volume outside 0..=1, negative or non-finite peer volume, and an event buffer too small for even one record (nothing is lost; poll again with a bigger buffer).
- **Memory:** each wrapper call uses a confined `Arena` for strings and small buffers, freed on return. `yakvc_push_world` and `yakvc_poll_events` run every tick, so their handles use `Linker.Option.critical(true)`, and Java passes `MemorySegment.ofArray(...)` heap arrays with no copy. Native code never keeps a pointer after the call returns.
- **Events** are little-endian TLV records `u16 type | u16 len | payload` written into a reusable buffer: `JoinRequest{id, server_id}`, `RendezvousState`, `PeerState{uuid, connecting|direct|relayed|relay_full|failed}`, `Talking{uuid, bool}`, `MicLevel{f32}`, `Error{msg}`. Records are never split; if the buffer fills, the rest stay queued for the next poll.
- **Threads:** `YakVcEngine` is `Send + Sync`. Every function except `yakvc_destroy` may be called from any thread: the client thread feeds the world on each tick, and `SessionJoiner`'s worker calls `complete_join`. Java makes `yakvc_destroy` the last call with a closed flag.
- **Rust side (`yakvc-ffi`):** the handle is `Box<Bridge>` passed as an opaque pointer. Every export is `#[unsafe(no_mangle)] pub extern "C"` and runs inside an `ffi_guard` that does `catch_unwind` (a panic crossing `extern "C"` would abort the game) and maps errors and panics to codes plus the thread-local `last_error` message. After a panic the engine is marked poisoned, later calls return `YAKVC_ERR_POISONED`, and the mod disables voice with a toast. This is the only crate with `unsafe` allowed.
- **`yakvc-client` API** is FFI-agnostic: `Engine::builder(config).data_dir(dir).start() -> (Engine, Events)`, where `Engine` has the same operations as above with typed arguments and `Events` has `try_next()` for the game loop and async `next()` for the CLI. The CLI uses it directly; `yakvc-ffi` is only marshalling. `yakvc-ffi` has its own Rust tests that call the `extern "C"` functions directly, so the ABI is tested without a JVM.

**Lifecycle**

1. Game start: load natives, check `yakvc_abi_version`, `yakvc_create`. Engine authenticates with the rendezvous at the title screen, using the cached ticket if valid.
2. Join a server: `VoiceSession` starts. Tab list and positions flow every tick; matched peers connect in the background.
3. Leave: empty the tab list. Peers drop and the rendezvous session stays up.
4. Game exit: `yakvc_destroy` (graceful close, 500 ms cap). The library itself stays loaded until the JVM exits.

## Build, packaging and CI

`cargo xtask` is the single entry point that builds `yakvc-ffi` for each platform and stages it into the mod's resources. Gradle calls it for host-only dev builds, and CI builds each platform on its native runner and assembles one jar.

**Native targets**

| Target | Built on | Notes |
| --- | --- | --- |
| `x86_64-unknown-linux-gnu` | Linux runner, `cargo-zigbuild` targeting glibc 2.17 | ALSA linked dynamically; PipeWire/Pulse provide ALSA compatibility |
| `aarch64-unknown-linux-gnu` | Linux runner, `cargo-zigbuild` | Raspberry Pi / ARM Linux. `alsa-sys` needs arm64 `libasound` headers in a sysroot (or build on an ARM runner). |
| `x86_64-pc-windows-msvc` | Windows runner | WASAPI via `cpal` |
| `x86_64-apple-darwin` + `aarch64-apple-darwin` | macOS runner | Merged with `lipo` into one universal `.dylib` |

libopus is built from source by the `opus` crate (via `opusic-sys`, feature `bundled`) and linked statically (needs `cmake` in CI), so the jar has no system dependency besides the OS audio stack.

**xtask commands**

- `cargo xtask natives [--target host|all|<triple>]` builds `yakvc-ffi` in release and copies it to `mod/build/natives/<os>-<arch>/` (or `--out <dir>`), with a `natives.sha256` manifest. Gradle adds that directory to the jar under `natives/`.
- `cargo xtask header [--check]` regenerates `yakvc.h` with `cbindgen`, or fails if the checked-in copy is stale.
- `cargo xtask dist` stages natives from `--from <dir>` (CI artifacts) or builds them, runs `./gradlew build`, generates `THIRD_PARTY_LICENSES` (see licensing) into the jar and the server image, and writes the jar to `dist/`.
- `cargo xtask server-image` builds the `yakvc-server` Docker image.
- `cargo xtask dev` starts a dev-mode server and prints the config snippet for `runClient`.

**Gradle integration**

- `processResources` depends on a `cargoNatives` Exec task (`cargo xtask natives --target host`), so `./gradlew runClient` just works on a dev machine.
- `-Pyakvc.prebuiltNatives=<dir>` packages an already staged natives directory instead and skips cargo, for CI packaging.
- The Gradle Java toolchain is pinned to 25, and `runClient` passes `--enable-native-access=ALL-UNNAMED`.
- `NativeLoader` checks the extracted library against `natives.sha256` before opening it.

**CI (GitHub Actions)**

1. `rust`: `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`, `cargo xtask header --check`, `cargo deny check`, `cargo test --workspace` (includes `yakvc-testkit` integration tests with dev auth and null audio).
2. `natives` (matrix: ubuntu, windows, macos): `cargo xtask natives --target <list>`, upload artifacts.
3. `mod`: download natives, `cargo xtask dist --from artifacts/`, run Fabric client gametests (load mod, create engine, destroy). Loom runs them under Xvfb on Linux when `CI` is set.
4. `release` (on tag): publish the jar to Modrinth/CurseForge, the server image to GHCR, and `yakvc-cli` binaries (for `net report` in bug reports) as GitHub release assets.

**Licensing**

- Everything in the repo is `MIT OR Apache-2.0`, the usual Rust dual license. The SPDX expression is set once in `[workspace.package]` and repeated in `fabric.mod.json`.
- Copyleft dependencies are allowed (maintainer decision, 2026-10-04); add a licence to `deny.toml` when a dependency needs it. Our own code stays `MIT OR Apache-2.0`. A file-level copyleft licence like MPL-2.0 (`attohttpc`, under iroh's port mapper) only requires that file's source to stay available, which crates.io provides. A GPL dependency would put the distributed jar or server binary as a whole under GPL terms. `deny.toml` lists the allowed licences and `cargo deny check licenses` enforces them in CI.
- MIT, Apache-2.0 and the BSD licenses of statically linked libraries like libopus require their notices to ship with the binaries, so `cargo xtask dist` generates `THIRD_PARTY_LICENSES` with `cargo-about`.

**Versioning**

- Connection protocols (`rdv/1`, `peer/1`) are versioned in the ALPN; incompatible changes bump the ALPN, and the server serves old and new side by side during rollouts. Application protocols on a peer connection (`voice` v1) are versioned in `PeerHello`, and peers use the highest version both support.
- `ABI_VERSION` ties the Java and native halves of one mod build together; a mismatch fails at `yakvc_abi_version()` before the engine starts.
- Mod version and Rust workspace version stay in lockstep: one release = one version number.

## Security, privacy and abuse

The design rule is **you only send voice to, and only play voice from, players your game can currently see as entities within range.** Most privacy properties follow from that one rule plus Mojang-backed tickets.

| Threat | Mitigation |
| --- | --- |
| Impersonating another player | Ticket requires a Mojang `hasJoined` proof for the UUID; peers check `ticket.endpoint_id` against the QUIC-authenticated remote EndpointId |
| Eavesdropping in transit | QUIC/TLS 1.3 end to end between peers; relays forward ciphertext only |
| Remote eavesdropper (not on the server) | Discovery needs mutual tab-list presence; the sender only transmits to peers with a tracked entity in range |
| Vanished staff / spectators listening | No tracked entity means no audio is sent to them or played from them. Peers whose tab-list game mode is spectator are excluded both ways. |
| Peers learn your IP address | Inherent to direct P2P. Because matched peers connect eagerly, every Yak VC user on the same server (or network, with a global tab list) learns your IP, not just nearby players. The README and settings screen say so plainly. A `relay_only` setting (opt-in, off by default) removes Iroh's IP transports (`Builder::clear_ip_transports()`), so no direct addresses are advertised and all traffic goes through the relay. Under the relay budget, that means up to about 11 nearby listeners at once. |
| Rendezvous learns who plays with whom | Only co-located mod users and their IPs; other tab-list entries stay hashed. No logs of pairs are kept; documented in a privacy note. |
| Harassment | Per-player mute and volume persisted by UUID; players blocked in vanilla Social Interactions are muted both ways (`mute_blocked_players`, default on); "only hear friends" mode (a UUID allow-list in the client config); deafen. Mutes also tell the speaker to stop sending (`ReceiveState`). |
| Flooding a peer | Per-peer cap of 100 datagrams/s and 400 B/frame; overflow drops packets, and 5 consecutive seconds over the cap closes the connection with a 10-minute ban |
| Rendezvous abuse | Rate limits per EndpointId and IP, pair-set caps, dev tickets rejected by production clients |
| Relay used by unrelated Iroh apps | Relay access restricted to EndpointIds with a live rendezvous session, apart from a rate-limited 30 s grace period that lets clients register |
| Restricted (e.g. child) accounts | Voice is disabled when the Microsoft profile or launcher disables chat (`respect_chat_restrictions`, default on) |
| Tampered native library | Releases built only in CI and downloaded from Modrinth/CurseForge with their file hashes. The in-jar SHA-256 manifest only catches a corrupted or swapped extracted copy; anyone who can modify the jar can modify the manifest too. |
| Server owner doesn't want voice | No technical control exists. The client honours an opt-out marker `[no-yakvc]` in the server MOTD and disables itself there. The MOTD arrives after join in the server-data packet, so this works for direct connects too, not just the server list. Voice waits for that packet before starting. See server opt-out below. |

**Server opt-out**

v1 uses only the MOTD marker. A denylist on the rendezvous would require clients to send it server addresses, which the privacy model avoids. If owners object to a visible marker, the planned alternative is a DNS TXT record (`_yakvc.<host>`) that the client checks locally, which also keeps server addresses on the client.

**Dependencies on Mojang**

- Auth relies on the public `sessionserver` `hasJoined` endpoint and authlib's `joinServer`, the same mechanism third-party Minecraft login services use. If Mojang changes or rate-limits it, cached tickets keep existing users working for up to 24 h while a fix ships.
- Mojang does not document `sessionserver` limits; third-party reports put them at about 400 requests per 10 s per source IP, with HTTP 429 above that. With 24 h tickets renewed at a random point in their last 2 h, even 100k active users average about 1–1.5 auths/s from the rendezvous. Bursts (mass expiry, a client bug) are handled by the 429 policy under server limits. Real limits are measured at M5.
- Mojang access tokens never cross into Rust or leave the machine except to Mojang through authlib.

## Milestones

Build order runs Rust-first: every networking and audio milestone is provable with the CLI and testkit before any Java exists, and the mod arrives at M4 as a thin integration layer.

1. **M0 Skeleton.** Virtual workspace, all seven crates as stubs with the final dependency graph, license metadata and `deny.toml`, `xtask` stub, CI running fmt/clippy/deny/test.
   - Exit: `cargo test --workspace` is green; `cargo tree -p yakvc-server -e normal` shows no `cpal`/`opus`.
2. **M1 Audio.** `yakvc-audio` (devices, Opus, jitter buffer, mixer) and `cli audio loopback`.
   - Exit: loopback runs 10 min with no underruns; added latency ≤ 60 ms.
3. **M2 Direct call.** Datagram header in `shared`, voice send/receive in `client`, `cli call listen/dial`, `cli net report`.
   - Exit: two machines on different home networks talk; with UDP blocked the call continues over the relay.
4. **M3 Rendezvous (dev auth).** Server protocol, matcher, tickets; client rendezvous session; `yakvc-testkit` integration tests.
   - Exit: in a testkit test, three clients with overlapping fake tab lists connect only to mutual matches, and gain/pan follow their moves. A testkit test with 5% bursty loss and 30 ms of jitter plays continuous audio: every lost frame is covered by FEC or PLC, and playout delay stays within the jitter-buffer bounds.
5. **M4 Mod integration.** Fabric project, `NativeLoader`, `yakvc-ffi` + `NativeBridge` (FFM), `GameStateFeeder`, push-to-talk, dev mode.
   - Exit: two dev clients on a local vanilla server hear each other spatially. A gametest confirms spectators are excluded in both directions.
6. **M5 Real auth.** Mojang challenge on server and client, `SessionJoiner`, ticket cache, Mojang 429 handling. Measure the real `sessionserver` rate limits.
   - Exit: two real accounts on a public online-mode server, no dev flags.
7. **M6 Beta.** All-platform natives, settings and peer UI, talking indicators, relay-only mode, MOTD opt-out, production server deployed. Check the CurseForge project name is free, and check the default keybinds against popular modpacks. During the beta, resize the VPS from relayed-share and monthly-transfer metrics.
   - Exit: the same jar works on Windows, macOS and Linux; public beta on Modrinth.
8. **Post-v1.** Serverless discovery, OpenAL/HRTF output, groups, NeoForge port.
   - **Read-only Java API for other mods:** talking and peer-state events, plus per-player mute and volume, so HUD and social mods can integrate without data channels.
   - **Generic P2P channels for other mods**, only if mod authors ask for it. This would extract `net` into a separate library mod that other mods register protocols with. It must first solve: per-protocol consent (a mod can't send to peers that haven't installed it), relay quotas per protocol, the fact that every peer learns your IP, a server opt-out covering all protocols, and a stable Java API. Generic position-sharing channels make PvP radar cheats easy, so server-owner opt-out matters more here than for voice.
