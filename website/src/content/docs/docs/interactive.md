---
title: Interactive screen
description: Run cmw with no arguments to browse, open, create, hydrate and clean up worktrees from one screen.
---

Run `cmw` with no arguments in a terminal:

```sh
cmw
```

It opens a full-screen view of your worktrees. Everything it does is also a regular command, so you can switch between the two. You can try it on the [home page](/#try-title) without installing anything.

```
 ██████ ███    ███ ██     ██    shop
██      ████  ████ ██     ██    main
██      ██ ████ ██ ██  █  ██    v0.1.5
██      ██  ██  ██ ██ ███ ██
 ██████ ██      ██  ███ ███

➤ 1. Worktrees    Browse, open, hydrate, remove             4 linked
  2. New          Create a hydrated worktree
  3. Clean        Remove merged worktrees                   2 ready
  4. Slim         Delete build caches from idle worktrees   3.3 GB
  5. Setup        Shell integration, agent skill, hooks

↑↓  |  Enter  |  A All repos  |  Q Quit
```

The numbers on the right are live: linked worktrees, how many are merged and safe to remove, and how much cache the idle ones hold.

## Moving around

| Key | Action |
| --- | --- |
| `↑` `↓` / `j` `k` | Move |
| `Enter` | Open the selected item |
| `1`–`5` | Jump to a screen from the home menu |
| `Esc`, `Backspace` or `←` | Back to the home menu |
| `A` | Switch between this repo and all repos |
| `R` | Reload |
| `Q` | Quit |

The mouse works too. Click a menu item to open it. Click a row to select it, and click it again to open it. The keys in the footer and in dialogs are buttons. The wheel scrolls lists.

To select text in the terminal while cmw is open, hold Option (Terminal, iTerm2) or Shift while dragging.

## Worktrees

Every worktree of the repo, with its state, idle time and apparent size:

```
cmw › Worktrees   shop
──────────────────────────────────────────────────────

  BRANCH           STATE             IDLE       SIZE
  main  ◂ here     clean            today     1.9 GB
  chore/deps       ✓ merged            3d     1.5 GB
➤ feat/onboarding  ↑ 3 unpushed       12d     4.9 GB
  fix/checkout     ● 2 changed      today     1.5 GB
  spike/search     ✓ merged            9d     1.7 GB

  ~/.cmw/worktrees/shop/feat-onboarding
  not merged into origin/main
```

`STATE` is the first thing that applies: `● N changed` for uncommitted changes, `✓ merged` when the branch is [proven merged](/docs/cleanup/#merge-detection), `↑ N unpushed` for commits that aren't on the remote yet, otherwise `clean`. `◂ here` marks the worktree you started cmw in. The line under the table shows the selected worktree's path and why it is or isn't merged.

| Key | Action |
| --- | --- |
| `Enter` | `cd` into the worktree and quit |
| `H` | [Hydrate](/docs/hydrate/) it |
| `E` | [Sync its secrets](/docs/secrets/) from the main checkout |
| `D` | Remove it |
| `/` | Filter by branch, repo or path. `Enter` keeps the filter, `Esc` clears it. |

`Enter` changes your shell's directory only with [shell integration](/docs/install/#add-cmw-cd). Without it, cmw prints the path when it quits and copies it to the clipboard.

`D` asks before removing. If the worktree has uncommitted changes or unpushed commits, cmw refuses like [`cmw rm`](/docs/cleanup/#remove-one-worktree) does, then offers to force it: type the branch name to confirm that the work should be deleted.

## New

Two fields, the branch and its base. Leave the base empty to start from `origin/<default-branch>`. The line below shows where the worktree will go.

| Key | Action |
| --- | --- |
| `Tab` | Next field |
| `Enter` | Create and hydrate the worktree |
| `Ctrl-U` | Clear the field |
| `Esc` | Back |

When it's done, cmw shows the [hydrate summary](/docs/hydrate/). Press `Enter` to `cd` into the new worktree.

## Clean

The worktrees `cmw gc` would remove: merged into the default branch, with nothing uncommitted. All of them start selected.

| Key | Action |
| --- | --- |
| `Space` | Select or skip a worktree |
| `Enter` | Remove the selected worktrees |

Worktrees with work cmw can't account for never appear in the list. The count below it says how many were kept. See [Clean up](/docs/cleanup/) for the rules.

## Slim

Build caches (DerivedData, `.cxx`, `build/`, `.next`) in worktrees that have been idle for a while, like `cmw clean-artifacts`. The worktrees stay; the next build regenerates the caches.

| Key | Action |
| --- | --- |
| `←` `→` | Idle threshold: any time, 1, 3, 7, 14 or 30+ days. Default 7. |
| `Enter` | Delete the caches listed |

Sizes are apparent sizes, so they overstate what copy-on-write clones use. See [Disk usage](/docs/disk/).

## Setup

What's installed, and a way to install what's missing:

| Row | `Enter` does |
| --- | --- |
| Shell integration | Adds the `eval "$(cmw shell-init …)"` line to your shell's startup file |
| Agent skills | Installs the cmw skill for Claude Code, Codex and Grok, like `cmw agents install` |
| Claude Code hooks | Routes `claude --worktree` through cmw, like `cmw hook claude --install` |
| Agent setup prompt | Copies the prompt from `cmw agents prompt` |
| AGENTS.md snippet | Copies the snippet from `cmw skill print --agents-md` |

The Setup item shows `N to do` on the home screen when something required is missing.

## Updates

Each time it opens, the screen checks GitHub for a newer release in the background. When one is out, the home screen shows it next to the logo. Click it or press `U` to update; cmw restarts on the new version when it's done. Set `CMW_NO_UPDATE_CHECK=1` to turn the check off.

## Colors

cmw picks light or dark colors from your terminal's background. Set `CMW_THEME=light` or `CMW_THEME=dark` to choose, or `NO_COLOR=1` to turn colors off.

## Scripts and agents

The screen opens only when stdin and stdout are both a terminal. Anywhere else, and with `--json` or `CMW_JSON=1`, bare `cmw` prints help and exits `2`, so scripts and agents never get stuck in it.
