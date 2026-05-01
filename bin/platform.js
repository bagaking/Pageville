'use strict';

// Keep this list in sync with the target matrix in
// .github/workflows/npm-release.yml. Linux is split into glibc and musl so a
// statically linked binary is selected on Alpine and other musl distributions.
const TARGETS = Object.freeze({
  'darwin-arm64': 'aarch64-apple-darwin',
  'darwin-x64': 'x86_64-apple-darwin',
  'linux-arm64': 'aarch64-unknown-linux-gnu',
  'linux-x64': 'x86_64-unknown-linux-gnu',
  'linux-musl-arm64': 'aarch64-unknown-linux-musl',
  'linux-musl-x64': 'x86_64-unknown-linux-musl',
  'win32-x64': 'x86_64-pc-windows-msvc'
});

function isMusl() {
  if (process.platform !== 'linux') return false;
  try {
    // glibcVersionRuntime is present on glibc-linked Node builds and absent
    // on musl-linked builds. Older Node versions may not expose report; in
    // that case the glibc default is the least surprising choice.
    return !process.report?.getReport?.().header?.glibcVersionRuntime;
  } catch (_) {
    return false;
  }
}

function runtimeKey(platform = process.platform, arch = process.arch, musl = isMusl()) {
  if (platform === 'linux' && musl) return `linux-musl-${arch}`;
  return `${platform}-${arch}`;
}

function targetForRuntime(platform = process.platform, arch = process.arch, musl = isMusl()) {
  return TARGETS[runtimeKey(platform, arch, musl)] || null;
}

module.exports = { TARGETS, isMusl, runtimeKey, targetForRuntime };
