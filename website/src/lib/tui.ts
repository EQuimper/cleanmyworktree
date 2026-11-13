// The interactive screen (bare `cmw`) for the playground, drawn from the same pretend repo as
// the shell. Layout, labels and keys follow src/tui/ui.rs and src/tui/app.rs.

import { type Line, type Seg, type Sim, type Wt, humanBytes, pad, rpad, seg, slugify, truncate } from './sim';

type Screen = 'home' | 'worktrees' | 'new' | 'clean' | 'slim' | 'setup';
type Tone = 'ok' | 'warn' | 'error';

export interface TLine {
  segs: Seg[];
  hit?: string;
  sel?: boolean;
}

export interface ModalView {
  title: string;
  /** Border and title color. */
  tone: 'accent' | 'danger' | 'warn';
  body: Line[];
  keys: [string, string][];
}

export interface Frame {
  lines: TLine[];
  modal?: ModalView;
  busy?: { label: string; secs: number };
  toast?: { msg: string; tone: Tone };
  small?: boolean;
}

const LOGO = [
  ' ██████ ███    ███ ██     ██',
  '██      ████  ████ ██     ██',
  '██      ██ ████ ██ ██  █  ██',
  '██      ██  ██  ██ ██ ███ ██',
  ' ██████ ██      ██  ███ ███ ',
];
const LOGO_W = 28;
const SPINNER = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const IDLE_STEPS = [0, 1, 3, 7, 14, 30];

const MENU: { screen: Screen; label: string; desc: string }[] = [
  { screen: 'worktrees', label: 'Worktrees', desc: 'Browse, open, hydrate, remove' },
  { screen: 'new', label: 'New', desc: 'Create a hydrated worktree' },
  { screen: 'clean', label: 'Clean', desc: 'Remove merged worktrees' },
  { screen: 'slim', label: 'Slim', desc: 'Delete build caches from idle worktrees' },
  { screen: 'setup', label: 'Setup', desc: 'Shell integration, agent skill, hooks' },
];

const idleLabel = (d: number) => (d === 0 ? 'any time' : d === 1 ? '1+ day' : `${d}+ days`);
const plural = (n: number) => (n === 1 ? '' : 's');
const width = (segs: Seg[]) => segs.reduce((n, s) => n + [...s.t].length, 0);

type Action =
  | { kind: 'remove'; w: Wt; force: boolean }
  | { kind: 'clean'; picked: Wt[] }
  | { kind: 'slim' }
  | { kind: 'hooks' };

type Modal =
  | { kind: 'confirm'; title: string; lines: string[]; danger: boolean; action: Action }
  | { kind: 'type'; title: string; lines: string[]; expect: string; input: string; action: Action }
  | { kind: 'result'; title: string; text: Line[]; tone: Tone; open?: string };

/** `↑↓  |  Enter  |  Q Quit`. Keys a click can press are marked, as in ui.rs `key_code`. */
export function footer(keys: [string, string][]): Seg[] {
  const segs: Seg[] = [];
  keys.forEach(([k, label], i) => {
    if (i) segs.push(seg('  |  ', 'faint'));
    const pressable = ['Enter', 'Esc', 'Space', 'Tab'].includes(k) || [...k].length === 1;
    segs.push({ t: label ? `${k} ${label}` : k, c: 'dim', ...(pressable ? { k } : {}) });
  });
  return segs;
}

export class Tui {
  screen: Screen = 'home';
  homeSel = 0;
  wtSel = 0;
  filter = '';
  filtering = false;
  newBranch = '';
  newBase = '';
  newFocus = 0;
  gcSel = 0;
  gcChecked: boolean[] = [];
  slimIdle = 7;
  slimSel = 0;
  setupSel = 0;
  all: boolean;
  modal?: Modal;
  busy?: { label: string; started: number };
  toast?: { msg: string; tone: Tone; until: number };
  tick = 0;
  private loadedAt = 0;
  private sizedAt = 0;
  private slimAt = 0;
  private readonly here: string;
  /** Where cmw is running: the shell's directory, or `-C <dir>`. */
  private at: string;
  /** Set when the worktree cmw started in is removed, so the shell lands somewhere that exists. */
  private cdOnExit?: string;

  constructor(
    private readonly sim: Sim,
    cwd: string,
    private readonly onChange: () => void,
    private readonly onQuit: (cd?: string) => void,
    private readonly onCopy: (text: string) => void,
    private readonly snippet: string,
  ) {
    this.here = this.at = cwd;
    this.all = !this.inRepo();
    this.reload();
  }

  private quit(cd?: string) {
    this.onQuit(cd ?? this.cdOnExit);
  }

  // -- data -----------------------------------------------------------------------------------

  inRepo() {
    return !!this.sim.repoAt(this.at);
  }

  private get repo() {
    return this.sim.repoAt(this.at)?.name;
  }

  /** Anywhere in the pretend tree; commands run from the main checkout when cmw started outside a repo. */
  private get cwd() {
    return this.inRepo() ? this.at : '~/code/shop';
  }

  private reload() {
    const now = performance.now();
    this.loadedAt = now + 320;
    this.sizedAt = now + 950;
    this.slimAt = now + 520;
    this.gcChecked = [];
    setTimeout(this.onChange, 330);
    setTimeout(this.onChange, 960);
  }

  loading() {
    return performance.now() < this.loadedAt;
  }

  /** Spinner or timers on screen: the UI keeps redrawing. */
  animating() {
    const now = performance.now();
    return !!this.busy || now < this.sizedAt || now < this.slimAt || (this.toast !== undefined && now < this.toast.until);
  }

  private rows(): Wt[] {
    const repos = this.all ? ['shop', 'api'] : [this.repo!];
    return repos.flatMap((r) => this.sim.wtsOf(r));
  }

  private visible(): Wt[] {
    const f = this.filter.toLowerCase();
    return this.rows().filter((w) => !f || w.branch.toLowerCase().includes(f) || w.repo.includes(f) || w.path.toLowerCase().includes(f));
  }

  private plan() {
    const p = this.sim.gcPlan(this.cwd, this.all);
    if (this.gcChecked.length !== p.candidates.length) this.gcChecked = p.candidates.map(() => true);
    return p;
  }

  private setupRows(): { title: string; status: string; tone?: Tone }[] {
    return [
      { title: 'Shell integration', status: 'active', tone: 'ok' },
      this.sim.grok ? { title: 'Agent skills', status: 'installed (Claude Code, Codex, Grok)', tone: 'ok' } : { title: 'Agent skills', status: 'missing for Grok', tone: 'warn' },
      { title: 'Claude Code hooks', status: this.sim.hooks ? 'on' : 'off', tone: this.sim.hooks ? 'ok' : undefined },
      { title: 'Agent setup prompt', status: 'copy' },
      { title: 'AGENTS.md snippet', status: 'copy' },
    ];
  }

  private say(msg: string, tone: Tone) {
    this.toast = { msg, tone, until: performance.now() + 3500 };
    setTimeout(this.onChange, 3600);
  }

  private run(label: string, ms: number, done: () => void) {
    this.busy = { label, started: performance.now() };
    setTimeout(() => {
      this.busy = undefined;
      done();
      this.onChange();
    }, ms);
  }

  private error(title: string, e: unknown) {
    const err = e as { message: string; hint?: string };
    const text: Line[] = [[seg(err.message)]];
    if (err.hint) text.push([], [seg(`fix: ${err.hint}`, 'accent')]);
    this.modal = { kind: 'result', title, text, tone: 'error' };
  }

  // -- input ----------------------------------------------------------------------------------

  key(k: string, ctrl = false) {
    if (ctrl && k === 'c') return this.quit();
    if (this.busy) return;
    if (this.modal) return this.modalKey(k);
    if (this.screen === 'new') return this.newKey(k, ctrl);
    if (this.screen === 'worktrees' && this.filtering) return this.filterKey(k);
    const lower = k.length === 1 ? k.toLowerCase() : k;
    if (lower === 'q') return this.quit();
    if (['Escape', 'Backspace', 'ArrowLeft'].includes(k) && this.screen !== 'home' && this.screen !== 'slim') {
      this.filter = '';
      this.screen = 'home';
      return;
    }
    if (['Escape', 'Backspace'].includes(k) && this.screen === 'slim') {
      this.screen = 'home';
      return;
    }
    if (lower === 'a' && this.inRepo()) {
      this.all = !this.all;
      this.wtSel = 0;
      this.reload();
      this.say(`Showing ${this.all ? 'all repositories' : this.repo}`, 'ok');
      return;
    }
    if (lower === 'r') return this.reload();
    const up = k === 'ArrowUp' || k === 'k';
    const down = k === 'ArrowDown' || k === 'j';
    switch (this.screen) {
      case 'home': {
        const n = MENU.length;
        if (up) this.homeSel = (this.homeSel + n - 1) % n;
        else if (down || k === 'Tab') this.homeSel = (this.homeSel + 1) % n;
        else if (k === 'Enter' || k === 'ArrowRight' || k === 'l') this.screen = MENU[this.homeSel].screen;
        else if (/^[1-5]$/.test(k)) {
          this.homeSel = Number(k) - 1;
          this.screen = MENU[this.homeSel].screen;
        }
        return;
      }
      case 'worktrees': {
        const vis = this.visible();
        if (up) this.wtSel = Math.max(0, this.wtSel - 1);
        else if (down) this.wtSel = Math.min(vis.length - 1, this.wtSel + 1);
        else if (k === '/') this.filtering = true;
        else if (k === 'Enter') this.open();
        else if (lower === 'h' || lower === 'e' || lower === 'd') this.act(lower);
        return;
      }
      case 'clean':
        return this.cleanKey(k, up, down);
      case 'slim':
        return this.slimKey(k, up, down);
      case 'setup': {
        if (up) this.setupSel = Math.max(0, this.setupSel - 1);
        else if (down) this.setupSel = Math.min(4, this.setupSel + 1);
        else if (k === 'Enter') this.setupEnter();
      }
    }
  }

  click(hit: string) {
    if (this.busy) return;
    const [kind, arg] = hit.split(':');
    if (kind === 'key') return this.key(arg === 'Space' ? ' ' : arg === 'Esc' ? 'Escape' : arg.length === 1 ? arg.toLowerCase() : arg);
    const i = Number(arg);
    if (kind === 'menu') {
      this.homeSel = i;
      this.screen = MENU[i].screen;
    } else if (kind === 'row') {
      if (this.screen === 'worktrees') {
        this.filtering = false;
        if (this.wtSel === i) this.open();
        else this.wtSel = i;
      } else if (this.screen === 'clean') {
        if (this.gcSel === i) this.cleanKey(' ', false, false);
        else this.gcSel = i;
      } else if (this.screen === 'slim') this.slimSel = i;
      else if (this.screen === 'setup') {
        if (this.setupSel === i) this.setupEnter();
        else this.setupSel = i;
      } else if (this.screen === 'new') this.newFocus = Math.min(i, 1);
    }
  }

  private modalKey(k: string) {
    const m = this.modal!;
    this.modal = undefined;
    if (m.kind === 'confirm') {
      if (k === 'Enter' || k === 'y' || k === 'Y') this.perform(m.action);
      else if (!['Escape', 'n', 'N', 'q'].includes(k)) this.modal = m;
    } else if (m.kind === 'type') {
      if (k === 'Escape') return;
      if (k === 'Enter' && m.input === m.expect) return this.perform(m.action);
      if (k === 'Backspace') m.input = m.input.slice(0, -1);
      else if (k.length === 1) m.input += k;
      this.modal = m;
    } else if (k === 'Enter' && m.open) {
      this.quit(m.open);
    }
  }

  private newKey(k: string, ctrl: boolean) {
    const field = this.newFocus === 0 ? 'newBranch' : 'newBase';
    if (k === 'Escape') this.screen = 'home';
    else if (['Tab', 'ArrowUp', 'ArrowDown'].includes(k)) this.newFocus = 1 - this.newFocus;
    else if (k === 'Backspace') this[field] = this[field].slice(0, -1);
    else if (ctrl && k === 'u') this[field] = '';
    else if (k === 'Enter') {
      const branch = this.newBranch.trim();
      if (!branch) {
        this.newFocus = 0;
        return this.say('Type a branch name', 'warn');
      }
      const base = this.newBase.trim() || undefined;
      this.run(`Creating ${branch}`, 1500, () => {
        try {
          const r = this.sim.create(this.cwd, branch, { base });
          const text: Line[] = [[seg(r.w.path)]];
          if (r.hydrate) text.push([], ...this.sim.summary(r.hydrate.report, true));
          this.modal = { kind: 'result', title: r.created ? `Created ${r.w.branch}` : `${r.w.branch} already has a worktree`, text, tone: 'ok', open: r.w.path };
          this.newBranch = this.newBase = '';
          this.newFocus = 0;
          this.screen = 'worktrees';
          this.wtSel = Math.max(0, this.visible().indexOf(r.w));
          this.reload();
        } catch (e) {
          this.error('Could not create the worktree', e);
        }
      });
    } else if (k.length === 1 && !/\s/.test(k)) this[field] += k;
  }

  private filterKey(k: string) {
    if (k === 'Enter') this.filtering = false;
    else if (k === 'Escape') {
      this.filtering = false;
      this.filter = '';
    } else if (k === 'Backspace') this.filter = this.filter.slice(0, -1);
    else if (k === 'ArrowUp') this.wtSel = Math.max(0, this.wtSel - 1);
    else if (k === 'ArrowDown') this.wtSel = Math.min(this.visible().length - 1, this.wtSel + 1);
    else if (k.length === 1) this.filter += k;
    this.wtSel = Math.min(this.wtSel, Math.max(0, this.visible().length - 1));
  }

  private open() {
    const w = this.visible()[this.wtSel];
    if (w) this.quit(w.path);
  }

  private act(op: 'h' | 'e' | 'd') {
    const w = this.visible()[this.wtSel];
    if (!w) return;
    const label = w.branch;
    if (op === 'h') {
      if (w.main) return this.say('The main checkout is the source; nothing to hydrate', 'warn');
      return this.run(`Hydrating ${label}`, 900, () => {
        const { report } = this.sim.hydrate(w, false);
        this.modal = { kind: 'result', title: `Hydrated ${label}`, text: this.sim.summary(report, true), tone: 'ok' };
        this.reload();
      });
    }
    if (op === 'e') {
      if (w.main) return this.say('The main checkout is the source; nothing to sync', 'warn');
      return this.run(`Syncing env for ${label}`, 450, () => {
        const warnings: string[] = [];
        const r = this.sim.envSync({ flags: new Set(), pos: [w.path] }, w.path, warnings);
        const text = [...r.human, ...warnings.map((x): Line => [seg(`warning: ${x}`, 'warn')])];
        this.modal = { kind: 'result', title: `Synced env for ${label}`, text, tone: warnings.length ? 'warn' : 'ok' };
      });
    }
    if (w.main) return this.say('cmw never removes the main checkout', 'warn');
    this.modal = {
      kind: 'confirm',
      title: `Remove ${label}?`,
      lines: [w.path, w.via ? 'Deletes the directory, its build caches, and the merged branch.' : 'Deletes the directory and its build caches. The branch is kept.'],
      danger: true,
      action: { kind: 'remove', w, force: false },
    };
  }

  private cleanKey(k: string, up: boolean, down: boolean) {
    if (this.loading()) return;
    const n = this.plan().candidates.length;
    if (up) this.gcSel = Math.max(0, this.gcSel - 1);
    else if (down) this.gcSel = Math.min(Math.max(0, n - 1), this.gcSel + 1);
    else if (k === ' ' && n) this.gcChecked[this.gcSel] = !this.gcChecked[this.gcSel];
    else if (k === 'Enter') {
      const picked = this.plan().candidates.filter((_, i) => this.gcChecked[i]);
      if (!picked.length) return this.say('Nothing selected', 'warn');
      this.modal = {
        kind: 'confirm',
        title: `Remove ${picked.length} merged worktree${plural(picked.length)}?`,
        lines: [...picked.map((w) => `• ${w.branch}`), '', 'Directories, build caches and merged branches are deleted. Commits stay in the default branch.'],
        danger: true,
        action: { kind: 'clean', picked },
      };
    }
  }

  private slimKey(k: string, up: boolean, down: boolean) {
    if (['ArrowLeft', 'ArrowRight', 'h', 'l'].includes(k)) {
      const i = IDLE_STEPS.indexOf(this.slimIdle);
      const j = k === 'ArrowLeft' || k === 'h' ? Math.max(0, i - 1) : Math.min(IDLE_STEPS.length - 1, i + 1);
      if (IDLE_STEPS[j] !== this.slimIdle) {
        this.slimIdle = IDLE_STEPS[j];
        this.slimSel = 0;
        this.slimAt = performance.now() + 420;
        setTimeout(this.onChange, 430);
      }
      return;
    }
    if (performance.now() < this.slimAt) return;
    const r = this.sim.artifacts(this.cwd, this.all, this.slimIdle);
    if (up) this.slimSel = Math.max(0, this.slimSel - 1);
    else if (down) this.slimSel = Math.min(Math.max(0, r.worktrees.length - 1), this.slimSel + 1);
    else if (k === 'Enter') {
      if (!r.worktrees.length) return this.say('Nothing to delete', 'warn');
      const n = r.worktrees.length;
      this.modal = {
        kind: 'confirm',
        title: `Delete ${humanBytes(r.total)} of build caches?`,
        lines: [`From ${n} worktree${plural(n)} idle for ${idleLabel(this.slimIdle)}.`, 'Everything deleted is regenerated by the next build.'],
        danger: false,
        action: { kind: 'slim' },
      };
    }
  }

  private setupEnter() {
    switch (this.setupSel) {
      case 0:
        return this.say('Shell integration is active', 'ok');
      case 1:
        return this.say(this.sim.agentsInstall().summary, 'ok');
      case 2:
        if (this.sim.hooks) return this.say('Claude Code hooks are on', 'ok');
        this.modal = {
          kind: 'confirm',
          title: 'Route Claude Code worktrees through cmw?',
          lines: [
            'Adds WorktreeCreate and WorktreeRemove hooks to ~/.claude/settings.json.',
            '',
            'claude --worktree then creates worktrees in ~/.cmw/worktrees and keeps any with unsaved work on removal.',
          ],
          danger: false,
          action: { kind: 'hooks' },
        };
        return;
      case 3:
        this.onCopy(this.sim.prompt);
        return this.say('Prompt copied. Paste it into Claude Code, Codex, Grok or T3 Code', 'ok');
      default:
        this.onCopy(this.snippet);
        return this.say('AGENTS.md snippet copied', 'ok');
    }
  }

  private perform(a: Action) {
    if (a.kind === 'remove') {
      const label = a.w.branch;
      return this.run(`Removing ${label}`, 700, () => {
        try {
          const main = this.sim.mainOf(a.w.repo).path;
          const r = this.sim.remove(a.w.path, a.w.path, { force: a.force });
          if (this.at === a.w.path || this.at.startsWith(`${a.w.path}/`)) {
            // We (and the caller's shell) were inside it: carry on from the main checkout.
            this.at = this.cdOnExit = main;
          }
          this.say(`Removed ${label}${r.report.branch_deleted ? ' and its branch' : ''}`, 'ok');
          this.wtSel = Math.min(this.wtSel, this.visible().length - 1);
        } catch (e) {
          const err = e as { kind: string; message: string };
          if (err.kind === 'refused' && !a.force) {
            this.modal = {
              kind: 'type',
              title: `Remove ${label} anyway?`,
              lines: [err.message, 'Forcing it permanently deletes that work.'],
              expect: label,
              input: '',
              action: { ...a, force: true },
            };
          } else this.error(`Could not remove ${label}`, e);
        }
        this.reload();
      });
    }
    if (a.kind === 'clean') {
      return this.run('Removing merged worktrees', 900, () => {
        const text = a.picked.flatMap((w) => this.sim.rmHuman(this.sim.remove(w.path, w.path).report));
        this.modal = { kind: 'result', title: 'Cleaned up', text, tone: 'ok' };
        this.gcSel = 0;
        this.reload();
      });
    }
    if (a.kind === 'slim') {
      return this.run('Deleting build caches', 900, () => {
        const { out } = this.sim.slim(this.cwd, this.all, this.slimIdle, true);
        this.modal = { kind: 'result', title: 'Slimmed down', text: out, tone: 'ok' };
        this.reload();
      });
    }
    this.sim.hooks = true;
    this.say('Claude Code hooks added in ~/.claude/settings.json', 'ok');
  }

  // -- drawing --------------------------------------------------------------------------------

  private spinner() {
    return SPINNER[this.tick % SPINNER.length];
  }

  render(cols: number, rows: number): Frame {
    const frame: Frame = { lines: [] };
    if (this.toast && performance.now() < this.toast.until) frame.toast = { msg: this.toast.msg, tone: this.toast.tone };
    if (cols < 44 || rows < 14) return { lines: [], small: true };
    const w = cols - 4;
    const h = rows - 2;
    const body = this.screen === 'home' ? this.home(w, h) : this.page(w, h);
    frame.lines = [{ segs: [] }, ...body.slice(0, h).map((l) => ({ ...l, segs: [seg('  '), ...l.segs] }))];
    if (this.modal) frame.modal = this.modalView();
    if (this.busy) frame.busy = { label: this.busy.label, secs: Math.floor((performance.now() - this.busy.started) / 1000) };
    return frame;
  }

  private home(w: number, h: number): TLine[] {
    const lines: TLine[] = [];
    const showLogo = h >= LOGO.length + MENU.length + 6;
    const scope = this.all ? 'All repositories' : this.repo!;
    const branch = this.inRepo() ? this.sim.wtAt(this.at)?.branch ?? '' : '';
    const info: Seg[][] = [[], [seg(scope, 'b')], [seg(branch, 'dim')], [seg(`v${this.sim.version}`, 'faint')], []];
    if (showLogo) {
      const side = w >= LOGO_W + 4 + 20;
      LOGO.forEach((row, i) => lines.push({ segs: [seg(row, `logo g${i}`), ...(side ? [seg('    '), ...info[i]] : [])] }));
    } else {
      lines.push({ segs: [seg('cmw', 'accent b'), seg('  '), ...info[1]] });
    }
    lines.push({ segs: [] });
    const labelW = Math.max(...MENU.map((m) => m.label.length)) + 4;
    const descW = Math.max(...MENU.map((m) => m.desc.length)) + 3;
    MENU.forEach((item, i) => {
      const sel = i === this.homeSel;
      const segs = [
        seg(sel ? '➤ ' : '  ', 'accent'),
        seg(`${i + 1}. `, sel ? 'accent' : 'faint'),
        seg(pad(item.label, labelW), sel ? 'accent b' : undefined),
        seg(pad(item.desc, descW), sel ? 'accent' : 'dim'),
      ];
      const badge = this.badge(item.screen);
      if (badge) segs.push(seg(badge[0], badge[1]));
      lines.push({ segs, hit: `menu:${i}` });
    });
    lines.push({ segs: [] });
    const keys: [string, string][] = [['↑↓', ''], ['Enter', '']];
    if (this.inRepo()) keys.push(['A', this.all ? 'This repo' : 'All repos']);
    keys.push(['Q', 'Quit']);
    lines.push({ segs: footer(keys) });
    return lines;
  }

  private badge(screen: Screen): [string, string] | undefined {
    if (screen === 'setup') return undefined;
    if (screen === 'new') return undefined;
    if (this.loading()) return [this.spinner(), 'faint'];
    if (screen === 'worktrees') return [`${this.rows().filter((x) => !x.main).length} linked`, 'faint'];
    if (screen === 'clean') {
      const n = this.plan().candidates.length;
      return n ? [`${n} ready`, 'accent'] : ['all clean', 'faint'];
    }
    const total = this.sim.artifacts(this.cwd, this.all, this.slimIdle).total;
    return total ? [humanBytes(total), 'accent'] : ['nothing idle', 'faint'];
  }

  private page(w: number, h: number): TLine[] {
    const titles: Record<Screen, string> = { worktrees: 'Worktrees', new: 'New worktree', clean: 'Clean', slim: 'Slim', setup: 'Setup', home: '' };
    const scope = this.all ? 'all repositories' : this.repo!;
    const head: Seg[] = [seg('cmw', 'accent b'), seg(' › ', 'faint'), seg(titles[this.screen], 'b')];
    if (this.screen !== 'setup') head.push(seg(`   ${scope}`, 'dim'));
    if (this.screen === 'worktrees' && (this.filtering || this.filter)) {
      const f = [seg('/ ', 'accent'), seg(`${this.filter}${this.filtering ? '▏' : ''}`)];
      const gap = Math.max(1, w - width(head) - width(f));
      head.push(seg(' '.repeat(gap)), ...f);
    }

    let keys: [string, string][];
    switch (this.screen) {
      case 'worktrees':
        keys = this.filtering
          ? [['Enter', 'Done'], ['Esc', 'Clear']]
          : [['↑↓', ''], ['Enter', 'Open'], ['H', 'Hydrate'], ['E', 'Env'], ['D', 'Remove'], ['/', 'Filter']];
        break;
      case 'new':
        keys = [['Tab', 'Next field'], ['Enter', 'Create'], ['Esc', 'Back']];
        break;
      case 'clean':
        keys = [['↑↓', ''], ['Space', 'Toggle'], ['Enter', 'Remove']];
        break;
      case 'slim':
        keys = [['←→', 'Idle days'], ['Enter', 'Delete']];
        break;
      default:
        keys = [['↑↓', ''], ['Enter', '']];
    }
    if (!['new', 'setup'].includes(this.screen) && !this.filtering && this.inRepo()) keys.push(['A', this.all ? 'This repo' : 'All repos']);
    if (this.screen !== 'new' && !this.filtering) keys.push(['Esc', 'Back']);

    const bodyH = Math.max(0, h - 6);
    const content = (() => {
      switch (this.screen) {
        case 'worktrees':
          return this.worktrees(w, bodyH);
        case 'new':
          return this.newPage();
        case 'clean':
          return this.clean(w, bodyH);
        case 'slim':
          return this.slim(bodyH);
        default:
          return this.setup();
      }
    })();
    const body = [...content.slice(0, bodyH)];
    while (body.length < bodyH) body.push({ segs: [] });
    // The rule is drawn long and clipped: box-drawing glyphs fall back to a narrower font than the grid.
    const rule = { segs: [seg('─'.repeat(w * 2), 'faint')] };
    return [{ segs: head }, rule, { segs: [] }, ...body, { segs: [] }, { segs: footer(keys) }];
  }

  private worktrees(w: number, bodyH: number): TLine[] {
    if (this.loading()) return [{ segs: [seg(`${this.spinner()} `, 'accent'), seg('Reading worktrees…', 'dim')] }];
    const vis = this.visible();
    if (!vis.length) return [{ segs: [seg(this.filter ? 'Nothing matches the filter' : 'No worktrees', 'dim')] }];
    const [stateW, idleW, sizeW] = [14, 6, 9];
    const repoW = this.all ? Math.min(22, Math.max(4, ...vis.map((r) => r.repo.length))) : 0;
    const suffix = (r: Wt) => (r.path === this.sim.wtAt(this.here)?.path ? '  ◂ here' : '');
    const fixed = 2 + stateW + idleW + sizeW + 6 + (this.all ? repoW + 2 : 0);
    const contentW = Math.max(...vis.map((r) => r.branch.length + suffix(r).length));
    const branchW = Math.min(Math.max(contentW, 10), Math.max(10, w - fixed));
    const lines: TLine[] = [];
    let head = '  ';
    if (this.all) head += pad('REPO', repoW + 2);
    head += pad('BRANCH', branchW + 2) + pad('STATE', stateW + 2) + rpad('IDLE', idleW) + rpad('SIZE', sizeW + 2);
    lines.push({ segs: [seg(head, 'faint')] });
    const tableH = Math.max(1, bodyH - 3);
    const offset = this.wtSel < tableH ? 0 : this.wtSel + 1 - tableH;
    vis.slice(offset, offset + tableH).forEach((r, j) => {
      const pos = j + offset;
      const sel = pos === this.wtSel;
      const sfx = suffix(r);
      const label = truncate(r.branch, branchW - sfx.length);
      const segs: Seg[] = [seg(sel ? '➤ ' : '  ', 'accent')];
      if (this.all) segs.push(seg(pad(r.repo, repoW + 2), 'dim'));
      segs.push(seg(label, sel ? 'accent b' : undefined), seg(sfx, 'faint'), seg(' '.repeat(Math.max(0, branchW + 2 - label.length - sfx.length))));
      const [state, style] = r.dirty
        ? [`● ${r.dirty} changed`, 'warn']
        : !r.main && r.via
          ? ['✓ merged', 'accent']
          : r.unpushed
            ? [`↑ ${r.unpushed} unpushed`, 'info']
            : ['clean', 'dim'];
      segs.push(seg(pad(state, stateW + 2), style), seg(rpad(r.idle ? `${r.idle}d` : 'today', idleW), 'dim'));
      segs.push(seg(rpad(performance.now() < this.sizedAt ? this.spinner() : humanBytes(r.bytes), sizeW + 2), 'dim'));
      lines.push({ segs, hit: `row:${pos}`, sel });
    });
    while (lines.length < bodyH - 2) lines.push({ segs: [] });
    const cur = vis[this.wtSel];
    if (cur) {
      lines.push({ segs: [seg(`  ${cur.path}`, 'dim')] });
      const reason = cur.reason === 'no_commits' ? 'no commits of its own yet' : cur.reason;
      lines.push({ segs: cur.main ? [] : [seg(`  ${reason}`, 'faint')] });
    }
    return lines;
  }

  private newPage(): TLine[] {
    const field = (label: string, value: string, placeholder: string, focus: boolean, i: number): TLine => {
      const segs = [seg(focus ? '➤ ' : '  ', 'accent'), seg(label.padEnd(8), focus ? 'accent b' : 'dim')];
      if (!value) segs.push(...(focus ? [seg('▏', 'accent')] : []), seg(placeholder, 'faint'));
      else segs.push(seg(value), ...(focus ? [seg('▏', 'accent')] : []));
      return { segs, hit: `row:${i}` };
    };
    const lines: TLine[] = [
      field('Branch', this.newBranch, 'feat/my-change', this.newFocus === 0, 0),
      { segs: [] },
      field('Base', this.newBase, 'origin/main', this.newFocus === 1, 1),
      { segs: [] },
      { segs: [] },
    ];
    const branch = this.newBranch.trim();
    if (branch) lines.push({ segs: [seg('  → ', 'faint'), seg(`~/.cmw/worktrees/${this.repo ?? 'shop'}/${slugify(branch)}`, 'dim')] });
    return lines;
  }

  private clean(w: number, bodyH: number): TLine[] {
    if (this.loading()) return [{ segs: [seg(`${this.spinner()} `, 'accent'), seg('Checking what’s merged…', 'dim')] }];
    const plan = this.plan();
    const items = plan.candidates;
    const lines: TLine[] = [];
    if (!items.length) lines.push({ segs: [seg('✓ ', 'accent'), seg('Nothing to clean. No merged worktrees.')] });
    else {
      const branchW = Math.min(40, Math.max(10, ...items.map((i) => i.branch.length)));
      const repoW = this.all ? Math.min(22, Math.max(4, ...items.map((i) => i.repo.length))) + 2 : 0;
      const [viaW, sizeW] = [12, 10];
      const pathW = Math.max(0, w - (4 + repoW + branchW + 2 + viaW + sizeW + 2));
      lines.push({ segs: [seg(`    ${pad(this.all ? 'REPO' : '', repoW)}${pad('BRANCH', branchW + 2)}${pad('MERGED', viaW)}${rpad('SIZE', sizeW)}  PATH`, 'faint')] });
      const height = Math.max(1, bodyH - 5);
      const offset = this.gcSel < height ? 0 : this.gcSel + 1 - height;
      items.slice(offset, offset + height).forEach((item, j) => {
        const i = j + offset;
        const sel = i === this.gcSel;
        const checked = this.gcChecked[i];
        const segs: Seg[] = [seg(sel ? '➤ ' : '  ', 'accent'), seg(checked ? '◉ ' : '○ ', checked ? 'accent' : 'faint')];
        if (this.all) segs.push(seg(pad(item.repo, repoW), 'dim'));
        segs.push(
          seg(pad(item.branch, branchW + 2), sel ? 'accent b' : undefined),
          seg(pad(item.via ?? 'yes', viaW), 'dim'),
          seg(rpad(performance.now() < this.sizedAt ? this.spinner() : humanBytes(item.bytes), sizeW), 'dim'),
          seg(`  ${truncate(item.path, pathW)}`, 'faint'),
        );
        lines.push({ segs, hit: `row:${i}`, sel });
      });
      const picked = items.filter((_, i) => this.gcChecked[i]);
      lines.push({ segs: [] });
      lines.push({ segs: [seg(`${picked.length} selected  `, 'dim'), seg(humanBytes(picked.reduce((n, x) => n + x.bytes, 0)), 'accent b'), seg('  apparent size', 'faint')] });
    }
    lines.push({ segs: [] });
    if (plan.skipped.length) lines.push({ segs: [seg(`${plan.skipped.length} kept with uncommitted or unpushed work`, 'faint')] });
    return lines;
  }

  private slim(bodyH: number): TLine[] {
    const first = this.slimIdle === IDLE_STEPS[0];
    const last = this.slimIdle === IDLE_STEPS[IDLE_STEPS.length - 1];
    const lines: TLine[] = [
      {
        segs: [
          seg('Idle for  ', 'dim'),
          seg(first ? '  ' : '‹ ', 'accent'),
          seg(idleLabel(this.slimIdle), 'accent b'),
          seg(last ? '' : ' ›', 'accent'),
        ],
      },
      { segs: [] },
    ];
    if (performance.now() < this.slimAt) {
      lines.push({ segs: [seg(`${this.spinner()} `, 'accent'), seg('Measuring build caches…', 'dim')] });
      return lines;
    }
    const r = this.sim.artifacts(this.cwd, this.all, this.slimIdle);
    if (!r.worktrees.length) {
      lines.push({ segs: [seg('✓ ', 'accent'), seg('No reclaimable caches in idle worktrees.')] });
      return lines;
    }
    const branchW = Math.min(40, Math.max(10, ...r.worktrees.map((x) => x.w.branch.length)));
    const height = Math.max(1, bodyH - 10);
    const offset = this.slimSel < height ? 0 : this.slimSel + 1 - height;
    r.worktrees.slice(offset, offset + height).forEach((x, j) => {
      const i = j + offset;
      const sel = i === this.slimSel;
      const bytes = x.items.reduce((n, it) => n + it.apparent_bytes, 0);
      lines.push({
        segs: [
          seg(sel ? '➤ ' : '  ', 'accent'),
          seg(pad(x.w.branch, branchW + 2), sel ? 'accent b' : undefined),
          seg(pad(`${x.items.length} cache${plural(x.items.length)}`, 12), 'dim'),
          seg(rpad(humanBytes(bytes), 10), 'accent'),
        ],
        hit: `row:${i}`,
        sel,
      });
    });
    const cur = r.worktrees[this.slimSel];
    if (cur) {
      lines.push({ segs: [] });
      for (const it of cur.items.slice(0, 4)) {
        const rel = it.path.startsWith(`${cur.w.path}/`) ? it.path.slice(cur.w.path.length + 1) : it.path;
        lines.push({ segs: [seg(rpad(humanBytes(it.apparent_bytes), 12), 'faint'), seg(`  ${rel}`, 'faint')] });
      }
    }
    lines.push({ segs: [] });
    lines.push({ segs: [seg('Total  ', 'dim'), seg(humanBytes(r.total), 'accent b'), seg('  apparent size', 'faint')] });
    return lines;
  }

  private setup(): TLine[] {
    const rows = this.setupRows();
    const titleW = Math.max(...rows.map((s) => s.title.length)) + 4;
    return rows.map((s, i) => {
      const sel = i === this.setupSel;
      const [mark, style] = s.tone ? ['● ', s.tone === 'ok' ? 'accent' : s.tone === 'warn' ? 'warn' : 'danger'] : ['→ ', 'faint'];
      return { segs: [seg(sel ? '➤ ' : '  ', 'accent'), seg(pad(s.title, titleW), sel ? 'accent b' : undefined), seg(mark, style), seg(s.status, 'dim')], hit: `row:${i}`, sel };
    });
  }

  private modalView(): ModalView {
    const m = this.modal!;
    if (m.kind === 'confirm') {
      return { title: m.title, tone: m.danger ? 'danger' : 'accent', body: m.lines.map((l) => [seg(l)]), keys: [['Enter', 'Confirm'], ['Esc', 'Cancel']] };
    }
    if (m.kind === 'type') {
      const matches = m.input === m.expect;
      return {
        title: m.title,
        tone: 'danger',
        body: [
          ...m.lines.map((l) => [seg(l)]),
          [],
          [seg('Type ', 'dim'), seg(m.expect, 'b'), seg(' to confirm', 'dim')],
          [seg('› ', 'danger'), seg(m.input, matches ? 'danger b' : undefined), seg('▏', 'danger')],
        ],
        keys: matches ? [['Enter', 'Delete'], ['Esc', 'Cancel']] : [['Esc', 'Cancel']],
      };
    }
    const icon = m.tone === 'ok' ? '✓' : m.tone === 'warn' ? '!' : '✗';
    const keys: [string, string][] = m.open ? [['Enter', 'Open'], ['Esc', 'Close']] : [['Esc', 'Close']];
    return { title: `${icon} ${m.title}`, tone: m.tone === 'ok' ? 'accent' : m.tone === 'warn' ? 'warn' : 'danger', body: m.text, keys };
  }
}

