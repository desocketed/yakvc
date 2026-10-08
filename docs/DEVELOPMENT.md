# Yak VC — Development process

How the implementation is built: milestones from [DESIGN.md](DESIGN.md), worked on by parallel agents against agreed crate APIs. DESIGN.md says *what* to build; this file says *how* the work is organized and where it stands.

## Status

Open work, exit checks and decisions are tracked as issues under the milestones on GitHub (`desocketed/yakvc`); this table is the summary.

| Milestone | State |
| --- | --- |
| M0 Skeleton | Done. Workspace, crate stubs, xtask (`header`, `natives`), CI, plus a minimal Fabric mod that loads `yakvc-ffi` through FFM (verified in `runClient` on 26.3). A Fabric client gametest (`scripts/gametest-headless.sh`, also in CI) checks that the mod and native engine load and a singleplayer world renders; it passed from a fresh clone 2026-10-04. |
| Relay feature check (from M2) | Done 2026-10-04. All needed `iroh-relay` 1.3 server features exist; details in DESIGN.md dependency rules. Found and fixed a gap: `relay_only` clients could not reach the rendezvous, so the relay now gives new clients a 30 s grace admission. |
| Contract pass | Done. Drafted and approved by the user 2026-10-04 (public APIs as compiling `todo!()` stubs in every crate). |
| Round 1 | Done 2026-10-04, merged to `main`. All crates implemented; 163 tests pass, including all 7 M3 acceptance tests (stable over 5 runs). Only the 3 audio-hardware tests are skipped. Deviations recorded in DESIGN.md. |
| Round 2 | Done 2026-10-04, merged to `main`. Direct calls and `net report` in the CLI, device health, the 64-peer cap with ranking and eviction, snapshot interpolation, exact send range, client port mapper, `yakvc_set_game_device`, file-TLS relay domain. 227 tests pass three runs in a row (9 M3 acceptance tests); only 3 audio-hardware tests and a doctest are skipped. No end-to-end test of the 64-peer cap (it would need 65 endpoints). |
| Round 3 | Done 2026-10-04 by the coordinator (too small for agents): `peer::CloseCode::TooManyPeers` for cap refusals and evictions; `relay.open` server option so direct calls get relay fallback from a self-hosted relay (the M2 "UDP blocked" check). Waiting for the relay URL in `endpoint_addr()` needed no API: `call listen` already reprints the address when it changes. |
| M1 exit | Needs a machine with sound hardware: `yakvc audio loopback` for 10 min with no underruns and ≤ 60 ms added latency. |
| M4 | Done 2026-10-04. Java integration on the unchanged C ABI (15 functions), voice session with MOTD opt-out and chat restrictions, game-state feeder, push-to-talk, HUD text, dev test audio. Verified headlessly: client gametest (push-to-talk, remote player tracked then dropped as spectator, local spectator sends nothing) and `scripts/two-clients.sh` (two clients hear each other through the engine's `Talking` events, not while one is a spectator, and again after). Not built: `mute_blocked_players`, failure toasts (M6). |
| M5 | Done 2026-10-04. Two real, verified accounts heard each other on a Realms server on 2026-10-08 (#4), and a restart reused the cached ticket (#3). Server takes the `hasJoined` URL from Mojang's discovery document; the mod's `SessionJoiner` answers `JoinRequest` through authlib's `joinServer`, refusing during the game's own login. The login challenge stays the default: choosing profile keys and measuring Mojang's rate limits need real users, so both wait for the beta. |
| M6 | In progress. Done 2026-10-04: one jar with natives for Linux (x86_64, aarch64), Windows and macOS (universal), built on each OS in CI; `cargo xtask dist` with `THIRD_PARTY_LICENSES`; `Enable-Native-Access` in the manifest; server image and `deploy/` files; release workflow on tags; settings screen (device dropdowns and a microphone level meter added 2026-10-06, #100; debug overlay 2026-10-07, #102), voice menu with per-player volume and mute, talking indicator, HUD icons, blocked-player muting and failure toasts, checked by gametest screenshots and `scripts/two-clients.sh` (both clients connect directly and hear each other on the M6 build). Open: the default production servers (#12, after the hostname decision #6), deployment (#11), store tokens (#19), and the beta itself. Testing on real Windows and macOS is deferred to Post-v1 (#34, #78). |
| M2 exit | Done 2026-10-08 (#2): two players on different home networks talked on a Realms server. They couldn't connect until the test server got a relay, so the relay fallback is covered too. |

**Decisions settled 2026-10-04:** wire encoding (see DESIGN.md wire format); Java package `io.github.desocketed.yakvc`, the reverse-domain form of the project's GitHub account `desocketed`; `yakvc-server` is Linux-only for good (the mod still targets every Minecraft Java platform); copyleft dependency licences are allowed, and the client enables iroh's port mapper.

## Manual real-account check (M5)

Real Mojang authentication can't run in CI: it needs a signed-in Minecraft account. On a Linux machine with a launcher that runs Fabric 26.3 (Fabric API `0.161.0+26.3`):

1. Run `scripts/real-auth-rendezvous.sh` and leave it running. It starts a local `yakvc-server` without dev auth, and prints the mod jar path (built for this host only) and a `client.toml`.
2. In the Fabric instance, put the jar in `mods/`, write the printed text to `config/yakvc/client.toml` (replacing the whole file), and delete `config/yakvc/ticket.bin` if it exists.
3. Launch signed in to your account and stay on the title screen. `logs/latest.log` shows `Voice sign-in: joined the Mojang session`, then `Rendezvous REGISTERED`. Without `dev_mode` the client rejects dev tickets, so this is a real one (`dev: false`). The server's counters (the `curl` command it printed) show `yakvc_auth_ok_total 1` and `yakvc_mojang_calls_total 1`.
4. Quit and launch again. The log shows `Rendezvous REGISTERED` with no new `Voice sign-in` line, and the counters show `yakvc_auth_cached_total 1` with `yakvc_mojang_calls_total` still 1: the cached ticket was reused without `joinServer`.

A `Voice sign-in failed: ...` line names authlib's error, for example an unreachable session server or an expired login.

## Parallel development with consumer-driven contracts

Goal: crates are implemented in parallel, and each crate's public API is clean, minimal and ergonomic **for the sibling crates that consume it**.

1. **Contract pass (serial).** Write every crate's public API as compiling code: types, traits, signatures and doc comments, with `todo!()` bodies. The workspace builds, so every agent codes against real signatures. The user reviews the contract before fan-out.
2. **Consumers shape APIs.** The crate that uses an API decides what it should look like. Each provider agent is given its consumers' usage alongside the contract. Done means consumer code reads naturally.
3. **Parallel implementation.** One agent per crate group, each in its own git worktree. Agents change their internals freely, but never change a public API on their own. Every agent's report ends with requested contract changes, to its own API or a sibling's.
4. **Reconcile in rounds.** The coordinating session collects change requests, resolves conflicts (taking ergonomic tradeoffs to the user), updates the contract, merges, and runs the full workspace build and tests. Affected agents get the new contract. Expect 2–3 rounds.

Agents don't negotiate with each other directly; all coordination goes through the coordinating session.

**Agent split** (four agents):

| Agent | Crates | Milestone work |
| --- | --- | --- |
| protocol | `yakvc-shared`, `yakvc-server` | M3 server side |
| audio | `yakvc-audio` | M1 |
| engine | `yakvc-client` (`net` + `voice`) | M2, M3 client side |
| harness | `yakvc-cli`, `yakvc-testkit`, `yakvc-ffi` | CLI diagnostics for M1–M2, testkit, FFI growth |

**Seams the contract must pin down:**

| Provider → consumers | Contract |
| --- | --- |
| `shared` → client, server | Message types, `Ticket` sign/verify, `PairToken`, datagram header |
| `audio` → client | Capture/playback streams, `Encoder`/`Decoder`, `JitterBuffer`, `Mixer` (per-source pose and gain) |
| `client` → ffi, cli, testkit | `Engine::start`, `EngineHandle` operations, `Event` enum, `sim` hooks |
| `server` → testkit | `Server::builder(config).spawn()` and test-facing accessors |
| `client::net` → `client::voice` | The `Protocol` trait |

## Conventions

- Keep DESIGN.md current: when the implementation deviates, update it in the same commit.
- Update the status table above when a step finishes.
