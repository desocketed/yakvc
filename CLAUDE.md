# Yak VC

Client-only Fabric mod for peer-to-peer proximity voice chat. Rust engine (Iroh, Opus) under `crates/`, Java game integration under `mod/`, bridged with FFM over a C ABI.

- `docs/DESIGN.md`: the design and source of truth. Keep it current when the implementation deviates.
- `docs/DEVELOPMENT.md`: how the work is organized (parallel agents, consumer-driven API contracts), current status and open decisions. Read it before starting milestone work, and update its status table when a step finishes.

## Commands

Run inside `nix develop` (this machine has no system Rust or Java).

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`
- `cargo xtask header`: regenerate `crates/yakvc-ffi/include/yakvc.h` after any ABI change (CI runs `--check`)
- `cd mod && ./gradlew build`: builds host natives via `cargo xtask natives` and runs the JUnit FFM test
- No display here; `runClient` needs `xvfb-run` plus X11/GL libs on `LD_LIBRARY_PATH`. The game stops at renderer creation (no GLX), but mod init runs first.

## Git

Git has no identity configured here. Commit with `git -c user.name="Ben Whitley" -c user.email="337744892+desocketed@users.noreply.github.com" commit ...`. `docs/DESIGN.html` is the user's own Org export; leave it untracked.
