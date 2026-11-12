---
title: Clean up
description: Remove worktrees and their build caches without losing work, with cmw rm, cmw gc and cmw clean-artifacts.
---

Worktrees pile up, and each one leaves build caches behind: Xcode's DerivedData, Android's `.cxx`, `build/`, `.next`. cmw removes them together, and refuses whenever work could be lost.

## The safety rules

In order of importance:

1. Never touch the main checkout.
2. Never delete uncommitted changes, or commits that are both unpushed and unmerged, without `--force`.
3. Delete a branch only when it's proven merged.
4. `clean-artifacts` only deletes gitignored paths.

When cmw refuses, it exits with code `3` and prints the command that overrides it:

```
$ cmw rm fix/checkout
error: 2 uncommitted changes in ~/.cmw/worktrees/shop/fix-checkout
  fix: cmw rm fix/checkout --force
```

## Remove one worktree

```sh
cmw rm feat/login
```

This removes the worktree folder, the DerivedData that belongs to it, and the branch if it's merged. An unmerged branch is kept, so its commits stay recoverable.

| Flag | Effect |
| --- | --- |
| `--dry-run` | Show what would be removed. |
| `--keep-branch` | Keep the branch even if it's merged. |
| `--force` | Remove even with uncommitted changes or unpushed, unmerged commits. That work is destroyed. |

`rm` accepts a branch, slug, directory name or path, like [`cmw path`](/docs/worktrees/#find-a-worktree).

## Remove everything that's merged

```sh
cmw gc          # dry run
cmw gc --yes    # remove
```

`gc` finds worktrees whose branch is merged and that have no uncommitted changes, then removes them with their branches and DerivedData. Without `--yes` it only reports.

It also:

- prunes git's records of worktrees whose folder is already gone
- deletes DerivedData whose Xcode workspace no longer exists
- lists folders under `~/.cmw/worktrees/<repo>` that git doesn't know about, as `orphans`, with a hint. It never deletes them.
- with `--stale <days>`, lists unmerged worktrees with no activity for that long. It never removes them.

Locked worktrees (`git worktree lock`) and dirty merged worktrees are reported as `skipped` with the reason.

| Flag | Effect |
| --- | --- |
| `--yes` | Actually remove. |
| `--stale <days>` | Also report unmerged worktrees idle for this many days. |
| `--all` | Every repo under the [scan roots](/docs/worktrees/#list-worktrees), not just the current one. |

## Strip build caches, keep the worktree

```sh
cmw clean-artifacts          # dry run
cmw clean-artifacts --yes    # delete
```

For each worktree idle for 7 days or more, this deletes gitignored caches that rebuild on their own: `DerivedData`, `.build`, `.cxx`, `.gradle`, `.kotlin`, `build/`, `.next`, `.expo`, `.nuxt`, `.svelte-kit`, `.parcel-cache`, `.vite`, `coverage`, `.turbo` and `.nx/cache`. The main checkout is never touched.

| Flag | Effect |
| --- | --- |
| `--idle <days>` | Only worktrees with no commits or index changes for this many days. Default `7`. |
| `--yes` | Actually delete. |
| `--all` | Every repo under the scan roots. |

The dry run shows apparent sizes, which overcount clones. With `--yes`, cmw reports the real change in free disk space. See [Disk usage](/docs/disk/).

## Merge detection

A branch counts as merged only when one of these checks proves it, in order. The default branch is `origin/<default>` when it exists, otherwise the local one.

| `merged_via` | Check |
| --- | --- |
| `ancestor` | The branch tip is an ancestor of the default branch. |
| `patch_id` | The branch has no merge commits, and every commit has an equivalent patch on the default branch. Catches rebases and cherry-picks. |
| `gh` | The GitHub CLI finds a merged pull request whose head is the branch's current commit. Catches squash merges. |

A branch with no commits of its own is never treated as merged. Anything unproven is kept, and `cmw ls` shows `?` or `no`.

## Submodules

`rm` and `gc` refuse worktrees with initialized submodules unless you pass `--force`, because cmw can't yet check for unpushed commits inside submodules.
