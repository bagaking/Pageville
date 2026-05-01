#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { TARGETS } = require('../../bin/platform');

const root = path.resolve(__dirname, '../..');
const prebuilds = path.join(root, 'prebuilds');
const supportedTargets = new Set(Object.values(TARGETS));

function usage() {
  process.stderr.write(
    'Usage: node scripts/npm/stage.js --target <rust-target> --binary <path>\n' +
    '       node scripts/npm/stage.js --clean\n'
  );
}

function parse(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === '--clean') out.clean = true;
    else if (arg === '--target') out.target = argv[++i];
    else if (arg === '--binary') out.binary = argv[++i];
    else throw new Error(`unknown option: ${arg}`);
  }
  return out;
}

function stageBinary(target, binary) {
  if (!supportedTargets.has(target)) {
    throw new Error(`unsupported target: ${target}`);
  }
  const source = path.isAbsolute(binary) ? binary : path.resolve(root, binary);
  if (!fs.existsSync(source) || !fs.statSync(source).isFile()) {
    throw new Error(`binary does not exist: ${source}`);
  }
  const name = target.endsWith('-pc-windows-msvc') ? 'pageville.exe' : 'pageville';
  const destinationDir = path.join(prebuilds, target);
  fs.mkdirSync(destinationDir, { recursive: true });
  const destination = path.join(destinationDir, name);
  fs.copyFileSync(source, destination);
  if (name !== 'pageville.exe') fs.chmodSync(destination, 0o755);
  process.stdout.write(`staged ${target} -> ${path.relative(root, destination)}\n`);
}

if (require.main === module) {
  try {
    const args = parse(process.argv.slice(2));
    if (args.clean) {
      fs.rmSync(prebuilds, { recursive: true, force: true });
      process.stdout.write('cleaned prebuilds/\n');
    } else if (!args.target || !args.binary) {
      usage();
      process.exitCode = 2;
    } else {
      stageBinary(args.target, args.binary);
    }
  } catch (error) {
    process.stderr.write(`npm stage: ${error.message}\n`);
    process.exitCode = 1;
  }
}

module.exports = { stageBinary, prebuilds, supportedTargets };
