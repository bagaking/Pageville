#!/usr/bin/env node
'use strict';

const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { TARGETS } = require('../../bin/platform');
const { stageBinary } = require('./stage');

function readOption(name) {
  const index = process.argv.indexOf(name);
  return index === -1 ? null : process.argv[index + 1];
}

const target = readOption('--target');
if (!target || !Object.values(TARGETS).includes(target)) {
  process.stderr.write(
    `Usage: npm run npm:build -- --target <rust-target>\nSupported: ${Object.values(TARGETS).join(', ')}\n`
  );
  process.exitCode = 2;
} else {
  const tool = process.env.PAGEVILLE_BUILD_TOOL || 'cargo';
  const result = spawnSync(tool, ['build', '--locked', '--release', '--target', target], {
    cwd: path.resolve(__dirname, '../..'),
    stdio: 'inherit'
  });
  if (result.error) {
    process.stderr.write(`npm build: could not run ${tool}: ${result.error.message}\n`);
    process.exitCode = 1;
  } else if (result.status !== 0) {
    process.exitCode = result.status || 1;
  } else {
    const binaryName = target.endsWith('-pc-windows-msvc') ? 'pageville.exe' : 'pageville';
    stageBinary(target, path.join('target', target, 'release', binaryName));
  }
}
