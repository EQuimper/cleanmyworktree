import { getCollection, type CollectionEntry } from 'astro:content';
import { sidebar } from './nav';
import { agentPrompt, brewInstall, repo, version } from './release';

export const site = 'https://cmw.equimper.com';

export const summary =
  'cmw makes git worktrees fast to create, cheap on disk, and safe to clean up. It fills a new worktree with the main checkout’s gitignored files: dependencies like node_modules, Pods and vendor become copy-on-write clones, and .env files are copied. A worktree is ready in seconds and costs almost no extra disk. Cleanup removes build caches with the worktree and never deletes uncommitted or unmerged work.';

export const facts = [
  `Install: \`${brewInstall}\`. Version ${version}, a single binary for macOS and Linux (ARM64 and x86-64), written in Rust. MIT license.`,
  'Main commands: `cmw new <branch>` creates and hydrates a worktree, `cmw hydrate` fills an existing one, `cmw ls` lists worktrees, `cmw cd <name>` jumps into one, `cmw rm` and `cmw gc` clean up, and `cmw` alone opens an interactive screen.',
  'Built for AI agents: `--json` on every command, stable exit codes, no prompts, fix hints on every refusal. Works with Claude Code, Codex, Grok and T3 Code.',
  `Source: ${repo}`,
];

type Doc = { label: string; group: string; entry: CollectionEntry<'docs'> };

/** Every docs page, in sidebar order. */
export async function docs(): Promise<Doc[]> {
  const entries = new Map((await getCollection('docs')).map((e) => [e.id, e]));
  return sidebar.flatMap((group) =>
    group.items.map(({ label, slug }) => {
      const entry = entries.get(slug);
      if (!entry) throw new Error(`llms: no docs page for sidebar slug "${slug}"`);
      return { label, group: group.label, entry };
    }),
  );
}

/** Where a page's Markdown is served: /docs/ → /docs/index.md, /docs/install/ → /docs/install.md. */
export function mdPath(id: string) {
  return id === 'docs' ? '/docs/index.md' : `/${id}.md`;
}

export const mdUrl = (id: string) => site + mdPath(id);

/** A page as plain Markdown, with links pointing at the other pages' Markdown. */
export function toMarkdown(entry: CollectionEntry<'docs'>) {
  let body = (entry.body ?? '')
    .replace(/^import .*\n/gm, '')
    .replace(/<Code code=\{agentPrompt\}[^>]*\/>/g, '```md\n' + agentPrompt + '\n```')
    .replace(/\]\(\/docs\/([^)#]*?)\/?(#[^)]*)?\)/g, (_, slug: string, hash = '') =>
      `](${mdUrl(slug ? `docs/${slug}` : 'docs')}${hash})`,
    )
    .replace(/\]\(\//g, `](${site}/`)
    .trim();

  // An MDX component left in the text would reach agents as raw JSX.
  const jsx = body.match(/^\s*<[A-Z][\w.]*[\s/>]/m);
  if (jsx) throw new Error(`llms: ${entry.id} uses <${jsx[0].trim()}…>, add a Markdown replacement for it`);

  const { title, description } = entry.data;
  return `# ${title}\n\n> ${description}\n\n${body}\n`;
}
