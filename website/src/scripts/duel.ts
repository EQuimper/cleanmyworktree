import type { Step } from '../lib/duel';

const duel = document.querySelector<HTMLElement>('[data-duel]');
const reduce = window.matchMedia('(prefers-reduced-motion: reduce)');

const SPINNER = '⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏';

function sleep(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal.addEventListener('abort', () => {
      clearTimeout(t);
      reject(signal.reason);
    });
  });
}

function formatClock(ms: number) {
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)} s`;
  return `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
}

function el(tag: string, cls: string, text = '') {
  const node = document.createElement(tag);
  node.className = cls;
  node.textContent = text;
  return node;
}

/** A session clock that can run faster than real time. */
class Clock {
  elapsed = 0;
  rate = 1;
  private last = 0;
  private timer = 0;

  constructor(private readonly label: HTMLElement) {}

  start() {
    this.elapsed = 0;
    this.last = performance.now();
    this.timer = window.setInterval(() => this.tick(), 50);
  }

  tick() {
    const now = performance.now();
    this.elapsed += (now - this.last) * this.rate;
    this.last = now;
    this.label.textContent = formatClock(this.elapsed);
  }

  stop(text: string) {
    window.clearInterval(this.timer);
    this.label.textContent = text;
  }

  dispose() {
    window.clearInterval(this.timer);
  }
}

async function play(tty: HTMLElement, signal: AbortSignal) {
  const steps: Step[] = JSON.parse(tty.dataset.steps ?? '[]');
  const screen = tty.querySelector<HTMLElement>('[data-screen]')!;
  const ff = tty.querySelector<HTMLElement>('[data-ff]')!;
  const clock = new Clock(tty.querySelector<HTMLElement>('[data-clock]')!);
  signal.addEventListener('abort', () => clock.dispose());

  screen.replaceChildren();
  tty.classList.remove('done');
  ff.hidden = true;
  clock.stop('0.0 s');

  for (const step of steps) {
    if ('cmd' in step) {
      const line = el('span', 'line');
      const typed = el('span', 'cmd');
      const cursor = el('span', 'cursor');
      line.append(el('span', 'prompt', `${step.prompt} `), typed, cursor);
      screen.append(line);
      await sleep(250, signal);
      for (const ch of step.cmd) {
        typed.textContent += ch;
        await sleep(22 + Math.random() * 30, signal);
      }
      await sleep(200, signal);
      cursor.remove();
      if (step.startClock) clock.start();
    } else if ('out' in step) {
      const line = el('span', 'line out', step.out);
      if (step.ok) line.append(el('span', 'ok', step.ok));
      screen.append(line);
      await sleep(70, signal);
    } else if ('run' in step) {
      await sleep(step.run, signal);
    } else if ('stopClock' in step) {
      clock.tick();
      const prefix = tty.dataset.tty === 'cmw' ? 'ready in' : 'done in';
      clock.stop(`${prefix} ${formatClock(clock.elapsed)}`);
      tty.classList.add('done');
    } else if ('spin' in step) {
      const line = el('span', 'line spin', SPINNER[0]);
      screen.append(line);
      let i = 0;
      while (clock.elapsed < step.until) {
        if (clock.rate === 1 && clock.elapsed >= step.ffAfter) {
          clock.rate = step.speed;
          ff.hidden = false;
        }
        await sleep(80, signal);
        line.textContent = SPINNER[++i % SPINNER.length];
      }
      clock.rate = 1;
      clock.elapsed = step.until;
      ff.hidden = true;
      line.remove();
    } else if ('idle' in step) {
      const line = el('span', 'line');
      line.append(el('span', 'prompt', `${step.idle} `), el('span', 'cursor'));
      screen.append(line);
    }
  }
}

if (duel && !reduce.matches) {
  const ttys = [...duel.querySelectorAll<HTMLElement>('[data-tty]')];
  let controller: AbortController | undefined;

  const start = () => {
    controller?.abort();
    controller = new AbortController();
    duel.classList.add('playing');
    for (const tty of ttys) play(tty, controller.signal).catch(() => {});
  };

  const observer = new IntersectionObserver(
    (entries) => {
      if (!entries.some((e) => e.isIntersecting)) return;
      observer.disconnect();
      start();
    },
    { threshold: 0.5 },
  );
  observer.observe(duel);

  duel.querySelector('[data-replay]')?.addEventListener('click', start);
} else {
  duel?.querySelector<HTMLElement>('[data-replay]')?.setAttribute('hidden', '');
}
