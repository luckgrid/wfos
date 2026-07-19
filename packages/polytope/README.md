# Polytope — package translator (planned)

Polytope is the language-agnostic high-level-to-low-level package interface (`cth package`): it turns
workflow logic, rules, policies, profiles, and patterns into packages, artifacts, adapters,
and runtime contracts. It packages more than code.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Package classes: code, pattern, policy, profile, agent, asset, component, adapter,
  deployment.
- Does not replace native package managers — it compiles, wraps, links, validates, and
  describes packages; Cargo/pnpm/bun/Wasmtime still execute native responsibilities.
- Package contracts live in [Ontarch](../ontarch/README.md); [Cthulhu](../cthulhu/README.md) may
  invoke `cth package` during routing.

Design: [`../../docs/package-translator.md`](../../docs/package-translator.md).
