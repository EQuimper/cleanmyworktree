---
name: cmw
description: Use when creating, hydrating, listing, switching or cleaning git worktrees, when a worktree is missing node_modules, Pods, vendor or .env, or when cutting worktree disk usage; cmw makes worktrees fast, copy-on-write and safe to remove.
---

# cmw: git worktrees for agents

## Already in a worktree?

T3 Code, the Codex app, Claude Code (`--worktree`) and similar tools often start you in a worktree they created. Check with `git rev-parse --git-dir --git-common-dir`: two different paths mean a linked worktree.

- Work where you are. Don't run `cmw new` from inside it.
- If dependencies or `.env` are missing, run `cmw hydrate --json`, then run each `data.needs_setup[].command`.
- Leave removal to the tool that created it. Never `cmw rm` the worktree you are in.

`cmw hydrate` exits 3 with "target is the main checkout" when run in the main checkout. Nothing changed; you are not in a worktree.

## Starting isolated work yourself

Run `cmw new <branch> --json`. Work in `data.path`. If `data.hydrate.ready` is false, run each `data.hydrate.needs_setup[].command` inside that path. Running it again for the same branch returns the same path.

## Commands

- `cmw ls --json`: list worktrees (branch, dirty, merged, idle, hydrated).
- `cmw path <name>`: print a worktree's path.
- `cmw hydrate`: (re)hydrate the current worktree. Safe to re-run.
- `cmw env sync`: refresh .env and other copied files from the main checkout.
- `cmw rm <name>`: remove a worktree you created (refuses unsafe removals).
- `cmw gc`: dry run of cleanup; `cmw gc --yes` applies it.
- `cmw clean-artifacts`: dry run of build-cache cleanup; `--yes` applies it.

## JSON envelope

Success: `{ok, command, data, warnings}`. Failure: `{ok: false, error: {kind, message, hint}}`.

## Exit codes

0 ok, 1 error, 2 usage, 3 refused (unsafe), 4 not found.

## Rules

- When refused (exit 3), show `error.hint` to the person instead of running it; it usually adds `--force`.
- Never pass `--force` to `rm` unless the person explicitly asked to discard work.
- Prefer `-C <dir>` over `cd`.
- `CMW_JSON=1` makes every command emit JSON.

## What hydrate does

Clones dependencies copy-on-write, copies .env and secrets, skips build caches, and runs installs only when lockfiles differ from main.
