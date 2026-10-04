# Yak VC

Client-only Fabric mod for peer-to-peer proximity voice chat. Rust engine (Iroh, Opus) under `crates/`, Java game integration under `mod/`, bridged with FFM over a C ABI.

- `docs/DESIGN.md`: the design and source of truth. Keep it current when the implementation deviates.
- `docs/DEVELOPMENT.md`: how the work is organized (parallel agents, consumer-driven API contracts), current status and open decisions. Read it before starting milestone work, and update its status table when a step finishes.

## Code style

Prioritize a simple API and human readability over everything else: few, plain types and functions; obvious control flow; no speculative generality or clever abstractions. Comments explain why, not what.

## Commands

Run inside `nix develop` (this machine has no system Rust or Java). The container is Debian 13 with root, but only `/nix`, `$HOME/.claude`, `$HOME/.cache/nix` and this repository survive a restart: `apt-get` installs, `~/.config` and the Gradle cache do not. Add tools you need again to `flake.nix`; use `apt-get` only for one-offs.

- `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo deny check`
- `cargo xtask header`: regenerate `crates/yakvc-ffi/include/yakvc.h` after any ABI change (CI runs `--check`)
- `cd mod && ./gradlew build`: builds host natives via `cargo xtask natives` and runs the JUnit FFM test
- `scripts/ci-local.sh [--fast]`: everything CI runs, locally and in order (about 2 min with the gametest; `--fast` skips the mod). Run it before pushing to `main` rather than waiting on GitHub.
- `scripts/two-clients.sh --accept-eula`: local end-to-end check with a dev `yakvc-server`, a local Minecraft server and two headless clients (about 1 min). Accepting the Minecraft EULA is the maintainer's call: pass the flag only with their consent.
- `scripts/gametest-headless.sh`: runs the Fabric client gametest (real client, singleplayer world, screenshots in `mod/build/run/clientGameTest/screenshots/`) with no display or GPU, via Xvfb and Mesa lavapipe. Minecraft 26.3 finds no sRGB GLX config on Xvfb and falls back to Vulkan.

## Git

Git has no identity configured here. Commit as Claude's GitHub account: `git -c user.name="desocketed" -c user.email="337744892+desocketed@users.noreply.github.com" commit ...`. `docs/DESIGN.html` is the maintainer's own Org export, tracked at their request; don't edit it.

The GitHub repo is `github.com/desocketed/yakvc` (private), on the `desocketed` account, which the maintainer created for Claude. `gh` is in the dev shell and is logged in as `desocketed`; its credentials live in `$HOME/.claude/gh` (persistent, outside the repo; never copy them into it), so export `GH_CONFIG_DIR=$HOME/.claude/gh` for every `gh` or `git push`. `.git/config` is read-only, so there is no `origin` remote. Inside `nix develop`, push with:

```sh
GH_CONFIG_DIR=$HOME/.claude/gh git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push https://github.com/desocketed/yakvc.git main
```

**Workflow:** Claude manages this repository and may push verified work directly to `main` (never force-push or rewrite pushed history). Open a pull request instead only for changes the maintainer should review first, such as design decisions or risky, hard-to-reverse changes; say what changed, why, how it was verified and what to look at. The maintainer merges those PRs; address review comments with new commits on the branch.
