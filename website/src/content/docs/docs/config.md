---
title: .cmw.toml
description: Add your own hydrate rules and post-hydrate commands, committed with the repo.
---

Put `.cmw.toml` at the root of your repo and commit it, so everyone and every agent gets the same behavior. It's optional.

```toml
[[rule]]
pattern = "ios/Secrets.xcconfig"   # glob vs repo-relative path
strategy = "copy"                  # clone | copy | regen | skip

[hydrate]
post = ["npm run codegen"]         # run in the worktree after hydrate
```

An unknown key or a bad pattern is a usage error (exit `2`), so typos don't pass silently.

## Rules

Each `[[rule]]` matches gitignored entries in the main checkout:

| Key | Required | Meaning |
| --- | --- | --- |
| `pattern` | yes | Glob matched against the repo-relative path, without a trailing slash. `**/` matches any depth. |
| `strategy` | yes | `clone`, `copy`, `regen` or `skip`. See [How hydrate works](/docs/hydrate/#four-strategies). |
| `dir` | no | `true` matches only folders, `false` only files. Omit to match both. |
| `reclaimable` | no | `true` lets `clean-artifacts` and `rm` delete matching entries. Default `false`. |

Your rules are checked before the [built-in rules](/docs/rules/), and the first match wins. That lets you override a default:

```toml
# Keep this build folder in worktrees instead of skipping it
[[rule]]
pattern = "packages/ui/build"
strategy = "clone"
dir = true

# Never copy this file
[[rule]]
pattern = "**/.env.production"
strategy = "skip"
```

Preview the effect with:

```sh
cmw hydrate --dry-run
```

## Post hooks

```toml
[hydrate]
post = ["npm run codegen", "npx prisma generate"]
```

Each command runs with `sh -c` in the worktree root, in order, after hydrate.

If a hook replaces a dependency folder (`node_modules`, `Pods`, `vendor`, `.venv`), cmw notices and reports the worktree as `needs_setup`. Patching individual files inside those folders goes unnoticed. If your hooks patch dependencies, re-run the package manager after them.
