//! Local repository activity and branch comparisons, without fetching missing objects.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use git2::{BranchType, Oid};

use crate::{Error, MergedReason, Repo, Result, Signature, cli, partial};

const DAY_SECONDS: i64 = 86_400;
const WEEK_SECONDS: i64 = 7 * DAY_SECONDS;
const WEEK_COUNT: usize = 52;
const STALE_SECONDS: i64 = 90 * DAY_SECONDS;

/// One UTC Monday-based week; the final bucket is the current, incomplete week.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeekActivity {
    pub start: i64,
    pub commits: u64,
}

/// An author after applying Git's mailmap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorActivity {
    pub name: String,
    pub email: String,
    pub commits: u64,
}

/// A commit with its recorded author and committer date.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricCommit {
    pub oid: Oid,
    pub summary: String,
    pub author: Signature,
    pub committed_at: i64,
}

/// Activity across all local refs and HEAD, counting each reachable commit once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetrics {
    pub weeks: Vec<WeekActivity>,
    pub authors: Vec<AuthorActivity>,
    pub last_commit: Option<MetricCommit>,
    /// Loose plus packed object storage, excluding the working tree (in bytes).
    pub size_bytes: u64,
    /// Loose plus packed objects, as reported by `git count-objects -v`.
    pub objects: u64,
}

/// Commits reachable only on the left / only on the right of a comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    pub ahead: usize,
    pub behind: usize,
}

/// A local branch's activity. Age means days since its tip commit, not creation time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchMetrics {
    pub name: String,
    pub default_branch: Option<String>,
    pub default_divergence: Option<Divergence>,
    pub upstream: Option<String>,
    pub upstream_divergence: Option<Divergence>,
    /// Commits not reachable from any other local branch.
    pub unique_commits: u64,
    pub last_commit: MetricCommit,
    pub age_days: u64,
    pub stale: bool,
    pub merged: Option<MergedReason>,
}

fn now() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::invalid(e.to_string()))?;
    i64::try_from(elapsed.as_secs()).map_err(|e| Error::invalid(e.to_string()))
}

fn commit_data(commit: &git2::Commit<'_>) -> MetricCommit {
    let author = commit.author();
    MetricCommit {
        oid: commit.id(),
        summary: String::from_utf8_lossy(commit.message_bytes())
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned(),
        author: Signature {
            name: String::from_utf8_lossy(author.name_bytes()).into_owned(),
            email: String::from_utf8_lossy(author.email_bytes()).into_owned(),
            time: author.when().seconds(),
            offset_minutes: author.when().offset_minutes(),
        },
        committed_at: commit.time().seconds(),
    }
}

impl Repo {
    /// The last 52 UTC calendar weeks, mailmap-aware authors, latest commit and object storage.
    /// Counts include local refs and detached HEAD; shallow history is necessarily incomplete.
    pub fn repository_metrics(&self) -> Result<RepositoryMetrics> {
        self.repository_metrics_at(now()?)
    }

    fn repository_metrics_at(&self, now: i64) -> Result<RepositoryMetrics> {
        // 1970-01-01 was Thursday: Monday is three days before the Unix epoch.
        let monday =
            (now + 3 * DAY_SECONDS).div_euclid(WEEK_SECONDS) * WEEK_SECONDS - 3 * DAY_SECONDS;
        let start = monday - 51 * WEEK_SECONDS;
        let mut weeks: Vec<_> = (0..WEEK_COUNT)
            .map(|i| WeekActivity {
                start: start + i64::try_from(i).unwrap_or_default() * WEEK_SECONDS,
                commits: 0,
            })
            .collect();
        let mut walk = self.git().revwalk()?;
        walk.push_glob("refs/*")?;
        if let Ok(head) = self.git().head() {
            walk.push(head.peel_to_commit()?.id())?;
        }
        let mut last_commit: Option<MetricCommit> = None;
        for id in walk {
            let commit = self.git().find_commit(id?)?;
            let time = commit.time().seconds();
            if time <= now && time >= start {
                let index = usize::try_from((time - start) / WEEK_SECONDS)
                    .map_err(|e| Error::invalid(e.to_string()))?;
                if let Some(week) = weeks.get_mut(index) {
                    week.commits += 1;
                }
            }
            if last_commit
                .as_ref()
                .is_none_or(|last| (time, commit.id()) > (last.committed_at, last.oid))
            {
                last_commit = Some(commit_data(&commit));
            }
        }

        // Unborn HEAD is not a revision; shortlog with --all still handles an empty repo.
        let authors = if self.git().head().is_ok() {
            self.local_metrics_command(&["shortlog", "-sne", "--all", "HEAD"])?
        } else {
            self.local_metrics_command(&["shortlog", "-sne", "--all"])?
        };
        let authors = authors.lines().map(parse_author).collect::<Result<_>>()?;
        let storage = self.local_metrics_command(&["count-objects", "-v"])?;
        let values: BTreeMap<_, _> = storage
            .lines()
            .filter_map(|line| line.split_once(": "))
            .filter(|(key, _)| matches!(*key, "count" | "in-pack" | "size" | "size-pack"))
            .map(|(key, value)| {
                value
                    .parse::<u64>()
                    .map(|value| (key, value))
                    .map_err(|e| Error::invalid(format!("Invalid count-objects output: {e}")))
            })
            .collect::<Result<_>>()?;
        let value = |key| values.get(key).copied().unwrap_or_default();
        Ok(RepositoryMetrics {
            weeks,
            authors,
            last_commit,
            size_bytes: (value("size") + value("size-pack")) * 1024,
            objects: value("count") + value("in-pack"),
        })
    }

    /// Every local branch compared with the cached default branch and its upstream.
    /// Uses remote HEAD (origin first), then main/master; an unknown default stays absent.
    /// Squash detection is skipped for partial clones and custom merge/filter drivers, so this read
    /// cannot fetch missing objects or execute repository-defined programs.
    pub fn branch_metrics(&self) -> Result<Vec<BranchMetrics>> {
        self.branch_metrics_at(now()?)
    }

    fn metrics_default(&self) -> Result<Option<(String, Oid)>> {
        let mut remotes: Vec<_> = self
            .git()
            .remotes()?
            .iter()
            .flatten()
            .flatten()
            .map(str::to_owned)
            .collect();
        remotes.sort_by_key(|name| (name != "origin", name.clone()));
        for remote in remotes {
            if let Ok(head) = self
                .git()
                .find_reference(&format!("refs/remotes/{remote}/HEAD"))
            {
                let resolved = match head.resolve() {
                    Ok(resolved) => resolved,
                    Err(error) if error.code() == git2::ErrorCode::NotFound => continue,
                    Err(error) => return Err(error.into()),
                };
                if let Some(full) = resolved.name()?.strip_prefix("refs/remotes/") {
                    // Prefer the local counterpart of the cached remote default.
                    let short = full.strip_prefix(&format!("{remote}/")).unwrap_or(full);
                    if let Ok(branch) = self.git().find_branch(short, BranchType::Local) {
                        return Ok(Some((
                            short.to_owned(),
                            branch.get().peel_to_commit()?.id(),
                        )));
                    }
                    return Ok(Some((full.to_owned(), resolved.peel_to_commit()?.id())));
                }
            }
        }
        for name in ["main", "master"] {
            if let Ok(branch) = self.git().find_branch(name, BranchType::Local) {
                return Ok(Some((name.to_owned(), branch.get().peel_to_commit()?.id())));
            }
        }
        Ok(None)
    }

    fn branch_metrics_at(&self, now: i64) -> Result<Vec<BranchMetrics>> {
        let base = self.metrics_default()?;
        let squash = !partial::is_partial_clone(self.git()) && self.metrics_squash_is_safe()?;
        let merged = base
            .as_ref()
            .map(|(_, id)| self.merged_branches(&id.to_string(), squash))
            .transpose()?
            .unwrap_or_default();
        let branches: Vec<_> = self
            .git()
            .branches(Some(BranchType::Local))?
            .map(|item| {
                let (branch, _) = item?;
                let name = branch
                    .name()?
                    .ok_or_else(|| Error::invalid("Non-UTF-8 branch name"))?;
                Ok((name.to_owned(), branch.get().peel_to_commit()?.id()))
            })
            .collect::<Result<_>>()?;
        let mut out = Vec::new();
        for (name, tip) in &branches {
            let branch = self.git().find_branch(name, BranchType::Local)?;
            let upstream = match branch.upstream() {
                Ok(upstream) => Some(upstream),
                Err(e) if e.code() == git2::ErrorCode::NotFound => None,
                Err(e) => return Err(e.into()),
            };
            let upstream_name = upstream
                .as_ref()
                .map(git2::Branch::name)
                .transpose()?
                .flatten()
                .map(str::to_owned);
            let upstream_divergence = upstream
                .as_ref()
                .map(|b| self.metrics_divergence(*tip, b.get().peel_to_commit()?.id()))
                .transpose()?;
            let mut walk = self.git().revwalk()?;
            walk.push(*tip)?;
            for (other, other_tip) in &branches {
                if other != name {
                    walk.hide(*other_tip)?;
                }
            }
            let mut unique_commits = 0;
            for id in walk {
                id?;
                unique_commits += 1;
            }
            let last_commit = commit_data(&self.git().find_commit(*tip)?);
            let age_days = u64::try_from((now - last_commit.committed_at).max(0) / DAY_SECONDS)
                .map_err(|e| Error::invalid(e.to_string()))?;
            let default_divergence = base
                .as_ref()
                .map(|(_, base)| self.metrics_divergence(*tip, *base))
                .transpose()?;
            let stale = now - last_commit.committed_at > STALE_SECONDS;
            let reason = merged
                .iter()
                .find(|b| b.name == *name)
                .and_then(|b| b.reason);
            out.push(BranchMetrics {
                name: name.clone(),
                default_branch: base.as_ref().map(|(name, _)| name.clone()),
                default_divergence,
                upstream: upstream_name,
                upstream_divergence,
                unique_commits,
                last_commit,
                age_days,
                stale,
                // The default branch itself is not classified as merged.
                merged: if base.as_ref().is_some_and(|(base, _)| base == name) {
                    None
                } else {
                    reason
                },
            });
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn metrics_squash_is_safe(&self) -> Result<bool> {
        // Git also sees environment configuration overrides that libgit2 does not read.
        let keys = self.local_metrics_command(&["config", "--name-only", "--list"])?;
        Ok(!keys.lines().any(|key| {
            let key = key.to_ascii_lowercase();
            (key.starts_with("merge.") && key.ends_with(".driver"))
                || (key.starts_with("filter.")
                    && [".clean", ".smudge", ".process"]
                        .iter()
                        .any(|suffix| key.ends_with(suffix)))
                || (key.starts_with("remote.") && key.ends_with(".promisor"))
                || key == "extensions.partialclone"
        }))
    }

    fn metrics_divergence(&self, tip: Oid, base: Oid) -> Result<Divergence> {
        let (ahead, behind) = self.git().graph_ahead_behind(tip, base)?;
        Ok(Divergence { ahead, behind })
    }

    fn local_metrics_command(&self, args: &[&str]) -> Result<String> {
        let out = cli::command(Some(self.workdir()))
            .env("GIT_NO_LAZY_FETCH", "1")
            .args(args)
            .output()
            .map_err(Error::GitMissing)?;
        if !out.status.success() {
            return Err(Error::Cli {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

fn parse_author(line: &str) -> Result<AuthorActivity> {
    let invalid = || Error::invalid(format!("Invalid shortlog output: {line}"));
    let (count, identity) = line.trim().split_once('\t').ok_or_else(invalid)?;
    let (name, email) = identity.rsplit_once(" <").ok_or_else(invalid)?;
    Ok(AuthorActivity {
        name: name.to_owned(),
        email: email.strip_suffix('>').ok_or_else(invalid)?.to_owned(),
        commits: count.trim().parse().map_err(|_error| invalid())?,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    use super::*;

    const MONDAY: i64 = 1_704_067_200; // 2024-01-01 UTC

    #[expect(
        clippy::disallowed_methods,
        reason = "test oracle and throwaway repository setup"
    )]
    fn git(path: &Path, args: &[&str], date: i64) -> String {
        let date = format!("{date} +0000");
        let out = Command::new("git")
            .current_dir(path)
            .args(["-c", "core.hooksPath=", "-c", "commit.gpgsign=false"])
            .args(args)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env(
                "GIT_CONFIG_GLOBAL",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            )
            .env("GIT_AUTHOR_NAME", "Ada")
            .env("GIT_AUTHOR_EMAIL", "ada@example.test")
            .env("GIT_COMMITTER_NAME", "Ada")
            .env("GIT_COMMITTER_EMAIL", "ada@example.test")
            .env("GIT_AUTHOR_DATE", &date)
            .env("GIT_COMMITTER_DATE", date)
            .output()
            .expect("git runs");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    #[test]
    fn repository_metrics_calendar_mailmap_storage_and_empty() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"], MONDAY);
        let repo = Repo::open(p).expect("repo");
        let empty = repo.repository_metrics_at(MONDAY).expect("empty metrics");
        assert!(empty.last_commit.is_none());
        assert_eq!(empty.authors, []);
        assert_eq!(empty.weeks.len(), 52);
        git(
            p,
            &["commit", "--allow-empty", "-qm", "old"],
            MONDAY - 52 * WEEK_SECONDS,
        );
        git(p, &["commit", "--allow-empty", "-qm", "Sunday"], MONDAY - 1);
        git(p, &["commit", "--allow-empty", "-qm", "Monday"], MONDAY);
        git(p, &["branch", "other"], MONDAY);
        git(p, &["switch", "-q", "--detach"], MONDAY);
        git(
            p,
            &["commit", "--allow-empty", "-qm", "detached"],
            MONDAY + 1,
        );
        std::fs::write(
            p.join(".mailmap"),
            "Countess <countess@example.test> Ada <ada@example.test>\n",
        )
        .expect("mailmap");
        let metrics = repo.repository_metrics_at(MONDAY + 2).expect("metrics");
        assert_eq!(metrics.weeks[50].commits, 1);
        assert_eq!(metrics.weeks[51].commits, 2);
        assert_eq!(metrics.weeks.iter().map(|w| w.commits).sum::<u64>(), 3);
        assert_eq!(metrics.authors[0].name, "Countess");
        assert_eq!(metrics.authors[0].email, "countess@example.test");
        assert_eq!(metrics.authors[0].commits, 4);
        let oracle = git(p, &["shortlog", "-sne", "--all", "HEAD"], MONDAY);
        assert_eq!(
            metrics.authors,
            oracle
                .lines()
                .map(parse_author)
                .collect::<Result<Vec<_>>>()
                .expect("oracle")
        );
        assert_eq!(
            metrics.last_commit.as_ref().expect("last").oid.to_string(),
            git(p, &["rev-parse", "HEAD"], MONDAY).trim()
        );
        // Pack reachable objects and leave the detached tip loose: both counters must contribute.
        git(p, &["repack", "-ad"], MONDAY);
        let packed = repo
            .repository_metrics_at(MONDAY + 2)
            .expect("packed metrics");
        let counts = git(p, &["count-objects", "-v"], MONDAY);
        let count = |key: &str| {
            counts
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{key}: ")))
                .expect("counter")
                .parse::<u64>()
                .expect("number")
        };
        assert_eq!(packed.objects, count("count") + count("in-pack"));
        assert_eq!(
            packed.size_bytes,
            (count("size") + count("size-pack")) * 1024
        );
    }

    #[test]
    fn branch_metrics_divergence_unique_age_and_merge() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"], MONDAY);
        git(p, &["commit", "--allow-empty", "-qm", "root"], MONDAY);
        git(p, &["branch", "done"], MONDAY);
        git(p, &["switch", "-qc", "topic"], MONDAY);
        std::fs::write(p.join("topic.txt"), "topic\n").expect("file");
        git(p, &["add", "--", "topic.txt"], MONDAY);
        git(p, &["commit", "-qm", "topic"], MONDAY + DAY_SECONDS);
        git(
            p,
            &["update-ref", "refs/remotes/origin/topic", "HEAD"],
            MONDAY,
        );
        git(
            p,
            &[
                "config",
                "remote.origin.url",
                "https://invalid.example/repo",
            ],
            MONDAY,
        );
        git(
            p,
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/*:refs/remotes/origin/*",
            ],
            MONDAY,
        );
        git(
            p,
            &["branch", "--set-upstream-to=origin/topic", "topic"],
            MONDAY,
        );
        git(
            p,
            &["commit", "--allow-empty", "-qm", "topic2"],
            MONDAY + 2 * DAY_SECONDS,
        );
        git(p, &["switch", "-q", "main"], MONDAY);
        git(
            p,
            &["commit", "--allow-empty", "-qm", "main2"],
            MONDAY + 3 * DAY_SECONDS,
        );
        let repo = Repo::open(p).expect("repo");
        let metrics = repo
            .branch_metrics_at(MONDAY + 93 * DAY_SECONDS)
            .expect("metrics");
        let topic = metrics.iter().find(|b| b.name == "topic").expect("topic");
        assert_eq!(topic.default_branch.as_deref(), Some("main"));
        let oracle = git(
            p,
            &["rev-list", "--left-right", "--count", "topic...main"],
            MONDAY,
        );
        let divergence = topic.default_divergence.expect("default comparison");
        assert_eq!(
            format!("{}\t{}", divergence.ahead, divergence.behind),
            oracle.trim()
        );
        assert_eq!(
            topic.upstream_divergence,
            Some(Divergence {
                ahead: 1,
                behind: 0
            })
        );
        assert_eq!(
            topic.unique_commits.to_string(),
            git(
                p,
                &["rev-list", "--count", "topic", "--not", "main", "done"],
                MONDAY
            )
            .trim()
        );
        assert_eq!(topic.age_days, 91);
        assert!(topic.stale);
        assert_eq!(topic.last_commit.author.name, "Ada");
        assert_eq!(topic.last_commit.committed_at, MONDAY + 2 * DAY_SECONDS);
        assert_eq!(topic.merged, None);
        assert_eq!(
            metrics
                .iter()
                .find(|b| b.name == "done")
                .expect("done")
                .merged,
            Some(MergedReason::Reachable)
        );
        let boundary = repo
            .branch_metrics_at(MONDAY + 92 * DAY_SECONDS)
            .expect("boundary");
        assert!(
            !boundary
                .iter()
                .find(|b| b.name == "topic")
                .expect("topic")
                .stale
        );
        git(p, &["merge", "--squash", "topic"], MONDAY);
        git(p, &["commit", "-qm", "squashed"], MONDAY + 4 * DAY_SECONDS);
        let metrics = repo
            .branch_metrics_at(MONDAY + 93 * DAY_SECONDS)
            .expect("squashed metrics");
        if cli::has_merge_tree_write_tree() {
            assert_eq!(
                metrics
                    .iter()
                    .find(|b| b.name == "topic")
                    .expect("topic")
                    .merged,
                Some(MergedReason::SquashMerged)
            );
        }
        // Without any known default, comparisons stay absent rather than comparing to HEAD.
        git(p, &["branch", "-m", "main", "trunk"], MONDAY);
        assert!(
            repo.branch_metrics_at(MONDAY)
                .expect("unknown default")
                .iter()
                .all(|b| b.default_divergence.is_none())
        );
        git(
            p,
            &["update-ref", "refs/remotes/origin/trunk", "HEAD"],
            MONDAY,
        );
        git(
            p,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/trunk",
            ],
            MONDAY,
        );
        assert!(
            repo.branch_metrics_at(MONDAY)
                .expect("cached default")
                .iter()
                .all(|b| b.default_branch.as_deref() == Some("trunk"))
        );
    }
    #[test]
    fn repository_metrics_accepts_alternate_object_stores() {
        let source = tempfile::tempdir().expect("source");
        let dest = tempfile::tempdir().expect("destination");
        git(source.path(), &["init", "-q", "-b", "main"], MONDAY);
        git(
            source.path(),
            &["commit", "--allow-empty", "-qm", "root"],
            MONDAY,
        );
        git(
            source.path(),
            &[
                "clone",
                "--shared",
                "--",
                source.path().to_str().expect("source path"),
                dest.path().to_str().expect("destination path"),
            ],
            MONDAY,
        );
        let oracle = git(dest.path(), &["count-objects", "-v"], MONDAY);
        assert!(oracle.lines().any(|line| line.starts_with("alternate: ")));
        let repo = Repo::open(dest.path()).expect("repo");
        let metrics = repo
            .repository_metrics_at(MONDAY)
            .expect("metrics with alternate store");
        assert_eq!(metrics.objects, 0);
        assert_eq!(metrics.size_bytes, 0);
        assert_eq!(metrics.authors[0].commits, 1);
        assert_eq!(metrics.weeks[51].commits, 1);
        assert!(metrics.last_commit.is_some());
    }
    #[test]
    fn branch_metrics_falls_back_when_cached_remote_head_is_dangling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"], MONDAY);
        git(p, &["commit", "--allow-empty", "-qm", "root"], MONDAY);
        git(
            p,
            &[
                "config",
                "remote.origin.url",
                "https://invalid.example/repo",
            ],
            MONDAY,
        );
        git(
            p,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/deleted",
            ],
            MONDAY,
        );
        let repo = Repo::open(p).expect("repo");
        let metrics = repo
            .branch_metrics_at(MONDAY)
            .expect("dangling default falls back");
        assert_eq!(metrics[0].default_branch.as_deref(), Some("main"));
        assert_eq!(
            metrics[0].default_divergence,
            Some(Divergence {
                ahead: 0,
                behind: 0
            })
        );
    }
    #[test]
    fn branch_metrics_does_not_execute_custom_merge_drivers() {
        if let Some(path) = std::env::var_os("CANOPY_METRICS_DRIVER_FIXTURE") {
            let path = Path::new(&path);
            Repo::open(path)
                .expect("child repo")
                .branch_metrics_at(MONDAY)
                .expect("child metrics");
            assert!(
                !path.join("driver-ran").exists(),
                "environment merge driver executed"
            );
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"], MONDAY);
        std::fs::write(p.join(".gitattributes"), "tracked.txt merge=probe\n").expect("attributes");
        std::fs::write(p.join("tracked.txt"), "base\n").expect("base");
        git(p, &["add", "--", ".gitattributes", "tracked.txt"], MONDAY);
        git(p, &["commit", "-qm", "root"], MONDAY);
        git(p, &["switch", "-qc", "topic"], MONDAY);
        std::fs::write(p.join("tracked.txt"), "topic\n").expect("topic");
        git(p, &["commit", "-qam", "topic"], MONDAY);
        git(p, &["switch", "-q", "main"], MONDAY);
        std::fs::write(p.join("tracked.txt"), "main\n").expect("main");
        git(p, &["commit", "-qam", "main"], MONDAY);
        git(
            p,
            &[
                "config",
                "merge.probe.driver",
                "echo ran > driver-ran; true",
            ],
            MONDAY,
        );
        let repo = Repo::open(p).expect("repo");
        let metrics = repo.branch_metrics_at(MONDAY).expect("metrics");
        assert!(
            !p.join("driver-ran").exists(),
            "insights executed the repository's merge driver"
        );
        assert_eq!(
            metrics
                .iter()
                .find(|b| b.name == "topic")
                .expect("topic")
                .merged,
            None
        );
        // Git sees these process-local overrides; libgit2 does not.
        git(p, &["config", "--unset", "merge.probe.driver"], MONDAY);
        #[expect(
            clippy::disallowed_methods,
            reason = "a child test process isolates Git environment overrides"
        )]
        let child = Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "metrics::tests::branch_metrics_does_not_execute_custom_merge_drivers",
            ])
            .env("CANOPY_METRICS_DRIVER_FIXTURE", p)
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "merge.probe.driver")
            .env("GIT_CONFIG_VALUE_0", "echo ran > driver-ran; true")
            .output()
            .expect("child test");
        assert!(
            child.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&child.stdout),
            String::from_utf8_lossy(&child.stderr)
        );
    }
    #[test]
    fn branch_metrics_does_not_execute_renormalization_filters() {
        let dir = tempfile::tempdir().expect("tempdir");
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"], MONDAY);
        std::fs::write(p.join(".gitattributes"), "tracked.txt filter=probe\n").expect("attributes");
        std::fs::write(p.join("tracked.txt"), "base\n").expect("base");
        git(p, &["add", "--", ".gitattributes", "tracked.txt"], MONDAY);
        git(p, &["commit", "-qm", "root"], MONDAY);
        git(p, &["switch", "-qc", "topic"], MONDAY);
        std::fs::write(p.join("tracked.txt"), "topic\n").expect("topic");
        git(p, &["commit", "-qam", "topic"], MONDAY);
        git(p, &["switch", "-q", "main"], MONDAY);
        std::fs::write(p.join("tracked.txt"), "main\n").expect("main");
        git(p, &["commit", "-qam", "main"], MONDAY);
        git(p, &["config", "merge.renormalize", "true"], MONDAY);
        git(
            p,
            &["config", "filter.probe.clean", "echo ran > filter-ran; cat"],
            MONDAY,
        );
        let repo = Repo::open(p).expect("repo");
        let metrics = repo.branch_metrics_at(MONDAY).expect("metrics");
        assert!(
            !p.join("filter-ran").exists(),
            "insights executed the repository's filter"
        );
        assert_eq!(
            metrics
                .iter()
                .find(|b| b.name == "topic")
                .expect("topic")
                .merged,
            None
        );
    }
}
