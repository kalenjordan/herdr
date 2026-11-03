//! Reconcile missing or stale session identity using evidence from a live Codex TUI.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

// Recovery polls periodically. Keep missing-evidence diagnostics visible at the
// default log level without writing the same failure on every poll.
fn log_missing_evidence(target: &RecoveryTarget, pid: u32) {
    static LAST_LOGGED: OnceLock<Mutex<HashMap<u32, Instant>>> = OnceLock::new();
    let Ok(mut logged) = LAST_LOGGED
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    else {
        return;
    };
    let now = Instant::now();
    logged.retain(|_, last| now.duration_since(*last) < Duration::from_secs(300));
    if logged
        .get(&pid)
        .is_some_and(|last| now.duration_since(*last) < Duration::from_secs(60))
    {
        return;
    }
    logged.insert(pid, now);
    tracing::info!(pane_id = ?target.pane_id, terminal_id = %target.terminal_id, process_pid = pid, reason = "no_corroborated_session_evidence", "Codex session recovery found no association");
}

use serde::{Deserialize, Serialize};

use crate::{layout::PaneId, terminal::TerminalId};

const RECOVERY_SCRIPT: &str = include_str!("integration/assets/codex/recover-session.py");

pub(crate) fn current_title(runtime: &crate::terminal::TerminalRuntime) -> String {
    // The live footer is stable while the OSC title can carry a working spinner.
    if let Some(title) = footer_title(&runtime.detection_text()) {
        return title;
    }
    let title = runtime.agent_osc_title();
    if !title.is_empty() {
        return title;
    }
    // Handoff reconstructs screen cells but may have no OSC title yet. Use
    // only the live control footer, never a title mentioned in transcript text.
    String::new()
}

fn footer_title(text: &str) -> Option<String> {
    let footer = live_footer(text)?;
    let mut parts = footer.split(" · ");
    let model = parts.next()?;
    if !model.starts_with("GPT-") && !model.starts_with("gpt-") {
        return None;
    }
    let title = parts.next()?.trim();
    parts.next()?;
    (!title.is_empty() && !title.contains('…')).then(|| title.to_owned())
}

pub(crate) fn footer_checkout(text: &str) -> Option<PathBuf> {
    footer_title(text)?;
    let footer = live_footer(text)?;
    let mut paths = footer.split(" · ").skip(2).filter_map(|part| {
        let part = part.trim();
        if part.contains('…') {
            return None;
        }
        if let Some(rest) = part.strip_prefix("~/") {
            return std::env::var_os("HOME").map(|home| PathBuf::from(home).join(rest));
        }
        let path = PathBuf::from(part);
        path.is_absolute().then_some(path)
    });
    let path = paths.next()?;
    paths.next().is_none().then_some(path)
}

pub(crate) fn recovery_checkout(terminal_cwd: &std::path::Path, text: &str) -> Option<PathBuf> {
    // A narrow footer may truncate the checkout while preserving the complete
    // title. Python then requires a globally unique title and corroborating final
    // output, and accepts only an exact transcript/terminal checkout match.
    footer_title(text)?;
    if footer_checkout(text).is_some() {
        return crate::codex_checkout::validated_live_checkout(terminal_cwd, text);
    }
    live_footer(text)?
        .split(" · ")
        .skip(2)
        .any(|part| part.contains('…'))
        .then(|| terminal_cwd.to_path_buf())
}

fn live_footer(text: &str) -> Option<&str> {
    let mut bottom = text
        .lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty());
    let controls = bottom.next()?;
    if !controls.contains("← for agents") || !controls.contains("? for shortcuts") {
        return None;
    }
    bottom.next()
}

#[derive(Clone, Debug)]
pub(crate) struct RecoveryTarget {
    pub pane_id: PaneId,
    pub terminal_id: TerminalId,
    pub shell_pid: u32,
    pub title: String,
    pub report_seq: u64,
    pub previous_session_id: Option<String>,
    pub checkout: Option<PathBuf>,
    pub detection_text: String,
    pub excluded_session: Option<String>,
}

impl RecoveryTarget {
    pub(crate) fn accepts_session_snapshot(
        &self,
        current_session: Option<&str>,
        excluded_session: Option<&str>,
        recovered_session: &str,
    ) -> bool {
        current_session == self.previous_session_id.as_deref()
            && excluded_session == self.excluded_session.as_deref()
            && current_session != Some(recovered_session)
            && excluded_session != Some(recovered_session)
    }
}

#[derive(Debug)]
pub(crate) struct RecoveredSession {
    pub target: RecoveryTarget,
    pub process_pid: u32,
    pub session_id: String,
}

#[derive(Serialize)]
struct ProcessTarget<'a> {
    pid: u32,
    title: &'a str,
    checkout: Option<&'a std::path::Path>,
    detection_text: &'a str,
    excluded_session: Option<&'a str>,
}

#[derive(Deserialize)]
struct ProcessSession {
    pid: u32,
    session_id: String,
}

pub(crate) fn recover(targets: Vec<RecoveryTarget>) -> Vec<RecoveredSession> {
    let processes: Vec<_> = targets
        .iter()
        .filter_map(|target| {
            let job = crate::detect::foreground_job(target.shell_pid)?;
            let mut codex = job.processes.iter().filter(|p| p.name == "codex");
            let pid = codex.next()?.pid;
            codex.next().is_none().then_some((target, pid))
        })
        .collect();
    if processes.is_empty() {
        tracing::debug!(
            reason = "no_unique_foreground_codex_process",
            "Codex session recovery skipped"
        );
        return Vec::new();
    }
    let input: Vec<_> = processes
        .iter()
        .map(|(target, pid)| ProcessTarget {
            pid: *pid,
            title: &target.title,
            checkout: target.checkout.as_deref(),
            detection_text: &target.detection_text,
            excluded_session: target.excluded_session.as_deref(),
        })
        .collect();
    let Some(sessions) = query(&input) else {
        tracing::debug!(
            target_count = input.len(),
            reason = "query_failed",
            "Codex session recovery failed"
        );
        return Vec::new();
    };
    for (target, pid) in &processes {
        if !sessions.iter().any(|session| session.pid == *pid) {
            log_missing_evidence(target, *pid);
        }
    }
    sessions
        .into_iter()
        .filter_map(|session| {
            let (target, _) = processes.iter().find(|(_, pid)| *pid == session.pid)?;
            crate::agent_resume::AgentSessionRef::id(&session.session_id)?;
            Some(RecoveredSession {
                target: (*target).clone(),
                process_pid: session.pid,
                session_id: session.session_id,
            })
        })
        .collect()
}

fn query(targets: &[ProcessTarget<'_>]) -> Option<Vec<ProcessSession>> {
    let input = serde_json::to_vec(targets).ok()?;
    let mut child = Command::new("python3")
        .args(["-c", RECOVERY_SCRIPT])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(&input).is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
    }
    let output = child.wait_with_output().ok()?;
    output.status.success().then_some(())?;
    serde_json::from_slice(&output.stdout).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn narrow_live_footer_can_corroborate_terminal_checkout_but_not_truncated_title() {
        let cwd = std::path::Path::new("/terminal/checkout");
        let footer =
            "GPT-6.1-Sol low · Review calls · repo · ~/rep…\n← for agents · ? for shortcuts";
        assert_eq!(recovery_checkout(cwd, footer), Some(cwd.to_path_buf()));
        assert_eq!(
            recovery_checkout(cwd, &footer.replace("Review calls", "Review…")),
            None
        );
        assert_eq!(
            recovery_checkout(cwd, &format!("{footer}\nuser transcript")),
            None
        );
        assert_eq!(
            recovery_checkout(cwd, &footer.replace("~/rep…", "/unrelated/repo")),
            None
        );
    }

    #[test]
    fn reconciliation_rejects_hook_and_clear_races_and_old_suppressed_identity() {
        let mut target = RecoveryTarget {
            pane_id: PaneId::alloc(),
            terminal_id: TerminalId::alloc(),
            shell_pid: 1,
            title: "Current conversation".into(),
            report_seq: 20,
            previous_session_id: Some("old".into()),
            checkout: None,
            detection_text: String::new(),
            excluded_session: None,
        };
        assert!(target.accepts_session_snapshot(Some("old"), None, "new"));
        assert!(!target.accepts_session_snapshot(Some("newer-hook"), None, "new"));
        assert!(!target.accepts_session_snapshot(Some("old"), Some("old"), "new"));
        assert!(!target.accepts_session_snapshot(Some("old"), None, "old"));
        target.excluded_session = Some("old".into());
        assert!(!target.accepts_session_snapshot(Some("old"), Some("old"), "old"));
        assert!(target.accepts_session_snapshot(Some("old"), Some("old"), "new"));
    }

    #[tokio::test]
    async fn title_recovers_from_live_footer_after_handoff_without_osc() {
        let runtime = crate::terminal::TerminalRuntime::test_with_screen_bytes(
            120,
            6,
            "› Ask Codex to do anything\r\n\r\nGPT-6.1-Sol low · Check smart lead auto-pruning · ~/.codex/w…\r\n← for agents · ? for shortcuts\r\n".as_bytes(),
        );
        assert!(runtime.agent_osc_title().is_empty());
        assert_eq!(current_title(&runtime), "Check smart lead auto-pruning");
    }

    #[test]
    fn transcript_titles_and_truncated_titles_do_not_supply_recovery_evidence() {
        assert!(
            footer_title("GPT-6.1-Sol low · Old session · /repo\nmore transcript text").is_none()
        );
        assert!(footer_title(
            "GPT-6.1-Sol low · Truncated… · /repo\n← for agents · ? for shortcuts"
        )
        .is_none());
    }

    #[test]
    fn live_footer_identifies_checkout_without_using_transcript_text() {
        let screen = "• Mentioned /tmp/other-repo in a reply\n\n› Ask Codex to do anything\nGPT-6.1-Sol low · Check smart lead auto-pruning · outbound-dash · ~/.codex/worktrees/e4b8/outbound-dash · codex/smartlead-auto-pruning\n← for agents · ? for shortcuts\n";
        let expected = std::env::var_os("HOME")
            .map(PathBuf::from)
            .map(|home| home.join(".codex/worktrees/e4b8/outbound-dash"));
        assert_eq!(footer_checkout(screen), expected);
        assert_eq!(
            footer_checkout("• Mentioned /tmp/other-repo in a reply"),
            None
        );
        assert_eq!(
            footer_checkout(
                "GPT-6.1-Sol low · Title · repo · /tmp/re…\n← for agents · ? for shortcuts"
            ),
            None
        );
    }
}
