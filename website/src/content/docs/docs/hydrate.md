---
title: How hydrate works
description: What cmw clones, copies, rebuilds and skips when it fills a worktree, and when it runs installs.
---

Hydrate brings everything git ignores from the main checkout into a worktree. `cmw new` runs it for you. Run it yourself in a worktree made by another tool:

```sh
cmw hydrate            # the current worktree
cmw hydrate ../other   # another one
```

## Four strategies

cmw lists the gitignored files in the main checkout and asks one question about each: does it still work after moving to another path?

| Strategy | Examples | Why |
| --- | --- | --- |
| <span class="tag">clone</span> | `node_modules`, `Pods`, `vendor`, `Carthage`, `target`, `.turbo` | Moves safely. A copy-on-write clone is instant and shares disk blocks. |
| <span class="tag">copy</span> | `.env*`, `local.properties`, `google-services.json`, `GoogleService-Info.plist`, `*.keystore`, `*.jks`, `.xcode.env.local` | Secrets and local config. Each worktree gets its own copy. |
| <span class="tag">regen</span> | `.venv`, `.dart_tool` | Holds absolute paths. A cloned venv's `pip` would install into main's venv. |
| <span class="tag">skip</span> | `DerivedData`, `.cxx`, `build/`, `.gradle/`, `.next`, `.expo` | Build output tied to the original path. |

The full list is in [Built-in rules](/docs/rules/). Add your own in [`.cmw.toml`](/docs/config/).

Ignored entries that no rule matches are left alone and listed as `unmatched` in the JSON report.

### Why clone and not symlink

Symlinked `node_modules` break npm, Metro, watchman and Node's module resolution (you get two copies of React), and changes leak between checkouts. A clone behaves like a real folder. On APFS it costs nothing until a file changes, so `npm install new-lib` in a worktree only pays for the new lib.

On a filesystem without copy-on-write, or across volumes, cmw falls back to a plain copy and warns about the disk cost.

## Installs

After cloning, cmw compares each lockfile in the worktree with main's:

- **Same lockfile:** no install runs.
- **Different lockfile:** cmw runs the project's install command in that folder.

| Lockfile | Command |
| --- | --- |
| `package-lock.json` | `npm install --no-audit --no-fund` |
| `pnpm-lock.yaml` | `pnpm install --prefer-offline` |
| `yarn.lock` | `yarn install` |
| `bun.lock`, `bun.lockb` | `bun install` |
| `composer.lock` | `composer install --no-interaction` |
| `Podfile.lock` | `pod install` |
| `uv.lock` | `uv sync` |
| `poetry.lock` | `poetry install --no-interaction` |
| `Gemfile.lock` | `bundle install` |
| `pubspec.lock` | `flutter pub get` |

npm always gets `npm install`, never `npm ci`, because `npm ci` deletes the cloned `node_modules`.

| Flag | Effect |
| --- | --- |
| `--no-install` | Never run a package manager. Clone and copy only. |
| `--no-slow` | Skip `pod install`. It's reported in `needs_setup` with the exact command to run. |
| `--dry-run` | Show what would happen without changing anything. |
| `--from <dir>` | Hydrate from this checkout instead of the main one. |

### The first hydrate of an existing worktree

cmw only skips an install when it can prove which lockfile built the dependencies. For a worktree made by another tool, the first hydrate runs the package manager once. For npm on an up-to-date tree that's typically 1–2 s.

## Expo and React Native

Some Expo projects generate `ios/` and `android/` instead of committing them. When those folders are gitignored, cmw clones them and drops the build caches inside.

If `app.json`, `app.config.*` or `package.json` differ from main, the cloned native project may be out of date. cmw reports it as `stale_native` with the command to regenerate it.

## Post hooks

Commands listed under `[hydrate] post` in `.cmw.toml` run in the worktree after hydrate. See [`.cmw.toml`](/docs/config/#post-hooks).

## Reading the result

The human summary looks like this:

```
hydrated ~/.cmw/worktrees/shop/feat-login in 1.6s: 3 cloned, 2 copied, 4 skipped; installs: 1 up to date; ready
```

`ready` means nothing is left to do. Otherwise check `needs_setup` and `stale_native` in the [JSON output](/docs/json/#hydrate). Each entry includes the command that finishes the job.
