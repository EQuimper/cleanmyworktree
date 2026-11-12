import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

// Cargo.toml is the only place the version is written down.
const manifest = readFileSync(resolve(process.cwd(), '../Cargo.toml'), 'utf8');
const match = manifest.match(/^\[package\][^[]*?^version\s*=\s*"([^"]+)"/m);

if (!match) throw new Error('package.version is missing from Cargo.toml');

export const version = match[1];
export const repo = 'https://github.com/EQuimper/cleanmyworktree';
export const brewInstall = 'brew install equimper/tap/cmw';
