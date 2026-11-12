---
title: Create worktrees
description: Create, find and switch between worktrees with cmw new, cmw path and cmw cd.
---

## `cmw new`

```sh
cmw new <branch>
```

- If `<branch>` exists, cmw checks it out in a new worktree.
- If it doesn't, cmw creates it from `origin/<default-branch>`, or the local default branch when there's no remote.
- If a worktree for `<branch>` already exists, cmw returns its path and changes nothing. It's safe to run twice.

The worktree goes to `~/.cmw/worktrees/<repo>/<branch-slug>`. The slug is the branch in lowercase with `/` and other separators turned into `-`, so `feat/login` becomes `feat-login`.

| Flag | Effect |
| --- | --- |
| `--base <ref>` | Start a new branch from `<ref>` instead of the default branch. |
| `--fetch` | Run `git fetch origin` first, so the base is current. |
| `--path <dir>` | Create the worktree at `<dir>`. |
| `--no-hydrate` | Only create the worktree. Run `cmw hydrate` later. |
| `--no-install` | Clone and copy, but never run a package manager. |
| `--no-slow` | Skip `pod install`. It's reported as `needs_setup` instead. |

Without `--json`, `cmw new` prints a hydrate summary and then the path on the last line:

```sh
cd "$(cmw new feat/login)"
```

### Where worktrees live

Set `CMW_HOME` to move cmw's folder. Worktrees then go to `$CMW_HOME/worktrees`.

```sh
export CMW_HOME="$HOME/dev/.cmw"
```

## Find a worktree

`cmw path` prints the path of a worktree. `cmw cd` jumps into it once [shell integration](/docs/install/#add-cmw-cd) is set up. Both accept:

- the branch: `feat/login`
- the slug: `feat-login`
- the directory name: `t3-0a8fd5f2-c0f8`
- a path

```sh
code "$(cmw path feat/login)"
cmw cd feat/login
```

## List worktrees

```sh
cmw ls
```

```
shop (~/code/shop/.git)
BRANCH        STATE    MERGED  IDLE  HYDRATED  PATH
main [main]   clean    ?       0d    no        ~/code/shop
feat/login    clean    no      0d    yes       ~/.cmw/worktrees/shop/feat-login
fix/checkout  2 dirty  no      1d    yes       ~/.cmw/worktrees/shop/fix-checkout
```

| Column | Meaning |
| --- | --- |
| `STATE` | `clean`, or the number of uncommitted changes. |
| `MERGED` | `yes` only when cmw can prove it. See [merge detection](/docs/cleanup/#merge-detection). |
| `IDLE` | Days since the last commit or git index change. |
| `HYDRATED` | Whether `cmw hydrate` has run in this worktree. |

| Flag | Effect |
| --- | --- |
| `--all` | List every repo under `~/.cmw/worktrees`, `~/.t3/worktrees` and `$CMW_SCAN_ROOTS`. |
| `--sizes` | Add the apparent size of each worktree. Clones are counted at full size. See [Disk usage](/docs/disk/). |

`CMW_SCAN_ROOTS` is a colon-separated list of extra folders, like `PATH`.

## Run against another repo

Every command accepts `-C <dir>`, like `git -C`:

```sh
cmw -C ~/code/shop ls
```
