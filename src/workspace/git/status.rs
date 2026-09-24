use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::workspace::WorkspaceGitStatusSnapshot;

use super::{
    config::{read_branch_config, upstream_full_ref},
    discovery::{
        canonicalize_best_effort_path, git_branch, git_ref_storage_is_reftable,
        git_rev_parse_verify, git_space_metadata, git_symbolic_head_full, git_worktree_info,
        read_ref_oid, GitWorktreeInfo,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusCacheEntry {
    pub fingerprint: GitStatusFingerprint,
    pub snapshot: WorkspaceGitStatusSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatusFingerprint {
    pub git_dir: PathBuf,
    pub git_common_dir: PathBuf,
    pub head: GitHeadIdentity,
    pub upstream: Option<GitUpstreamIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHeadIdentity {
    Branch {
        full_ref: String,
        short_name: String,
        oid: Option<String>,
    },
    Detached {
        oid: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitUpstreamIdentity {
    pub remote: String,
    pub merge_ref: String,
    pub full_ref: String,
    pub oid: Option<String>,
}

pub fn git_status_cache_key(cwd: &Path) -> Option<PathBuf> {
    git_worktree_info(cwd).map(|info| canonicalize_best_effort_path(&info.repo_root))
}

pub fn git_status_snapshot_for_cwd(
    cwd: &Path,
    cached: Option<&GitStatusCacheEntry>,
) -> (WorkspaceGitStatusSnapshot, Option<GitStatusCacheEntry>) {
    let space = git_space_metadata(cwd);
    let dirty_count = git_dirty_count(cwd);
    let Some(fingerprint) = git_status_fingerprint(cwd) else {
        return (
            WorkspaceGitStatusSnapshot {
                branch: git_branch(cwd),
                ahead_behind: None,
                dirty_count,
                space,
            },
            None,
        );
    };
    let branch = fingerprint.branch_name().map(str::to_string);

    if let Some(cached) = cached.filter(|entry| entry.fingerprint == fingerprint) {
        let snapshot = WorkspaceGitStatusSnapshot {
            branch,
            ahead_behind: cached.snapshot.ahead_behind,
            dirty_count,
            space,
        };
        return (
            snapshot.clone(),
            Some(GitStatusCacheEntry {
                fingerprint,
                snapshot,
            }),
        );
    }

    let ahead_behind = fingerprint
        .head_oid()
        .zip(fingerprint.upstream_oid())
        .and_then(|(head_oid, upstream_oid)| git_ahead_behind_between(cwd, head_oid, upstream_oid));
    let snapshot = WorkspaceGitStatusSnapshot {
        branch,
        ahead_behind,
        dirty_count,
        space,
    };
    (
        snapshot.clone(),
        Some(GitStatusCacheEntry {
            fingerprint,
            snapshot,
        }),
    )
}

pub(crate) fn git_dirty_count(cwd: &Path) -> Option<usize> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=normal"])
        .current_dir(cwd)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let mut count = 0;
    let mut unstaged_modified = Vec::new();
    let mut records = output.stdout.split(|byte| *byte == b'\0');
    while let Some(record) = records.next() {
        if record.is_empty() {
            continue;
        }
        count += 1;
        if record.len() < 4 {
            continue;
        }
        if record[0] == b' ' && record[1] == b'M' {
            unstaged_modified.push(&record[3..]);
        }
        // Porcelain -z adds a second path after rename and copy records.
        if matches!(record[0], b'R' | b'C') || matches!(record[1], b'R' | b'C') {
            records.next();
        }
    }

    if !unstaged_modified.is_empty() {
        let diff = Command::new("git")
            .args(["diff", "--name-only", "-z", "--no-ext-diff"])
            .current_dir(cwd)
            .output()
            .ok()?;
        if !diff.status.success() {
            return None;
        }
        let changed: HashSet<&[u8]> = diff.stdout.split(|byte| *byte == b'\0').collect();
        count -= unstaged_modified
            .iter()
            .filter(|path| !changed.contains(**path))
            .count();
    }
    Some(count)
}

pub(super) fn git_status_fingerprint(cwd: &Path) -> Option<GitStatusFingerprint> {
    let info = git_worktree_info(cwd)?;
    let head = read_head_identity(&info)?;
    let upstream = match &head {
        GitHeadIdentity::Branch { short_name, .. } => read_upstream_identity(&info, short_name),
        GitHeadIdentity::Detached { .. } => None,
    };

    Some(GitStatusFingerprint {
        git_dir: canonicalize_best_effort_path(&info.git_dir),
        git_common_dir: canonicalize_best_effort_path(&info.git_common_dir),
        head,
        upstream,
    })
}

impl GitStatusFingerprint {
    fn branch_name(&self) -> Option<&str> {
        match &self.head {
            GitHeadIdentity::Branch { short_name, .. } => Some(short_name.as_str()),
            GitHeadIdentity::Detached { .. } => None,
        }
    }

    fn head_oid(&self) -> Option<&str> {
        match &self.head {
            GitHeadIdentity::Branch { oid, .. } => oid.as_deref(),
            GitHeadIdentity::Detached { oid } => Some(oid.as_str()),
        }
    }

    fn upstream_oid(&self) -> Option<&str> {
        self.upstream
            .as_ref()
            .and_then(|upstream| upstream.oid.as_deref())
    }
}

fn read_head_identity(info: &GitWorktreeInfo) -> Option<GitHeadIdentity> {
    if git_ref_storage_is_reftable(&info.git_common_dir) {
        return read_head_identity_from_git(info);
    }

    read_head_identity_from_files(info)
}

fn read_head_identity_from_git(info: &GitWorktreeInfo) -> Option<GitHeadIdentity> {
    if let Some(full_ref) = git_symbolic_head_full(&info.repo_root) {
        let short_name = full_ref.strip_prefix("refs/heads/")?.to_string();
        let oid = git_rev_parse_verify(&info.repo_root, &full_ref);
        return Some(GitHeadIdentity::Branch {
            full_ref,
            short_name,
            oid,
        });
    }

    git_rev_parse_verify(&info.repo_root, "HEAD").map(|oid| GitHeadIdentity::Detached { oid })
}

fn read_head_identity_from_files(info: &GitWorktreeInfo) -> Option<GitHeadIdentity> {
    let head = std::fs::read_to_string(info.git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(full_ref) = head.strip_prefix("ref: ") {
        let short_name = full_ref.strip_prefix("refs/heads/")?.to_string();
        let oid = read_ref_oid(&info.git_common_dir, full_ref);
        return Some(GitHeadIdentity::Branch {
            full_ref: full_ref.to_string(),
            short_name,
            oid,
        });
    }

    (!head.is_empty()).then(|| GitHeadIdentity::Detached {
        oid: head.to_string(),
    })
}

fn read_upstream_identity(info: &GitWorktreeInfo, branch: &str) -> Option<GitUpstreamIdentity> {
    let config = read_branch_config(info, branch)?;
    let full_ref = upstream_full_ref(&config)?;
    let oid = if git_ref_storage_is_reftable(&info.git_common_dir) {
        git_rev_parse_verify(&info.repo_root, &full_ref)
    } else {
        read_ref_oid(&info.git_common_dir, &full_ref)
    };
    Some(GitUpstreamIdentity {
        remote: config.remote,
        merge_ref: config.merge_ref,
        full_ref,
        oid,
    })
}

#[cfg(test)]
pub(crate) fn git_ahead_behind(cwd: &Path) -> Option<(usize, usize)> {
    super::discovery::git_repo_root(cwd)?;

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-list", "--left-right", "--count", "HEAD...@{upstream}"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_git_ahead_behind_output(&stdout)
}

fn git_ahead_behind_between(
    cwd: &Path,
    head_oid: &str,
    upstream_oid: &str,
) -> Option<(usize, usize)> {
    let range = format!("{head_oid}...{upstream_oid}");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(["rev-list", "--left-right", "--count", &range])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8(output.stdout).ok()?;
    parse_git_ahead_behind_output(&stdout)
}

fn parse_git_ahead_behind_output(stdout: &str) -> Option<(usize, usize)> {
    let mut parts = stdout.split_whitespace();
    let ahead = parts.next()?.parse().ok()?;
    let behind = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workspace::git::test_support::{run_git, temp_test_dir, write_fake_tracked_repo};

    fn initialized_repo(name: &str) -> PathBuf {
        let root = temp_test_dir(name);
        std::fs::create_dir_all(&root).unwrap();
        run_git(&root, &["init", "-b", "main"]);
        run_git(&root, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&root, &["config", "user.name", "Herdr Test"]);
        std::fs::write(root.join("tracked.txt"), "initial\n").unwrap();
        run_git(&root, &["add", "tracked.txt"]);
        run_git(&root, &["commit", "-m", "initial"]);
        root
    }

    #[test]
    fn git_dirty_reports_worktree_states_and_ignores_ignored_files() {
        let root = initialized_repo("dirty-states");
        assert_eq!(git_dirty_count(&root), Some(0));

        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(1));
        std::fs::write(root.join("another-untracked.txt"), "new\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(2));
        std::fs::remove_file(root.join("untracked.txt")).unwrap();
        std::fs::remove_file(root.join("another-untracked.txt")).unwrap();

        std::fs::write(root.join("tracked.txt"), "modified\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(1));
        run_git(&root, &["add", "tracked.txt"]);
        assert_eq!(git_dirty_count(&root), Some(1));
        run_git(&root, &["reset", "--hard", "HEAD"]);

        run_git(&root, &["mv", "tracked.txt", "renamed.txt"]);
        assert_eq!(git_dirty_count(&root), Some(1));
        run_git(&root, &["reset", "--hard", "HEAD"]);

        std::fs::write(root.join(".gitignore"), "ignored.txt\n").unwrap();
        run_git(&root, &["add", ".gitignore"]);
        run_git(&root, &["commit", "-m", "ignore test file"]);
        std::fs::write(root.join("ignored.txt"), "ignored\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(0));

        std::fs::remove_file(root.join("tracked.txt")).unwrap();
        assert_eq!(git_dirty_count(&root), Some(1));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_dirty_ignores_changes_removed_by_clean_filter() {
        let root = initialized_repo("dirty-clean-filter");
        std::fs::write(
            root.join(".gitattributes"),
            "tracked.txt filter=stripspace\n",
        )
        .unwrap();
        run_git(
            &root,
            &["config", "filter.stripspace.clean", "git stripspace"],
        );
        run_git(&root, &["add", ".gitattributes"]);
        run_git(&root, &["commit", "-m", "add clean filter"]);

        std::fs::write(root.join("tracked.txt"), "initial\n\n").unwrap();
        let status = Command::new("git")
            .args(["status", "--porcelain=v1"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(status.status.success());
        assert_eq!(status.stdout, b" M tracked.txt\n");
        assert_eq!(git_dirty_count(&root), Some(0));

        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(1));
        run_git(&root, &["add", "untracked.txt"]);
        assert_eq!(git_dirty_count(&root), Some(1));

        std::fs::write(root.join("tracked.txt"), "changed\n").unwrap();
        assert_eq!(git_dirty_count(&root), Some(2));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_dirty_is_recomputed_when_status_fingerprint_is_cached() {
        let root = initialized_repo("dirty-cache-refresh");
        let (clean, cache) = git_status_snapshot_for_cwd(&root, None);
        assert_eq!(clean.dirty_count, Some(0));

        std::fs::write(root.join("untracked.txt"), "new\n").unwrap();
        let (dirty, _) = git_status_snapshot_for_cwd(&root, cache.as_ref());

        assert_eq!(dirty.dirty_count, Some(1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_dirty_returns_none_outside_a_repository() {
        let root = temp_test_dir("dirty-non-git");
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(git_dirty_count(&root), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_dirty_reports_conflicts() {
        let root = initialized_repo("dirty-conflict");
        run_git(&root, &["checkout", "-b", "conflict"]);
        std::fs::write(root.join("tracked.txt"), "conflict branch\n").unwrap();
        run_git(&root, &["commit", "-am", "conflict branch"]);
        run_git(&root, &["checkout", "main"]);
        std::fs::write(root.join("tracked.txt"), "main branch\n").unwrap();
        run_git(&root, &["commit", "-am", "main branch"]);

        let merge = Command::new("git")
            .args(["merge", "conflict"])
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(!merge.status.success());
        assert_eq!(git_dirty_count(&root), Some(1));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_dirty_is_scoped_to_each_linked_worktree() {
        let base = temp_test_dir("dirty-linked-worktree");
        let root = base.join("repo");
        let linked = base.join("linked");
        std::fs::create_dir_all(&base).unwrap();
        let root_arg = root.to_string_lossy().to_string();
        run_git(&base, &["init", "-b", "main", &root_arg]);
        run_git(&root, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&root, &["config", "user.name", "Herdr Test"]);
        std::fs::write(root.join("tracked.txt"), "initial\n").unwrap();
        run_git(&root, &["add", "tracked.txt"]);
        run_git(&root, &["commit", "-m", "initial"]);
        let linked_arg = linked.to_string_lossy().to_string();
        run_git(&root, &["worktree", "add", "-b", "linked", &linked_arg]);

        std::fs::write(linked.join("untracked.txt"), "new\n").unwrap();

        assert_eq!(git_dirty_count(&root), Some(0));
        assert_eq!(git_dirty_count(&linked), Some(1));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn git_status_cache_key_ignores_invalid_git_marker() {
        let base = temp_test_dir("invalid-git-root");
        let cwd = base.join("workspace");
        std::fs::create_dir_all(base.join(".git")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();

        assert_eq!(git_status_cache_key(&cwd), None);

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn git_status_reuses_cached_ahead_behind_when_fingerprint_matches() {
        let root = temp_test_dir("cache-hit");
        write_fake_tracked_repo(&root);
        let fingerprint = git_status_fingerprint(&root).unwrap();
        let cached = GitStatusCacheEntry {
            fingerprint,
            snapshot: WorkspaceGitStatusSnapshot {
                branch: Some("main".into()),
                ahead_behind: Some((2, 1)),
                dirty_count: Some(0),
                space: git_space_metadata(&root),
            },
        };

        let (snapshot, update) = git_status_snapshot_for_cwd(&root, Some(&cached));

        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.ahead_behind, Some((2, 1)));
        assert_eq!(update.unwrap().snapshot.ahead_behind, Some((2, 1)));

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_status_does_not_reuse_cache_when_branch_changes_at_same_oid() {
        let root = temp_test_dir("branch-switch");
        write_fake_tracked_repo(&root);
        let fingerprint = git_status_fingerprint(&root).unwrap();
        let cached = GitStatusCacheEntry {
            fingerprint,
            snapshot: WorkspaceGitStatusSnapshot {
                branch: Some("main".into()),
                ahead_behind: Some((4, 0)),
                dirty_count: Some(0),
                space: git_space_metadata(&root),
            },
        };
        std::fs::write(root.join(".git/HEAD"), "ref: refs/heads/feature\n").unwrap();
        std::fs::write(
            root.join(".git/refs/heads/feature"),
            "1111111111111111111111111111111111111111\n",
        )
        .unwrap();
        std::fs::write(
            root.join(".git/config"),
            "[branch \"feature\"]\n\tremote = origin\n\tmerge = refs/heads/main\n",
        )
        .unwrap();

        let (snapshot, _) = git_status_snapshot_for_cwd(&root, Some(&cached));

        assert_eq!(snapshot.branch.as_deref(), Some("feature"));
        assert_eq!(snapshot.ahead_behind, None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_status_clears_ahead_behind_when_upstream_is_unset() {
        let root = temp_test_dir("upstream-unset");
        write_fake_tracked_repo(&root);
        let fingerprint = git_status_fingerprint(&root).unwrap();
        let cached = GitStatusCacheEntry {
            fingerprint,
            snapshot: WorkspaceGitStatusSnapshot {
                branch: Some("main".into()),
                ahead_behind: Some((0, 3)),
                dirty_count: Some(0),
                space: git_space_metadata(&root),
            },
        };
        std::fs::write(root.join(".git/config"), "").unwrap();

        let (snapshot, _) = git_status_snapshot_for_cwd(&root, Some(&cached));

        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.ahead_behind, None);

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_status_fingerprint_reads_packed_refs() {
        let root = temp_test_dir("packed-refs");
        write_fake_tracked_repo(&root);
        std::fs::remove_file(root.join(".git/refs/remotes/origin/main")).unwrap();
        std::fs::write(
            root.join(".git/packed-refs"),
            "# pack-refs with: peeled fully-peeled sorted\n2222222222222222222222222222222222222222 refs/remotes/origin/main\n",
        )
        .unwrap();

        let fingerprint = git_status_fingerprint(&root).unwrap();

        assert_eq!(
            fingerprint.upstream.unwrap().oid.as_deref(),
            Some("2222222222222222222222222222222222222222")
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_status_cache_key_is_per_linked_worktree_checkout() {
        let base = temp_test_dir("linked-worktree-keys");
        let common_dir = base.join("repo/.git");
        let worktree_one = base.join("one");
        let worktree_two = base.join("two");
        let git_dir_one = common_dir.join("worktrees/one");
        let git_dir_two = common_dir.join("worktrees/two");
        std::fs::create_dir_all(&git_dir_one).unwrap();
        std::fs::create_dir_all(&git_dir_two).unwrap();
        std::fs::create_dir_all(&worktree_one).unwrap();
        std::fs::create_dir_all(&worktree_two).unwrap();
        std::fs::write(
            worktree_one.join(".git"),
            format!("gitdir: {}\n", git_dir_one.display()),
        )
        .unwrap();
        std::fs::write(
            worktree_two.join(".git"),
            format!("gitdir: {}\n", git_dir_two.display()),
        )
        .unwrap();
        std::fs::write(git_dir_one.join("HEAD"), "ref: refs/heads/one\n").unwrap();
        std::fs::write(git_dir_two.join("HEAD"), "ref: refs/heads/two\n").unwrap();

        assert_ne!(
            git_status_cache_key(&worktree_one),
            git_status_cache_key(&worktree_two)
        );

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn git_status_fingerprint_reads_reftable_branch_identity() {
        let root = temp_test_dir("reftable-fingerprint");
        let root_arg = root.to_string_lossy().to_string();
        let output = std::process::Command::new("git")
            .args(["init", "--ref-format=reftable", "-b", "main", &root_arg])
            .output()
            .unwrap();
        if !output.status.success() {
            std::fs::remove_dir_all(root).unwrap();
            return;
        }
        run_git(&root, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&root, &["config", "user.name", "Herdr Test"]);
        run_git(&root, &["commit", "--allow-empty", "-m", "initial"]);

        let fingerprint = git_status_fingerprint(&root).unwrap();

        assert_eq!(
            fingerprint.head,
            GitHeadIdentity::Branch {
                full_ref: "refs/heads/main".into(),
                short_name: "main".into(),
                oid: git_rev_parse_verify(&root, "HEAD"),
            }
        );

        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn git_status_recomputes_ahead_behind_when_head_moves() {
        let base = temp_test_dir("head-moves");
        let remote = base.join("remote.git");
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let remote_arg = remote.to_string_lossy().to_string();
        run_git(&base, &["init", "--bare", &remote_arg]);
        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.email", "herdr@example.invalid"]);
        run_git(&repo, &["config", "user.name", "Herdr Test"]);
        run_git(&repo, &["commit", "--allow-empty", "-m", "initial"]);
        run_git(&repo, &["branch", "-M", "main"]);
        run_git(&repo, &["remote", "add", "origin", &remote_arg]);
        run_git(&repo, &["push", "-u", "origin", "main"]);

        let (initial, cache_entry) = git_status_snapshot_for_cwd(&repo, None);
        assert_eq!(initial.ahead_behind, Some((0, 0)));
        run_git(&repo, &["commit", "--allow-empty", "-m", "ahead"]);

        let (updated, _) = git_status_snapshot_for_cwd(&repo, cache_entry.as_ref());

        assert_eq!(updated.branch.as_deref(), Some("main"));
        assert_eq!(updated.ahead_behind, Some((1, 0)));

        std::fs::remove_dir_all(base).unwrap();
    }
}
