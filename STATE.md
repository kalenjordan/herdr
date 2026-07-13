# Current objective

Keep Herdr's local work upgrade-safe through Kalen's fork while preserving
concurrent workspace-close edits in this checkout.

# Completed work

- Landed the clickable-target series on `fork/master` (`kalenjordan/herdr`),
  fast-forwarding it from `66be0b6` to `cca3d77`:
  - `d3886be fix: make terminal targets clickable`
  - `1531a4c fix: improve terminal selection contrast`
  - `f44aa06 feat: support clickable file URLs`
  - `ee3e2d5 feat: add command shift x detach shortcut`
  - `cca3d77 fix: restore plugins after live handoff`
- Plain `file:///...` terminal text is clickable and opens through the existing
  platform URL handler. File URLs deliberately bypass web-link plugins.
- Detach retains `prefix+q` and also accepts `cmd+shift+x`; the generated
  config example and next-release configuration docs reflect it.
- Fixed handoff plugin restoration: `App::new_from_handoff` reloads the
  persisted plugin registry after moving from the temporary no-session shell
  to persistent mode. Live handoff showed `herdr-focus-notify` enabled and its
  test action completed successfully.
- Used a temporary worktree to rebase the series on current `fork/master`;
  resolved one selection-render conflict, pushed the fork, then removed the
  temporary worktree and branch.
- Committed and pushed the remaining product changes to `fork/master`:
  - `10a3f20 docs: require upgrade-safe local commits`
  - `bcaa43b fix: preserve active workspace when closing background workspace`

# Decisions and rationale

- `fork` is Kalen's own upstream and is the promotion target for this work.
  `origin` is the canonical `ogulcancelik/herdr` repository and was not
  changed.
- No release was created. The authenticated GitHub account is `kalenjordan`,
  so the repository's maintainer-only release workflow does not permit release
  commands or release-channel changes.
- The upgrade-safety commit policy is now on `fork/master`.

# Remaining work

1. Before any release, run the pre-release audit, add the missing
   `docs/next/CHANGELOG.md` entries, complete release-doc finalization, and run
   the required release checks. A maintainer account is required for the
   canonical release flow.
2. Consider adding a direct `new_from_handoff` plugin-registry regression test;
   the existing `plugin_registry` tests do not cover that transition.

# Risks and unresolved points

- Focused tests passed on the original local commits, but could not be rerun in
  the integration worktree: Zig 0.15.2 failed to link its macOS build runner
  with unresolved system symbols. This happened in both debug and release test
  modes and is an environment/toolchain failure, not a test failure.
- `just` and `nix` are unavailable in this environment. `just check`,
  `just release-docs-check`, and Nix release validation remain unrun.
- The fork integration contains no new release tag or update manifest. Whether
  fork users receive it automatically depends on the fork's own update/release
  configuration.

# Verification

- Passed before fork integration on the original feature commits:
  - `env ZIG=/tmp/herdr-zig-0.15.2/zig cargo test clickable_spans_include_plain_file_urls`
  - `env ZIG=/tmp/herdr-zig-0.15.2/zig cargo test detach_defaults_to_prefix_q_and_command_shift_x`
  - `env ZIG=/tmp/herdr-zig-0.15.2/zig cargo test plugin_registry`
  - `env ZIG=/tmp/herdr-zig-0.15.2/zig cargo build --release`
  - `cargo fmt --check`
  - `git diff --check`
- Passed after each integration: `cargo fmt --check` and `git diff --check`.
- The three new background-workspace-close tests were invoked in release mode
  after integration, but did not begin because Zig failed while linking its
  build runner (the environment failure above).
- Live validation passed:
  - `herdr server live-handoff --import-exe /Users/kalen/repos/herdr/target/release/herdr --expected-protocol 16 --expected-version 0.7.3`
  - `herdr plugin list --json` showed enabled `herdr-focus-notify`.
  - `herdr plugin action invoke test --plugin herdr-focus-notify` succeeded.

# Important current files

- This checkpoint: `STATE.md` (untracked, not committed).
