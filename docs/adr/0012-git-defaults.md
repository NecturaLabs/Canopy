# ADR 0012: Canopy's git defaults are today's recommended practice, and pulls look before they leap

**Status:** accepted, amended 2026-10-10 (editable global and repository Git configuration), 2026-10-11 (one pull recommendation), 2026-10-14 (audit of every recommendation against Git's documentation) and 2026-10-21 (second audit: `submodule.recurse` recommended off; Edit my git config, off by default)
**Date:** 2026-10-06

## Context

AGENTS.md makes Canopy the authority on what it does to a repository: every action runs with the
option chosen in Canopy, passed to git explicitly. Until now those choices started from the user's
git configuration and otherwise from git's own defaults, which are conservative for backward
compatibility rather than what git's developers and tool makers now recommend. Several of Canopy's
commands also still let configuration change their outcome: a fetch with `fetch.pruneTags=true`
deleted local tags that were never pushed, a commit dropped message lines starting with `#`, a
bare push followed `push.default`, and `git init` followed `init.defaultBranch`. A pull that hit
conflicts surprised the user half-way, and a stash put back after a pull could conflict with
nothing said. The owner asked for the best current practice as Canopy's defaults, for pull
settings in Settings only (no toolbar menu), and for a pull that says beforehand when it would
conflict. On review, the owner added: "respect existing git settings, dont force things on them,
as long as we show them the possible options along with our recommendations in settings".

Research (git-config and git-fetch/pull docs, git release notes 2.17 to 2.49, the GitButler survey
of core git developers' settings, Julia Evans' survey of popular options) is summarised in the
change that introduced this ADR.

## Decision

Follow my git config (Settings → Git, on by default): where the user's git configuration sets
one of the choices below, it decides, and Canopy passes nothing for it (for pruning, Canopy's
choice stands in for an unset `fetch.prune` / `fetch.pruneTags`, so each remote's own setting still
decides for that remote). Where the configuration sets nothing, or the switch is off, Canopy passes
its own choice explicitly:

| Choice | Canopy's default | Configuration that decides while followed | Passed as |
|---|---|---|---|
| Pull strategy | Fast-forward only; when the branches diverged Canopy asks Rebase (recommended) or Merge (optionally remembered for the repository or, where Canopy's setting asked for fast-forwards, every repository) | `branch.<name>.rebase`, `pull.rebase`, `pull.ff` | strategy flags |
| Prune on fetch | on | `remote.<name>.prune`, else `fetch.prune` | `-c fetch.prune=…` while following, else `--prune` / `--no-prune` |
| Prune tags too | off | `remote.<name>.pruneTags`, else `fetch.pruneTags` | `-c fetch.pruneTags=…` while following, else `--prune-tags` / `--no-prune-tags` |
| Autostash for pull, rebase and merge | on | `pull.autoStash` for pulls when set (git 2.51 and later), else `rebase.autoStash` / `merge.autoStash`; direct rebase/merge use their own key | `--autostash` / `--no-autostash` (merges: git 2.27 and later; a squash merge always `--no-autostash`, because git leaves a squash's stash unapplied in `.git/MERGE_AUTOSTASH`, out of `git stash list`) |
| Push annotated tags with their commits | on | `push.followTags` | `--follow-tags` / `--no-follow-tags` |
| Update submodules after a pull or checkout, and in a clone | on | `submodule.recurse` (pull and checkout only: git ignores it for `clone`) | git's pull, switch and rebase always run with `-c submodule.recurse=false`; afterwards `git submodule update --recursive` when Canopy's choice or a followed `submodule.recurse = true` says so (since 2026-10-21: a recursive switch drops edits inside a submodule, the update keeps them); `clone --recurse-submodules` |
| Push to | the branch's upstream, when the names match | `push.default` (a force push follows only `simple`, `upstream` or `current`, so it never sends other branches) | `-c push.default=simple` (or upstream, current) |
| Conflict style | both sides and the original, trimmed | `merge.conflictStyle` | `-c merge.conflictStyle=zdiff3` (`diff3` before git 2.35) |
| First branch of a new repository | `main` | `init.defaultBranch` | native `git init` when following configuration; `symbolic-ref` sets Canopy's fallback only for a newly initialized, unconfigured repository |
| Message cleanup | trailing spaces and surrounding blank lines only: the commit box is never an editor, so this is what git's own `default` does for it | `commit.cleanup` | `--cleanup=whitespace` (or strip, verbatim) |

`rerere`, `rebase.updateRefs` and `diff.algorithm` stay off or unused: the first two change results
without the UI saying so yet, and Canopy's diffs come from libgit2.

A pull first fetches its upstream and compares the branch with it in memory
(`Repo::pull_preview`: `git merge-tree --write-tree` where both moved on, and the incoming files
against uncommitted ones). When the branches diverged under fast-forward only, or the pull would
likely conflict, it asks first, naming the files. It then brings in the upstream as fetched
(`Repo::pull_fetched`: `git merge @{upstream}` or `git rebase`), without asking the remote again.
A pull or rebase that still stops on conflicts, or a pull, rebase or merge whose stashed changes
conflict when they come back, takes the user to Changes with a plain message.

Settings shows every choice with Canopy's recommendation; where the user's configuration sets
it, the note names the setting, its value and where it is set. Canopy writes no git
configuration unless Edit my git config (Settings → Git) is on; it is off by default (the owner's
decision, 2026-10-21), as Fork, Sourcetree and GitKraken write none either, and every choice is
passed to the git command that needs it, so nothing depends on a written value. Off, both editors
show the values read only; a settings file cannot turn it on. On, supported values are editable:
Settings → Git writes the main global file, This repository writes the common local file.
Unset/Inherit removes only that file's values. Included, system, worktree and command values
remain in force according to Git's precedence. Global Settings shows global values; the
repository page shows the effective configuration across scopes.

Every supported setting carries a recommendation (the owner's decision, 2026-10-13), and a row
the configuration leaves unset shows the recommended value, or Git's built-in one for line endings
and signing, which Canopy leaves to Git; never a bare "Unset". The pull recommendation is fast-forward only (`pull.ff = only`) with
`pull.rebase = false` beside it (what each does is under "Audit of the recommendations"); when
branches diverge Canopy still asks Rebase (recommended) or Merge. Line endings follow
[GitHub's guidance](https://docs.github.com/en/get-started/git-basics/configuring-git-to-handle-line-endings)
(`core.autocrlf = true` on Windows, `input` elsewhere); signing is recommended off until the user
sets up a key, then with an SSH key (`gpg.format = ssh`), the simplest format
[GitHub verifies](https://docs.github.com/en/authentication/managing-commit-signature-verification/about-commit-signature-verification).

Use recommended (shown only while Edit my git config is on) confirms and writes the
non-sensitive recommendations globally on the Git worker. It leaves identity, signing, hooks and
line endings alone, and it writes a value only when that value is safe for git used outside
Canopy too: the configuration it writes reaches the terminal and every other tool, where
Canopy's own safeguards do not run (2026-10-21, after `submodule.recurse`). A failed batch reports saved
progress; Settings rereads actual state. Canopy fallback controls remain available when Follow
my git config is off. Existing repository preferences remain compatible.

The recommendations are product choices, not universal Git requirements; the audit below gives
each one's reason and source. Autostash, follow-tags, recursive updates and `main` remain explicit
Canopy defaults; projects may prefer other values.

## Audit of the recommendations (2026-10-14)

Issue #233. Every recommendation was re-derived from Git's own documentation and release notes
only (blog posts and surveys, which the first version of this ADR also read, are not a source),
checked on 2026-10-14 against the installed Git 2.55.0 and the git-scm.com documentation for
2.56.0 (released 2026-09-28). "Git's default" is what git does with the key unset. Each row names
the tests that run the claim against the git executable (`crates/canopy-core/tests/git_recommendations.rs`;
tests below are named without their long suffixes). `recommendation_for` (`crates/canopy-core/src/config.rs`),
`Settings::default` and this table agree for every key; a test fails when one changes alone.

| Key | Recommended | Git's default | Why | Sources |
|---|---|---|---|---|
| `pull.rebase` | `false` | unset: a diverged pull stops with "Need to specify how to reconcile divergent branches" (git 2.55) | Merge is the neutral value beside `pull.ff = only`. Since git 2.34, `pull.ff = only` wins over `pull.rebase = true` and `branch.<name>.rebase` (only `--rebase` on the command line overrides it); before 2.34 the rebase won, so `false` keeps older git from rebasing behind the user's back. This replaces the first version's claim that `false` is what lets `only` take effect on current git. | [git-config pull.rebase](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pullrebase), [2.34 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.34.0.adoc) ("git pull --ff-only did not stop but went ahead and rebased"); test `pulls_fast_forward_or_stop` |
| `pull.ff` | `only` | unset, which behaves as `only` when branches diverged | Never a surprise merge commit or rewrite; Canopy then asks Rebase (recommended) or Merge. Written, it pins git's documented default against a later `pull.rebase`. | [git-pull](https://git-scm.com/docs/git-pull) ("`--ff-only` ... is the default when no method for reconciling divergent histories is provided"), [git-config pull.ff](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pullff); `pulls_fast_forward_or_stop` |
| `pull.autoStash` | `true` | unset: `rebase.autoStash` or `merge.autoStash` decide | A pull over local changes goes ahead and puts them back. Set to either value it overrides the other two keys for pulls. Only git 2.51 and later read it; older git, and Canopy there, use the next two rows, which are written too and give the same result. | [git-config pull.autoStash](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pullautoStash), [2.51 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.51.0.adoc); `the_three_autostash_keys` |
| `rebase.autoStash` | `true` | `false` | A rebase runs over local changes. Git's caution: "the final stash application after a successful rebase might result in non-trivial conflicts"; Canopy reports a stash that conflicts when it comes back. A product choice (owner, 2026-10-13). | [git-config rebase.autoStash](https://git-scm.com/docs/git-config#Documentation/git-config.txt-rebaseautoStash); `the_three_autostash_keys` |
| `merge.autoStash` | `true` | `false` | The same for a merge (`git merge --autostash`, git 2.27). Canopy's Merge now passes it too (#249, found by #110): `canopys_merge_stashes`. Not for a squash, which git leaves with its stash unapplied and hidden even when this key is set: `a_squash_never_hides_your_changes`. | [git-config merge.autoStash](https://git-scm.com/docs/git-config#Documentation/git-config.txt-mergeautoStash), [2.27 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.27.0.adoc); `the_three_autostash_keys`, `canopys_merge_stashes`, `a_squash_never_hides_your_changes` |
| `fetch.prune` | `true` | `false` | Removes the remote-tracking branches whose branch was deleted on the remote, never a local branch. git-fetch: "To prune references as part of your normal workflow without needing to remember to run that, set fetch.prune globally". | [git-fetch, Pruning](https://git-scm.com/docs/git-fetch#_pruning); `fetch_prunes_stale_branches` |
| `fetch.pruneTags` | `false` | `false` | Written so a later `true` stands out. `--prune-tags` "will remove any local references (local tags) that have been created": a tag never pushed disappears with the next fetch (shown by the test). | [git-fetch `--prune-tags`](https://git-scm.com/docs/git-fetch#Documentation/git-fetch.txt--P), [Pruning](https://git-scm.com/docs/git-fetch#_pruning); `fetch_prunes_stale_branches` |
| `push.followTags` | `true` | `false` | A push also sends the annotated tags (never lightweight ones) that point at the commits it sends and are missing from the remote. The cost: a private annotated tag on a pushed commit is published. A product choice. | [git-config push.followTags](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pushfollowTags), [git-push `--follow-tags`](https://git-scm.com/docs/git-push#Documentation/git-push.txt---follow-tags); `a_plain_push_sends_annotated_tags` |
| `submodule.recurse` | `false` | `false` | Changed by the second audit from `true`. Canopy's Update submodules (on) brings submodules along with `git submodule update`, which keeps uncommitted edits inside them. Git's own recursion does not: with the key on, `git switch` and `git checkout` drop an edit inside a submodule to a file both submodule commits share without a word (git 2.56.0, shown by the tests; git-checkout says the checkout "will fail" where local modifications would be overwritten), `reset` and `restore` reset submodule working trees, and a push also pushes the submodule commits it needs and is aborted where it cannot (#250). Use recommended removes a `true` from the main global file instead of writing `false`, because a followed `false` decides that Update submodules does nothing. Fork, Sourcetree and GitKraken likewise keep submodule updates as their own preference and write no Git configuration. | [git-config submodule.recurse](https://git-scm.com/docs/git-config#Documentation/git-config.txt-submodulerecurse), [git-checkout `--recurse-submodules`](https://git-scm.com/docs/git-checkout), [push.recurseSubmodules](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pushrecurseSubmodules); `git_recursing_into_submodules_drops_edits`, workflow test `a_checkout_keeps_uncommitted_edits_inside_a_submodule` |
| `push.default` | `simple` | `simple` (since git 2.0) | A bare push sends the branch to its upstream only when the names match, and refuses otherwise (the test shows the refusal and `upstream` sending it). Written, it holds against a later `matching`. | [git-config push.default](https://git-scm.com/docs/git-config#Documentation/git-config.txt-pushdefault); `simple_pushes_only` |
| `merge.conflictStyle` | `zdiff3` (`diff3` before git 2.35) | `merge` | Shows the original text between the sides, and leaves out the lines both sides share, which `diff3` keeps inside the conflict. git older than 2.35 reports `zdiff3` as an unknown style, so Canopy writes `diff3` there. | [git-config merge.conflictStyle](https://git-scm.com/docs/git-config#Documentation/git-config.txt-mergeconflictStyle), [2.35 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.35.0.adoc) ("Zealous diff3"); `conflicts_show_the_original` |
| `init.defaultBranch` | `main` | `master`; git 3.0 makes it `main` | The branch new repositories start on; git is moving to the same name. | [git-config init.defaultBranch](https://git-scm.com/docs/git-config#Documentation/git-config.txt-initdefaultBranch), [BreakingChanges, Git 3.0](https://github.com/git/git/blob/master/Documentation/BreakingChanges.adoc); `new_repositories_start_on_main` |
| `commit.cleanup` | `default` | `default` | Corrected by this audit from `whitespace`. `whitespace` keeps every `#` line, git's own template included: written globally, a `git commit` in a terminal commits "Please enter the commit message..." and the status lines, as does `--amend` and a squash in `rebase -i` (shown by the test); git-config warns "you will have to remove the help lines that begin with the comment character in the commit log template yourself, if you do this". `scissors` drops the template of a commit but still leaks a squash's. `default` is `whitespace` for a message that is not edited, which is every commit Canopy makes (`-F`), so Canopy's own choice stays Whitespace and keeps a typed `#` line, and strip, which drops `#` lines from a message written in an editor, for one that is. | [git-commit `--cleanup`](https://git-scm.com/docs/git-commit#Documentation/git-commit.txt---cleanupltmodegt), [git-config commit.cleanup](https://git-scm.com/docs/git-config#Documentation/git-config.txt-commitcleanup); `the_recommended_commit_cleanup` |
| `core.autocrlf` | `input` (`true` on Windows) | unset: no conversion | Not written, see below. GitHub's guidance for Windows is `true`, for macOS and Linux `input`, and a `.gitattributes` file in the repository overrides it for every contributor. | [git-config core.autocrlf](https://git-scm.com/docs/git-config#Documentation/git-config.txt-coreautocrlf), [GitHub: Configuring Git to handle line endings](https://docs.github.com/en/get-started/git-basics/configuring-git-to-handle-line-endings); `changing_line_endings_is_not_neutral` |
| `commit.gpgsign` | `false` | `false` | Not written, see below. Signing stays off until the user has a key. git-config: use of it "when doing operations such as rebase can result in a large number of commits being signed". | [git-config commit.gpgSign](https://git-scm.com/docs/git-config#Documentation/git-config.txt-commitgpgSign) |
| `gpg.format` | `ssh` (git 2.34 and later, else `openpgp`) | `openpgp` | Not written, see below. The simplest format GitHub verifies; "SSH signature verification is available in Git 2.34 or later". | [git-config gpg.format](https://git-scm.com/docs/git-config#Documentation/git-config.txt-gpgformat), [2.34 release notes](https://github.com/git/git/blob/master/Documentation/RelNotes/2.34.0.adoc), [GitHub: About commit signature verification](https://docs.github.com/en/authentication/managing-commit-signature-verification/about-commit-signature-verification) |

**Why line endings and signing are recommended but not written.** `core.autocrlf` changes how
every existing repository's files are read: a file committed with CRLF is stored with LF the next
time git normalises it (`git add --renormalize`, and every touch), turning the file into a
whole-file change in someone else's history; the right value depends on the platform and on the
project's own `.gitattributes`, which overrides it. `commit.gpgsign = true` needs a key Canopy
cannot create, and `gpg.format = ssh` needs `user.signingKey` and a user who is not already
signing with a GnuPG key under git's default `openpgp`: writing it would break that user's
signing. `commit.gpgsign = false` is git's own default, so writing it adds nothing. They stay
shown, with their recommendation and where they are set, and the user writes them. The owner's
decision of 2026-10-09 keeps `commit.gpgsign` and `core.hooksPath` the user's.

**Corrections the audit made.** `commit.cleanup` from `whitespace` to `default` (above); the
claim that `pull.rebase = false` lets `pull.ff = only` take effect (above); `gpg.format` is
`openpgp` on git older than 2.34, where `ssh` is not a valid value; Canopy reads `pull.autoStash`
only on git 2.51 and later, as git does; and Canopy's Merge passes Canopy's Autostash choice, as
this ADR's first table already said (#249). A machine that already holds `commit.cleanup =
whitespace` from an earlier Use recommended is reported by the notification center, and Use
recommended replaces it.

## Second audit (2026-10-21)

Every recommendation was checked again against the git-scm.com documentation, Git's release notes
up to 2.56.0 (the latest release) and `Documentation/BreakingChanges.adoc`. Git 3.0 has not shipped
and has no release date ("There is no planned release date for this breaking version yet"); its
planned defaults (`main`, SHA-256, reftable) change none of these keys. Every value stayed except
`submodule.recurse` (row above). Keys that no Canopy action depends on are not added
(`push.autoSetupRemote`, `rebase.autoSquash`, `rerere.enabled`, `diff.algorithm`): Canopy passes
`--set-upstream` itself, has no interactive rebase yet and diffs with libgit2.

## Consequences

- A user's git configuration keeps deciding what it sets; Canopy's recommendations fill in the
  rest, and Settings shows both. Turning Follow my git config off makes every outcome Canopy's.
- A fast-forward-only default means diverged branches lead to a decision, never to a surprise
  merge commit or rewrite, unless the user's configuration chose a strategy.
- A pull fetches once (the preview's fetch), so a credential prompt comes once too. Its merge
  commit says "Merge remote-tracking branch 'origin/main'", where `git pull` names the URL.
- `merge-tree --write-tree` needs git 2.38; with an older git the preview names only the files the
  incoming commits share with uncommitted changes. For a rebase the prediction merges the two
  tips: a rebase replays commit by commit and can still conflict in a commit between them.
