# `package-translator` — Polytope 📦 (planned)

The package-translator (Polytope) is the language-agnostic high-level-to-low-level package interface (`takogami package`): it turns
workflow logic, rules, policies, profiles, and patterns into packages, artifacts, adapters,
and runtime contracts. It packages more than code.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Package classes: code, pattern, policy, profile, agent, asset, component, adapter,
  deployment.
- Does not replace native package managers — it compiles, wraps, links, validates, and
  describes packages; Cargo/pnpm/bun/Wasmtime still execute native responsibilities.
- Package contracts live in [metadata-plane (Ontarch)](../ontarch/README.md); [runtime-controller (Takogami)](../takogami/README.md) may
  invoke `takogami package` during routing.

Design: [`../../docs/package-translator.md`](../../docs/package-translator.md).
