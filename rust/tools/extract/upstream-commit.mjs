// Pin the *upstream JS data* revision for every extraction artifact, never the
// local Rust HEAD. A Rust-side commit touching only rust/ must not change a data
// artifact, otherwise every gate's DETERM byte-identity check reports a spurious
// one-off "commit-stamp refresh" (D-T37-3, Task 48/49/50/52/55 precedent).
//
// `paths` is the set of upstream JS files the caller actually reads. Missing
// paths are filtered out (some stages have no stages/<id>/layout.js). If the
// scoped query returns nothing we fall back to the last commit touching src/,
// then to HEAD, so a caller always gets a full 40-char sha — the Rust sim/tuning
// geometry tests assert `source.commit.len() == 40`.
import { existsSync } from 'node:fs';
import { resolve } from 'node:path';
import { execFileSync } from 'node:child_process';

function git(repoRoot, args) {
  try {
    return execFileSync('git', args, {
      cwd: repoRoot,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch {
    return '';
  }
}

const SHA = /^[0-9a-f]{40}$/;

export function upstreamCommit(repoRoot, paths) {
  const existing = [...new Set(paths)].filter((p) => existsSync(resolve(repoRoot, p)));
  if (existing.length) {
    const sha = git(repoRoot, ['log', '-1', '--format=%H', '--', ...existing]);
    if (SHA.test(sha)) return sha;
  }
  const treeSha = git(repoRoot, ['log', '-1', '--format=%H', '--', 'src/']);
  if (SHA.test(treeSha)) return treeSha;
  const head = git(repoRoot, ['rev-parse', 'HEAD']);
  if (SHA.test(head)) return head;
  // Last-resort placeholder outside a git checkout: still a full-width value so
  // the Rust geometry tests (`source.commit.len() == 40`) never see 'unknown'.
  return '0'.repeat(40);
}
