import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

export default defineConfig({
  site: 'https://cmw.equimper.com',
  integrations: [
    starlight({
      title: 'cmw',
      description: 'Fast, disk-cheap git worktrees for humans and AI agents.',
      favicon: '/favicon.svg',
      logo: { src: './src/assets/mark.svg' },
      customCss: [
        '@fontsource-variable/archivo/wdth.css',
        '@fontsource-variable/martian-mono/index.css',
        './src/styles/docs.css',
      ],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/EQuimper/cleanmyworktree' }],
      editLink: { baseUrl: 'https://github.com/EQuimper/cleanmyworktree/edit/main/website/' },
      sidebar: [
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
      ],
    }),
  ],
});
