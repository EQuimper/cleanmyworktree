---
title: Commands
description: Every cmw command and flag.
---

## Global flags

| Flag | Effect |
| --- | --- |
| `--json` | Print one JSON document on stdout. Same as `CMW_JSON=1`. |
| `-C <dir>` | Run as if started in `<dir>`, like `git -C`. |
| `-V`, `--version` | Print the version. |
| `-h`, `--help` | Print help for cmw or any subcommand. `-h` prints a shorter summary. `cmw help <command>` works too. |

## `cmw`

With no arguments in a terminal, cmw opens an [interactive screen](/docs/interactive/): browse worktrees with their state and size, open one, create, hydrate, sync env, remove, clean up merged worktrees, slim build caches, and set up shell integration and the agent skill. Arrow keys, `Enter`, `Esc` and `Q` get you around, and the mouse works too.

Without a terminal on stdin and stdout, or with `--json` / `CMW_JSON=1`, bare `cmw` prints help and exits 2, so scripts and agents are unaffected.

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

## `cmw update`

Upgrade cmw to the latest release through whatever installed it: `brew upgrade equimper/tap/cmw` for Homebrew, the release installer script for the shell installer, `cargo install cmw --locked` for crates.io. A build from a source checkout can't update itself, so cmw refuses (exit 3) with the command to run instead.

| Flag | Effect |
| --- | --- |
| `--check` | Only report the current and latest versions, the install method and the upgrade command. |

The latest version is read from GitHub's `releases/latest` redirect. Only `cmw update` and the interactive screen check for updates; the interactive screen checks in the background each time it opens. Other commands never touch the network.

## `cmw shell-init <shell>`

Print the shell function that adds `cmw cd`, and lets **Open** in the interactive screen change your directory. `<shell>` is `zsh`, `bash` or `fish`.

## `cmw agents`

Show which agents are set up: the cmw skill for Claude Code, Codex and Grok (`installed`, `outdated` or `missing`), Claude Code's worktree hooks, and T3 Code. [Guide](/docs/agents/)

## `cmw agents install`

Write the cmw skill for every agent found: `~/.claude/skills/cmw`, `~/.agents/skills/cmw` (Codex) and `~/.grok/skills/cmw`.

| Flag | Effect |
| --- | --- |
| `--agent <claude\|codex\|grok>` | Install for these agents only, even if they aren't found. Repeatable. |
| `--claude-hooks` | Also add the Claude Code worktree hooks to `~/.claude/settings.json`. |
| `--refresh` | Only update cmw skill files that already exist. `cmw update` runs this. |

## `cmw agents prompt`

Print the agent setup prompt. `--copy` puts it on the clipboard instead. Paste it into Claude Code, Codex, Grok or T3 Code.

## `cmw skill install`

Same as `cmw agents install`. `--dir <path>` writes the skill to one folder instead, such as a project's `.claude/skills/cmw`.

## `cmw skill print`

Print the skill. `--agents-md` prints a snippet for `AGENTS.md` or `CLAUDE.md` instead.

## `cmw hook claude`

Print Claude Code `settings.json` hooks that create and remove worktrees through cmw. `--install` adds them to `~/.claude/settings.json`, keeping your other hooks; add `--project` to use the repo's `.claude/settings.json` instead. [Guide](/docs/agents/#claude-code-worktrees)

## Environment variables

| Variable | Effect |
| --- | --- |
| `CMW_JSON` | `1` turns on `--json` for every command. |
| `CMW_HOME` | cmw's folder. Default `~/.cmw`. Worktrees go to `$CMW_HOME/worktrees`. |
| `CMW_SCAN_ROOTS` | Extra folders for `--all`, separated by `:`. |
| `CMW_THEME` | `light` or `dark` for the interactive screen. Default: detected from the terminal. |
| `NO_COLOR` | Any value turns off colors in the interactive screen. |
| `CMW_NO_UPDATE_CHECK` | `1` stops the interactive screen from checking for new releases. |
