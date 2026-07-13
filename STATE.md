# Current objective

Maintain Kalen's Herdr fork on `master`. The Command-E recent-workspace
switcher is implemented and live-validated.

# Current repository state

- Checkout: `/Users/kalen/repos/herdr`, branch `master`.
- `master` is three committed changes ahead of `origin/master`:
  - `523f075 feat: show repository dirty file count`
  - `d984e46 docs: add local notification workflow`
  - `7936006 docs: update development checkpoint`
- The MRU feature is ready to commit directly on fork `master`. It has not
  been pushed.

# Implemented MRU behavior

- `keys.recent_workspace` defaults to `cmd+e` and is a configurable direct
  action. Existing positional previous/next workspace actions are unchanged.
- First Command-E opens a name-only overlay and selects the most
  recently left workspace without changing focus.
- Further discrete E presses cycle and wrap, including back to the current
  workspace. Repeat events are suppressed.
- Left or right Super release commits. Enter commits as a fallback. Escape
  cancels without changing focus or history.
- History is a bounded in-memory list of stable workspace IDs. It is not added
  to the protocol or persisted session snapshot. A full restart therefore
  clears history and Command-E is a no-op until a workspace transition occurs.
- Workspace names resolve at render time. Reorder does not alter MRU order;
  closing a workspace removes it; stale IDs are defensively skipped.
- User-facing configuration docs and the next changelog are updated.
- The overlay is 36 to 80 columns wide, has an eight-row minimum height, and
  displays only the `release cmd to switch` hint.
- Standalone modifier reporting is enabled only while the overlay is active,
  preserving IME-safe keyboard reporting otherwise. This adds protocol 17.

# Live state and operational decisions

- The optimized feature build is installed at `/Users/kalen/.local/bin/herdr`.
- The running server reports version `0.7.3`, protocol `17`, and compatible.
- Safe local testing workflow is: build release, atomically replace
  `~/.local/bin/herdr`, then hand off with that same installed path. Never hand
  directly to `target/debug/herdr`: reconnecting clients race the importer,
  start the installed server, and restore fresh panes, disrupting Codex
  sessions.
- The latest normal-path handoff transferred 30 panes without replacement pane
  spawning or rollback.
- Kalen's local config previously had `last_pane = "cmd+e"`, which displaced
  the new default. It is now reloaded with:

  ```toml
  last_pane = ""
  recent_workspace = "cmd+e"
  ```

# Verification

- `ZIG=/opt/homebrew/Cellar/zig@0.15/0.15.2/bin/zig cargo test --quiet`
  passed all 2,507 core tests. The subsequent `api_ping` integration target had
  the known repeated timeout in
  `events_subscribe_streams_output_and_agent_status_events` (10 of 11 passed).
- Focused MRU, release routing, config, close/reorder, cancellation, and rename
  rendering tests passed.
- `python3 -m unittest scripts.test_agent_detection_manifest_check scripts.test_changelog scripts.test_docs_translation_parity scripts.test_preview scripts.test_vendor_libghostty_vt scripts.test_vendor_portable_pty`
  passed 64 tests.
- `cargo build --release --locked` passed with Zig 0.15.2.
- `cargo fmt --all -- --check` and `git diff --check` passed.
- Full clippy is blocked by an unrelated existing warning at the current
  `src/app/actions.rs` token-span chain: redundant `.into_iter()`.
- `just check` was not run because `just` is unavailable.

# Remaining work

1. Commit the accepted MRU feature on fork `master`.
2. Decide whether to replace terminal Control-D/EOF with a separate switcher
   filtered to workspaces containing done or blocked agents.
3. No upstream PR, push, or release is authorized.

# Important files

- Plan: `.local/prd/recent-workspace-switcher.md`
- MRU state/actions: `src/app/state.rs`, `src/app/actions.rs`
- Input routing: `src/app/input/mod.rs`, `src/app/input/navigate.rs`,
  `src/app/runtime.rs`, `src/app/mod.rs`
- Configuration: `src/config/model.rs`, `src/config/keybinds.rs`, `src/main.rs`
- Rendering: `src/ui.rs`
- Local config: `~/.config/herdr/config.toml`
