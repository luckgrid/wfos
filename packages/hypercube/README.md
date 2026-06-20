# Hypercube — package translator (planned)

Hypercube is the language-agnostic high-level-to-low-level package interface (`hqb`): it turns
workflow logic, rules, policies, profiles, and patterns into packages, artifacts, adapters,
and runtime contracts. It packages more than code.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Package classes: code, pattern, policy, profile, agent, asset, component, adapter,
  deployment.
- Does not replace native package managers — it compiles, wraps, links, validates, and
  describes packages; Cargo/pnpm/bun/Wasmtime still execute native responsibilities.
- Package contracts live in [Archon](../archon/README.md); [Kraken](../kraken/README.md) may
  invoke `hqb` during routing.

Design: [`../../docs/package-translator.md`](../../docs/package-translator.md).
