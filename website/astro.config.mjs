import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { sidebar } from './src/lib/nav';

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
      sidebar,
    }),
  ],
});
