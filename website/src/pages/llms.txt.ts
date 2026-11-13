import type { APIRoute } from 'astro';
import { docs, facts, mdUrl, site, summary } from '../lib/llms';
import { repo } from '../lib/release';

export const GET: APIRoute = async () => {
  const pages = await docs();
  const groups = [...new Set(pages.map((p) => p.group))];

  const sections = groups.map((group) => {
    const links = pages
      .filter((p) => p.group === group)
      .map(({ label, entry }) => `- [${label}](${mdUrl(entry.id)}): ${entry.data.description}`);
    return `## ${group}\n\n${links.join('\n')}`;
  });

  const text = [
    '# cmw',
    `> ${summary}`,
    facts.map((f) => `- ${f}`).join('\n'),
    ...sections,
    [
      '## Optional',
      '',
      `- [All docs in one file](${site}/llms-full.txt): every page above, in order.`,
      `- [Source code](${repo}): Rust, MIT license.`,
      `- [Releases](${repo}/releases): changelog and binaries.`,
    ].join('\n'),
  ].join('\n\n');

  return new Response(text + '\n', { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
