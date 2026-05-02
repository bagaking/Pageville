#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.resolve(__dirname, '../..');
const testDir = path.join(root, 'test');
const files = fs.existsSync(testDir)
  ? fs.readdirSync(testDir)
      .filter((name) => name.endsWith('.test.js'))
      .sort()
      .map((name) => path.join(testDir, name))
  : [];

if (files.length === 0) {
  process.stderr.write('npm test: no JavaScript tests found\n');
  process.exitCode = 1;
} else {
  const result = spawnSync(process.execPath, ['--test', ...files], {
    cwd: root,
    stdio: 'inherit'
  });
  if (result.error) {
    process.stderr.write(`npm test: could not run node: ${result.error.message}\n`);
    process.exitCode = 1;
  } else {
    process.exitCode = result.status === null ? 1 : result.status;
  }
}
