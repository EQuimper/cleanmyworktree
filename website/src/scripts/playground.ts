import { type Ev, type Line, type Seg, Sim, agentsMd } from '../lib/sim';
import { type Frame, Tui } from '../lib/tui';

const root = document.querySelector<HTMLElement>('[data-play]');

const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');
const coarse = window.matchMedia('(pointer: coarse)');
const MAX_LINES = 600;
const MIN_COLS = 46;

/** Keeps an empty line one line tall. */
const BLANK = '\u200b';

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function segEl(s: Seg) {
  const span = document.createElement('span');
  if (s.c) span.className = s.c.split(' ').map((c) => `c-${c}`).join(' ');
  if (s.k) span.dataset.hit = `key:${s.k}`;
  span.textContent = s.t;
  return span;
}

function lineEl(l: Line, cls = 'line') {
  const el = document.createElement('span');
  el.className = cls;
  el.append(...l.map(segEl));
  if (!l.some((x) => x.t)) el.append(BLANK);
  return el;
}

if (root) {
  const screen = root.querySelector<HTMLElement>('[data-screen]')!;
  const log = root.querySelector<HTMLElement>('[data-log]')!;
  const tuiEl = root.querySelector<HTMLElement>('[data-tui]')!;
  const title = root.querySelector<HTMLElement>('[data-title]')!;
  const keypad = root.querySelector<HTMLElement>('[data-keypad]')!;
  const input = root.querySelector<HTMLInputElement>('[data-input]')!;
  const promptLine = root.querySelector<HTMLElement>('[data-prompt-line]')!;
  const promptText = root.querySelector<HTMLElement>('[data-prompt]')!;
  const measure = root.querySelector<HTMLElement>('[data-measure]')!;

  let sim = new Sim(root.dataset.version ?? '', root.dataset.agentPrompt ?? '');
  let tui: Tui | undefined;
  let busy = false;
  let history: string[] = [];
  let at = 0;
  let draft = '';
  let frameTimer = 0;
  let generation = 0;

  const prompt = () => `${sim.cwd} $ `;

  const syncPrompt = () => {
    promptText.textContent = prompt();
    title.textContent = sim.cwd;
  };

  const scrollDown = () => {
    screen.scrollTop = screen.scrollHeight;
  };

  const print = (l: Line, cls?: string) => {
    log.append(lineEl(l, cls));
    while (log.childElementCount > MAX_LINES) log.firstElementChild!.remove();
  };

  /** The prompt line as it was submitted. */
  const freeze = (text: string, suffix?: Line) => {
    print([{ t: prompt(), c: 'prompt' }, { t: text, c: 'cmd' }, ...(suffix ?? [])], 'line entered');
  };

  const focus = () => {
    if (!coarse.matches || tui) input.focus({ preventScroll: true });
  };

  // -- the interactive screen ------------------------------------------------------------------

  const fit = () => {
    tuiEl.style.removeProperty('font-size');
    const box = tuiEl.getBoundingClientRect();
    const ch = measure.getBoundingClientRect().width / 10;
    let cols = Math.floor((box.width - 2) / ch);
    if (cols < MIN_COLS) {
      const base = parseFloat(getComputedStyle(tuiEl).fontSize);
      tuiEl.style.fontSize = `${Math.max(7, (base * cols) / MIN_COLS)}px`;
      cols = MIN_COLS;
    }
    const lh = parseFloat(getComputedStyle(tuiEl).lineHeight);
    const rows = Math.floor(box.height / lh);
    return { cols, rows };
  };

  const draw = () => {
    if (!tui) return;
    tui.tick++;
    const { cols, rows } = fit();
    const frame = tui.render(cols, rows);
    paint(frame, rows);
    window.clearTimeout(frameTimer);
    if (tui.animating()) frameTimer = window.setTimeout(draw, 80);
  };

  const paint = (frame: Frame, rows: number) => {
    const nodes: HTMLElement[] = [];
    if (frame.small) {
      const msg = document.createElement('div');
      msg.className = 'tui-small c-dim';
      msg.textContent = 'Make the terminal a little bigger';
      tuiEl.replaceChildren(msg);
      return;
    }
    for (const l of frame.lines.slice(0, rows - 1)) {
      const el = document.createElement('div');
      el.className = l.sel ? 'tl sel' : 'tl';
      if (l.hit) el.dataset.hit = l.hit;
      el.append(...l.segs.map(segEl));
      if (!l.segs.some((x) => x.t)) el.append(BLANK);
      nodes.push(el);
    }
    while (nodes.length < rows - 1) {
      const el = document.createElement('div');
      el.className = 'tl';
      el.append(BLANK);
      nodes.push(el);
    }
    const toast = document.createElement('div');
    toast.className = 'tl';
    if (frame.toast) {
      const [icon, c] = frame.toast.tone === 'ok' ? ['✓', 'accent'] : frame.toast.tone === 'warn' ? ['!', 'warn'] : ['✗', 'danger'];
      toast.append(segEl({ t: `  ${icon} ${frame.toast.msg}`, c }));
    } else toast.append(BLANK);
    nodes.push(toast);

    if (frame.modal || frame.busy) {
      const box = document.createElement('div');
      const tone = frame.busy ? 'accent' : frame.modal!.tone;
      box.className = `tui-box tone-${tone}`;
      const lines: Line[] = [];
      if (frame.busy) {
        lines.push([
          { t: `${['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'][tui!.tick % 10]} `, c: 'accent' },
          { t: frame.busy.label, c: 'b' },
          { t: `   ${frame.busy.secs}s`, c: 'faint' },
        ]);
      } else {
        const m = frame.modal!;
        const head = document.createElement('div');
        head.className = `tui-box-title c-${tone} c-b`;
        head.textContent = ` ${m.title} `;
        box.append(head);
        lines.push(...m.body, [], footerLine(m.keys));
      }
      for (const l of lines) {
        const el = document.createElement('div');
        el.className = 'tl';
        el.append(...l.map(segEl));
        if (!l.some((x) => x.t)) el.append(BLANK);
        box.append(el);
      }
      nodes.push(box);
    }
    tuiEl.replaceChildren(...nodes);
  };

  const footerLine = (keys: [string, string][]): Line => {
    const segs: Seg[] = [];
    keys.forEach(([k, label], i) => {
      if (i) segs.push({ t: '  |  ', c: 'faint' });
      segs.push({ t: `${k} ${label}`, c: 'dim', k });
    });
    return segs;
  };

  const copy = (text: string) => {
    navigator.clipboard?.writeText(text).catch(() => {});
  };

  const openTui = (cwd: string) => {
    tui = new Tui(sim, cwd, draw, closeTui, copy, agentsMd);
    // The prompt's input stays focusable (and invisible) so keys and virtual keyboards reach the screen.
    root.classList.add('in-tui');
    log.hidden = true;
    promptLine.hidden = false;
    tuiEl.hidden = false;
    keypad.hidden = false;
    input.value = '';
    draw();
    focus();
  };

  function closeTui(cd?: string) {
    if (!tui) return;
    tui = undefined;
    window.clearTimeout(frameTimer);
    root!.classList.remove('in-tui');
    tuiEl.hidden = true;
    tuiEl.replaceChildren();
    keypad.hidden = true;
    log.hidden = false;
    if (cd) {
      sim.prev = sim.cwd;
      sim.cwd = cd;
    }
    syncPrompt();
    scrollDown();
    focus();
  }

  // -- the shell -------------------------------------------------------------------------------

  const play = async (evs: Ev[], gen: number) => {
    for (const e of evs) {
      if (gen !== generation) return;
      if ('wait' in e) {
        root.classList.add('running');
        scrollDown();
        await sleep(e.wait);
        root.classList.remove('running');
      } else if ('out' in e) print(e.out, e.err ? 'line stderr' : 'line');
      else if ('clear' in e) log.replaceChildren();
      else if ('copy' in e) copy(e.copy);
      else if ('tui' in e) return openTui(e.cwd);
    }
  };

  const submit = async (text: string) => {
    if (busy) return;
    busy = true;
    const gen = generation;
    const typing = document.activeElement === input;
    freeze(text);
    input.value = '';
    if (text.trim()) {
      if (history[history.length - 1] !== text) history.push(text);
      at = history.length;
      promptLine.hidden = true;
      await play(sim.exec(text), gen);
      if (gen !== generation) return;
      promptLine.hidden = false;
    }
    busy = false;
    syncPrompt();
    scrollDown();
    if (typing) input.focus({ preventScroll: true });
  };

  /** Type a command into the prompt, then run it. */
  const typeAndRun = async (cmd: string) => {
    if (busy) return;
    if (tui) closeTui();
    busy = true;
    const gen = generation;
    input.value = '';
    if (!reduce.matches) {
      for (const ch of cmd) {
        if (gen !== generation) return;
        input.value += ch;
        await sleep(18 + Math.random() * 22);
      }
      await sleep(160);
    }
    if (gen !== generation) return;
    busy = false;
    await submit(cmd);
    focus();
  };

  input.addEventListener('keydown', (e) => {
    if (tui) {
      const named = ['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Enter', 'Escape', 'Backspace', 'Tab'];
      if (e.ctrlKey && (e.key === 'c' || e.key === 'u')) {
        tui.key(e.key, true);
      } else if (named.includes(e.key) || (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.isComposing)) {
        tui.key(e.key);
      } else return;
      e.preventDefault();
      draw();
      return;
    }
    if (busy) {
      if (e.key !== 'Tab') return;
      e.preventDefault();
      return;
    }
    if (e.key === 'Enter') {
      e.preventDefault();
      submit(input.value);
    } else if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      e.preventDefault();
      if (at === history.length) draft = input.value;
      at = Math.max(0, Math.min(history.length, at + (e.key === 'ArrowUp' ? -1 : 1)));
      input.value = at === history.length ? draft : history[at];
      input.setSelectionRange(input.value.length, input.value.length);
    } else if (e.key === 'Tab') {
      e.preventDefault();
      const r = sim.complete(input.value);
      if (r.options.length) {
        freeze(input.value);
        print([{ t: r.options.join('  '), c: 'dim' }]);
      }
      input.value = r.value;
      scrollDown();
    } else if (e.ctrlKey && e.key === 'c') {
      e.preventDefault();
      freeze(input.value, [{ t: '^C', c: 'dim' }]);
      input.value = '';
      scrollDown();
    } else if (e.ctrlKey && e.key === 'l') {
      e.preventDefault();
      log.replaceChildren();
    } else if (e.ctrlKey && e.key === 'u') {
      e.preventDefault();
      input.value = '';
    }
  });

  // Virtual keyboards send text and deletions through `beforeinput`/`input`, not keydown.
  input.addEventListener('beforeinput', (e) => {
    if (!tui || !e.inputType.startsWith('delete')) return;
    e.preventDefault();
    tui.key('Backspace');
    draw();
  });

  input.addEventListener('input', () => {
    const text = input.value;
    if (!tui) return;
    input.value = '';
    for (const ch of text) {
      // A key can close the screen (`q`); the rest of a paste goes nowhere.
      if (!tui) break;
      tui.key(ch);
    }
    draw();
  });

  tuiEl.addEventListener('click', (e) => {
    const hit = (e.target as HTMLElement).closest<HTMLElement>('[data-hit]')?.dataset.hit;
    if (hit && tui) {
      tui.click(hit);
      draw();
    }
    focus();
  });

  tuiEl.addEventListener(
    'wheel',
    (e) => {
      if (!tui) return;
      e.preventDefault();
      tui.key(e.deltaY < 0 ? 'ArrowUp' : 'ArrowDown');
      draw();
    },
    { passive: false },
  );

  keypad.addEventListener('click', (e) => {
    const key = (e.target as HTMLElement).closest<HTMLElement>('[data-key]')?.dataset.key;
    if (!key || !tui) return;
    tui.key(key);
    draw();
  });

  screen.addEventListener('click', () => {
    if (window.getSelection()?.toString()) return;
    focus();
    if (coarse.matches) input.focus({ preventScroll: true });
  });

  for (const chip of root.querySelectorAll<HTMLButtonElement>('[data-run]')) {
    chip.addEventListener('click', () => typeAndRun(chip.dataset.run!));
  }

  root.querySelector('[data-reset]')?.addEventListener('click', () => {
    generation++;
    closeTui();
    sim = new Sim(root.dataset.version ?? '', root.dataset.agentPrompt ?? '');
    history = [];
    at = 0;
    busy = false;
    root.classList.remove('running');
    log.replaceChildren();
    promptLine.hidden = false;
    input.value = '';
    syncPrompt();
    focus();
  });

  window.addEventListener('resize', () => tui && draw());

  // Arriving at #try (the hero button, or a shared link) puts the cursor in the terminal.
  const arrive = () => {
    if (location.hash === '#try' && !coarse.matches) input.focus({ preventScroll: true });
  };
  window.addEventListener('hashchange', arrive);
  // Clicking it again when the URL already ends in #try fires no hashchange.
  for (const a of document.querySelectorAll('a[href="#try"]')) a.addEventListener('click', () => setTimeout(arrive));
  // On a shared link, wait until the browser has finished its own jump to the fragment.
  if (document.readyState === 'complete') setTimeout(arrive);
  else window.addEventListener('load', () => setTimeout(arrive), { once: true });

  // The page renders `cmw ls` at build time; replay it so the session has that history.
  sim.exec('cmw ls');
  history.push('cmw ls');
  at = history.length;
  root.classList.add('live');
  syncPrompt();
}
