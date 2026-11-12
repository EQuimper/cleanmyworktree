## Worktrees (cmw)

- Already in a linked worktree (T3 Code, Codex app, `claude --worktree`)? Work there. If deps or `.env` are missing, run `cmw hydrate --json` and each `data.needs_setup[].command`. Don't `cmw new` or `cmw rm` from inside it.
- Starting isolated work yourself: `cmw new <branch> --json`; use `data.path`, and run `data.hydrate.needs_setup[].command` if `data.hydrate.ready` is false.
- Other commands: `cmw ls`, `cmw path <name>`, `cmw env sync`, `cmw rm <name>`, `cmw gc` (dry run) / `cmw gc --yes`.
- Output: `{ok, command, data, warnings}` or `{ok: false, error: {kind, message, hint}}`. Exit codes: 0 ok, 1 error, 2 usage, 3 refused, 4 not found.
- On refusal, show the `hint` instead of running it. Never use `rm --force` unless the user asked to discard work. Prefer `-C <dir>` over cd.
