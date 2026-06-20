# Kraken — runtime controller (planned)

Kraken is the WfOS runtime CLI and low-level control interface (`krk`): discovery, routing,
sessions, and agent rails. It is not the package manager ([Hypercube](../hypercube/README.md))
and not the tools themselves ([Dust](../dust/README.md)) — it coordinates them.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Built on [starbase](https://github.com/moonrepo/starbase) (app shell) + `clap` (parsing),
  with Tokio for native tool proxying and Ratatui for a later TUI.
- Routes to the [moon](../../docs/monorepo.md) task graph and [Dust](../dust/README.md) for
  execution; reads [Archon](../archon/README.md) for metadata and policy.
- The Cargo workspace (`Cargo.toml`) and a `moon.yml` land with this crate; add `kraken` to
  `.moon/workspace.yml` `projects.sources` at that point.

Design: [`../../docs/runtime-controller.md`](../../docs/runtime-controller.md) ·
engine: [`../../docs/runtime-architecture.md`](../../docs/runtime-architecture.md).
