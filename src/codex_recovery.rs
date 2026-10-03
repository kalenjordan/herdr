//! Recover missing session identity from evidence owned by a live Codex TUI.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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
        return Vec::new();
    }
    let input: Vec<_> = processes
        .iter()
        .map(|(target, pid)| ProcessTarget {
            pid: *pid,
            title: &target.title,
        })
        .collect();
    let Some(sessions) = query(&input) else {
        return Vec::new();
    };
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
