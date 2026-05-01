#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { TARGETS, runtimeKey, targetForRuntime } = require('./platform');

function fail(message) {
  process.stderr.write(`pageville: ${message}\n`);
  process.exitCode = 1;
}

const target = targetForRuntime();
if (!target) {
  const supported = Object.keys(TARGETS).sort().join(', ');
  fail(`unsupported platform (${runtimeKey()}); supported runtimes: ${supported}`);
} else {
  // PAGEVILLE_BINARY is intentionally an escape hatch for development and
  // packagers. Normal npm installs use the immutable bundled binary; a source
  // checkout may fall back to target/<target>/release for local smoke tests.
  const filename = process.platform === 'win32' ? 'pageville.exe' : 'pageville';
  const packaged = path.join(__dirname, '..', 'prebuilds', target, filename);
  const localBuild = path.join(__dirname, '..', 'target', target, 'release', filename);
  const candidates = process.env.PAGEVILLE_BINARY
    ? [process.env.PAGEVILLE_BINARY]
    : [packaged, localBuild];
  const executable = candidates.find((candidate) => fs.existsSync(candidate));

  if (!executable) {
    fail(`the npm package has no binary for ${target}; reinstall pageville or build it with ` +
      '`npm run npm:build -- --target ' + target + '`');
  } else {
    const result = spawnSync(executable, process.argv.slice(2), { stdio: 'inherit' });
    if (result.error) {
      fail(`could not start ${executable}: ${result.error.message}`);
    } else if (result.signal) {
      // Match the conventional shell exit code when a child is terminated by
      // a signal. Numbers come from the platform (SIGBUS differs between
      // Linux and macOS), not a hand-maintained table.
      process.exitCode = 128 + (os.constants.signals[result.signal] || 1);
    } else {
      process.exitCode = result.status === null ? 1 : result.status;
    }
  }
}
