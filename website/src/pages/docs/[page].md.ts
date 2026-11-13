import type { APIRoute, GetStaticPaths } from 'astro';
import { docs, mdPath, toMarkdown } from '../../lib/llms';

// The Markdown of every docs page, next to its HTML: /docs/install/ → /docs/install.md.
export const getStaticPaths = (async () =>
  (await docs()).map(({ entry }) => ({
    params: { page: mdPath(entry.id).slice('/docs/'.length, -'.md'.length) },
    props: { entry },
  }))) satisfies GetStaticPaths;

export const GET: APIRoute = ({ props }) =>
  new Response(toMarkdown(props.entry), { headers: { 'Content-Type': 'text/markdown; charset=utf-8' } });
