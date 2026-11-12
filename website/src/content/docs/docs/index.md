---
title: Get started
description: Install cmw and create your first hydrated worktree.
---

cmw makes git worktrees fast to create, cheap on disk, and safe to clean up. Dependencies like `node_modules`, `Pods` and `vendor` are copy-on-write clones of your main checkout, so a new worktree is ready in seconds and costs almost no extra disk. It's built for people and AI agents running many worktrees in parallel.

## Install

cmw runs on macOS, on Apple silicon and Intel.

```sh
brew install equimper/tap/cmw
```

See [Install](/docs/install/) for the shell installer and `cmw cd`.

## Create a worktree

Run this from anywhere inside your repo:

```sh
cmw new feat/login
```

cmw creates the branch from `origin/<default-branch>`, adds a worktree at `~/.cmw/worktrees/<repo>/feat-login`, and hydrates it:

- `node_modules`, `Pods`, `vendor` and `target` are cloned from the main checkout.
- `.env` files, `local.properties`, keystores and other ignored config are copied.
- If a lockfile differs from main's, cmw runs the normal install. If it matches, nothing runs.

The last line of output is the worktree path, so this works:

```sh
cd "$(cmw new feat/login)"
```

## See what you have

```sh
cmw ls
```

Each row shows the branch, uncommitted changes, whether it's merged, days idle, and whether cmw hydrated it.

## Clean up

```sh
cmw rm feat/login   # remove one worktree, its caches, and the branch if merged
cmw gc              # dry run: list every merged, clean worktree
cmw gc --yes        # remove them and report the space freed
```

cmw never deletes uncommitted changes or unpushed commits unless you pass `--force`. See [Clean up](/docs/cleanup/).

## Worktrees from other tools

Worktrees made by T3 Code, Claude Code or plain `git worktree add` work too. Run `cmw hydrate` inside one to fill in its dependencies and config.

## Next

- [How hydrate works](/docs/hydrate/) explains what gets cloned, copied, rebuilt or skipped.
- [Use with agents](/docs/agents/) covers the one-prompt setup for Claude Code, Codex, Grok and T3 Code, plus `--json` and exit codes.
