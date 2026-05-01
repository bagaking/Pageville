#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { TARGETS } = require('../../bin/platform');

const root = path.resolve(__dirname, '../..');
const missing = [];
const targets = new Set(Object.values(TARGETS));
for (const target of targets) {
  const name = target.endsWith('-pc-windows-msvc') ? 'pageville.exe' : 'pageville';
  const binary = path.join(root, 'prebuilds', target, name);
  if (!fs.existsSync(binary)) {
    missing.push(`${target}: ${path.relative(root, binary)}`);
  } else if (name !== 'pageville.exe' && (fs.statSync(binary).mode & 0o111) === 0) {
    missing.push(`${target}: ${path.relative(root, binary)} is not executable`);
  }
}
if (fs.existsSync(path.join(root, 'prebuilds'))) {
  for (const entry of fs.readdirSync(path.join(root, 'prebuilds'), { withFileTypes: true })) {
    if (!targets.has(entry.name)) missing.push(`unexpected target directory: ${entry.name}`);
  }
}
if (missing.length) {
  process.stderr.write('npm verify failed; missing staged binaries:\n');
  for (const item of missing) process.stderr.write(`- ${item}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`npm package binaries OK (${targets.size} targets)\n`);
}
