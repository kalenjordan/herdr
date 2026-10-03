//! Resolve checkout metadata without changing terminal or resumable session identity.
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, serde::Deserialize)]
struct SessionMetadata {
    id: String,
    cwd: PathBuf,
    forked_from_id: Option<String>,
}

#[derive(Default)]
struct CheckoutIndex {
    root: PathBuf,
    refreshed: Option<Instant>,
    seen: HashSet<PathBuf>,
    sessions: HashMap<String, SessionMetadata>,
    reported: HashMap<String, PathBuf>,
}

static INDEX: OnceLock<Mutex<CheckoutIndex>> = OnceLock::new();

pub(crate) struct PresentationContext {
    pub session_id: Option<String>,
    pub checkout: PathBuf,
}

/// Resolve the focused Codex pane's display context without changing its
/// persisted, resumable session identity.
pub(crate) fn presentation_context(
    reported_session_id: Option<&str>,
    terminal_cwd: &Path,
    detection_text: Option<&str>,
) -> PresentationContext {
    let live_checkout = detection_text.and_then(|text| validated_live_checkout(terminal_cwd, text));
    if let Some(checkout) = live_checkout {
        let session_id = checkout_index().and_then(|index| {
            presentation_session_for_checkout(&index.sessions, reported_session_id, &checkout)
        });
        return PresentationContext {
            session_id,
            checkout,
        };
    }

    let checkout = reported_session_id
        .and_then(|session_id| load_checkout(session_id, terminal_cwd))
        .unwrap_or_else(|| terminal_cwd.to_path_buf());
    PresentationContext {
        session_id: reported_session_id.map(str::to_owned),
        checkout,
    }
}

fn validated_live_checkout(terminal_cwd: &Path, detection_text: &str) -> Option<PathBuf> {
    let path = crate::codex_recovery::footer_checkout(detection_text)?;
    crate::workspace::git_space_metadata(terminal_cwd)
        .zip(crate::workspace::git_space_metadata(&path))
        .filter(|(terminal, displayed)| terminal.key == displayed.key)
        .map(|_| path)
}

fn presentation_session_for_checkout(
    sessions: &HashMap<String, SessionMetadata>,
    reported_session_id: Option<&str>,
    checkout: &Path,
) -> Option<String> {
    if let Some(id) = reported_session_id.filter(|id| {
        sessions
            .get(*id)
            .is_some_and(|session| session.cwd == checkout)
    }) {
        return Some(id.to_owned());
    }
    let mut matches = sessions.values().filter(|session| session.cwd == checkout);
    let only = matches.next()?;
    matches.next().is_none().then(|| only.id.clone())
}

pub(crate) fn load_checkout(session_id: &str, terminal_cwd: &Path) -> Option<PathBuf> {
    let mut index = checkout_index()?;
    let cwd = resolve_checkout(&index.sessions, session_id)?;
    let current = crate::workspace::git_space_metadata(terminal_cwd)?;
    let candidate = crate::workspace::git_space_metadata(&cwd)?;
    if current.key != candidate.key
        || (current.checkout_key != candidate.checkout_key && !candidate.is_linked_worktree)
    {
        return None;
    }
    if current.checkout_key != candidate.checkout_key
        && index.reported.get(session_id) != Some(&cwd)
    {
        tracing::info!(session_id, checkout = %cwd.display(), "resolved Codex worktree checkout for Git status");
        index.reported.insert(session_id.to_owned(), cwd.clone());
    }
    Some(cwd)
}

fn checkout_index() -> Option<std::sync::MutexGuard<'static, CheckoutIndex>> {
    let root = std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".codex")))?
        .join("sessions");
    let mut index = INDEX
        .get_or_init(|| Mutex::new(CheckoutIndex::default()))
        .lock()
        .ok()?;
    if index.root != root {
        *index = CheckoutIndex {
            root: root.clone(),
            ..CheckoutIndex::default()
        };
    }
    if index
        .refreshed
        .is_none_or(|time| time.elapsed() >= Duration::from_secs(10))
    {
        index.discover(&root);
        index.refreshed = Some(Instant::now());
    }
    Some(index)
}

/// Latest unambiguous descendant first, then its ancestors for reply fallback.
pub(crate) fn reply_session_lineage(session_id: &str) -> Vec<String> {
    let Some(index) = checkout_index() else {
        return vec![session_id.to_owned()];
    };
    resolve_reply_lineage(&index.sessions, session_id)
}

fn resolve_reply_lineage(sessions: &HashMap<String, SessionMetadata>, id: &str) -> Vec<String> {
    let mut current = id.to_owned();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        let children: Vec<_> = sessions
            .values()
            .filter(|session| session.forked_from_id.as_deref() == Some(current.as_str()))
            .collect();
        if children.len() != 1 {
            break;
        }
        current = children[0].id.clone();
    }
    let mut lineage = Vec::new();
    let mut seen = HashSet::new();
    while seen.insert(current.clone()) {
        lineage.push(current.clone());
        let Some(parent) = sessions
            .get(&current)
            .and_then(|session| session.forked_from_id.clone())
        else {
            break;
        };
        current = parent;
    }
    lineage
}

impl CheckoutIndex {
    fn discover(&mut self, root: &Path) {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                self.discover(&path);
            } else if kind.is_file()
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
                && !self.seen.contains(&path)
            {
                if let Some(metadata) = read_metadata(&path) {
                    self.sessions.insert(metadata.id.clone(), metadata);
                    self.seen.insert(path);
                }
            }
        }
    }
}

fn read_metadata(path: &Path) -> Option<SessionMetadata> {
    let file = std::fs::File::open(path).ok()?;
    // Only read the header; command content and growing transcript bodies are irrelevant.
    let mut header = String::new();
    BufReader::new(file.take(64 * 1024))
        .read_line(&mut header)
        .ok()?;
    let record: serde_json::Value = serde_json::from_str(&header).ok()?;
    if record["type"] != "session_meta" {
        return None;
    }
    serde_json::from_value(record["payload"].clone()).ok()
}

fn resolve_checkout(sessions: &HashMap<String, SessionMetadata>, id: &str) -> Option<PathBuf> {
    let original = sessions.get(id)?;
    let mut descendants = HashSet::from([id.to_owned()]);
    loop {
        let before = descendants.len();
        for session in sessions.values() {
            if session
                .forked_from_id
                .as_ref()
                .is_some_and(|parent| descendants.contains(parent))
            {
                descendants.insert(session.id.clone());
            }
        }
        if descendants.len() == before {
            break;
        }
    }
    let mut leaves = sessions.values().filter(|session| {
        descendants.contains(&session.id)
            && !sessions.values().any(|child| {
                descendants.contains(&child.id)
                    && child.forked_from_id.as_ref() == Some(&session.id)
            })
    });
    let first = leaves.next()?;
    // Multiple leaves can share a checkout, but different checkouts are ambiguous.
    if leaves.any(|leaf| leaf.cwd != first.cwd) {
        return Some(original.cwd.clone());
    }
    Some(first.cwd.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sessions(rows: &[(&str, &str, Option<&str>)]) -> HashMap<String, SessionMetadata> {
        rows.iter()
            .map(|(id, cwd, parent)| {
                (
                    id.to_string(),
                    SessionMetadata {
                        id: id.to_string(),
                        cwd: PathBuf::from(cwd),
                        forked_from_id: parent.map(str::to_owned),
                    },
                )
            })
            .collect()
    }

    #[test]
    fn reply_lineage_follows_unique_forks_and_falls_back_to_ancestors() {
        let index = sessions(&[
            ("root", "/main", None),
            ("a", "/worktree", Some("root")),
            ("b", "/worktree", Some("a")),
        ]);
        assert_eq!(
            resolve_reply_lineage(&index, "root"),
            vec!["b", "a", "root"]
        );
        assert_eq!(resolve_reply_lineage(&index, "b"), vec!["b", "a", "root"]);
        let divergent = sessions(&[
            ("root", "/main", None),
            ("a", "/a", Some("root")),
            ("b", "/b", Some("root")),
        ]);
        assert_eq!(resolve_reply_lineage(&divergent, "root"), vec!["root"]);
        assert_eq!(resolve_reply_lineage(&divergent, "a"), vec!["a", "root"]);
    }

    #[test]
    fn checkout_metadata_reads_header_without_parsing_transcript_body() {
        let path = std::env::temp_dir().join(format!(
            "herdr-checkout-metadata-{}.jsonl",
            std::process::id()
        ));
        std::fs::write(&path, concat!(
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"fork\",\"cwd\":\"/worktree\",\"forked_from_id\":\"root\"}}\n",
            "unparseable transcript body\n"
        )).unwrap();
        let metadata = read_metadata(&path).unwrap();
        assert_eq!(metadata.id, "fork");
        assert_eq!(metadata.cwd, PathBuf::from("/worktree"));
        assert_eq!(metadata.forked_from_id.as_deref(), Some("root"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn checkout_follows_explicit_fork_lineage() {
        let index = sessions(&[
            ("root", "/main", None),
            ("fork", "/worktree", Some("root")),
            ("unrelated", "/other", None),
        ]);
        assert_eq!(
            resolve_checkout(&index, "root"),
            Some(PathBuf::from("/worktree"))
        );
        assert_eq!(
            resolve_checkout(&index, "fork"),
            Some(PathBuf::from("/worktree"))
        );
        assert_eq!(resolve_checkout(&index, "missing"), None);
    }

    #[test]
    fn checkout_does_not_guess_between_divergent_forks() {
        let index = sessions(&[
            ("root", "/main", None),
            ("a", "/worktree/a", Some("root")),
            ("b", "/worktree/b", Some("root")),
        ]);
        assert_eq!(
            resolve_checkout(&index, "root"),
            Some(PathBuf::from("/main"))
        );
    }

    #[test]
    fn checkout_follows_nested_forks_and_handles_cycles() {
        let index = sessions(&[
            ("root", "/main", None),
            ("a", "/worktree/a", Some("root")),
            ("b", "/worktree/b", Some("a")),
        ]);
        assert_eq!(
            resolve_checkout(&index, "root"),
            Some(PathBuf::from("/worktree/b"))
        );
        let cycle = sessions(&[("a", "/a", Some("b")), ("b", "/b", Some("a"))]);
        assert_eq!(resolve_checkout(&cycle, "a"), None);
    }

    #[test]
    fn presentation_session_uses_unique_live_checkout_when_report_is_stale_or_missing() {
        let index = sessions(&[
            ("stale", "/repo", None),
            ("smartlead", "/worktrees/smartlead", None),
            ("audio", "/worktrees/audio", None),
        ]);
        assert_eq!(
            presentation_session_for_checkout(
                &index,
                Some("stale"),
                Path::new("/worktrees/smartlead")
            ),
            Some("smartlead".into())
        );
        assert_eq!(
            presentation_session_for_checkout(&index, None, Path::new("/worktrees/audio")),
            Some("audio".into())
        );
    }

    #[test]
    fn presentation_session_keeps_reported_fork_and_rejects_ambiguous_checkout() {
        let index = sessions(&[
            ("parent", "/worktree", None),
            ("child", "/worktree", Some("parent")),
            ("stale", "/repo", None),
        ]);
        assert_eq!(
            presentation_session_for_checkout(&index, Some("parent"), Path::new("/worktree")),
            Some("parent".into())
        );
        assert_eq!(
            presentation_session_for_checkout(&index, Some("stale"), Path::new("/worktree")),
            None
        );
    }

    #[test]
    fn live_checkout_must_belong_to_terminal_repository() {
        use std::process::Command;

        let base =
            std::env::temp_dir().join(format!("herdr-codex-presentation-{}", std::process::id()));
        let repo = base.join("repo");
        let worktree = base.join("worktree");
        let unrelated = base.join("unrelated");
        std::fs::create_dir_all(&repo).unwrap();
        std::fs::create_dir_all(&unrelated).unwrap();
        let git = |cwd: &Path, args: &[&str]| {
            assert!(Command::new("git")
                .args(args)
                .current_dir(cwd)
                .status()
                .unwrap()
                .success());
        };
        git(&repo, &["init", "--quiet"]);
        git(
            &repo,
            &[
                "-c",
                "user.name=Herdr Test",
                "-c",
                "user.email=test@example.com",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "base",
            ],
        );
        git(
            &repo,
            &[
                "worktree",
                "add",
                "--quiet",
                "-b",
                "test-worktree",
                worktree.to_str().unwrap(),
            ],
        );
        git(&unrelated, &["init", "--quiet"]);

        let footer = |path: &Path| {
            format!(
                "GPT-6.1-Sol low · Task · repo · {} · branch\n← for agents · ? for shortcuts",
                path.display()
            )
        };
        assert_eq!(
            validated_live_checkout(&repo, &footer(&worktree)),
            Some(worktree.clone())
        );
        assert_eq!(validated_live_checkout(&repo, &footer(&unrelated)), None);
        std::fs::remove_dir_all(base).unwrap();
    }
}
