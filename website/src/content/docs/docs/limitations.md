---
title: Limitations
description: Known limitations of cmw v0.1.
---

## macOS only

cmw v0.1 targets macOS and APFS. On other filesystems, or across volumes, it falls back to a plain copy and warns about the full disk cost. Linux reflink support is planned.

## The first hydrate of an existing worktree runs one install

cmw only skips an install when it can prove which lockfile built the dependencies. For a worktree made by another tool, the first hydrate runs the package manager once, typically 1–2 s for npm on an up-to-date tree. With `--no-slow`, `pod install` is reported as `needs_setup` instead.

## Post hooks that edit inside dependency folders

If a `[hydrate] post` command replaces `node_modules`, `Pods`, `vendor` or `.venv`, cmw notices and reports `needs_setup`. Patching individual files inside those folders goes unnoticed. If your hooks patch dependencies, re-run the package manager after them.

## Merge detection is conservative

`git branch --merged` misses squash merges, so cmw also compares patch IDs and asks the `gh` CLI. A branch counts as merged only when one of those checks proves it. A branch with no commits of its own is never treated as merged, and anything unproven is kept. Install and sign in to `gh` to catch squash merges.

## Submodules

`rm` and `gc` refuse worktrees with initialized submodules unless you pass `--force`, because cmw can't yet check for unpushed commits inside submodules.

## Report a problem

Open an issue on [GitHub](https://github.com/EQuimper/cleanmyworktree/issues) with the command, its `--json` output and your macOS version.
