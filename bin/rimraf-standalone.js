#!/usr/bin/env node
// Minimal recursive-remove helper used only by the `clean` npm script.
// Previously this file bundled rimraf@2; that bundle contained ReDoS-vulnerable
// regexes (see issue #8098). Node 14.14+ ships `fs.rm` with recursive+force,
// which does the same job in a handful of lines.

const fs = require('fs');
const path = require('path');

const targets = process.argv.slice(2);
if (targets.length === 0) {
  console.error('usage: rimraf-standalone.js <path> [<path> ...]');
  process.exit(1);
}

for (const target of targets) {
  try {
    fs.rmSync(path.resolve(target), {recursive: true, force: true});
  } catch (err) {
    console.error(`failed to remove ${target}:`, err.message);
    process.exitCode = 1;
  }
}
