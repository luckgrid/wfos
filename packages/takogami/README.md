# runtime-controller — Takogami (planned)

The runtime-controller (Takogami) is the WfOS runtime CLI and low-level control interface (`takogami`): discovery, routing,
sessions, and agent rails. It is not the package manager ([package-translator (Polytope)](../polytope/README.md))
and not the tools themselves ([native-toolchain (Panoply)](../panoply/README.md)) — it coordinates them.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Built on [starbase](https://github.com/moonrepo/starbase) (app shell) + `clap` (parsing),
  with Tokio for native tool proxying and Ratatui for a later TUI.
- Routes to the [moon](../../docs/monorepo.md) task graph and [native-toolchain (Panoply)](../panoply/README.md) for
  execution; reads [metadata-plane (Ontarch)](../ontarch/README.md) for metadata and policy.
- The Cargo workspace (`Cargo.toml`) and a `moon.yml` land with this crate; add `takogami` to
  `.moon/workspace.yml` `projects.sources` at that point.
- Runtime integrations (archetype `runtime-integration`, brand vocabulary **Tendril**) live
  inside this package under `src/integrations/` — there is no separate integration package;
  `wfos-takogami` is the sole distribution unit.

Design: [`../../docs/runtime-controller.md`](../../docs/runtime-controller.md) ·
engine: [`../../docs/runtime-architecture.md`](../../docs/runtime-architecture.md).
