Set up cmw on this Mac for you and my other coding agents. cmw fills git worktrees with copy-on-write clones of node_modules, Pods, .env and other ignored files from the main checkout. Docs: https://cmw.equimper.com/docs/agents/

1. Run `cmw --version`. If cmw is missing, run `brew install equimper/tap/cmw`. If Homebrew is missing too, stop and ask me.
2. Run `cmw agents install --json`. It installs the cmw skill for every agent it finds (Claude Code, Codex, Grok). Then run `cmw agents --json` and check that each skill is `installed`.
3. If you have the T3 Code tools (t3_project_list, t3_project_read, t3_project_update), make every T3 project run `cmw hydrate` on each new worktree:
   - Call t3_project_list, then t3_project_read for each project.
   - Skip a project that already has a script with command `cmw hydrate` and `runOnWorktreeCreate: true`.
   - If it has a `cmw hydrate` script with `runOnWorktreeCreate` false or missing, set only that field to true.
   - Otherwise add this script:
     {"id": "cmw-hydrate", "name": "Hydrate worktree", "command": "cmw hydrate", "icon": "configure", "runOnWorktreeCreate": true}
   - `scripts` in t3_project_update replaces the whole list: send every existing script with all of its fields, unchanged apart from the edit above.
4. If you are running in the Codex app, tell me to open Settings → Environments and set each project's setup script to `cmw hydrate`. Don't edit Codex's config files yourself.
5. If ~/.claude exists, ask me whether Claude Code's own worktrees (`claude --worktree`) should go through cmw. They would live in ~/.cmw/worktrees, and one with uncommitted or unpushed work is kept when Claude removes it. Only if I say yes, run `cmw agents install --claude-hooks --json`.
6. If the current directory is a linked git worktree, run `cmw hydrate --json`, then run each `data.needs_setup[].command`. Exit 3 with "target is the main checkout" means there is nothing to hydrate here.
7. End with a short list of what you changed.
