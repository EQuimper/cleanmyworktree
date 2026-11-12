---
title: Secrets and .env
description: How .env files and machine-local config follow every worktree, and how to refresh them.
---

Ignored secrets and machine-local config are copied into every worktree when it's hydrated. Each worktree gets its own copy, so editing `.env` in one doesn't change the others.

Copied by default:

- `.env`, `.env.*`, `*.env`, `.envrc`, `.dev.vars`, `*.pem`
- `.npmrc` and Composer's `auth.json`
- Android: `local.properties`, `google-services.json`, `*.keystore`, `*.jks`
- iOS: `GoogleService-Info.plist`, `.xcode.env.local`, `*.xcconfig`, `*.p8`, `*.p12`
- Rails: `master.key`

Local agent config follows too, so every agent sees the same skills and settings: ignored `.claude` and `.agents` folders are cloned, and ignored files under `.codex`, `.cursor`, `.gemini` and `.windsurf` are copied, along with `CLAUDE.local.md` and `.mcp.json`. Claude Code's own `.claude/worktrees` is never cloned.

Only ignored files are copied. Anything tracked by git already comes with the checkout.

## Refresh after a change

When you change a secret in the main checkout, push it to a worktree:

```sh
cmw env sync                 # the current worktree
cmw env sync ~/.cmw/worktrees/shop/feat-login
```

| Status | Meaning |
| --- | --- |
| `copied` | The file was missing in the worktree. |
| `unchanged` | Already identical. |
| `kept` | The worktree's copy differs. cmw leaves it alone and warns. |
| `overwritten` | Replaced with main's copy (only with `--force`). |

Files you changed inside the worktree are kept unless you pass `--force`:

```sh
cmw env sync --dry-run   # preview
cmw env sync --force     # overwrite worktree copies with main's
```

## Copy something else

Add a `copy` rule to `.cmw.toml`:

```toml
[[rule]]
pattern = "ios/Secrets.xcconfig"
strategy = "copy"
```
