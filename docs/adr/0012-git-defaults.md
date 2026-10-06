# ADR 0012: Canopy's git defaults are today's recommended practice, and pulls look before they leap

**Status:** accepted, amended 2026-10-07 (the user's git configuration decides what it sets)
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
| Pull strategy | Fast-forward only; when the branches diverged Canopy asks Rebase or Merge (optionally remembered per repository) | `branch.<name>.rebase`, `pull.rebase`, `pull.ff` | strategy flags |
| Prune on fetch | on | `remote.<name>.prune`, else `fetch.prune` | `-c fetch.prune=…` while following, else `--prune` / `--no-prune` |
| Prune tags too | off | `remote.<name>.pruneTags`, else `fetch.pruneTags` | `-c fetch.pruneTags=…` while following, else `--prune-tags` / `--no-prune-tags` |
| Autostash for pull and rebase | on | `rebase.autoStash` (rebases), `merge.autoStash` (merges) | `--autostash` / `--no-autostash` (merges: git 2.27 and later) |
| Push annotated tags with their commits | on | `push.followTags` | `--follow-tags` / `--no-follow-tags` |
| Push to | the branch's upstream, when the names match | `push.default` (a force push follows only `simple`, `upstream` or `current`, so it never sends other branches) | `-c push.default=simple` (or upstream, current) |
| Conflict style | both sides and the original, trimmed | `merge.conflictStyle` | `-c merge.conflictStyle=zdiff3` (`diff3` before git 2.35) |
| First branch of a new repository | `main` | `init.defaultBranch` | `git init --initial-branch=<name>` (`symbolic-ref` before git 2.28) |
| Message cleanup | trailing spaces and surrounding blank lines only | `commit.cleanup` | `--cleanup=whitespace` (or strip, verbatim) |

`rerere`, `rebase.updateRefs` and `diff.algorithm` stay off or unused: the first two change results
without the UI saying so yet, and Canopy's diffs come from libgit2.

A pull first fetches its upstream and compares the branch with it in memory
(`Repo::pull_preview`: `git merge-tree --write-tree` where both moved on, and the incoming files
against uncommitted ones). When the branches diverged under fast-forward only, or the pull would
likely conflict, it asks first, naming the files. It then brings in the upstream as fetched
(`Repo::pull_fetched`: `git merge @{upstream}` or `git rebase`), without asking the remote again.
A pull or rebase that still stops on conflicts, or whose stashed changes conflict when they come
back, takes the user to Changes with a plain message.

Settings shows every choice with Canopy's recommendation; where the user's configuration sets
it, the note names the setting, its value and where it is set, and the control is unavailable
while that configuration decides.

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
