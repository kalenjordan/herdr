# Current objective

Use Kalen's fork `master` as the canonical development line and prepare three
improvements:

1. A macOS-style Command-E MRU workspace switcher.
2. Persistent top-right status chrome showing repository dirty state.
3. A durable notification toggle whose state can later appear in that chrome.

# Completed work

- Normalized Git configuration:
  - `origin` is `git@github.com:kalenjordan/herdr.git`.
  - `upstream` is `https://github.com/ogulcancelik/herdr.git`.
  - Local `master` tracks `origin/master`; both are at `3251a44`.
  - Preserved the older `local/v0.7.2-kalen` branch at `55c78ac`.
- Wrote ignored local plans:
  - `.local/prd/recent-workspace-switcher.md`
  - `.local/prd/codex-dirty-state-and-notification-toggle.md`
- Added an uncommitted local-machine instruction to `AGENTS.md`: Kalen's
  `disable notifications`, `enable notifications`, and `toggle notifications`
  requests update the focus-notify plugin's durable `.env` setting.
- Executed `disable notifications`; the local plugin config now contains
  `HERDR_FOCUS_NOTIFY_ENABLED=0`.
- Reviewed Codex status-line support. It has no documented custom command-backed
  dirty indicator, so Herdr remains the desired persistent display surface.
- Reviewed Herdr's Git refresh. While an app client is attached, it runs every
  1.5 seconds on a spawned thread, computes branch/ahead-behind/worktree facts,
  and sends `GitStatusRefreshed` back to the app. Dirty state can be recomputed
  on this path without running Git during render.
- Implemented repository dirty state in the existing background Git refresh,
  a reserved top-right `dirty N` label, the sidebar branch marker, and retention
  of the status row for dirty single-tab workspaces. Added coverage for clean,
  modified, staged, deleted, conflicted, untracked, ignored-only, non-Git,
  cached-fingerprint, linked-worktree, layout, rendering, and state application
  behavior. Staged the user-facing note in `docs/next/CHANGELOG.md`.
- Reviewed the live Herdr plugin marketplace and used a critic subagent. The
  marketplace indexed 169 repositories at review time. Existing plugins prove
  that overlays, MRU history, background pollers, and per-pane sidebar
  `custom_status` badges are possible.

# Decisions and rationale

- Proceed mostly with core changes because Kalen normally keeps the sidebar
  closed. Plugin `custom_status` is therefore not an acceptable primary surface.
- Add a right-aligned status cluster to the top tab row. Reserve its width
  before tab layout and retain the row when single-tab hiding is enabled but a
  status item is visible.
- Implement repository dirty state as shared workspace Git state in core and
  render it in the top cluster. Define dirty as staged, unstaged, deleted,
  conflicted, or untracked changes; ignored-only changes remain clean.
- Keep the current 1.5-second Git refresh interval initially. The worker must
  recompute dirty even when its HEAD/upstream fingerprint cache is unchanged.
- Notification delivery and durable enablement remain owned by
  `herdr-focus-notify`. Herdr core should expose a neutral plugin-status path,
  not read that plugin's private `.env` schema directly.
- Use the plugin's `.env` toggle rather than Herdr's whole-plugin disable for
  the eventual keybinding. Disabling the whole plugin also disables the action
  needed to turn notification delivery back on.
- The exact Command-E UX remains core work: first press previews the previous
  workspace, repeated E cycles, and Command release commits. Plugins already
  offer ordinary recent-workspace overlays, but cannot observe raw modifier
  release outside their terminal pane.
- MRU state should use stable workspace IDs and remain client-local/in-memory
  in the first version. Do not add protocol or session persistence fields.
- Existing fork patch classification after plugin review:
  - detach shortcut could be local configuration;
  - clickable file URLs are only partially replaceable by link handlers;
  - native terminal target hit-testing, selection contrast, workspace-close
    semantics, and live-handoff plugin restoration require core changes.

# Remaining work

1. Implement the exact Command-E MRU switcher, including narrow release-event
   routing and stable-ID close/reorder tests.
2. Design a neutral plugin status contribution/API for the top cluster.
3. In the external `herdr-focus-notify` source repository, add atomic enable,
   disable, toggle, and status publication actions. Do not edit its managed
   installation cache.
4. Choose the notification-toggle keybinding and revisit the initial text dirty
   marker only if manual use suggests a more compact terminal-safe treatment.
5. Separately decide whether to rebase the fork's 8 unique commits onto the 22
   newer upstream commits. This would rewrite fork history and has not been
   authorized or attempted.

# Risks and unresolved questions

- Command/Super release reporting varies by terminal. Validate actual
  crossterm events and keep Enter as a commit fallback.
- The top cluster needs explicit priority and narrow-width behavior so it does
  not break tab scrolling or controls.
- A new shared plugin-status API may require protocol/version review. Keep its
  data model neutral and separate from TUI placement.
- `AGENTS.md` and this checkpoint are tracked, uncommitted changes. `.local/prd/`
  is ignored. No source implementation, issue, discussion, PR, release, commit,
  or push was created for these plans.

# Verification and latest results

- `git status --short --branch` showed `master...origin/master` with tracked
  modifications to `AGENTS.md` and `STATE.md` after this checkpoint.
- `git rev-list --left-right --count master...origin/master` returned `0 0`.
- `git rev-list --left-right --count master...upstream/master` returned `8 22`.
- `rg -n '^HERDR_FOCUS_NOTIFY_ENABLED=' ~/.config/herdr/plugins/config/herdr-focus-notify/.env`
  returned `3:HERDR_FOCUS_NOTIFY_ENABLED=0`.
- No build or test suite was run because this work changed instructions,
  ignored plans, local plugin configuration, and checkpoint state only.

# Important files

- `AGENTS.md`
- `STATE.md`
- `.local/prd/recent-workspace-switcher.md`
- `.local/prd/codex-dirty-state-and-notification-toggle.md`
- `~/.config/herdr/plugins/config/herdr-focus-notify/.env`
