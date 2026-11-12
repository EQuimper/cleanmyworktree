---
title: Use with agents
description: Drive cmw from Claude Code, Codex, T3 Code or any agent with JSON output, stable exit codes and fix hints.
---

cmw is designed so an agent can drive it without supervision.

| Contract | Behavior |
| --- | --- |
| Output | `--json` or `CMW_JSON=1` prints one JSON document on stdout. See [JSON output](/docs/json/). |
| Errors | `{"ok":false,"error":{"kind":"refused","message":"…","hint":"cmw rm feat-x --force"}}` |
| Exit codes | `0` ok · `1` error · `2` usage · `3` refused (unsafe) · `4` not found |
| No prompts | Never reads stdin. `gc` and `clean-artifacts` are dry runs unless `--yes`. |
| Idempotent | `cmw new <branch>` on an existing worktree returns its path. `hydrate` can be re-run safely. |
| Parallel-safe | A lock per repo serializes concurrent cmw runs. |
| Plain mode | `cmw new` and `cmw path` print the path as the last line, so `cd "$(cmw new x)"` works. |
| `-C <dir>` | Run against a repo without `cd`, like `git -C`. |

## A typical loop

```sh
cmw new fix/flaky-test --json
# data.path                     where to work
# data.hydrate.ready            true when nothing is left to do
# data.hydrate.needs_setup[]    each has .command to run, e.g. pod install

# ... work in data.path ...

cmw rm fix/flaky-test --json
# refused (exit 3) with error.hint if anything would be lost
```

When a command is refused, `error.hint` is the exact command that overrides it. An agent should show it to the person rather than run it, since it usually means `--force`.

## Teach your agent about cmw

### Claude Code skill

```sh
cmw skill install
```

Writes the cmw skill to `~/.claude/skills/cmw/SKILL.md`. Use `--dir <path>` to write it elsewhere, such as a project's `.claude/skills/cmw`.

### AGENTS.md or CLAUDE.md

```sh
cmw skill print --agents-md >> AGENTS.md
```

`cmw skill print` without the flag prints the full skill.

## Claude Code worktrees

Claude Code can create worktrees for its sessions. Route them through cmw so they're hydrated and cleaned up safely:

```sh
cmw hook claude
```

This prints `settings.json` hooks for `WorktreeCreate` and `WorktreeRemove`. Merge them into `~/.claude/settings.json` or your project's `.claude/settings.json`:

```json
{
  "hooks": {
    "WorktreeCreate": [{ "hooks": [{ "type": "command", "command": "cmw hook claude-create" }] }],
    "WorktreeRemove": [{ "hooks": [{ "type": "command", "command": "cmw hook claude-remove" }] }]
  }
}
```

Removal goes through `cmw rm` without `--force`, so a worktree with uncommitted or unpushed work is kept.

## T3 Code

Set the project's worktree setup script to:

```sh
cmw hydrate
```

`cmw ls --all`, `cmw gc --all` and `cmw clean-artifacts --all` include `~/.t3/worktrees` automatically.

## Other tools

Any tool that creates worktrees with `git worktree add` works the same way: run `cmw hydrate` in the new worktree. Add its worktree folder to `CMW_SCAN_ROOTS` so `--all` finds it.
