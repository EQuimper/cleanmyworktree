---
title: Built-in rules
description: The full list of patterns cmw clones, copies, rebuilds and skips.
---

cmw checks each gitignored entry of the main checkout against these rules, in order, after your [`.cmw.toml`](/docs/config/) rules. The first match wins. Patterns match the repo-relative path at any depth.

Entries marked **reclaimable** can be deleted by `cmw clean-artifacts` and `cmw rm`.

## Dependencies: clone

| Pattern | Ecosystem |
| --- | --- |
| `node_modules/` | node |
| `.yarn/cache/` | node |
| `.pnpm-store/` | node |
| `Pods/` | ios |
| `Carthage/` | ios |
| `vendor/` | php |
| `target/` | rust |
| `.turbo/` (reclaimable) | node |
| `.nx/cache/` (reclaimable) | node |

## Expo and React Native native projects: clone

| Pattern | Ecosystem |
| --- | --- |
| `ios/` | expo |
| `android/` | expo |

These only apply when the whole folder is gitignored, as in Expo's continuous native generation. Build caches inside them are dropped, and cmw warns `stale_native` when the app config or `package.json` differs from main.

## Agent config: clone folders, copy files

| Pattern | Strategy |
| --- | --- |
| `.claude/worktrees/` | skip |
| `.claude/`, `.agents/`, `.grok/` | clone |
| files under `.claude/`, `.agents/`, `.codex/`, `.cursor/`, `.grok/`, `.gemini/`, `.windsurf/` | copy |
| `CLAUDE.local.md`, `.mcp.json` | copy |

## Secrets and local config: copy

| Pattern | Ecosystem |
| --- | --- |
| `.env`, `.env.*`, `*.env`, `.envrc`, `.dev.vars`, `*.pem` | env |
| `.npmrc` | node |
| `auth.json` | php |
| `master.key` | ruby |
| `local.properties`, `google-services.json`, `*.keystore`, `*.jks` | android |
| `GoogleService-Info.plist`, `.xcode.env.local`, `*.xcconfig`, `*.p8`, `*.p12` | ios |

## Path-bound environments: regen

| Pattern | Ecosystem |
| --- | --- |
| `.venv/`, `venv/` | python |
| `.dart_tool/` | flutter |

## Build output and caches: skip

| Pattern | Ecosystem | Reclaimable |
| --- | --- | --- |
| `DerivedData/`, `.build/` | ios | yes |
| `xcuserdata/` | ios | no |
| `.cxx/`, `.gradle/`, `.kotlin/` | android | yes |
| `build/`, `coverage/` | build | yes |
| `dist/` | build | no |
| `.next/`, `.nuxt/`, `.svelte-kit/`, `.parcel-cache/`, `.vite/` | node | yes |
| `.expo/` | expo | yes |
| `.DS_Store`, `*.log` | build | no |

A trailing `/` means the rule only matches folders.
