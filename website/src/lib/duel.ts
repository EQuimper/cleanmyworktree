// The two terminal sessions in the hero. One source for the static render and the animation.

export type Step =
  | { cmd: string; prompt: string; startClock?: boolean }
  | { out: string; ok?: string }
  | { run: number }
  | { stopClock: true }
  | { spin: true }
  | { idle: string };

const wt = '~/.cmw/worktrees/shop/feat-login';

export const sessions: { id: 'git' | 'cmw'; title: string; disk: string; steps: Step[] }[] = [
  {
    id: 'git',
    title: 'git worktree add + npm install',
    disk: 'Disk: +1.4 GB, a second full node_modules',
    steps: [
      { prompt: '~/code/shop $', cmd: 'git worktree add ../shop-login -b feat/login', startClock: true },
      { out: "Preparing worktree (new branch 'feat/login')" },
      { out: 'HEAD is now at 9c8787e Add checkout flow' },
      { prompt: '~/code/shop $', cmd: 'cd ../shop-login' },
      { prompt: '~/code/shop-login $', cmd: 'npm install' },
      { spin: true },
    ],
  },
  {
    id: 'cmw',
    title: 'cmw new',
    disk: 'Disk: almost nothing extra, the blocks are shared',
    steps: [
      { prompt: '~/code/shop $', cmd: 'cmw new feat/login', startClock: true },
      { run: 1600 },
      { stopClock: true },
      { out: `hydrated ${wt} in 1.6s: 3 cloned, 2 copied, 4 skipped; installs: 1 up to date; `, ok: 'ready' },
      { out: wt },
      { prompt: '~/code/shop $', cmd: 'cmw cd feat/login' },
      { idle: `${wt} $` },
    ],
  },
];
