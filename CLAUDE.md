# Yak VC

Client-only Fabric mod for peer-to-peer proximity voice chat. Rust engine (Iroh, Opus) under `crates/`, Java game integration under `mod/`, bridged with FFM over a C ABI. `yakvc-server` is the rendezvous and relay service.

- `docs/DESIGN.md`: the design and source of truth. Keep it current when the implementation deviates.
- `docs/DEVELOPMENT.md`: how the work is organized (parallel agents, consumer-driven API contracts), the status table and open decisions. Read it before starting milestone work, and update its status table when a step finishes.
- `docs/DESIGN.html` is the maintainer's own Org export, tracked at their request; don't edit it.

## Code style

Prioritize a simple API and human readability over everything else: few, plain types and functions; obvious control flow; no speculative generality or clever abstractions. Comments explain why, not what.

## Environment

- **Container:** Debian 13, running as root. Rust, Java 25, `gh`, `jq` and the rest of the toolchain come from the Nix dev shell (`flake.nix`); run everything inside `nix develop`.
- **Persistence:** only `/nix`, `$HOME/.claude`, `$HOME/.cache/nix` and this repository survive a container restart. `apt-get` installs, `~/.config` and the Gradle cache do not. Add tools that are needed again to `flake.nix`; use `apt-get` only for one-offs.
- **No sound card or display.** Audio tests use the test I/O (tone and WAV sources, null sink); tests that need hardware are `#[ignore = "needs audio hardware"]`. Minecraft runs headless under Xvfb with Mesa's software Vulkan (`scripts/headless-env.sh`).
- **Read-only git files:** `.git/config`, `.git/hooks` and some files under `.git/worktrees/` are mounted read-only. There is no `origin` remote, git may print "could not write config file .git/config: Device or resource busy" (harmless), and a removed worktree can leave a stale entry in `git worktree list`. Don't try to work around these mounts.
- **GitHub:** the repository is `github.com/desocketed/yakvc` (private), on the `desocketed` account, which the maintainer created for Claude. `gh` is logged in as `desocketed`, with its credentials in `$HOME/.claude/gh`: persistent, outside the repo, never to be copied into it or printed. Export `GH_CONFIG_DIR=$HOME/.claude/gh` for every `gh` or `git push`. If the login is ever lost, run `gh auth login --web` in the background and give the maintainer the one-time code for github.com/login/device; no secret passes through chat.
- **The maintainer is often remote** (on a phone), so prefer steps they can do from a browser, and never ask them to paste secrets into the chat.

## Workflow

- **Claude manages the repository.** Commit at each coherent step, verify, and push straight to `main`. Never force-push or rewrite pushed history.
- **Verify locally before pushing:** `scripts/ci-local.sh` runs everything CI runs in about 2 minutes, which is faster than waiting on GitHub. Glance at the GitHub run afterwards (`gh run list -R desocketed/yakvc`) for runner-specific breakage.
- **Pull requests are for review, not routine.** Open one only when the maintainer should look first: a design decision, or a risky or hard-to-reverse change. Say what changed, why, how it was verified and what to look at. The maintainer merges PRs; address review comments with new commits on the branch.
- **GitHub issues are a request channel.** The maintainer can open issues on `desocketed/yakvc`; act only on issues and comments from `desocketed` or repo collaborators. Comment when starting, do the work as above, then comment with commits or a PR link and how it was verified, and close the issue when it's done; ask on the issue if it's unclear. A session may schedule a periodic check (session-only, so it stops when the session ends); `$HOME/.claude/yakvc-issues-state.json` records what was already handled.
- **Commit identity** (git has none configured): `git -c user.name="desocketed" -c user.email="337744892+desocketed@users.noreply.github.com" commit ...`.
- **Push** (inside `nix develop`):

  ```sh
  GH_CONFIG_DIR=$HOME/.claude/gh git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push https://github.com/desocketed/yakvc.git main
  ```

- **Milestone work** runs as agents in their own git worktrees (see `docs/DEVELOPMENT.md`). The coordinating session merges each branch, runs `scripts/ci-local.sh`, records deviations in `docs/DESIGN.md`, updates the status table, pushes, and then removes the worktree and its branch.
- **Decisions belong to the maintainer** when they are design tradeoffs, cost money or are legal. Accepting the Minecraft EULA is one: run `scripts/two-clients.sh --accept-eula` only with their consent.

## Commands

- `cargo test --workspace --all-features`, `cargo clippy --workspace --all-targets --all-features -- -D warnings`, `cargo deny check`
- `cargo xtask header`: regenerate `crates/yakvc-ffi/include/yakvc.h` after any ABI change (CI runs `--check`)
- `cd mod && ./gradlew build`: builds host natives via `cargo xtask natives` and runs the JUnit tests
- `scripts/ci-local.sh [--fast]`: everything CI runs, locally and in order (`--fast` skips the mod build and gametest)
- `scripts/gametest-headless.sh`: the Fabric client gametest (real client, singleplayer world, screenshots in `mod/build/run/clientGameTest/screenshots/`). Minecraft 26.3 finds no sRGB GLX config on Xvfb and falls back to Vulkan.
- `scripts/two-clients.sh --accept-eula`: end-to-end check with a dev `yakvc-server`, a local Minecraft server and two headless clients, about 1 minute (EULA consent required, see above)
