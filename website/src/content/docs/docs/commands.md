---
title: Commands
description: Every cmw command and flag.
---

## Global flags

| Flag | Effect |
| --- | --- |
| `--json` | Print one JSON document on stdout. Same as `CMW_JSON=1`. |
| `-C <dir>` | Run as if started in `<dir>`, like `git -C`. |
| `--version` | Print the version. |
| `--help` | Print help for cmw or any subcommand. |

## `cmw new <branch>`

Create a worktree for `<branch>` and hydrate it. Prints the path as the last line. [Guide](/docs/worktrees/)

| Flag | Effect |
| --- | --- |
| `--base <ref>` | Base for a new branch. Default: `origin/<default-branch>`, else `<default-branch>`. |
| `--path <dir>` | Where to create the worktree. Default: `~/.cmw/worktrees/<repo>/<branch-slug>`. |
| `--fetch` | `git fetch origin` before creating the branch. |
| `--no-hydrate` | Only create the worktree. |
| `--no-install` | Never run package managers. |
| `--no-slow` | Skip `pod install`; report it as `needs_setup`. |

## `cmw hydrate [path]`

Clone dependencies, copy secrets and rebuild environments from the main checkout. Default: the current worktree. [Guide](/docs/hydrate/)

| Flag | Effect |
| --- | --- |
| `--from <dir>` | Source checkout. Default: the repo's main checkout. |
| `--dry-run` | Show what would happen. |
| `--no-install` | Never run package managers. |
| `--no-slow` | Skip `pod install`; report it as `needs_setup`. |

## `cmw ls`

List worktrees with branch, dirty state, merge state, idle days and hydrate state.

| Flag | Effect |
| --- | --- |
| `--all` | Every repo under `~/.cmw/worktrees`, `~/.t3/worktrees` and `$CMW_SCAN_ROOTS`. |
| `--sizes` | Include apparent size per worktree. Slower; clones are counted at full size. |

## `cmw path <name>`

Print the path of a worktree. `<name>` is a branch, branch slug, directory name or path.

## `cmw cd <name>`

Change into a worktree. Needs [shell integration](/docs/install/#add-cmw-cd).

## `cmw rm <name>`

Remove a worktree, its build caches and, if merged, its branch. [Guide](/docs/cleanup/#remove-one-worktree)

| Flag | Effect |
| --- | --- |
| `--dry-run` | Show what would be removed. |
| `--keep-branch` | Keep the branch even if it's merged. |
| `--force` | Remove even with uncommitted changes or unpushed, unmerged commits. Destroys that work. |

## `cmw gc`

Find and remove merged, clean worktrees. Dry run unless `--yes`. [Guide](/docs/cleanup/#remove-everything-thats-merged)

| Flag | Effect |
| --- | --- |
| `--yes` | Actually remove. |
| `--stale <days>` | Also report unmerged worktrees idle this long. Never removed. |
| `--all` | Every repo under the scan roots. |

## `cmw clean-artifacts`

Delete regenerable caches from idle worktrees. Dry run unless `--yes`. [Guide](/docs/cleanup/#strip-build-caches-keep-the-worktree)

| Flag | Effect |
| --- | --- |
| `--idle <days>` | Only worktrees idle this long. Default `7`. |
| `--yes` | Actually delete. |
| `--all` | Every repo under the scan roots. |

## `cmw env sync [path]`

Re-copy secrets and config from the main checkout. Files changed in the worktree are kept unless `--force`. [Guide](/docs/secrets/)

| Flag | Effect |
| --- | --- |
| `--dry-run` | Show what would change. |
| `--force` | Overwrite files that differ in the worktree. |

## `cmw shell-init <shell>`

Print the shell function that adds `cmw cd`. `<shell>` is `zsh`, `bash` or `fish`.

## `cmw skill install`

Write the cmw skill to `~/.claude/skills/cmw/SKILL.md`. `--dir <path>` writes it elsewhere.

## `cmw skill print`

Print the skill. `--agents-md` prints a snippet for `AGENTS.md` or `CLAUDE.md` instead.

## `cmw hook claude`

Print Claude Code `settings.json` hooks that create and remove worktrees through cmw. [Guide](/docs/agents/#claude-code-worktrees)

## Environment variables

| Variable | Effect |
| --- | --- |
| `CMW_JSON` | `1` turns on `--json` for every command. |
| `CMW_HOME` | cmw's folder. Default `~/.cmw`. Worktrees go to `$CMW_HOME/worktrees`. |
| `CMW_SCAN_ROOTS` | Extra folders for `--all`, separated by `:`. |
