---
name: cmw
description: Use when creating, hydrating, listing, switching or cleaning git worktrees, or when cutting worktree disk usage; cmw makes worktrees fast, copy-on-write and safe to remove.
---

# cmw: git worktrees for agents

## When to use it

Starting isolated work: run `cmw new <branch> --json`. Read `data.path` (the worktree), then check `data.hydrate.ready`. If it is false, run each `data.hydrate.needs_setup[].command` inside that path.

## Commands

- `cmw ls --json`: list worktrees.
- `cmw path <name>`: print a worktree's path.
- `cmw hydrate`: (re)hydrate the current worktree.
- `cmw env sync`: sync .env files with the main worktree.
- `cmw rm <name>`: remove a worktree (refuses unsafe removals).
- `cmw gc`: dry run of cleanup; `cmw gc --yes` to apply.
- `cmw clean-artifacts`: delete build artifacts to free disk.

## JSON envelope

Success: `{ok, command, data, warnings}`. Failure: `{ok: false, error: {kind, message, hint}}`.

## Exit codes

0 ok, 1 error, 2 usage, 3 refused (unsafe), 4 not found.

## Rules

- When refused (exit 3), run `error.hint` only if it is safe.
- Never pass `--force` to `rm` unless the user explicitly asked to discard work.
- Prefer `-C <dir>` over `cd`.
- `CMW_JSON=1` makes every command emit JSON.

## What hydrate does

Clones dependencies copy-on-write, copies .env and secrets, skips build caches, and runs installs only when lockfiles differ from main.
