// The landing page playground: a pretend shell over a pretend repo. Nothing touches the visitor's
// machine. Output follows the real `human()` and `--json` formats in src/*.rs; help text and
// static output come from cli-snapshot.json, which `cargo test` keeps in sync with the binary.
// Paths print as `~/…` where the real binary prints them absolute.

import snapshot from './cli-snapshot.json';

/** A run of text. `c` is a CSS class; `k` makes it a clickable key in the interactive screen. */
export type Seg = { t: string; c?: string; k?: string };
export type Line = Seg[];

/** What running one input line does, in order. The UI plays these back. */
export type Ev =
  | { out: Line; err?: boolean }
  | { wait: number }
  | { tui: true }
  | { clear: true }
  | { copy: string };

const KB = 1024;
const MB = KB ** 2;
const GB = KB ** 3;
const DAY = 86_400;

export const HOME = '/Users/you';

const help = snapshot.help as Record<string, { long: string; short?: string }>;
const output = snapshot.output as Record<string, string>;
export const agentsMd = output['skill print --agents-md'];

// ---------------------------------------------------------------------------------------------
// Text helpers

export const seg = (t: string, c?: string): Seg => (c ? { t, c } : { t });
const line = (t: string, c?: string): Line => [seg(t, c)];
const textLines = (text: string, c?: string): Line[] => text.split('\n').map((l) => line(l, c));

export function humanBytes(bytes: number) {
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let v = bytes;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return i === 0 ? `${bytes} B` : `${v.toFixed(1)} ${units[i]}`;
}

export const pad = (s: string, w: number) => truncate(s, w).padEnd(w);
export const rpad = (s: string, w: number) => truncate(s, w).padStart(w);

export function truncate(s: string, w: number) {
  const chars = [...s];
  if (chars.length <= w) return s;
  if (w <= 1) return '…'.slice(0, w);
  return chars.slice(0, w - 1).join('') + '…';
}

/** Columns separated by two spaces, the last one unpadded (inventory.rs `table`). */
function table(rows: string[][]): string[] {
  const widths = rows[0].map((_, i) => Math.max(...rows.map((r) => r[i].length)));
  return rows.map((r) => r.map((cell, i) => (i + 1 === r.length ? cell : cell.padEnd(widths[i]))).join('  '));
}

/** Pretty JSON with literals highlighted, like the snippets elsewhere on the page. */
function json(value: unknown): Line[] {
  return JSON.stringify(value, null, 2)
    .split('\n')
    .map((l) => {
      const m = l.match(/^(\s*(?:"[^"]*": )?)(true|false|null|-?\d[\d.e+-]*)(,?)$/);
      return m ? [seg(m[1]), seg(m[2], 'lit'), seg(m[3])] : [seg(l)];
    });
}

export const expand = (p: string) => (p === '~' ? HOME : p.startsWith('~/') ? HOME + p.slice(1) : p);
export const tilde = (p: string) => (p === HOME ? '~' : p.startsWith(`${HOME}/`) ? `~${p.slice(HOME.length)}` : p);

/** git::slugify: one lowercase path segment per branch. */
export const slugify = (branch: string) =>
  branch
    .toLowerCase()
    .replace(/[^a-z0-9._]+/g, '-')
    .replace(/^-+|-+$/g, '');

function shQuote(s: string) {
  return /^[A-Za-z0-9_./~:@%+=-]+$/.test(s) ? s : `'${s.replace(/'/g, `'\\''`)}'`;
}

// ---------------------------------------------------------------------------------------------
// The pretend machine

interface Cache {
  path: string;
  bytes: number;
}

export interface Wt {
  repo: string;
  branch: string;
  path: string;
  main: boolean;
  dirty: number;
  unpushed: number;
  /** How the branch is proven merged, or null. */
  via: 'ancestor' | 'patch_id' | 'gh' | null;
  reason: string;
  idle: number;
  /** cmw hydrated it. */
  hydrated: boolean;
  /** Dependencies are present, however they got there. */
  deps: boolean;
  /** Apparent size: clones counted at full size. */
  bytes: number;
  /** Blocks not shared with the main checkout: what removing it frees. */
  own: number;
  caches: Cache[];
  /** .env.local contents, or null when missing. */
  env: string[] | null;
  head: string;
}

interface Entry {
  path: string;
  strategy: 'clone' | 'copy' | 'regen' | 'skip';
  ecosystem: string;
}

interface Repo {
  name: string;
  main: string;
  entries: Entry[];
  lockfile: string;
  install: string;
}

const REPOS: Repo[] = [
  {
    name: 'shop',
    main: '~/code/shop',
    entries: [
      { path: '.env.local', strategy: 'copy', ecosystem: 'env' },
      { path: '.expo/', strategy: 'skip', ecosystem: 'expo' },
      { path: '.next/', strategy: 'skip', ecosystem: 'node' },
      { path: '.turbo/', strategy: 'clone', ecosystem: 'node' },
      { path: 'android/.gradle/', strategy: 'skip', ecosystem: 'android' },
      { path: 'android/app/build/', strategy: 'skip', ecosystem: 'android' },
      { path: 'android/local.properties', strategy: 'copy', ecosystem: 'android' },
      { path: 'ios/Pods/', strategy: 'clone', ecosystem: 'ios' },
      { path: 'node_modules/', strategy: 'clone', ecosystem: 'node' },
    ],
    lockfile: 'package-lock.json',
    install: 'npm install --no-audit --no-fund',
  },
  {
    name: 'api',
    main: '~/code/api',
    entries: [
      { path: '.env', strategy: 'copy', ecosystem: 'env' },
      { path: '.venv/', strategy: 'regen', ecosystem: 'python' },
      { path: '__pycache__/', strategy: 'skip', ecosystem: 'python' },
    ],
    lockfile: 'uv.lock',
    install: 'uv sync',
  },
];

const ENV = ['API_URL=http://localhost:3000', 'MAPS_KEY=dev-only'];
const DERIVED = '~/Library/Developer/Xcode/DerivedData/Shop-fqkzmdbtwyhlcd';

function wt(repo: string, branch: string, o: Partial<Wt> = {}): Wt {
  return {
    repo,
    branch,
    path: `~/.cmw/worktrees/${repo}/${slugify(branch)}`,
    main: false,
    dirty: 0,
    unpushed: 0,
    via: null,
    reason: 'no_commits',
    idle: 0,
    hydrated: true,
    deps: true,
    bytes: 1.52 * GB,
    own: 18 * MB,
    caches: [],
    env: [...ENV],
    head: 'c41d7e2',
    ...o,
  };
}

function seed(): Wt[] {
  return [
    wt('shop', 'main', {
      path: '~/code/shop',
      main: true,
      reason: 'main checkout',
      hydrated: false,
      bytes: 1.94 * GB,
      head: '9c8787e',
    }),
    wt('shop', 'chore/deps', {
      via: 'patch_id',
      reason: 'every commit has an equivalent patch in origin/main',
      idle: 3,
      own: 41 * MB,
      head: '5b0e9a1',
    }),
    wt('shop', 'feat/onboarding', {
      unpushed: 3,
      reason: 'not merged into origin/main',
      idle: 12,
      bytes: 4.9 * GB,
      own: 3.4 * GB,
      caches: [
        { path: DERIVED, bytes: 2.31 * GB },
        { path: 'android/app/.cxx', bytes: 642 * MB },
        { path: 'android/app/build', bytes: 418 * MB },
      ],
      head: 'e07f3c4',
    }),
    wt('shop', 'fix/checkout', { dirty: 2, reason: 'no_commits', head: '9c8787e' }),
    wt('shop', 'spike/search', {
      via: 'ancestor',
      reason: 'HEAD is an ancestor of origin/main',
      idle: 9,
      bytes: 1.73 * GB,
      own: 236 * MB,
      caches: [{ path: '.next', bytes: 212 * MB }],
      head: 'a93d1f0',
    }),
    wt('api', 'main', {
      path: '~/code/api',
      main: true,
      reason: 'main checkout',
      hydrated: false,
      bytes: 412 * MB,
      env: ['DATABASE_URL=postgres://localhost/api'],
      head: '1f4b2aa',
    }),
    wt('api', 't3/7c1e04b2', {
      path: '~/.t3/worktrees/api/t3-7c1e04b2',
      reason: 'not merged into origin/main',
      idle: 1,
      bytes: 380 * MB,
      env: ['DATABASE_URL=postgres://localhost/api'],
      head: '62de0b9',
    }),
  ];
}

const within = (p: string, dir: string) => p === dir || p.startsWith(`${dir}/`);

// ---------------------------------------------------------------------------------------------
// Errors

class CmwError {
  constructor(
    readonly kind: 'error' | 'usage' | 'refused' | 'not_found',
    readonly message: string,
    readonly hint?: string,
  ) {}
  get code() {
    return { error: 1, usage: 2, refused: 3, not_found: 4 }[this.kind];
  }
}

/** A clap parse error: printed as clap prints it, exit 2. */
class UsageError {
  constructor(
    readonly message: string,
    readonly usage?: string,
  ) {}
}

// ---------------------------------------------------------------------------------------------
// Argument parsing, shaped like src/cli.rs

interface Spec {
  flags?: string[];
  opts?: string[];
  pos?: string[];
  required?: boolean;
}

const SPECS: Record<string, Spec> = {
  new: { flags: ['--fetch', '--no-hydrate', '--no-install', '--no-slow'], opts: ['--base', '--path'], pos: ['<BRANCH>'], required: true },
  hydrate: { flags: ['--dry-run', '--no-install', '--no-slow'], opts: ['--from'], pos: ['[PATH]'] },
  ls: { flags: ['--all', '--sizes'] },
  path: { pos: ['<NAME>'], required: true },
  rm: { flags: ['--force', '--keep-branch', '--dry-run'], pos: ['<NAME>'], required: true },
  gc: { flags: ['--yes', '--all'], opts: ['--stale'] },
  'clean-artifacts': { flags: ['--yes', '--all'], opts: ['--idle'] },
  'env sync': { flags: ['--force', '--dry-run'], pos: ['[PATH]'] },
  'shell-init': { pos: ['<SHELL>'], required: true },
  agents: {},
  'agents install': { flags: ['--claude-hooks', '--refresh'], opts: ['--agent'] },
  'agents prompt': { flags: ['--copy'] },
  'skill install': { opts: ['--dir'] },
  'skill print': { flags: ['--agents-md'] },
  update: { flags: ['--check'] },
  'hook claude': { flags: ['--install', '--project'] },
};

/** Commands that need a subcommand: bare, they print their help and exit 2. */
const GROUPS: Record<string, string[]> = {
  env: ['sync'],
  agents: ['install', 'prompt'],
  skill: ['install', 'print'],
  hook: ['claude'],
};

export const COMMANDS = ['new', 'hydrate', 'ls', 'path', 'rm', 'gc', 'clean-artifacts', 'env', 'shell-init', 'agents', 'skill', 'update', 'hook', 'help'];

interface Parsed {
  cmd: string;
  flags: Set<string>;
  opts: Map<string, string>;
  pos: string[];
}

const usageOf = (cmd: string) => help[cmd]?.long.match(/^Usage: .*$/m)?.[0].slice('Usage: '.length) ?? 'cmw [OPTIONS] [COMMAND]';

function parseSub(cmd: string, args: string[]): Parsed {
  const spec = SPECS[cmd];
  const p: Parsed = { cmd, flags: new Set(), opts: new Map(), pos: [] };
  for (let i = 0; i < args.length; i++) {
    const a = args[i];
    const [name, inline] = a.startsWith('--') && a.includes('=') ? [a.slice(0, a.indexOf('=')), a.slice(a.indexOf('=') + 1)] : [a, undefined];
    if (spec.flags?.includes(name)) p.flags.add(name);
    else if (spec.opts?.includes(name)) {
      const v = inline ?? args[++i];
      if (v === undefined) {
        const meta = name.slice(2).toUpperCase().replace(/-/g, '_');
        throw new UsageError(`a value is required for '${name} <${meta}>' but none was supplied`);
      }
      p.opts.set(name, v);
    } else if (a.startsWith('-') && a !== '-') {
      throw new UsageError(`unexpected argument '${a}' found`, usageOf(cmd));
    } else if (p.pos.length < (spec.pos?.length ?? 0)) p.pos.push(a);
    else throw new UsageError(`unexpected argument '${a}' found`, usageOf(cmd));
  }
  for (const name of ['--idle', '--stale']) {
    const v = p.opts.get(name);
    if (v !== undefined && !/^\d+$/.test(v)) throw new UsageError(`invalid value '${v}' for '${name} <DAYS>': invalid digit found in string`);
  }
  if (cmd === 'shell-init' && p.pos[0] && !['zsh', 'bash', 'fish'].includes(p.pos[0])) {
    throw new UsageError(`invalid value '${p.pos[0]}' for '<SHELL>'\n  [possible values: zsh, bash, fish]`);
  }
  if (spec.required && p.pos.length === 0) {
    throw new UsageError(`the following required arguments were not provided:\n  ${spec.pos![0]}`, `cmw ${cmd} ${spec.pos![0]}`);
  }
  return p;
}

// ---------------------------------------------------------------------------------------------
// Shell

interface Output {
  evs: Ev[];
  code: number;
}

export class Sim {
  cwd = '~/code/shop';
  prev = '~/code/shop';
  last = 0;
  wts: Wt[] = seed();
  /** Branches without a worktree. */
  branches = new Set(['feat/payments']);
  grok = false;
  hooks = false;

  constructor(
    readonly version: string,
    readonly prompt: string,
  ) {}

  // -- repo lookups ----------------------------------------------------------------------------

  repoAt(dir: string): Repo | undefined {
    const w = this.wtAt(dir);
    return w && REPOS.find((r) => r.name === w.repo);
  }

  wtAt(dir: string): Wt | undefined {
    return this.wts
      .filter((w) => within(dir, w.path))
      .sort((a, b) => b.path.length - a.path.length)[0];
  }

  mainOf(repo: string) {
    return this.wts.find((w) => w.repo === repo && w.main)!;
  }

  wtsOf(repo: string) {
    const main = this.mainOf(repo);
    const linked = this.wts.filter((w) => w.repo === repo && !w.main).sort((a, b) => slugify(a.branch).localeCompare(slugify(b.branch)));
    return [main, ...linked];
  }

  /** git::resolve_worktree: branch, slug, directory name or path. */
  resolve(cwd: string, name: string): Wt | undefined {
    const repo = this.repoAt(cwd);
    if (!repo) return undefined;
    const path = this.path(cwd, name);
    return this.wtsOf(repo.name).find(
      (w) => w.branch === name || slugify(w.branch) === name || w.path.split('/').pop() === name || w.path === path,
    );
  }

  needRepo(cwd: string): Repo {
    const repo = this.repoAt(cwd);
    if (!repo) throw new CmwError('not_found', `not inside a git repository: ${expand(cwd)}`);
    return repo;
  }

  // -- filesystem -------------------------------------------------------------------------------

  /** Resolve a path argument against `cwd`, in `~` form. */
  path(cwd: string, arg: string): string {
    let p = arg;
    if (p.startsWith(HOME)) p = tilde(p);
    let parts: string[];
    if (p === '~' || p.startsWith('~/')) parts = p.split('/');
    else if (p.startsWith('/')) parts = ['', ...p.slice(1).split('/')];
    else parts = [...cwd.split('/'), ...p.split('/')];
    const out: string[] = [];
    for (const part of parts) {
      if (part === '' && out.length > 0) continue;
      if (part === '.') continue;
      if (part === '..') {
        if (out.length === 1 && out[0] === '~') out.splice(0, 1, '', 'Users');
        else if (out.length > 1) out.pop();
        continue;
      }
      out.push(part);
    }
    const joined = out.join('/') || '/';
    return tilde(expand(joined));
  }

  dirs(): Set<string> {
    const d = new Set(['~', '~/code', '~/.cmw', '~/.cmw/worktrees', '~/.t3', '~/.t3/worktrees', '/', '/Users']);
    d.add(HOME);
    for (const w of this.wts) {
      for (let p = w.path; p.includes('/'); p = p.slice(0, p.lastIndexOf('/'))) d.add(p);
      for (const sub of this.subdirs(w)) d.add(`${w.path}/${sub}`);
    }
    return d;
  }

  subdirs(w: Wt) {
    const s = w.repo === 'shop' ? ['android', 'ios', 'src'] : ['app', 'tests'];
    if (w.deps) s.push(w.repo === 'shop' ? 'node_modules' : '.venv');
    return s;
  }

  isDir(p: string) {
    return this.dirs().has(p);
  }

  children(dir: string, all: boolean): string[] {
    const w = this.wts.find((x) => x.path === dir);
    if (w) {
      const files = w.repo === 'shop' ? ['README.md', 'app.json', 'package-lock.json', 'package.json'] : ['README.md', 'pyproject.toml', 'uv.lock'];
      const hidden = all ? ['.git', '.gitignore', ...(w.env ? [w.repo === 'shop' ? '.env.local' : '.env'] : [])] : [];
      if (all && w.main && w.repo === 'shop') hidden.push('.next', '.turbo');
      return [...hidden, ...this.subdirs(w), ...files].sort();
    }
    const kids = [...this.dirs()].filter((p) => p.startsWith(`${dir === '/' ? '' : dir}/`) && !p.slice(dir.length + 1).includes('/') && p !== dir);
    return kids
      .map((p) => p.slice(p.lastIndexOf('/') + 1))
      .filter((n) => all || !n.startsWith('.'))
      .sort();
  }

  // -- entry points ------------------------------------------------------------------------------

  /** Run one line of input. */
  exec(input: string): Ev[] {
    const evs: Ev[] = [];
    for (const { op, cmd } of splitChain(input)) {
      if ((op === '&&' && this.last !== 0) || (op === '||' && this.last === 0)) continue;
      const r = this.run(cmd, false);
      evs.push(...r.evs);
      this.last = r.code;
      if (r.evs.some((e) => 'tui' in e)) break;
    }
    return evs;
  }

  /** One simple command, after `$(…)` and `$?` expansion. */
  run(raw: string, captured: boolean): Output {
    const evs: Ev[] = [];
    const expanded = raw.replace(/\$\(([^()]*)\)/g, (_, inner: string) => {
      const r = this.run(inner, true);
      this.last = r.code;
      for (const e of r.evs) if ('wait' in e) evs.push(e);
      for (const e of r.evs) if ('out' in e && e.err) evs.push(e);
      return r.evs
        .filter((e): e is { out: Line } => 'out' in e && !e.err)
        .map((e) => e.out.map((s) => s.t).join(''))
        .join('\n')
        .trimEnd();
    });
    const words = tokenize(expanded.replace(/\$\?/g, String(this.last)));
    if (!words) return { evs: [...evs, { out: line('zsh: unmatched quote', 'err'), err: true }], code: 1 };
    const env: Record<string, string> = {};
    while (words.length && /^[A-Z_][A-Z0-9_]*=/.test(words[0])) {
      const [k, ...v] = words.shift()!.split('=');
      env[k] = v.join('=');
    }
    if (words.length === 0) return { evs, code: 0 };
    const r = this.command(words, env, captured);
    return { evs: [...evs, ...r.evs], code: r.code };
  }

  private command(words: string[], env: Record<string, string>, captured: boolean): Output {
    const [name, ...args] = words;
    const out = (text: string, c?: string): Output => ({ evs: textLines(text, c).map((l) => ({ out: l })), code: 0 });
    const fail = (text: string, code = 1): Output => ({ evs: textLines(text).map((l) => ({ out: l, err: true })), code });
    switch (name) {
      case 'cmw': {
        if (args[0] !== 'cd') return this.cmw(args, env, captured);
        // The shell-init function: `p="$(command cmw path "$@")" || return $?; builtin cd -- "$p"`.
        const r = this.cmw(['path', ...args.slice(1)], env, true);
        if (r.code !== 0) return r;
        const dest = r.evs.flatMap((e) => ('out' in e && !e.err ? [e.out.map((x) => x.t).join('')] : [])).pop()!;
        this.prev = this.cwd;
        this.cwd = dest;
        return { evs: [], code: 0 };
      }
      case 'cd': {
        const target = args[0] === undefined ? '~' : args[0] === '-' ? this.prev : this.path(this.cwd, args[0]);
        if (!this.isDir(target)) return fail(`cd: no such file or directory: ${args[0]}`);
        this.prev = this.cwd;
        this.cwd = target;
        return { evs: [], code: 0 };
      }
      case 'pwd':
        return out(expand(this.cwd));
      case 'ls': {
        const all = args.some((a) => /^-[a-zA-Z]*a/i.test(a));
        const target = args.find((a) => !a.startsWith('-'));
        const dir = target ? this.path(this.cwd, target) : this.cwd;
        if (!this.isDir(dir)) return fail(`ls: ${target}: No such file or directory`);
        const kids = this.children(dir, all);
        return { evs: kids.length ? [{ out: kids.map((k, i) => seg(i ? `  ${k}` : k, this.isDir(`${dir}/${k}`) ? 'dir' : undefined)) }] : [], code: 0 };
      }
      case 'cat': {
        const file = args[0] ?? '';
        const p = this.path(this.cwd, file);
        const w = this.wtAt(p);
        const envName = w?.repo === 'shop' ? '.env.local' : '.env';
        if (w && p === `${w.path}/${envName}` && w.env) return out(w.env.join('\n'));
        return fail(`cat: ${file}: No such file or directory`);
      }
      case 'echo': {
        const at = args.indexOf('>>');
        if (at === -1) return out(args.join(' '));
        const file = args[at + 1] ?? '';
        const p = this.path(this.cwd, file);
        const w = this.wtAt(p);
        const envName = w?.repo === 'shop' ? '.env.local' : '.env';
        if (!w || p !== `${w.path}/${envName}`) return fail(`echo: only ${envName} is writable here`);
        w.env = [...(w.env ?? []), args.slice(0, at).join(' ')];
        return { evs: [], code: 0 };
      }
      case 'clear':
        return { evs: [{ clear: true }], code: 0 };
      case 'git':
        return this.git(args);
      case 'npm':
        return this.npm(args);
      case 'brew':
        if (args[0] === 'install' && args[1]?.endsWith('cmw')) {
          return out(`Warning: equimper/tap/cmw ${this.version} is already installed and up-to-date.`, 'warn');
        }
        return fail(`brew: only \`brew install equimper/tap/cmw\` works here`);
      case 'help':
        return out(
          [
            'A pretend shell over a pretend repo. Nothing runs on your machine.',
            '',
            '  cmw …                 every cmw command, with --json and --help',
            '  cmw                   the interactive screen',
            '  cd, ls, pwd, cat      move around and look',
            '  echo X >> .env.local  edit a secret, then try cmw env sync',
            '  git worktree add      make a worktree the plain way, then cmw hydrate',
            '  git status, git branch, npm install, clear',
            '',
            'Tab completes, ↑ ↓ walk history, && and $(…) work.',
          ].join('\n'),
        );
      default:
        return fail(`zsh: command not found: ${name}`, 127);
    }
  }

  // -- git, npm --------------------------------------------------------------------------------

  private git(args: string[]): Output {
    const fail = (text: string, code = 128): Output => ({ evs: textLines(text).map((l) => ({ out: l, err: true })), code });
    const here = this.wtAt(this.cwd);
    if (!here) return fail('fatal: not a git repository (or any of the parent directories): .git');
    const out = (lines: Line[]): Output => ({ evs: lines.map((l) => ({ out: l })), code: 0 });
    const [sub, ...rest] = args;
    if (sub === 'status') {
      const head = [line(`On branch ${here.branch}`)];
      if (here.dirty === 0) return out([...head, line('nothing to commit, working tree clean')]);
      return out([
        ...head,
        line('Changes not staged for commit:'),
        line('	modified:   src/checkout/total.ts', 'err'),
        line(''),
        line('Untracked files:'),
        line('	src/checkout/coupon.ts', 'err'),
      ]);
    }
    if (sub === 'branch') {
      const repo = here.repo;
      const names = [...new Set([...this.wts.filter((w) => w.repo === repo).map((w) => w.branch), ...(repo === 'shop' ? this.branches : [])])].sort();
      return out(
        names.map((b) => {
          const w = this.wts.find((x) => x.repo === repo && x.branch === b);
          if (w === here) return [seg('* '), seg(b, 'ok')];
          return w ? [seg('+ '), seg(b, 'info')] : [seg(`  ${b}`)];
        }),
      );
    }
    if (sub === 'worktree' && rest[0] === 'list') {
      const ws = this.wtsOf(here.repo);
      const w = Math.max(...ws.map((x) => expand(x.path).length));
      return out(ws.map((x) => line(`${expand(x.path).padEnd(w)} ${x.head} [${x.branch}]`)));
    }
    if (sub === 'worktree' && rest[0] === 'add') {
      const a = rest.slice(1);
      const b = a.indexOf('-b');
      const branch = b === -1 ? undefined : a[b + 1];
      const pos = a.filter((x, i) => !x.startsWith('-') && (b === -1 || (i !== b + 1)));
      if (!pos[0]) return fail('usage: git worktree add [-b <new-branch>] <path> [<commit-ish>]', 129);
      const path = this.path(this.cwd, pos[0]);
      const name = branch ?? path.split('/').pop()!;
      if (this.isDir(path)) return fail(`fatal: '${pos[0]}' already exists`);
      if (this.wts.some((x) => x.repo === here.repo && x.branch === name)) {
        return fail(`fatal: '${name}' is already used by worktree at '${expand(this.wts.find((x) => x.branch === name)!.path)}'`);
      }
      const known = this.branches.has(name) || this.wts.some((x) => x.repo === here.repo && x.branch === name);
      if (branch && known) return fail(`fatal: a branch named '${name}' already exists`);
      this.branches.delete(name);
      this.wts.push(wt(here.repo, name, { path, hydrated: false, deps: false, env: null, bytes: 6.2 * MB, own: 6.2 * MB, head: this.mainOf(here.repo).head }));
      return out([line(known ? `Preparing worktree (checking out '${name}')` : `Preparing worktree (new branch '${name}')`), line(`HEAD is now at ${this.mainOf(here.repo).head} Add checkout flow`)]);
    }
    if (sub === 'log') {
      return out([
        [seg(here.head, 'warn'), seg(` (HEAD -> ${here.branch}) `), seg(here.main ? 'Add checkout flow' : 'WIP')],
        [seg('3f2a1b9', 'warn'), seg(' Cache product images')],
        [seg('d81c0e4', 'warn'), seg(' Initial commit')],
      ]);
    }
    return fail(`git: '${sub ?? ''}' isn't available here. Try git status, branch, log or worktree.`, 1);
  }

  private npm(args: string[]): Output {
    const here = this.wtAt(this.cwd);
    if (!['install', 'i', 'ci'].includes(args[0] ?? '')) {
      return { evs: [{ out: line('npm: only `npm install` works here', 'err'), err: true }], code: 1 };
    }
    if (!here || here.repo !== 'shop' || this.cwd !== here.path) {
      return { evs: [{ out: line(`npm error enoent Could not read package.json: ${expand(this.cwd)}/package.json`, 'err'), err: true }], code: 254 };
    }
    if (here.deps) {
      return { evs: [{ wait: 900 }, { out: line('up to date, audited 1413 packages in 1s') }, { out: line('') }, { out: line('found 0 vulnerabilities') }], code: 0 };
    }
    here.deps = true;
    here.bytes += 1.4 * GB;
    here.own += 1.4 * GB;
    return {
      evs: [{ wait: 2600 }, { out: line('added 1412 packages, and audited 1413 packages in 2m') }, { out: line('') }, { out: line('found 0 vulnerabilities') }],
      code: 0,
    };
  }

  // -- cmw --------------------------------------------------------------------------------------

  private cmw(argv: string[], env: Record<string, string>, captured: boolean): Output {
    let jsonMode = ['1', 'true', 'yes', 'on'].includes((env.CMW_JSON ?? '').toLowerCase());
    let cwd = this.cwd;
    const rest: string[] = [];
    let wantHelp: 'long' | 'short' | null = null;
    for (let i = 0; i < argv.length; i++) {
      const a = argv[i];
      if (a === '--json') jsonMode = true;
      else if (a === '-C') {
        const dir = argv[++i];
        if (dir === undefined) return this.usageOut(new UsageError(`a value is required for '-C <DIR>' but none was supplied`), jsonMode);
        cwd = this.path(cwd, dir);
      } else if (a === '--help') wantHelp ??= 'long';
      else if (a === '-h') wantHelp ??= 'short';
      else rest.push(a);
    }

    const [first, second] = rest;
    if (rest.length === 0) {
      if (wantHelp) return this.print(wantHelp === 'short' ? help[''].short! : help[''].long);
      if (jsonMode || captured) return { evs: textLines(help[''].long).map((l) => ({ out: l, err: true })), code: 2 };
      return { evs: [{ tui: true }], code: 0 };
    }
    if (first === '-V' || first === '--version') return this.print(`cmw ${this.version}`);
    if (first === 'help') {
      const key = rest.slice(1).join(' ');
      if (help[key]) return this.print(help[key].long);
      return this.usageOut(new UsageError(`unrecognized subcommand '${rest[1]}'`, 'cmw [OPTIONS] [COMMAND]'), jsonMode);
    }
    if (!COMMANDS.includes(first)) {
      if (first.startsWith('-')) return this.usageOut(new UsageError(`unexpected argument '${first}' found`, 'cmw [OPTIONS] [COMMAND]'), jsonMode);
      return this.usageOut(new UsageError(`unrecognized subcommand '${first}'`, 'cmw [OPTIONS] [COMMAND]'), jsonMode);
    }

    let cmd = first;
    let args = rest.slice(1);
    if (GROUPS[first]) {
      if (second && GROUPS[first].includes(second)) {
        cmd = `${first} ${second}`;
        args = rest.slice(2);
      } else if (second && !second.startsWith('-')) {
        return this.usageOut(new UsageError(`unrecognized subcommand '${second}'`, usageOf(first)), jsonMode);
      } else if (first !== 'agents' && !wantHelp) {
        return { evs: textLines(help[first].long).map((l) => ({ out: l, err: true })), code: 2 };
      }
    }
    if (wantHelp) {
      const h = help[cmd] ?? help[''];
      return this.print(wantHelp === 'short' ? (h.short ?? h.long) : h.long);
    }

    let parsed: Parsed;
    try {
      parsed = parseSub(cmd, args);
    } catch (e) {
      if (e instanceof UsageError) return this.usageOut(e, jsonMode);
      throw e;
    }

    const name = cmd;
    const warnings: string[] = [];
    try {
      const r = this.dispatch(parsed, cwd, warnings);
      const evs: Ev[] = r.wait ? [{ wait: r.wait }] : [];
      if (r.copy) evs.push({ copy: r.copy });
      if (jsonMode) {
        evs.push(...json({ ok: true, command: name, data: r.data, warnings }).map((l) => ({ out: l })));
      } else {
        evs.push(...warnings.map((w) => ({ out: [seg('warning:', 'warn'), seg(` ${w}`)], err: true })));
        evs.push(...(r.stderr ?? []).map((l) => ({ out: l, err: true })));
        evs.push(...r.human.map((l) => ({ out: l })));
      }
      return { evs, code: 0 };
    } catch (e) {
      if (!(e instanceof CmwError)) throw e;
      if (jsonMode) {
        const error = e.hint ? { kind: e.kind, message: e.message, hint: e.hint } : { kind: e.kind, message: e.message };
        return { evs: json({ ok: false, command: name, error, warnings }).map((l) => ({ out: l })), code: e.code };
      }
      const evs: Ev[] = [{ out: [seg('error:', 'err'), seg(` ${e.message}`)], err: true }];
      if (e.hint) evs.push({ out: [seg('  fix: ', 'dim'), seg(e.hint, 'cmd')], err: true });
      return { evs, code: e.code };
    }
  }

  private print(text: string): Output {
    return { evs: textLines(text).map((l) => ({ out: l })), code: 0 };
  }

  private usageOut(e: UsageError, jsonMode: boolean): Output {
    if (jsonMode) {
      const message = e.message.split('\n')[0];
      return {
        evs: json({ ok: false, command: 'cmw', error: { kind: 'usage', message, hint: 'cmw --help' }, warnings: [] }).map((l) => ({ out: l })),
        code: 2,
      };
    }
    const text = [`error: ${e.message}`, '', ...(e.usage ? [`Usage: ${e.usage}`, ''] : []), "For more information, try '--help'."];
    return {
      evs: text.flatMap((t) => textLines(t)).map((l, i) => ({ out: i === 0 ? [seg('error:', 'err'), seg(l[0].t.slice(6))] : l, err: true })),
      code: 2,
    };
  }

  private dispatch(p: Parsed, cwd: string, warnings: string[]): { human: Line[]; data: unknown; stderr?: Line[]; wait?: number; copy?: string } {
    switch (p.cmd) {
      case 'new':
        return this.new(p, cwd);
      case 'hydrate':
        return this.hydrateCmd(p, cwd);
      case 'ls':
        return this.ls(p, cwd);
      case 'path': {
        this.needRepo(cwd);
        const w = this.resolve(cwd, p.pos[0]);
        if (!w) throw new CmwError('not_found', `no worktree matches '${p.pos[0]}'`, 'cmw ls');
        return { human: [line(w.path)], data: { path: w.path, branch: w.branch } };
      }
      case 'rm':
        return this.rm(p, cwd);
      case 'gc':
        return this.gc(p, cwd);
      case 'clean-artifacts':
        return this.cleanArtifacts(p, cwd);
      case 'env sync':
        return this.envSync(p, cwd, warnings);
      case 'shell-init': {
        const sh = p.pos[0];
        return { human: textLines(output[`shell-init ${sh}`]), data: { shell: sh, script: output[`shell-init ${sh}`] } };
      }
      case 'agents':
        return this.agents();
      case 'agents install':
      case 'skill install':
        return this.agentsInstall(p);
      case 'agents prompt':
        if (p.flags.has('--copy')) {
          return { human: [line('copied the setup prompt; paste it into Claude Code, Codex, Grok or T3 Code')], data: { copied: true }, copy: this.prompt };
        }
        return { human: textLines(this.prompt), data: { text: this.prompt } };
      case 'skill print': {
        const text = output[p.flags.has('--agents-md') ? 'skill print --agents-md' : 'skill print'];
        return { human: textLines(text), data: { text } };
      }
      case 'update':
        return {
          human: [line(`cmw ${this.version} is up to date`)],
          data: { current: this.version, latest: this.version, update_available: false, method: 'homebrew', command: 'brew upgrade equimper/tap/cmw', updated: false },
          wait: 500,
        };
      case 'hook claude':
        if (p.flags.has('--install')) {
          const status = this.hooks ? 'unchanged' : 'added';
          this.hooks = true;
          const path = p.flags.has('--project') ? '~/code/shop/.claude/settings.json' : '~/.claude/settings.json';
          return { human: [line(`${status} ${path}`)], data: { path, status } };
        }
        return { human: textLines(output['hook claude']), data: JSON.parse(output['hook claude']) };
    }
    throw new CmwError('error', `${p.cmd}: not available in the playground`);
  }

  // -- hydrate ----------------------------------------------------------------------------------

  /** hydrate::hydrate, against the pretend tree. Mutates `w` unless `dry`. */
  hydrate(w: Wt, dry: boolean, made = false) {
    const repo = REPOS.find((r) => r.name === w.repo)!;
    const main = this.mainOf(w.repo);
    const fresh = !w.hydrated;
    const entries = repo.entries.map((e) => {
      let action: string;
      if (e.strategy === 'skip') action = 'skipped';
      else if (e.strategy === 'regen') action = 'regen';
      else if (e.strategy === 'copy') action = w.env && !fresh ? 'exists' : dry ? 'would_copy' : 'copied';
      else action = w.deps ? 'exists' : dry ? 'would_clone' : 'cloned';
      return { path: e.path, strategy: e.strategy, ecosystem: e.ecosystem, action };
    });
    // The first hydrate of a worktree cmw didn't make can't prove which lockfile built its deps.
    const status = fresh && !made ? (dry ? 'would_run' : 'ran') : 'up_to_date';
    const installs = [{ lockfile: repo.lockfile, dir: '', ecosystem: w.repo === 'shop' ? 'node' : 'python', command: repo.install, status }];
    const ms = dry ? 40 : made ? 1400 + Math.round(Math.random() * 400) : fresh ? (w.deps ? 1900 : 3100) : 280;
    if (!dry) {
      if (!w.deps) {
        w.deps = true;
        w.bytes += main.bytes - 0.3 * GB;
      }
      w.env ??= [...(main.env ?? [])];
      w.hydrated = true;
    }
    return {
      report: {
        target: w.path,
        source: main.path,
        dry_run: dry,
        ready: true,
        duration_ms: ms,
        entries,
        pruned: [],
        unmatched: [],
        installs,
        stale_native: [],
        needs_setup: [],
      },
      ms,
    };
  }

  /** HydrateReport::summary. */
  summary(r: ReturnType<Sim['hydrate']>['report'], showPath: boolean): Line[] {
    const count = (pred: (a: string) => boolean) => r.entries.filter((e) => pred(e.action)).length;
    const join = (counts: [number, string][]) =>
      counts
        .filter(([n]) => n > 0)
        .map(([n, l]) => `${n} ${l}`)
        .join(', ');
    const files = join([
      [count((a) => a === 'cloned' || a === 'would_clone'), r.dry_run ? 'to clone' : 'cloned'],
      [count((a) => a === 'copied' || a === 'would_copy'), r.dry_run ? 'to copy' : 'copied'],
      [count((a) => a === 'exists'), 'already present'],
      [count((a) => a.startsWith('skipped')), 'skipped'],
    ]);
    const inst = (s: string) => r.installs.filter((i) => i.status === s).length;
    const installs = join([
      [inst('ran'), 'ran'],
      [inst('would_run'), 'to run'],
      [inst('up_to_date'), 'up to date'],
    ]);
    const head: Line = r.dry_run ? line('dry run, nothing changed') : [seg('ready', 'ok'), seg(` in ${(r.duration_ms / 1000).toFixed(1)}s`)];
    const rows: [string, string][] = [];
    if (showPath) rows.push(['path', r.target]);
    if (files) rows.push(['files', files]);
    if (installs) rows.push(['installs', installs]);
    const w = Math.max(...rows.map(([l]) => l.length));
    return [head, ...rows.map(([l, v]) => [seg('  '), seg(l.padEnd(w), 'dim'), seg(`  ${v}`)])];
  }

  private hydrateCmd(p: Parsed, cwd: string) {
    const target = p.pos[0] ? this.path(cwd, p.pos[0]) : cwd;
    this.needRepo(target);
    const w = this.wtAt(target)!;
    if (w.main) throw new CmwError('refused', 'target is the main checkout', 'cmw hydrate <worktree-path>');
    const { report, ms } = this.hydrate(w, p.flags.has('--dry-run'));
    return { human: this.summary(report, true), data: report, wait: ms };
  }

  // -- new --------------------------------------------------------------------------------------

  /** Create (or find) the worktree for `branch`. Shared with the interactive screen. */
  create(cwd: string, branch: string, o: { base?: string; path?: string; noHydrate?: boolean } = {}) {
    const repo = this.needRepo(cwd);
    if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(branch) || branch.includes('..') || branch.endsWith('/') || branch.endsWith('.lock')) {
      throw new CmwError('error', `git worktree add failed: fatal: '${branch}' is not a valid branch name`);
    }
    const existing = this.wts.find((w) => w.repo === repo.name && w.branch === branch);
    if (existing) {
      const h = o.noHydrate || existing.main ? undefined : this.hydrate(existing, false);
      return { w: existing, created: false, base: null, hydrate: h };
    }
    const path = o.path ? this.path(cwd, o.path) : `~/.cmw/worktrees/${repo.name}/${slugify(branch)}`;
    if (this.isDir(path)) throw new CmwError('refused', `${path} already exists and is not a worktree of ${repo.name}`, `cmw new ${branch} --path <other-dir>`);
    const reused = this.branches.delete(branch);
    const base = reused ? null : (o.base ?? 'origin/main');
    const main = this.mainOf(repo.name);
    const w = wt(repo.name, branch, { path, hydrated: false, deps: false, env: null, bytes: 6.2 * MB, own: 6.2 * MB, head: main.head });
    this.wts.push(w);
    let h: ReturnType<Sim['hydrate']> | undefined;
    // cmw made it, so the cloned lockfile is known to match: nothing installs.
    if (!o.noHydrate) h = this.hydrate(w, false, true);
    return { w, created: true, base, hydrate: h };
  }

  private new(p: Parsed, cwd: string) {
    const r = this.create(cwd, p.pos[0], { base: p.opts.get('--base'), path: p.opts.get('--path'), noHydrate: p.flags.has('--no-hydrate') });
    return {
      stderr: r.hydrate ? this.summary(r.hydrate.report, false) : [],
      human: [line(r.w.path)],
      data: { path: r.w.path, branch: r.w.branch, created: r.created, base: r.base, hydrate: r.hydrate?.report ?? null },
      wait: r.hydrate?.ms ?? 200,
    };
  }

  // -- ls ---------------------------------------------------------------------------------------

  /** The repos a command sees: this one, or every one with --all. */
  scope(cwd: string, all: boolean): Repo[] {
    return all ? REPOS : [this.needRepo(cwd)];
  }

  private ls(p: Parsed, cwd: string) {
    const sizes = p.flags.has('--sizes');
    const repos = this.scope(cwd, p.flags.has('--all'));
    const now = Math.floor(Date.now() / 1000);
    const blocks: Line[] = [];
    repos.forEach((repo, i) => {
      const head = ['BRANCH', 'STATE', 'MERGED', 'IDLE', 'HYDRATED', ...(sizes ? ['SIZE'] : []), 'PATH'];
      const rows = this.wtsOf(repo.name).map((w) => [
        w.main ? `${w.branch} [main]` : w.branch,
        w.dirty ? `${w.dirty} dirty` : 'clean',
        w.main ? '?' : w.via ? 'yes' : 'no',
        `${w.idle}d`,
        w.hydrated ? 'yes' : 'no',
        ...(sizes ? [humanBytes(w.bytes)] : []),
        w.path,
      ]);
      if (i) blocks.push(line(''));
      blocks.push(line(`${repo.name} (${repo.main}/.git)`));
      table([head, ...rows]).forEach((t, j) => blocks.push(line(t, j === 0 ? 'dim' : undefined)));
    });
    if (sizes) blocks.push(line(''), line('SIZE is apparent size: APFS clones (node_modules, Pods) are counted at full size, so real disk use is lower.', 'dim'));
    const data = {
      repos: repos.map((repo) => ({
        repo: repo.name,
        common_dir: `${repo.main}/.git`,
        default_branch: 'main',
        worktrees: this.wtsOf(repo.name).map((w) => ({
          path: w.path,
          branch: w.branch,
          head: w.head,
          is_main: w.main,
          bare: false,
          detached: false,
          locked: false,
          prunable: false,
          dirty: w.dirty,
          has_submodules: false,
          upstream: w.main ? 'origin/main' : w.unpushed ? null : 'origin/main',
          ahead: w.unpushed,
          behind: 0,
          unpushed: w.unpushed,
          merged: w.main ? null : !!w.via,
          merged_via: w.via,
          merge_reason: w.reason,
          last_commit_unix: now - w.idle * DAY,
          idle_days: w.idle,
          hydrated: w.hydrated,
          apparent_bytes: sizes ? Math.round(w.bytes) : null,
        })),
      })),
    };
    return { human: blocks, data };
  }

  // -- rm, gc -----------------------------------------------------------------------------------

  /** gc::rm. Throws the refusals the real command gives. */
  remove(cwd: string, name: string, o: { force?: boolean; keepBranch?: boolean; dryRun?: boolean } = {}) {
    this.needRepo(cwd);
    const w = this.resolve(cwd, name);
    if (!w) throw new CmwError('not_found', `no worktree matches '${name}'`, 'cmw ls');
    if (w.main) throw new CmwError('refused', `${w.path} is the main checkout; cmw never removes it`, 'cmw ls');
    if (!o.force) {
      if (w.dirty > 0) throw new CmwError('refused', `${w.dirty} uncommitted changes in ${w.path}`, `cmw rm ${shQuote(name)} --force`);
      if (w.unpushed > 0 && !w.via) {
        throw new CmwError(
          'refused',
          `${w.unpushed} unpushed commits in ${w.path} that are not merged into main; push them first, or pass --force to discard`,
          `git -C ${shQuote(w.path)} push -u origin ${shQuote(w.branch)}`,
        );
      }
    }
    const deleteBranch = !!w.via && !o.keepBranch;
    const kept = o.keepBranch ? '--keep-branch' : w.via ? null : `not known to be merged (${w.reason}); its commits stay recoverable on the branch`;
    const derived = w.caches.filter((c) => c.path.startsWith('~/Library/')).map((c) => c.path);
    if (!o.dryRun) {
      this.wts = this.wts.filter((x) => x !== w);
      if (!deleteBranch) this.branches.add(w.branch);
    }
    return {
      w,
      freed: w.own + w.caches.reduce((n, c) => n + c.bytes, 0),
      report: {
        path: w.path,
        branch: w.branch,
        dry_run: !!o.dryRun,
        removed: true,
        branch_deleted: deleteBranch,
        branch_kept_reason: kept,
        derived_data_removed: derived,
        watchman_unwatched: false,
        admin_cleanup: 'not_needed',
      },
    };
  }

  /** RmReport::human. */
  rmHuman(r: ReturnType<Sim['remove']>['report']): Line[] {
    const out = [line(`${r.dry_run ? 'would remove' : 'removed'} ${r.path}`)];
    if (r.branch_deleted) out.push(line(`  branch ${r.branch}: ${r.dry_run ? 'would delete' : 'deleted'}`));
    else out.push(line(`  branch ${r.branch}: kept (${r.branch_kept_reason})`));
    for (const d of r.derived_data_removed) out.push(line(`  derived data: ${d}`));
    return out;
  }

  private rm(p: Parsed, cwd: string) {
    const r = this.remove(cwd, p.pos[0], { force: p.flags.has('--force'), keepBranch: p.flags.has('--keep-branch'), dryRun: p.flags.has('--dry-run') });
    return { human: this.rmHuman(r.report), data: r.report, wait: r.report.dry_run ? 0 : 500 };
  }

  /** What gc would remove, and what it keeps. */
  gcPlan(cwd: string, all: boolean, stale?: number) {
    const candidates: Wt[] = [];
    const skipped: { repo: string; path: string; branch: string; reason: string }[] = [];
    const staleItems: { repo: string; path: string; branch: string; idle_days: number }[] = [];
    for (const repo of this.scope(cwd, all)) {
      for (const w of this.wtsOf(repo.name).filter((x) => !x.main)) {
        if (w.via && w.dirty === 0) {
          if (within(cwd, w.path)) skipped.push({ repo: w.repo, path: w.path, branch: w.branch, reason: 'the current directory is inside this worktree' });
          else candidates.push(w);
        } else if (stale !== undefined && w.idle >= stale) {
          staleItems.push({ repo: w.repo, path: w.path, branch: w.branch, idle_days: w.idle });
        } else if (w.via) {
          skipped.push({ repo: w.repo, path: w.path, branch: w.branch, reason: `${w.dirty} uncommitted changes` });
        }
      }
    }
    return { candidates, skipped, stale: staleItems };
  }

  private gc(p: Parsed, cwd: string) {
    const yes = p.flags.has('--yes');
    const all = p.flags.has('--all');
    const staleArg = p.opts.get('--stale');
    const stale = staleArg === undefined ? undefined : Number(staleArg);
    const plan = this.gcPlan(cwd, all, stale);
    const items = plan.candidates.map((w) => ({ repo: w.repo, path: w.path, branch: w.branch, merged_via: w.via, branch_deleted: true, admin_cleanup: 'not_needed' }));
    let freed = 0;
    if (yes) for (const w of plan.candidates) freed += this.remove(w.path, w.path, {}).freed;
    const out: Line[] = [];
    if (items.length) {
      out.push(line(`${yes ? 'removed' : 'would remove'} ${items.length} merged worktree(s):`));
      for (const i of items) out.push(line(`  ${i.branch}  [${i.merged_via}, branch deleted]  ${i.path}`));
    }
    if (plan.stale.length) {
      out.push(line('stale (not merged; never removed automatically):'));
      for (const s of plan.stale) out.push(line(`  ${s.branch}  idle ${s.idle_days}d  ${s.path}`));
    }
    if (plan.skipped.length) {
      out.push(line('skipped:'));
      for (const s of plan.skipped) out.push(line(`  ${s.branch}  ${s.path}  (${s.reason})`));
    }
    if (!yes) {
      if (items.length) out.push([seg('run: ', 'dim'), seg(`cmw gc --yes${all ? ' --all' : ''}`, 'cmd')]);
      else if (out.length === 0) out.push(line('nothing to clean up'));
    } else {
      if (out.length === 0) out.push(line('nothing to clean up'));
      out.push([seg(`freed ~${humanBytes(freed)}`, 'ok'), seg(' (change in free disk space)')]);
    }
    return {
      human: out,
      data: {
        dry_run: !yes,
        removed: yes ? items : [],
        would_remove: yes ? [] : items,
        pruned: [],
        stale: plan.stale,
        orphans: [],
        derived_data: [],
        skipped: plan.skipped,
        freed_bytes: yes ? freed : null,
      },
      wait: yes ? 700 : 300,
    };
  }

  // -- clean-artifacts --------------------------------------------------------------------------

  artifacts(cwd: string, all: boolean, idle: number) {
    const worktrees = this.scope(cwd, all)
      .flatMap((r) => this.wtsOf(r.name))
      .filter((w) => !w.main && w.idle >= idle && w.caches.length > 0)
      .map((w) => ({ w, items: w.caches.map((c) => ({ path: c.path.startsWith('~') ? c.path : `${w.path}/${c.path}`, apparent_bytes: c.bytes })) }));
    const total = worktrees.reduce((n, x) => n + x.items.reduce((m, i) => m + i.apparent_bytes, 0), 0);
    return { worktrees, total };
  }

  /** clean-artifacts --yes. Returns the CleanArtifactsReport::human text. */
  slim(cwd: string, all: boolean, idle: number, yes: boolean) {
    const { worktrees, total } = this.artifacts(cwd, all, idle);
    const out: Line[] = [];
    for (const { w, items } of worktrees) {
      out.push(line(`${w.branch}  ${w.path}`));
      for (const i of items) out.push([seg(`  ${rpad(humanBytes(i.apparent_bytes), 10)}  `, 'dim'), seg(i.path)]);
    }
    if (worktrees.length === 0) out.push(line('no idle worktrees with reclaimable artifacts'));
    else if (yes) out.push([seg('deleted; ', 'ok'), seg(`freed ~${humanBytes(total)}`, 'ok'), seg(' (change in free disk space)')]);
    else {
      out.push(line(`${humanBytes(total)} apparent (APFS clones counted at full size; real savings are lower)`));
      out.push([seg('run: ', 'dim'), seg('cmw clean-artifacts --yes', 'cmd')]);
    }
    const report = {
      dry_run: !yes,
      worktrees: worktrees.map(({ w, items }) => ({ path: w.path, branch: w.branch, items })),
      total_apparent_bytes: Math.round(total),
      freed_bytes: yes ? Math.round(total) : null,
    };
    if (yes) {
      for (const { w } of worktrees) {
        const bytes = w.caches.reduce((n, c) => n + c.bytes, 0);
        w.bytes -= bytes;
        w.own -= bytes;
        w.caches = [];
      }
    }
    return { out, report };
  }

  private cleanArtifacts(p: Parsed, cwd: string) {
    const idle = Number(p.opts.get('--idle') ?? '7');
    const yes = p.flags.has('--yes');
    const { out, report } = this.slim(cwd, p.flags.has('--all'), idle, yes);
    return { human: out, data: report, wait: yes ? 900 : 400 };
  }

  // -- env sync ---------------------------------------------------------------------------------

  /** env::sync. Pushes a warning when a file that differs is kept. */
  envSync(p: Parsed | { flags: Set<string>; pos: string[] }, cwd: string, warnings: string[]) {
    const target = p.pos[0] ? this.path(cwd, p.pos[0]) : cwd;
    const repo = this.needRepo(target);
    const w = this.wtAt(target)!;
    if (w.main) throw new CmwError('refused', 'target is the main checkout', 'cmw env sync <worktree-path>');
    const dry = p.flags.has('--dry-run');
    const force = p.flags.has('--force');
    const main = this.mainOf(repo.name);
    const copies = repo.entries.filter((e) => e.strategy === 'copy').map((e) => e.path);
    const files = copies.map((path) => {
      if (path.startsWith('.env')) {
        const same = w.env && main.env && w.env.join('\n') === main.env.join('\n');
        let status: string;
        if (!w.env) status = dry ? 'would_copy' : 'copied';
        else if (same) status = 'unchanged';
        else if (force) status = dry ? 'would_overwrite' : 'overwritten';
        else status = 'kept';
        if (!dry && (status === 'copied' || status === 'overwritten')) w.env = [...(main.env ?? [])];
        return { path, status };
      }
      return { path, status: w.hydrated ? 'unchanged' : dry ? 'would_copy' : 'copied' };
    });
    if (files.some((f) => f.status === 'kept')) warnings.push(`some files were kept; run cmw env sync ${shQuote(w.path)} --force`);
    const report = { target: w.path, source: main.path, dry_run: dry, files };
    return { human: [line(`synced ${files.length} files to ${w.path}`)], data: report, wait: 150 };
  }

  // -- agents -----------------------------------------------------------------------------------

  private agents() {
    const human = [
      line(`Claude Code  skill installed   hooks ${this.hooks ? 'on' : 'off'}`),
      line('Codex        skill installed'),
      line(`Grok         skill ${this.grok ? 'installed' : 'missing'}`),
      line('T3 Code      paste `cmw agents prompt --copy` into a T3 thread to set up worktrees'),
    ];
    const skill = (dir: string, status: string) => ({ path: `~/${dir}/skills/cmw/SKILL.md`, status });
    return {
      human,
      data: {
        agents: [
          { agent: 'claude', name: 'Claude Code', detected: true, skill: skill('.claude', 'installed'), hooks: this.hooks ? 'on' : 'off' },
          { agent: 'codex', name: 'Codex', detected: true, skill: skill('.agents', 'installed') },
          { agent: 'grok', name: 'Grok', detected: true, skill: this.grok ? skill('.grok', 'installed') : null },
          { agent: 't3', name: 'T3 Code', detected: true },
        ],
      },
    };
  }

  agentsInstall(p?: Parsed) {
    const grok = this.grok ? 'unchanged' : 'created';
    this.grok = true;
    const skills = [
      { agent: 'claude', path: '~/.claude/skills/cmw/SKILL.md', status: 'unchanged' },
      { agent: 'codex', path: '~/.agents/skills/cmw/SKILL.md', status: 'unchanged' },
      { agent: 'grok', path: '~/.grok/skills/cmw/SKILL.md', status: grok },
    ];
    const names: Record<string, string> = { claude: 'Claude Code', codex: 'Codex', grok: 'Grok' };
    const hooks = p?.flags.has('--claude-hooks') ? { path: '~/.claude/settings.json', status: this.hooks ? 'unchanged' : 'added' } : null;
    if (hooks) this.hooks = true;
    const human: Line[] = skills.map((s) => [seg(`${pad(names[s.agent], 12)}  `), seg(pad(s.status, 9), s.status === 'created' ? 'ok' : 'dim'), seg(`  ${s.path}`, 'dim')]);
    if (hooks) human.push([seg(`${pad('Claude hooks', 12)}  `), seg(pad(hooks.status, 9), hooks.status === 'added' ? 'ok' : 'dim'), seg(`  ${hooks.path}`, 'dim')]);
    human.push(line('T3 Code: paste `cmw agents prompt --copy` into a T3 thread to set up worktrees'));
    return { human, data: { skills, claude_hooks: hooks }, summary: grok === 'created' ? 'Installed the cmw skill for Grok' : 'Agent skills are up to date' };
  }

  // -- completion -------------------------------------------------------------------------------

  /** Tab completion for the last word of `input`. */
  complete(input: string): { value: string; options: string[] } {
    const m = input.match(/^(.*?)(\S*)$/)!;
    const [before, word] = [m[1], m[2]];
    const words = before.trim().split(/\s+/).filter(Boolean);
    let options: string[] = [];
    if (words.length === 0) options = ['cmw', 'cd', 'ls', 'pwd', 'cat', 'echo', 'git', 'npm', 'clear', 'help'];
    else if (words[0] === 'cmw') {
      const rest = words.slice(1).filter((w) => w !== '--json' && !w.startsWith('-C'));
      const sub = rest[0];
      if (!sub) options = word.startsWith('-') ? ['--json', '--help', '--version', '-C'] : [...COMMANDS, 'cd'];
      else if (GROUPS[sub] && rest.length === 1 && !word.startsWith('-')) options = GROUPS[sub];
      else {
        const key = GROUPS[sub] && rest[1] ? `${sub} ${rest[1]}` : sub;
        const spec = SPECS[key] ?? {};
        if (word.startsWith('-')) options = [...(spec.flags ?? []), ...(spec.opts ?? []), '--json', '--help'];
        else if (['rm', 'path', 'cd'].includes(key)) {
          const repo = this.repoAt(this.cwd);
          options = repo ? this.wtsOf(repo.name).filter((w) => key !== 'rm' || !w.main).map((w) => w.branch) : [];
        } else if (key === 'shell-init') options = ['zsh', 'bash', 'fish'];
        else if (['hydrate', 'env sync'].includes(key)) options = this.dirOptions(word);
      }
    } else if (['cd', 'ls'].includes(words[0])) options = this.dirOptions(word);
    else if (words[0] === 'cat') options = ['.env.local', '.env'];
    else if (words[0] === 'git') options = words.length === 1 ? ['status', 'branch', 'log', 'worktree'] : words[1] === 'worktree' && words.length === 2 ? ['add', 'list'] : [];
    const hits = [...new Set(options.filter((o) => o.startsWith(word)))];
    if (hits.length === 0) return { value: input, options: [] };
    if (hits.length === 1) return { value: before + hits[0] + (hits[0].endsWith('/') ? '' : ' '), options: [] };
    let common = hits[0];
    for (const h of hits) while (!h.startsWith(common)) common = common.slice(0, -1);
    return { value: before + common, options: common.length > word.length ? [] : hits };
  }

  private dirOptions(word: string) {
    const slash = word.lastIndexOf('/');
    const base = slash === -1 ? '' : word.slice(0, slash + 1);
    const dir = base ? this.path(this.cwd, base) : this.cwd;
    return this.children(dir, word.slice(slash + 1).startsWith('.'))
      .filter((n) => this.isDir(`${dir === '/' ? '' : dir}/${n}`))
      .map((n) => `${base}${n}/`);
  }
}

// ---------------------------------------------------------------------------------------------
// Parsing

/** Split on `&&`, `||` and `;` outside quotes and `$(…)`. */
function splitChain(input: string): { op: string; cmd: string }[] {
  const parts: { op: string; cmd: string }[] = [];
  let cur = '';
  let op = ';';
  let quote = '';
  let depth = 0;
  for (let i = 0; i < input.length; i++) {
    const c = input[i];
    if (quote) {
      if (c === quote) quote = '';
      cur += c;
      continue;
    }
    if (c === '"' || c === "'") quote = c;
    if (c === '$' && input[i + 1] === '(') depth++;
    if (c === ')' && depth) depth--;
    const two = input.slice(i, i + 2);
    if (!depth && (two === '&&' || two === '||' || c === ';')) {
      parts.push({ op, cmd: cur });
      op = c === ';' ? ';' : two;
      cur = '';
      if (c !== ';') i++;
      continue;
    }
    cur += c;
  }
  parts.push({ op, cmd: cur });
  return parts.filter((p) => p.cmd.trim());
}

/** Words, honoring single and double quotes. Null on an unmatched quote. */
function tokenize(s: string): string[] | null {
  const out: string[] = [];
  let cur = '';
  let has = false;
  let quote = '';
  for (const c of s) {
    if (quote) {
      if (c === quote) quote = '';
      else cur += c;
    } else if (c === '"' || c === "'") {
      quote = c;
      has = true;
    } else if (/\s/.test(c)) {
      if (has || cur) out.push(cur);
      cur = '';
      has = false;
    } else cur += c;
  }
  if (quote) return null;
  if (has || cur) out.push(cur);
  return out;
}
