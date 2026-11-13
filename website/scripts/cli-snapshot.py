#!/usr/bin/env python3
"""Write the playground's help text and static command output from the real CLI.

Usage: python3 website/scripts/cli-snapshot.py [path/to/cmw]
Default binary: target/release/cmw (run `cargo build --release` first).
`cargo test` fails when src/cli.rs and website/src/lib/cli-snapshot.json disagree.
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
BIN = sys.argv[1] if len(sys.argv) > 1 else str(ROOT / 'target/release/cmw')
COMMANDS = [
    '', 'new', 'hydrate', 'ls', 'path', 'rm', 'gc', 'clean-artifacts', 'env', 'env sync',
    'shell-init', 'agents', 'agents install', 'agents prompt', 'skill', 'skill install',
    'skill print', 'update', 'hook', 'hook claude',
]
# Commands whose output doesn't depend on the machine, printed verbatim by the playground.
OUTPUTS = ['shell-init zsh', 'shell-init bash', 'shell-init fish', 'hook claude', 'skill print', 'skill print --agents-md']


def run(args):
    return subprocess.run([BIN, *args], capture_output=True, text=True, check=True).stdout.rstrip()


help = {}
for cmd in COMMANDS:
    args = cmd.split()
    long, short = run([*args, '--help']), run([*args, '-h'])
    help[cmd] = {'long': long} if long == short else {'long': long, 'short': short}
output = {cmd: run(cmd.split()) for cmd in OUTPUTS}

data = {'help': help, 'output': output}
(ROOT / 'website/src/lib/cli-snapshot.json').write_text(json.dumps(data, indent=2, ensure_ascii=False) + '\n')
