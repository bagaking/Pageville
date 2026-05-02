# Contributing to Pageville

Pageville is a local-first v0 host. Keep changes small, explicit, and compatible
with the documented loopback-only boundary unless a change deliberately updates
the protocol and its threat model.

Before opening a pull request, run the same checks as CI:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
npm test
bash scripts/verify/t008-acceptance.sh
```

Changes to persistence, routing, lifecycle, or response headers need a regression
test that would fail against the old behavior. Tests should assert user-visible
behavior and should not depend on timing, machine-specific paths, or a developer's
existing daemon.

Please explain the user-facing contract, the failure mode the change closes, and
any compatibility or migration impact in the pull request. Do not commit build
outputs, `prebuilds/`, local runtime data, or files under `.tmp/`.

The project uses MIT licensing. New dependencies should have a compatible license,
be justified in the change description, and be added with a locked dependency graph.
