<p align="center">
  <img src="assets/icon.svg" width="96" height="96" alt="cmw logo">
</p>

<h1 align="center">CleanMyWorktree</h1>

<p align="center">
  <b>Git worktrees in seconds, not gigabytes.</b><br>
  Copy-on-write dependencies, <code>.env</code> that follows, cleanup that never loses work. Built for humans and AI agents.
</p>

<p align="center">
  <a href="https://cmw.equimper.com/docs/"><b>Documentation</b></a> ·
  <a href="https://cmw.equimper.com/#try">Try it in your browser</a> ·
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="https://cmw.equimper.com/docs/agents/">Use with agents</a>
</p>

<p align="center">
  <img alt="macOS" src="https://img.shields.io/badge/macOS-APFS-000?logo=apple&logoColor=white">
  <img alt="Rust" src="https://img.shields.io/badge/built_with-Rust-000?logo=rust&logoColor=white">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-4ef2a0">
</p>

---

```console
$ cmw new feat/login
ready in 1.6s
  files     3 cloned, 2 copied, 4 skipped
  installs  1 up to date
~/.cmw/worktrees/shop/feat-login
```

- **Seconds to set up.** Dependencies are copy-on-write cloned from your main checkout. On APFS a 1.4 GB `node_modules` (107k files) clones in about 1.6 s.
- **Near-zero extra disk.** Clones share blocks with the original until a file changes. `npm install new-lib` in a worktree only costs the new lib.
- **`.env` follows.** Ignored secrets and machine-local config are copied into every worktree, and `cmw env sync` refreshes them later.
- **Works with most stacks.** Node (npm, pnpm, yarn, bun), iOS (CocoaPods, SPM), Android (Gradle), Expo / React Native, PHP (Composer), Python (uv, poetry), Rust.
- **Safe cleanup.** Removes merged worktrees together with the DerivedData, `.cxx` and build caches they leave behind. It never deletes uncommitted or unpushed work unless you pass `--force`.
- **Agent-friendly.** `--json` everywhere, no prompts, stable exit codes, and every refusal comes with the exact command that fixes it.

## Install

```sh
brew install equimper/tap/cmw
eval "$(cmw shell-init zsh)"   # optional: `cmw cd <name>`, and Open in `cmw` changes directory; put it in ~/.zshrc
```

macOS only for v0.1. Other options, including bash and fish, are in the [install guide](https://cmw.equimper.com/docs/install/).

## Quick start

```sh
cmw                           # interactive: browse, open, create, clean up
cmw new feat/login            # create + hydrate; prints the path
cmw cd feat/login             # jump into it (needs shell-init)
cmw ls                        # branch, dirty, merged, idle, hydrated
cmw hydrate                   # fill a worktree made by T3, Claude Code or plain git
cmw rm feat/login             # remove the worktree, its caches, and the branch if merged
cmw gc                        # dry run: what is safe to remove
cmw gc --yes                  # remove it and report the real space freed
cmw clean-artifacts --yes     # strip DerivedData/.cxx/build/.next from idle worktrees
cmw update                    # upgrade through Homebrew, the installer or cargo
```

Every command takes `--json` and `--help`. The [command reference](https://cmw.equimper.com/docs/commands/) lists every flag.

`cmw new` puts worktrees in `~/.cmw/worktrees/<repo>/<branch>`; set `$CMW_HOME` to change that. Every other command works on any git worktree, including the ones under `~/.t3/worktrees`. `ls --all`, `gc --all` and `clean-artifacts --all` scan `~/.cmw/worktrees`, `~/.t3/worktrees` and any folders listed in `$CMW_SCAN_ROOTS`.

## Commands

| Command | What it does |
|---|---|
| `cmw` | Opens the [interactive screen](#interactive-screen) in a terminal. |
| `cmw new <branch>` | Creates a worktree and hydrates it. Prints the path last, so `cd "$(cmw new x)"` works. |
| `cmw hydrate [path]` | Clones deps, copies secrets and rebuilds envs from the main checkout. `--dry-run` shows the plan. |
| `cmw ls` | Lists worktrees: branch, uncommitted changes, merged, idle days, hydrated. `--sizes`, `--all`. |
| `cmw path <name>` / `cmw cd <name>` | Prints a worktree's path, or jumps into it. `<name>` is a branch, slug, folder or path. |
| `cmw env sync [path]` | Copies secrets and local config from the main checkout again. Files that differ in the worktree are kept unless `--force`. |
| `cmw rm <name>` | Removes a worktree, its caches, and its branch if merged. Refuses to lose work without `--force`. |
| `cmw gc` | Lists merged, clean worktrees. `--yes` removes them, `--stale <days>` reports old unmerged ones. |
| `cmw clean-artifacts` | Lists build caches in worktrees idle for 7+ days. `--yes` deletes them, `--idle <days>` changes the cutoff. |
| `cmw agents` | Shows the cmw skill and hooks for Claude Code, Codex, Grok and T3 Code. `install` and `prompt --copy` set them up. |
| `cmw shell-init <shell>` | Prints the shell function behind `cmw cd`, for zsh, bash or fish. |
| `cmw update` | Upgrades cmw through whatever installed it. `--check` only reports. |

Global flags: `--json` (or `CMW_JSON=1`), `-C <dir>` to run against another repo like `git -C`, `-h`/`--help`, `-V`/`--version`. Full flag lists are in the [command reference](https://cmw.equimper.com/docs/commands/).

## Interactive screen

Run `cmw` on its own to manage worktrees with the keyboard or the mouse:

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

- **Worktrees:** every worktree with its state, idle time and size. `Enter` opens it (with shell integration, your shell `cd`s there), `H` hydrates, `E` syncs env, `D` removes, `/` filters.
- **New:** type a branch and an optional base; cmw creates and hydrates it.
- **Clean:** the merged worktrees `cmw gc` would remove, as a checklist.
- **Slim:** build caches in idle worktrees, with a threshold you change with `←` `→`.
- **Setup:** shell integration, the agent skill and Claude Code hooks, each one keypress away.

`A` switches between this repo and all repos, `Esc` goes back, `Q` quits. Removing asks first, and work cmw can't account for has to be confirmed by typing the branch name. Outside a terminal, or with `--json`, bare `cmw` prints help and exits 2, so scripts and agents never land in it. Details: [Interactive screen](https://cmw.equimper.com/docs/interactive/).

## What hydrate does

cmw lists the gitignored files in the main checkout and handles each one according to whether it still works after moving to another path:

| Strategy | Examples | Why |
|---|---|---|
| **clone** | `node_modules`, `Pods`, `vendor`, `Carthage`, `target`, `.turbo` | Can be moved safely. A copy-on-write clone is instant and free. |
| **copy** | `.env*`, `local.properties`, `google-services.json`, `GoogleService-Info.plist`, `*.keystore`, `*.jks`, `.xcode.env.local` | Secrets and local config. Each worktree gets its own copy. |
| **regen** | `.venv`, `.dart_tool` | Contains absolute paths: a cloned venv's `pip` would install into main's venv. |
| **skip** | `DerivedData`, `.cxx`, `build/`, `.gradle/`, `.next`, `.expo` | Build outputs tied to the original path. |

Dependency directories are symlinked nowhere. Symlinked `node_modules` break npm, Metro/watchman and Node's module resolution (you get two copies of React), and changes leak between checkouts.

After cloning, cmw compares each lockfile with main's:

- **Same lockfile:** no install runs.
- **Different lockfile:** cmw runs the project's normal install. npm always gets `npm install`, never `npm ci`, because `npm ci` deletes the cloned `node_modules`.
- **`pod install`:** runs only when `Podfile.lock` changed. `--no-slow` skips it and reports the worktree as `needs_setup`.
- **Expo projects that generate `ios/` and `android/`** (instead of committing them): those folders are cloned, and cmw warns `stale_native` if the app config or `package.json` differs from main.

Every pattern is listed in [Built-in rules](https://cmw.equimper.com/docs/rules/).

### `.cmw.toml` (optional, commit it)

```toml
[[rule]]
pattern = "ios/Secrets.xcconfig"   # glob vs repo-relative path
strategy = "copy"                  # clone | copy | regen | skip

[hydrate]
post = ["npm run codegen"]         # run in the worktree after hydrate
```

Your rules are checked before the built-in ones. See [`.cmw.toml`](https://cmw.equimper.com/docs/config/).

## For agents

cmw is designed so an agent can drive it without supervision.

| Contract | Behavior |
|---|---|
| Output | `--json` or `CMW_JSON=1` prints one JSON document: `{"ok":true,"command":"new","data":{...},"warnings":[]}` |
| Errors | `{"ok":false,"error":{"kind":"refused","message":"2 uncommitted changes in …","hint":"cmw rm feat-x --force"}}` |
| Exit codes | `0` ok · `1` error · `2` usage · `3` refused (unsafe) · `4` not found |
| No prompts | Never reads stdin. `gc` and `clean-artifacts` are dry runs unless `--yes`. |
| Idempotent | `cmw new <branch>` on an existing worktree returns its path. `hydrate` can be re-run safely. |
| Parallel-safe | A lock per repo serializes concurrent cmw runs. |
| Plain mode | `cmw new` and `cmw path` print the path as the last line, so `cd "$(cmw new x)"` works. |
| `-C <dir>` | Run against a repo without `cd`, like `git -C`. |

A typical agent loop:

```sh
cmw new fix/flaky-test --json   # data.path, data.hydrate.ready, data.hydrate.needs_setup[].command
# ... work in data.path ...
cmw rm fix/flaky-test --json    # refused with a hint if anything would be lost
```

Teach your agent about cmw:

```sh
cmw agents install             # cmw skill for Claude Code, Codex and Grok
cmw agents prompt --copy       # setup prompt: paste it into Claude Code, Codex, Grok or T3 Code
cmw agents                     # what's installed
cmw hook claude --install      # optional: Claude Code's own worktrees go through cmw
```

Pasted into T3 Code, the prompt also adds `cmw hydrate` as the worktree setup script of every T3 project. The script runs in the background (`"async": true`), so the agent starts right away instead of waiting for hydrate.

## Disk usage, honestly

Copy-on-write clones fool `du`: each clone is reported at full size even though the blocks are shared. That's why `cmw ls --sizes` shows *apparent* size. `gc --yes` and `clean-artifacts --yes` report `freed_bytes` from the change in free disk space, which is the only number that reflects reality on APFS.

## Known limitations (v0.1)

- **macOS only.** On other filesystems, or across volumes, cmw falls back to a plain copy and warns about the full disk cost.
- **First hydrate of an existing worktree runs one install.** cmw only skips an install when it can prove which lockfile built the dependencies. For a worktree made by another tool, the first hydrate runs the package manager once, typically 1–2 s for npm on an up-to-date tree. `pod install` is reported as `needs_setup` under `--no-slow`.
- **Post hooks that edit inside dependency folders aren't detected.** If a `[hydrate] post` command replaces `node_modules` (or `Pods`, `vendor`, `.venv`), cmw notices and reports `needs_setup`. Patching individual files inside those folders goes unnoticed. If your hooks patch dependencies, re-run the package manager after them.
- **Merge detection is conservative.** `git branch --merged` misses squash merges, so cmw falls back to patch-id comparison and the `gh` CLI. A branch counts as merged only when one of those checks proves it. A branch with no commits of its own is never treated as merged, and anything unproven is kept.
- **`rm` and `gc` refuse worktrees with initialized submodules** unless you pass `--force`, because cmw can't yet check for unpushed commits inside submodules.

## Documentation

The full docs live at **[cmw.equimper.com](https://cmw.equimper.com/docs/)**.

## License

MIT
