// The two terminal sessions in the hero. One source for the static render and the animation.

export type Step =
  | { cmd: string; prompt: string; startClock?: boolean }
  | { out: string; ok?: string }
  | { run: number }
  | { stopClock: true }
  /** A long-running command: spins until the clock reaches `until` ms, fast-forwarding after `ffAfter`. */
  | { spin: true; until: number; ffAfter: number; speed: number }
  | { idle: string };

const wt = '~/.cmw/worktrees/shop/feat-login';

/** Example duration for a fresh npm install of this tree. Shown as an example, not a measurement. */
export const npmExampleMs = 107_000;

export const sessions: {
  id: 'git' | 'cmw';
  title: string;
  disk: string;
  finalClock: string;
  steps: Step[];
}[] = [
  {
    id: 'git',
    title: 'git worktree add + npm install',
    disk: 'Disk: +1.4 GB, a second full node_modules',
    finalClock: 'done in 1:48',
    steps: [
      { prompt: '~/code/shop $', cmd: 'git worktree add ../shop-login -b feat/login', startClock: true },
      { out: "Preparing worktree (new branch 'feat/login')" },
      { out: 'HEAD is now at 9c8787e Add checkout flow' },
      { prompt: '~/code/shop $', cmd: 'cd ../shop-login' },
      { prompt: '~/code/shop-login $', cmd: 'npm install' },
      { spin: true, until: npmExampleMs, ffAfter: 4000, speed: 30 },
      { out: 'added 1412 packages, and audited 1413 packages in 2m' },
      { out: 'found 0 vulnerabilities' },
      { prompt: '~/code/shop-login $', cmd: 'cp ../shop/.env.local .' },
      { stopClock: true },
      { idle: '~/code/shop-login $' },
    ],
  },
  {
    id: 'cmw',
    title: 'cmw new',
    disk: 'Disk: almost nothing extra, the blocks are shared',
    finalClock: 'ready in 1.6 s',
    steps: [
      { prompt: '~/code/shop $', cmd: 'cmw new feat/login', startClock: true },
      { run: 1600 },
      { stopClock: true },
      { out: `hydrated ${wt} in 1.6s: 3 cloned, 2 copied, 4 skipped; installs: 1 up to date; `, ok: 'ready' },
      { out: wt },
      { prompt: '~/code/shop $', cmd: 'cmw cd feat/login' },
      { prompt: `${wt} $`, cmd: 'npm install' },
      { run: 1000 },
      { out: 'up to date, audited 1413 packages in 1s' },
      { out: 'found 0 vulnerabilities' },
      { idle: `${wt} $` },
    ],
  },
];
