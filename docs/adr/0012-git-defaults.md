# ADR 0012: Canopy's git defaults are today's recommended practice, and pulls look before they leap

**Status:** accepted
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
conflict.

Research (git-config and git-fetch/pull docs, git release notes 2.17 to 2.49, the GitButler survey
of core git developers' settings, Julia Evans' survey of popular options) is summarised in the
change that introduced this ADR.

## Decision

Canopy's defaults (Settings → Git), each passed explicitly on every run:

| Choice | Default | Passed as |
|---|---|---|
| Pull strategy | Fast-forward only; when the branches diverged Canopy asks Rebase or Merge (optionally remembered per repository) | `git pull --ff-only` (strategy flags) |
| Follow my git config (pull) | on: a pull strategy the configuration sets still wins | the configured strategy, explicitly |
| Prune on fetch | on | `--prune` / `--no-prune` |
| Prune tags | never | `--no-prune-tags`, always |
| Autostash for pull and rebase | on | `--autostash` / `--no-autostash` |
| Push annotated tags with their commits | on | `--follow-tags` / `--no-follow-tags` |
| Push destination | the branch's same-named upstream | `-c push.default=simple` |
| Conflict style | both sides and the original, trimmed | `-c merge.conflictStyle=zdiff3` (`diff3` before git 2.35) |
| First branch of a new repository | `main` | `git init --initial-branch=<name>` |
| Commit message cleanup | trailing spaces and surrounding blank lines only | `--cleanup=whitespace` |

`rerere`, `rebase.updateRefs` and `diff.algorithm` stay off or unused: the first two change results
without the UI saying so yet, and Canopy's diffs come from libgit2.

A pull first fetches its upstream and compares the branch with it in memory
(`Repo::pull_preview`: `git merge-tree --write-tree` where both moved on, and the incoming files
against uncommitted ones). When the branches diverged under fast-forward only, or the pull would
likely conflict, it asks first, naming the files. A pull that still stops on conflicts, or whose
stashed changes conflict when they come back, takes the user to Changes with a plain message.

Settings shows, beside each choice, the git configuration value it overrides and where it is set.

## Consequences

- A user's git configuration no longer decides these outcomes; Settings says where it differs.
- A fast-forward-only default means diverged branches always lead to a decision, never to a
  surprise merge commit or rewrite.
- A pull fetches twice when it goes ahead (once to look, once in `git pull`); the second fetch
  transfers nothing new.
- `merge-tree --write-tree` needs git 2.38; with an older git the preview names only the files the
  incoming commits share with uncommitted changes.
