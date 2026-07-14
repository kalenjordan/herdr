# Current objective

Finish promotion of Kalen's Herdr customizations. The implementation is
committed and live; the remaining decisions are whether to commit this
checkpoint separately and push fork `master` so the installed behavior is
upgrade-safe.

# Completed work

- Persistent top-right chrome now shows neutral-gray notification state,
  compact Git state (`●N ↑N ↓N`), and focused Codex context usage.
- Codex usage is a percentage only. It refreshes every five seconds, stays gray
  through 50%, and turns orange above 50%.
- Codex usage matches Codex's own calculation: read the latest
  `last_token_usage` from the matched rollout, subtract Codex's 12,000-token
  baseline from usage and window, round remaining percent, then display
  `100 - remaining`.
- Headless server scheduling now executes and wakes for plugin and Codex status
  refreshes. This fixed the missing usage indicator in the normal installed
  server architecture.
- The built-in Codex integration is installed at
  `~/.codex/herdr-agent-state.sh` and reports exact session IDs. No cwd-based
  rollout fallback was added.
- cmux Codex hooks were removed. `~/.codex/hooks.json` now retains only Herdr's
  `SessionStart` hook and Kalen's `codex-herdr-tab-name` hook.
- Command-Shift-N remains configured for `herdr-focus-notify.toggle`. The linked
  plugin executable disappeared once and was rebuilt; action invocation then
  succeeded. Notifications currently end in the off state.
- Earlier committed features remain live: dirty/ahead/behind status, Command-E
  recent-tab switching, Command-D done/blocked-tab switching, and tab-aware
  targets with current tab first.

# Repository and promotion state

- Checkout: `/Users/kalen/repos/herdr`, branch `master`, eight commits ahead of
  fork `origin/master`.
- Latest commit: `cf15fa6 fix: align codex context usage status`.
- The latest commit includes the headless scheduler fix, exact Codex percentage,
  percentage-only rendering, and 50% color threshold.
- `STATE.md` is the only Herdr working-tree modification. It was intentionally
  excluded from the feature commit and has not been committed.
- Nothing has been pushed. The eight local fork commits are not upgrade-safe
  until Kalen explicitly authorizes pushing `master` to `origin/master`.
- `origin` is `git@github.com:kalenjordan/herdr.git`; `upstream` is
  `https://github.com/ogulcancelik/herdr.git`. Do not push upstream.
- Dotfiles `master` is two commits ahead. Existing modifications remain in
  `.codex/config.toml`, `.codex/hooks.json`, `claude/settings.json`, and
  `zsh/.zshrc`; preserve them.
- `/Users/kalen/repos/herdr-focus-notify` is clean on
  `kalen/status-toggle`, tracking its origin. Its release executable currently
  exists because it was rebuilt after a missing-binary failure.

# Remaining work

1. If Kalen wants the checkpoint tracked, propose
   `docs: update development checkpoint`, get alignment, and commit only
   `STATE.md`.
2. Obtain explicit authorization before pushing fork `master`. Once authorized,
   push only to `origin/master` and verify local/remote alignment.
3. If Command-Shift-N fails again, inspect plugin logs first. A prior failure was
   `target/release/herdr-focus-notify: No such file or directory`; rebuilding
   the linked plugin fixed it.

# Verification evidence

- Final pre-commit validation passed with Zig 0.15.2:
  - `cargo fmt --check`
  - focused `codex_usage` tests: 4 passed
  - `headless_scheduled_tasks_advance_status_refresh_deadlines`: passed
  - focused `headless_next_loop_deadline` tests: 2 passed
  - `cargo check --quiet`
  - `git diff --check`
- Optimized release build passed with
  `ZIG=/opt/homebrew/opt/zig@0.15/bin/zig cargo build --release`.
- Safe installed-path live handoff passed on protocol 17/version 0.7.3 without
  restarting sessions. `herdr status` currently reports compatible client and
  server with no restart needed.
- The visible Codex percentage was live-checked against transcript values and
  official Codex source behavior.
- `herdr integration status` reports `codex: current (v6)`.
- `just check` was not run because `just` is unavailable.

# Important files

- Codex reader and calculation: `src/codex_usage.rs`
- Headless refresh scheduling: `src/server/headless.rs`, `src/app/runtime.rs`,
  `src/app/mod.rs`
- Persistent status rendering: `src/ui/tabs.rs`, `src/ui.rs`
- Generic plugin status: `src/plugin_status.rs`
- Notification plugin: `/Users/kalen/repos/herdr-focus-notify`
- Herdr and Ghostty shortcuts:
  `/Users/kalen/repos/dotfiles/herdr/config.toml` and
  `/Users/kalen/repos/dotfiles/ghostty/config.ghostty`
- Codex hooks: `~/.codex/hooks.json`
