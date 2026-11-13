---
title: Disk usage
description: Why du overcounts worktrees, and how cmw measures the space it frees.
---

On APFS, a copy-on-write clone shares disk blocks with the original. A cloned 1.4 GB `node_modules` takes almost no new space. Only files you change in the worktree get their own blocks, so `npm install new-lib` costs the size of `new-lib`.

## `du` overcounts

`du`, Finder and most disk tools report each clone at its full size, because they can't see shared blocks. Ten worktrees with a 1.4 GB `node_modules` each show up as 14 GB, while the real cost is close to 1.4 GB.

That's why:

- `cmw ls --sizes` is labeled *apparent* size.
- `cmw clean-artifacts` dry runs say "APFS clones counted at full size; real savings are lower".
- `cmw gc --yes` and `cmw clean-artifacts --yes` report `freed_bytes` from the change in free disk space before and after. That's the only number that reflects reality on APFS.

`freed_bytes` can be affected by anything else writing to the disk at the same time, such as a build or Time Machine.

## Deletion is fast

cmw moves a worktree into a trash folder on the same volume, which is instant, then deletes it in the background. Your shell gets control back right away.

## Other filesystems

Copy-on-write works on APFS and supported Linux filesystems. See [filesystem limitations](/docs/limitations/). On unsupported filesystems or across volumes, cmw falls back to a plain copy and warns that it uses full disk space.
