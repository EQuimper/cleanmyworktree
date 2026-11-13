// The docs sidebar. llms.txt and llms-full.txt follow the same order.
export const sidebar = [
  {
    label: 'Start here',
    items: [
      { label: 'Get started', slug: 'docs' },
      { label: 'Install', slug: 'docs/install' },
    ],
  },
  {
    label: 'Use cmw',
    items: [
      { label: 'Create worktrees', slug: 'docs/worktrees' },
      { label: 'Interactive screen', slug: 'docs/interactive' },
      { label: 'How hydrate works', slug: 'docs/hydrate' },
      { label: 'Secrets and .env', slug: 'docs/secrets' },
      { label: 'Clean up', slug: 'docs/cleanup' },
      { label: 'Use with agents', slug: 'docs/agents' },
    ],
  },
  {
    label: 'Customize',
    items: [{ label: '.cmw.toml', slug: 'docs/config' }],
  },
  {
    label: 'Reference',
    items: [
      { label: 'Commands', slug: 'docs/commands' },
      { label: 'Built-in rules', slug: 'docs/rules' },
      { label: 'JSON output', slug: 'docs/json' },
      { label: 'Disk usage', slug: 'docs/disk' },
      { label: 'Limitations', slug: 'docs/limitations' },
    ],
  },
];
