#!/usr/bin/env node
'use strict';

const fs = require('node:fs');
const path = require('node:path');
const { TARGETS } = require('../../bin/platform');

const root = path.resolve(__dirname, '../..');
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
const cargo = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
const cargoVersion = cargo.match(/^version\s*=\s*"([^"]+)"/m)?.[1];

const errors = [];
if (pkg.version !== cargoVersion) errors.push(`package.json ${pkg.version} != Cargo.toml ${cargoVersion}`);
if (pkg.bin?.pageville !== 'bin/pageville.js') errors.push('package bin.pageville must point to bin/pageville.js');
if (!Array.isArray(pkg.files) || !pkg.files.includes('prebuilds')) errors.push('package files must include prebuilds');
if (pkg.scripts?.postinstall) errors.push('postinstall is not allowed; binaries are bundled in the package');
if (new Set(Object.values(TARGETS)).size !== Object.values(TARGETS).length) errors.push('duplicate Rust target mapping');
// Publishing without a license ships an all-rights-reserved package.
if (!pkg.license) errors.push('package.json must declare a license');
if (!/^license\s*=/m.test(cargo)) errors.push('Cargo.toml must declare a license');
if (!fs.existsSync(path.join(root, 'LICENSE'))) errors.push('LICENSE file is missing');
if (!pkg.files.includes('LICENSE')) errors.push('package files must include LICENSE');

if (errors.length) {
  process.stderr.write(`npm check failed:\n- ${errors.join('\n- ')}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`npm metadata OK (${pkg.name}@${pkg.version}; ${Object.keys(TARGETS).length} runtimes)\n`);
}
