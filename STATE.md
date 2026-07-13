# Current objective

Finish and live-validate tab-aware Command-E and Command-D switchers, then
commit the accepted Herdr changes on Kalen's fork `master` when requested.

# Repository state

- Checkout: `/Users/kalen/repos/herdr`, branch `master`, four commits ahead of
  `origin/master`.
- Latest commit: `f088b4e feat: add recent workspace switcher`.
- The tab-aware extension is uncommitted. Modified Herdr files are the next
  changelog/config docs/API schema, switcher state/actions/input/render/config,
  and protocol expectation tests shown by `git status`.
- No push, upstream PR, or release is authorized. Until pushed to fork
  `master`, the local commits and live build are not upgrade-safe.
- The separate dotfiles repo has pre-existing unrelated modifications. This
  task additionally changes only `ghostty/config.ghostty`; preserve all other
  dotfiles changes.

# Implemented behavior

- Command-E is now tab-aware. MRU entries use stable workspace ID + non-reused
  public tab number, so tabs within one workspace are independently navigable
  and tab reorder/rename is safe.
- Command-D (`keys.done_or_blocked_workspace`) opens the same hold-to-cycle
  overlay filtered to tabs containing a blocked agent or unseen idle/done
  agent. It defaults to `cmd+d`.
- Multi-tab labels render as `workspace · tab`; single-tab workspaces retain
  the workspace-only label. Releasing Command commits, matching Command-E.
- The existing scoped standalone-modifier reporting remains active only while
  the switcher is open. Protocol remains 17.
- Local Herdr config now explicitly includes
  `done_or_blocked_workspace = "cmd+d"`; `herdr server reload-config` returned
  `status: applied` with no diagnostics.
- The optimized build is installed at `~/.local/bin/herdr` and a safe
  installed-path live handoff completed on protocol 17.

# Ghostty conflict and unresolved live test

- Ghostty intercepted Command-D before Herdr. Its tracked config previously
  translated it to `text:\x02gd`, which opened the full Herdr Navigator with
  the done filter. The screenshot confirmed that behavior.
- Ghostty's built-in Command-D is `new_split:right`. Changing the override to
  `unbind` resulted in no Herdr action.
- Current uncommitted dotfiles change is:
  `keybind = super+d=text:\x1b[100;9u`, an explicit Kitty Super-D press event.
  `ghostty +show-config` confirms that effective binding.
- Kalen must reload Ghostty with Command-Shift-Comma and test Command-D. It is
  not yet confirmed whether standalone Command release is delivered after the
  synthetic press. Do not commit either repo until this interaction is
  accepted and commit scope/messages are aligned.

# Verification

- `ZIG=/opt/homebrew/Cellar/zig@0.15/0.15.2/bin/zig cargo test --quiet`:
  all 2,514 core tests passed. The `api_ping` integration target then had 9/11
  pass: the known `events_subscribe_streams_output_and_agent_status_events`
  timeout recurred, and a protocol-16 expectation failed.
- The protocol expectation was updated to 17. Focused
  `cargo test --quiet --test api_ping ping_over_socket_returns_version` passed.
- `HERDR_UPDATE_API_SCHEMA=1 ... cargo test --quiet generated_protocol_schema_artifact_is_current`
  passed and updated `docs/next/api/herdr-api.schema.json` to protocol 17.
- Focused recent-workspace and done-or-blocked tests passed, including stable
  tab identity, exact tab activation, filtering, modifier routing, and state
  invariants.
- `cargo fmt --all`, `git diff --check`, and the optimized release build with
  Zig 0.15.2 passed.
- A filtered `cli_wrapper` command matched zero tests; those protocol-17
  expectations compile but have not been run under their exact test name.
- `just check` remains unavailable because `just` is not installed.

# Remaining work

1. Reload Ghostty and live-test Command-D press, repeated D cycling, and
   Command release. Also verify Command-E lists Commerce Leak's two tabs
   separately and activates the selected tab.
2. If synthetic Command-D opens but release does not commit, determine the
   Ghostty release-sequence mapping or choose a non-conflicting native chord.
3. Rerun focused tests after any fix, then build/install and perform the known
   safe handoff using `~/.local/bin/herdr` as both installed and import path.
4. When Kalen requests commits, propose lowercase conventional messages. The
   Herdr and dotfiles changes are separate repositories and should be committed
   separately. Do not push without explicit authorization.

# Important files

- Switcher state/actions: `src/app/state.rs`, `src/app/actions.rs`
- Input/config: `src/app/input/mod.rs`, `src/app/input/navigate.rs`,
  `src/config/model.rs`, `src/config/keybinds.rs`, `src/main.rs`
- Rendering: `src/ui.rs`
- Next docs/schema: `docs/next/CHANGELOG.md`,
  `docs/next/website/src/content/docs/configuration.mdx`,
  `docs/next/api/herdr-api.schema.json`
- Local Herdr config: `~/.config/herdr/config.toml`
- Ghostty config: `/Users/kalen/repos/dotfiles/ghostty/config.ghostty`
