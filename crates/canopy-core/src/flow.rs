//! git-flow without the `git flow` program.
//!
//! The AVH configuration keys (`gitflow.branch.*`, `gitflow.prefix.*`) and the start and finish
//! steps for feature, bugfix, release and hotfix branches, run as plain `git` commands.
//!
//! Finishing checks every merge it will make with `git merge-tree --write-tree` (Git 2.38 or
//! later) before it changes anything, and refuses when one would conflict, so a finish never stops
//! half-way on a conflict the way the `git flow` scripts do.

use git2::{BranchType, Oid, RepositoryState};

use crate::cli;
use crate::{Error, Repo, Result};

/// The `gitflow.*` entries Canopy writes: five branch names, three prefixes.
const ENTRY_COUNT: usize = 8;
/// A feature or bugfix with exactly one commit beyond its base may finish by fast-forwarding.
const SINGLE_COMMIT: usize = 1;

/// A kind of git-flow branch. `bugfix` behaves like `feature`; `support` is not offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FlowKind {
    /// A feature branch, started from and finished into the development branch.
    Feature,
    /// A bugfix branch: a feature branch under its own prefix.
    Bugfix,
    /// A release branch, started from the development branch and finished into both branches
    /// with a tag.
    Release,
    /// A hotfix branch, started from the production branch and finished like a release.
    Hotfix,
}

impl FlowKind {
    /// Every kind, in menu order.
    pub const ALL: [Self; 4] = [Self::Feature, Self::Bugfix, Self::Release, Self::Hotfix];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Feature => "Feature",
            Self::Bugfix => "Bugfix",
            Self::Release => "Release",
            Self::Hotfix => "Hotfix",
        }
    }

    /// The word used in the configuration key `gitflow.prefix.<word>`.
    fn key(self) -> &'static str {
        match self {
            Self::Feature => "feature",
            Self::Bugfix => "bugfix",
            Self::Release => "release",
            Self::Hotfix => "hotfix",
        }
    }

    /// Release and hotfix finishes merge into production, tag and merge back.
    fn is_release_like(self) -> bool {
        matches!(self, Self::Release | Self::Hotfix)
    }
}

/// The git-flow configuration, as the AVH `git flow init` stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowConfig {
    /// Production branch (`gitflow.branch.master`).
    pub master: String,
    /// Development branch (`gitflow.branch.develop`).
    pub develop: String,
    /// `gitflow.prefix.feature`.
    pub feature: String,
    /// `gitflow.prefix.bugfix`.
    pub bugfix: String,
    /// `gitflow.prefix.release`.
    pub release: String,
    /// `gitflow.prefix.hotfix`.
    pub hotfix: String,
    /// `gitflow.prefix.support`; kept so writing the configuration does not lose it.
    pub support: String,
    /// `gitflow.prefix.versiontag`, put in front of release and hotfix names to make the tag.
    pub version_tag: String,
}

impl Default for FlowConfig {
    /// The AVH defaults.
    fn default() -> Self {
        Self {
            master: "master".into(),
            develop: "develop".into(),
            feature: "feature/".into(),
            bugfix: "bugfix/".into(),
            release: "release/".into(),
            hotfix: "hotfix/".into(),
            support: "support/".into(),
            version_tag: String::new(),
        }
    }
}

impl FlowConfig {
    /// The prefix for branches of `kind`.
    pub fn prefix(&self, kind: FlowKind) -> &str {
        match kind {
            FlowKind::Feature => &self.feature,
            FlowKind::Bugfix => &self.bugfix,
            FlowKind::Release => &self.release,
            FlowKind::Hotfix => &self.hotfix,
        }
    }

    /// The branch for `name` of `kind`, e.g. `feature/login`.
    pub fn branch(&self, kind: FlowKind, name: &str) -> String {
        format!("{}{name}", self.prefix(kind))
    }

    /// The tag a release or hotfix `name` gets, e.g. `v1.2.0`.
    pub fn tag(&self, name: &str) -> String {
        format!("{}{name}", self.version_tag)
    }

    /// Where a branch of `kind` starts when no base is given.
    pub fn default_base(&self, kind: FlowKind) -> &str {
        match kind {
            FlowKind::Hotfix => &self.master,
            FlowKind::Feature | FlowKind::Bugfix | FlowKind::Release => &self.develop,
        }
    }

    /// The kind and name of `branch` when it carries one of the prefixes (the longest matching
    /// prefix wins), e.g. `feature/login` gives `(Feature, "login")`.
    pub fn classify<'a>(&self, branch: &'a str) -> Option<(FlowKind, &'a str)> {
        FlowKind::ALL
            .into_iter()
            .filter(|k| !self.prefix(*k).is_empty())
            .filter_map(|k| {
                branch
                    .strip_prefix(self.prefix(k))
                    .filter(|n| !n.is_empty())
                    .map(|n| (k, n))
            })
            .min_by_key(|(_, n)| n.len())
    }

    fn entries(&self) -> [(&'static str, &str); ENTRY_COUNT] {
        [
            ("gitflow.branch.master", &self.master),
            ("gitflow.branch.develop", &self.develop),
            ("gitflow.prefix.feature", &self.feature),
            ("gitflow.prefix.bugfix", &self.bugfix),
            ("gitflow.prefix.release", &self.release),
            ("gitflow.prefix.hotfix", &self.hotfix),
            ("gitflow.prefix.support", &self.support),
            ("gitflow.prefix.versiontag", &self.version_tag),
        ]
    }
}

/// What finishing a branch does besides merging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowFinish {
    /// Delete the local branch afterwards (`git branch -d`, or `-D` after a squash merge).
    pub delete_branch: bool,
    /// Feature and bugfix only: squash the branch into one commit on the development branch
    /// (`git merge --squash`) instead of a merge commit.
    pub squash: bool,
    /// Release and hotfix only: tag the merge into production. Without a tag, the production
    /// branch is merged back instead of the tag.
    pub tag: bool,
    /// The annotated tag's message; the tag name when `None` or blank.
    pub tag_message: Option<String>,
}

impl Default for FlowFinish {
    fn default() -> Self {
        Self {
            delete_branch: true,
            squash: false,
            tag: true,
            tag_message: None,
        }
    }
}

/// The outcome of `git merge-tree --write-tree`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MergeTree {
    /// The merge is clean and gives this tree.
    Clean(Oid),
    /// The merge would conflict in these paths.
    Conflicts(Vec<String>),
}

/// Merges `ours` and `theirs` (commits or full refs) in memory with `git merge-tree
/// --write-tree`, which writes the result's objects but touches no ref, index or file. Needs Git
/// 2.38 or later.
pub(crate) fn merge_tree(repo: &Repo, ours: &str, theirs: &str) -> Result<MergeTree> {
    let args = [
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "-z",
        ours,
        theirs,
    ];
    let out = cli::run_unchecked(repo.workdir(), &args)?;
    let mut fields = out.stdout.split(|b| *b == 0);
    let tree = fields
        .next()
        .and_then(|t| std::str::from_utf8(t).ok())
        .and_then(|t| Oid::from_str(t.trim()).ok());
    match (out.code, tree) {
        (Some(0), Some(tree)) => Ok(MergeTree::Clean(tree)),
        (Some(1), Some(_)) => {
            let mut paths: Vec<String> = fields
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect();
            paths.dedup();
            Ok(MergeTree::Conflicts(paths))
        }
        _ => Err(out.error(&args)),
    }
}

fn heads(branch: &str) -> String {
    format!("refs/heads/{branch}")
}

impl Repo {
    /// The git-flow configuration, or `None` when git-flow is not initialised (either branch key
    /// is unset). Unset prefixes take the AVH defaults.
    pub fn flow_config(&self) -> Result<Option<FlowConfig>> {
        let config = self.git().config()?;
        let get = |key: &str| config.get_string(key).ok();
        let (Some(master), Some(develop)) =
            (get("gitflow.branch.master"), get("gitflow.branch.develop"))
        else {
            return Ok(None);
        };
        let defaults = FlowConfig::default();
        let prefix = |kind: &str, default: &str| {
            get(&format!("gitflow.prefix.{kind}")).unwrap_or_else(|| default.to_owned())
        };
        Ok(Some(FlowConfig {
            master,
            develop,
            feature: prefix("feature", &defaults.feature),
            bugfix: prefix("bugfix", &defaults.bugfix),
            release: prefix("release", &defaults.release),
            hotfix: prefix("hotfix", &defaults.hotfix),
            support: prefix("support", &defaults.support),
            version_tag: prefix("versiontag", &defaults.version_tag),
        }))
    }

    /// The configuration an init dialog should offer: the stored one when git-flow is
    /// initialised, otherwise the AVH defaults with the production branch set to whichever of
    /// `main` and `master` exists (`main` when both do).
    pub fn flow_suggested_config(&self) -> Result<FlowConfig> {
        if let Some(config) = self.flow_config()? {
            return Ok(config);
        }
        let mut config = FlowConfig::default();
        if self.local_branch_tip("main").is_some() || self.local_branch_tip("master").is_none() {
            "main".clone_into(&mut config.master);
        }
        Ok(config)
    }

    /// Initialises git-flow: checks `config`, writes every key to the repository's own
    /// configuration and creates the development branch from the production branch when it does
    /// not exist. The production branch must exist (so the repository has a commit).
    pub fn flow_init(&self, config: &FlowConfig) -> Result<()> {
        for branch in [&config.master, &config.develop] {
            if !self.is_valid_branch_name(branch) {
                return Err(Error::invalid(format!(
                    "'{branch}' is not a valid branch name."
                )));
            }
        }
        if config.master == config.develop {
            return Err(Error::invalid(
                "The production and development branches must differ.",
            ));
        }
        for kind in FlowKind::ALL {
            let sample = config.branch(kind, "name");
            if !self.is_valid_branch_name(&sample) {
                return Err(Error::invalid(format!(
                    "The {} prefix '{}' does not make valid branch names.",
                    kind.key(),
                    config.prefix(kind)
                )));
            }
        }
        let tag = config.tag("1.0");
        if config.version_tag.starts_with('-')
            || self
                .git_cmd(&["check-ref-format", &format!("refs/tags/{tag}")])
                .is_err()
        {
            return Err(Error::invalid(format!(
                "The version tag prefix '{}' does not make valid tag names.",
                config.version_tag
            )));
        }
        if self.local_branch_tip(&config.master).is_none() {
            return Err(Error::invalid(format!(
                "The production branch '{}' does not exist. git-flow needs at least one commit \
                 on it.",
                config.master
            )));
        }

        for (key, value) in config.entries() {
            self.git_cmd(&["config", "--local", key, value])?;
        }
        if self.local_branch_tip(&config.develop).is_none() {
            self.git_cmd(&[
                "branch",
                "--no-track",
                &config.develop,
                &heads(&config.master),
            ])?;
        }
        Ok(())
    }

    /// Starts a branch of `kind` called `name` (without its prefix) from `base`, or from the
    /// kind's default base (production for hotfixes, development otherwise), and switches to it:
    /// `git switch --no-track -c <prefix><name> <base>`. Returns the new branch's name.
    ///
    /// Like `git flow`, it refuses when the branch exists or the base does not; for a release or
    /// hotfix also when the working tree has changes, another branch of that kind exists, or the
    /// version tag exists already.
    pub fn flow_start(&self, kind: FlowKind, name: &str, base: Option<&str>) -> Result<String> {
        let config = self.require_flow()?;
        let branch = config.branch(kind, name);
        if name.is_empty() || !self.is_valid_branch_name(&branch) {
            return Err(Error::invalid(format!(
                "'{branch}' is not a valid branch name."
            )));
        }
        if self.local_branch_tip(&branch).is_some() {
            return Err(Error::invalid(format!("Branch '{branch}' already exists.")));
        }
        let base = base.unwrap_or(config.default_base(kind));
        if base.starts_with('-') || self.git().revparse_single(base).is_err() {
            return Err(Error::invalid(format!("The base '{base}' does not exist.")));
        }
        if kind.is_release_like() {
            self.require_clean_tree()?;
            if let Some(other) = self.flow_branches(&config, kind)?.into_iter().next() {
                return Err(Error::invalid(format!(
                    "Finish '{other}' first: git-flow allows one {} branch at a time.",
                    kind.key()
                )));
            }
            let tag = config.tag(name);
            self.require_valid_tag(&tag)?;
            if self.tag_exists(&tag) {
                return Err(Error::invalid(format!("Tag '{tag}' already exists.")));
            }
        }
        self.switch_cmd(&["-q", "--no-track", "-c", &branch, base])?;
        Ok(branch)
    }

    /// Finishes the branch of `kind` called `name` (without its prefix) and leaves the
    /// development branch checked out.
    ///
    /// Feature and bugfix: `git switch <develop>`, then `git merge --ff <branch>` for exactly one
    /// commit beyond develop, otherwise `--no-ff` (or `--squash` and a commit with git's prepared
    /// message), then `git branch -d` (`-D` after a squash).
    ///
    /// Release and hotfix: `git switch <master>`, `git merge --no-ff <branch>`,
    /// `git tag -a <versiontag><name>`, `git switch <develop>`, `git merge --no-ff <tag>` (or
    /// `<master>` without a tag), then `git branch -d`.
    ///
    /// Before changing anything it requires a clean working tree and no operation in progress,
    /// and checks every merge with `git merge-tree`; a merge that would conflict refuses the
    /// finish with the paths, so the user can merge the target into the branch first. Needs Git
    /// 2.38 or later. Returns a summary for a notification.
    pub fn flow_finish(&self, kind: FlowKind, name: &str, options: &FlowFinish) -> Result<String> {
        if !cli::has_merge_tree_write_tree() {
            return Err(Error::invalid(
                "Finishing a git-flow branch needs Git 2.38 or later.",
            ));
        }
        let config = self.require_flow()?;
        let branch = config.branch(kind, name);
        let Some(tip) = self.local_branch_tip(&branch) else {
            return Err(Error::invalid(format!("Branch '{branch}' does not exist.")));
        };
        for target in [&config.master, &config.develop] {
            if self.local_branch_tip(target).is_none() {
                return Err(Error::invalid(format!("Branch '{target}' does not exist.")));
            }
        }
        self.require_clean_tree()?;

        if !kind.is_release_like() {
            self.preflight(&config.develop, &branch)?;
            let fast_forward = if options.squash {
                false
            } else {
                let base = self.git().refname_to_id(&heads(&config.develop))?;
                self.git().graph_ahead_behind(tip, base)?.0 == SINGLE_COMMIT
            };
            self.switch_cmd(&["-q", &config.develop])?;
            if options.squash {
                // `--ff` so a `merge.ff=only` setting cannot refuse a diverged squash.
                self.git_cmd(&["merge", "--squash", "--ff", "-q", &branch])?;
                // Nothing to commit when develop already has every change.
                if self.has_staged_changes()? {
                    self.git_cmd(&["commit", "-q", "--no-edit"])?;
                }
            } else {
                let mode = if fast_forward { "--ff" } else { "--no-ff" };
                self.git_cmd(&["merge", mode, "--no-edit", "-q", &branch])?;
            }
            if options.delete_branch {
                let flag = if options.squash { "-D" } else { "-d" };
                self.git_cmd(&["branch", flag, &branch])?;
            }
            return Ok(format!("Merged {branch} into {}", config.develop));
        }

        let tag = config.tag(name);
        if options.tag {
            self.require_valid_tag(&tag)?;
            if self.tag_exists(&tag) {
                return Err(Error::invalid(format!("Tag '{tag}' already exists.")));
            }
        }
        let master_tip = self.local_branch_tip(&config.master).unwrap_or(tip);
        let into_master = self.preflight(&config.master, &branch)?;
        // The back-merge takes the commit the first merge will create. Build a stand-in with the
        // same tree and parents (an unreferenced object, never checked out) to check it.
        let master_tree = into_master.to_string();
        let master_parent = master_tip.to_string();
        let branch_parent = tip.to_string();
        let stand_in = self.git_cmd(&[
            "commit-tree",
            "--no-gpg-sign",
            "-p",
            &master_parent,
            "-p",
            &branch_parent,
            "-m",
            "canopy: git-flow pre-flight",
            &master_tree,
        ])?;
        if let MergeTree::Conflicts(paths) =
            merge_tree(self, &heads(&config.develop), stand_in.trim())?
        {
            return Err(Error::invalid(format!(
                "Merging the finished {branch} back into {} would conflict in {}. Nothing was \
                 changed; resolve the difference between {} and {branch} first.",
                config.develop,
                paths.join(", "),
                config.develop
            )));
        }

        self.switch_cmd(&["-q", &config.master])?;
        self.git_cmd(&["merge", "--no-ff", "--no-edit", "-q", &branch])?;
        let back = if options.tag {
            let message = options
                .tag_message
                .as_deref()
                .filter(|m| !m.trim().is_empty())
                .unwrap_or(&tag);
            self.git_cmd(&["tag", "-a", &tag, "-m", message, &heads(&config.master)])?;
            tag.clone()
        } else {
            config.master.clone()
        };
        self.switch_cmd(&["-q", &config.develop])?;
        self.git_cmd(&["merge", "--no-ff", "--no-edit", "-q", &back])?;
        if options.delete_branch {
            self.git_cmd(&["branch", "-d", &branch])?;
        }
        Ok(if options.tag {
            format!(
                "Merged {branch} into {} and {}, tagged {tag}",
                config.master, config.develop
            )
        } else {
            format!(
                "Merged {branch} into {} and {}",
                config.master, config.develop
            )
        })
    }

    /// Pushes what finishing `name` of `kind` changed to `remote`: the development branch, and for
    /// a release or hotfix also the production branch and the version tag (when it exists). Only
    /// those refs are pushed, never `--tags`. `delete_remote` also deletes the finished branch on
    /// the remote; callers confirm that first.
    pub fn flow_push(
        &self,
        kind: FlowKind,
        name: &str,
        remote: &str,
        delete_remote: bool,
        progress: impl FnMut(&str),
    ) -> Result<String> {
        if remote.is_empty() || remote.starts_with('-') || self.git().find_remote(remote).is_err() {
            return Err(Error::invalid(format!("'{remote}' is not a remote.")));
        }
        let config = self.require_flow()?;
        let mut refspecs = vec![heads(&config.develop)];
        if kind.is_release_like() {
            refspecs.push(heads(&config.master));
            let tag = config.tag(name);
            if self.tag_exists(&tag) {
                refspecs.push(format!("refs/tags/{tag}"));
            }
        }
        if delete_remote {
            refspecs.push(format!(":{}", heads(&config.branch(kind, name))));
        }
        let mut args = vec!["push", "--progress"];
        args.extend(self.push_submodules_flag());
        args.extend(["--", remote]);
        args.extend(refspecs.iter().map(String::as_str));
        cli::run_streaming(Some(self.workdir()), &args, progress)
    }

    /// Local branches of `kind`, by name.
    pub fn flow_branches(&self, config: &FlowConfig, kind: FlowKind) -> Result<Vec<String>> {
        let mut out = Vec::new();
        for item in self.git().branches(Some(BranchType::Local))? {
            let (branch, _) = item?;
            if let Some(name) = branch.name()?
                && config.classify(name).is_some_and(|(k, _)| k == kind)
            {
                out.push(name.to_owned());
            }
        }
        out.sort();
        Ok(out)
    }

    fn require_flow(&self) -> Result<FlowConfig> {
        self.flow_config()?
            .ok_or_else(|| Error::invalid("git-flow is not initialised in this repository."))
    }

    /// Checks that merging `branch` into `target` is clean and returns the merged tree; on
    /// conflicts, refuses with the paths and a hint to merge `target` into `branch` first.
    fn preflight(&self, target: &str, branch: &str) -> Result<Oid> {
        match merge_tree(self, &heads(target), &heads(branch))? {
            MergeTree::Clean(tree) => Ok(tree),
            MergeTree::Conflicts(paths) => Err(Error::invalid(format!(
                "Merging {branch} into {target} would conflict in {}. Nothing was changed; \
                     merge {target} into {branch} and resolve the conflicts first.",
                paths.join(", ")
            ))),
        }
    }

    /// Refuses unless no operation is in progress and tracked files have no staged or unstaged
    /// changes (submodules and untracked files do not count), as `git flow` checks.
    fn require_clean_tree(&self) -> Result<()> {
        if self.git().state() != RepositoryState::Clean {
            return Err(Error::invalid(
                "Finish or abort the operation in progress first.",
            ));
        }
        // Refresh stat information so a touched but unchanged file does not count.
        cli::run_unchecked(
            self.workdir(),
            &["update-index", "-q", "--ignore-submodules", "--refresh"],
        )?;
        let unstaged = ["diff-files", "--quiet", "--ignore-submodules", "--"];
        let out = cli::run_unchecked(self.workdir(), &unstaged)?;
        match out.code {
            Some(0) => {}
            Some(1) => {
                return Err(Error::invalid(
                    "The working tree has changes. Commit or stash them first.",
                ));
            }
            _ => return Err(out.error(&unstaged)),
        }
        if self.has_staged_changes()? {
            return Err(Error::invalid(
                "The index has staged changes. Commit or stash them first.",
            ));
        }
        Ok(())
    }

    fn has_staged_changes(&self) -> Result<bool> {
        let args = [
            "diff-index",
            "--cached",
            "--quiet",
            "--ignore-submodules",
            "HEAD",
            "--",
        ];
        let out = cli::run_unchecked(self.workdir(), &args)?;
        match out.code {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(out.error(&args)),
        }
    }

    pub(crate) fn local_branch_tip(&self, name: &str) -> Option<Oid> {
        self.git()
            .find_branch(name, BranchType::Local)
            .ok()
            .and_then(|b| b.get().target())
    }

    /// Refuses a tag name git would read as an option or reject.
    fn require_valid_tag(&self, tag: &str) -> Result<()> {
        let valid = !tag.starts_with('-')
            && self
                .git_cmd(&["check-ref-format", &format!("refs/tags/{tag}")])
                .is_ok();
        if valid {
            Ok(())
        } else {
            Err(Error::invalid(format!("'{tag}' is not a valid tag name.")))
        }
    }

    fn tag_exists(&self, tag: &str) -> bool {
        self.git()
            .find_reference(&format!("refs/tags/{tag}"))
            .is_ok()
    }

    /// Whether `name` is a branch name git accepts as it is (`check-ref-format --branch` also
    /// expands `@{-1}` and the like, so the answer must come back unchanged).
    pub(crate) fn is_valid_branch_name(&self, name: &str) -> bool {
        !name.is_empty()
            && !name.starts_with('-')
            && self
                .git_cmd(&["check-ref-format", "--branch", name])
                .is_ok_and(|out| out.trim_end_matches('\n') == name)
    }
}
