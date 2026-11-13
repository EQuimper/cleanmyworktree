import type { APIRoute } from 'astro';
import { docs, facts, summary, toMarkdown } from '../lib/llms';

export const GET: APIRoute = async () => {
  const pages = (await docs()).map(({ entry }) => toMarkdown(entry));
  const intro = ['# cmw', `> ${summary}`, facts.map((f) => `- ${f}`).join('\n')].join('\n\n');
  const text = [intro, ...pages].join('\n\n---\n\n');
  return new Response(text, { headers: { 'Content-Type': 'text/plain; charset=utf-8' } });
};
