# Yak VC

Client-only Fabric mod for peer-to-peer proximity voice chat. Rust engine (Iroh, Opus) under `crates/`, Java game integration under `mod/`, bridged with FFM over a C ABI.

- `docs/DESIGN.md`: the design and source of truth. Keep it current when the implementation deviates.
- `docs/DEVELOPMENT.md`: how the work is organized (parallel agents, consumer-driven API contracts), current status and open decisions. Read it before starting milestone work, and update its status table when a step finishes.

## Code style

Prioritize a simple API and human readability over everything else: few, plain types and functions; obvious control flow; no speculative generality or clever abstractions. Comments explain why, not what.

## Commands

Run inside `nix develop` (this machine has no system Rust or Java).

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`
- `cargo xtask header`: regenerate `crates/yakvc-ffi/include/yakvc.h` after any ABI change (CI runs `--check`)
- `cd mod && ./gradlew build`: builds host natives via `cargo xtask natives` and runs the JUnit FFM test
- `scripts/gametest-headless.sh`: runs the Fabric client gametest (real client, singleplayer world, screenshots in `mod/build/run/clientGameTest/screenshots/`) with no display or GPU, via Xvfb and Mesa lavapipe. Minecraft 26.3 finds no sRGB GLX config on Xvfb and falls back to Vulkan.

## Git

Git has no identity configured here. Commit with `git -c user.name="Ben Whitley" -c user.email="337744892+desocketed@users.noreply.github.com" commit ...`. `docs/DESIGN.html` is the user's own Org export, tracked at their request; don't edit it.

The GitHub repo is `github.com/desocketed/yakvc` (private). `.git/config` is read-only here, so there is no `origin` remote: push with `GIT_TERMINAL_PROMPT=0 git -c credential.helper= -c "credential.helper=!$HOME/.config/yakvc-bot/token.sh credential" push https://github.com/desocketed/yakvc.git <branch>` (a GitHub App token; the key never enters the repo). Push and open PRs only when asked.
