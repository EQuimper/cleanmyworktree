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
  const m = Math.floor(s / 60);
  return `${m}:${String(Math.floor(s % 60)).padStart(2, '0')}`;
}

function el(tag: string, cls: string, text = '') {
  const node = document.createElement(tag);
  node.className = cls;
  node.textContent = text;
  return node;
}

async function play(tty: HTMLElement, signal: AbortSignal) {
  const steps: Step[] = JSON.parse(tty.dataset.steps ?? '[]');
  const screen = tty.querySelector<HTMLElement>('[data-screen]')!;
  const clock = tty.querySelector<HTMLElement>('[data-clock]')!;
  let started = 0;
  let ticker = 0;

  const stopTicker = () => window.clearInterval(ticker);
  signal.addEventListener('abort', stopTicker);

  screen.replaceChildren();
  tty.classList.remove('done');
  clock.textContent = '0.0 s';

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
      if (step.startClock) {
        started = performance.now();
        ticker = window.setInterval(() => (clock.textContent = formatClock(performance.now() - started)), 100);
      }
    } else if ('out' in step) {
      const line = el('span', 'line out', step.out);
      if (step.ok) line.append(el('span', 'ok', step.ok));
      screen.append(line);
      await sleep(70, signal);
    } else if ('run' in step) {
      await sleep(step.run, signal);
    } else if ('stopClock' in step) {
      stopTicker();
      clock.textContent = `ready in ${formatClock(performance.now() - started)}`;
      tty.classList.add('done');
    } else if ('spin' in step) {
      const line = el('span', 'line spin', SPINNER[0]);
      screen.append(line);
      let i = 0;
      // Runs until replayed or the page goes away: that's the point.
      while (!signal.aborted) {
        await sleep(80, signal);
        line.textContent = SPINNER[++i % SPINNER.length];
      }
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
