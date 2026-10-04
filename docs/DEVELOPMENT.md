# Yak VC — Development process

How the implementation is built: milestones from [DESIGN.md](DESIGN.md), worked on by parallel agents against agreed crate APIs. DESIGN.md says *what* to build; this file says *how* the work is organized and where it stands.

## Status

| Milestone | State |
| --- | --- |
| M0 Skeleton | Done. Workspace, crate stubs, xtask (`header`, `natives`), CI, plus a minimal Fabric mod that loads `yakvc-ffi` through FFM (verified in `runClient` on 26.3). |
| Relay feature check (from M2) | Done 2026-10-04. All needed `iroh-relay` 1.3 server features exist; details in DESIGN.md dependency rules. Found and fixed a gap: `relay_only` clients could not reach the rendezvous, so the relay now gives new clients a 30 s grace admission. |
| Contract pass | Next. |
| M1–M3 | Planned as one parallel push (see below). |

**Decisions settled 2026-10-04:** wire encoding (see DESIGN.md wire format); Java package `io.github.desocketed.yakvc` under the maintainer's domain `desocketed.github.io`.

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
| harness | `yakvc-cli`, `yakvc-testkit`, `yakvc-ffi` | CLI commands for M1–M3, testkit, FFI growth |

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
