---
title: JSON output
description: The JSON envelope, error kinds, exit codes and the fields agents rely on.
---

Pass `--json`, or set `CMW_JSON=1`, and every command prints exactly one JSON document on stdout.

## Envelope

Success:

```json
{ "ok": true, "command": "new", "data": { }, "warnings": [] }
```

Failure:

```json
{
  "ok": false,
  "command": "rm",
  "error": {
    "kind": "refused",
    "message": "1 uncommitted changes in ~/.cmw/worktrees/shop/feat-login",
    "hint": "cmw rm feat/login --force"
  },
  "warnings": []
}
```

`hint` is present when a single command resolves the error. Agents can show it verbatim.

## Exit codes

| Code | `error.kind` | Meaning |
| --- | --- | --- |
| `0` | — | Success. |
| `1` | `error` | Something failed: git, IO, an install. |
| `2` | `usage` | Bad arguments or an invalid `.cmw.toml`. |
| `3` | `refused` | Unsafe: dirty tree, unpushed commits, the main checkout. |
| `4` | `not_found` | Worktree, repo or file not found. |

Exit codes are stable across releases.

## `new`

| Field | Type | Meaning |
| --- | --- | --- |
| `path` | string | The worktree's absolute path. |
| `branch` | string | The branch checked out. |
| `created` | bool | `false` when the worktree already existed. |
| `base` | string \| null | The base ref, when the branch was created. |
| `hydrate` | object \| null | The hydrate report below. `null` with `--no-hydrate`. |

## `hydrate`

| Field | Meaning |
| --- | --- |
| `ready` | `true` when nothing is left to do. |
| `target`, `source` | Worktree and the checkout it was hydrated from. |
| `entries[]` | One per ignored entry: `path`, `strategy`, `ecosystem`, `action`. |
| `installs[]` | One per lockfile: `lockfile`, `dir`, `command`, `status` (`up_to_date`, `ran`, `would_run`, `needs_setup`, `failed`). |
| `needs_setup[]` | Work cmw didn't do: `dir`, `ecosystem`, `command`, `reason`. Run `command` in `dir`. |
| `stale_native[]` | Expo native projects that may be out of date: `dir`, `hint`. |
| `unmatched[]` | Ignored entries no rule matched. |
| `pruned[]` | Build caches removed from cloned native projects. |
| `duration_ms` | Wall time. |

`entries[].action` is one of `cloned`, `copied`, `regen`, `skipped`, `exists`, or `would_clone` / `would_copy` in a dry run.

## `ls`

`data.repos[].worktrees[]` includes `path`, `branch`, `is_main`, `dirty` (count), `ahead`, `behind`, `unpushed`, `merged` (`true`, `false` or `null` when unknown), `merged_via`, `merge_reason`, `idle_days`, `hydrated`, `locked`, `has_submodules` and `apparent_bytes` (with `--sizes`).

## `gc` and `clean-artifacts`

Both report what they would do in a dry run and what they did with `--yes`. With `--yes` they include `freed_bytes`, the change in free disk space. See [Disk usage](/docs/disk/).
