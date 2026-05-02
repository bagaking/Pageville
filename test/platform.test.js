const test = require('node:test');
const assert = require('node:assert/strict');

const { TARGETS, runtimeKey, targetForRuntime } = require('../bin/platform');

test('runtime keys map every published target', () => {
  assert.equal(runtimeKey('darwin', 'arm64', false), 'darwin-arm64');
  assert.equal(targetForRuntime('darwin', 'arm64', false), 'aarch64-apple-darwin');
  assert.equal(runtimeKey('darwin', 'x64', false), 'darwin-x64');
  assert.equal(targetForRuntime('darwin', 'x64', false), 'x86_64-apple-darwin');
  assert.equal(runtimeKey('linux', 'x64', false), 'linux-x64');
  assert.equal(targetForRuntime('linux', 'x64', false), 'x86_64-unknown-linux-gnu');
  assert.equal(runtimeKey('linux', 'arm64', true), 'linux-musl-arm64');
  assert.equal(targetForRuntime('linux', 'arm64', true), 'aarch64-unknown-linux-musl');
  assert.equal(runtimeKey('win32', 'x64', false), 'win32-x64');
  assert.equal(targetForRuntime('win32', 'x64', false), 'x86_64-pc-windows-msvc');
});

test('unsupported runtimes fail closed', () => {
  assert.equal(targetForRuntime('freebsd', 'x64', false), null);
  assert.equal(targetForRuntime('linux', 'ppc64', false), null);
});

test('published target mapping is one-to-one', () => {
  const targets = Object.values(TARGETS);
  assert.equal(new Set(targets).size, targets.length);
  assert.equal(targets.length, 7);
});
