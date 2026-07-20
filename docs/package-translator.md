# Package translator — Polytope 📦 (planned)

The `package-translator` (Polytope) is the language-agnostic high-level-to-low-level package
interface (`takogami package`). It turns higher-level workflow logic, rules, policies, profiles, and
patterns into lower-level packages, artifacts, adapters, and runtime contracts.

It is the "higher-dimensional package manager" because it packages more than code. Status:
**planned** — this guide is the design target.

## What it packages

```txt
source code · Rust crates · WASM components · TypeScript/Python packages
configs · schemas · policies · profiles · workflow definitions
agent instructions · asset maps · runtime adapters · deployment metadata
```

## Responsibilities

Owns: package definitions and metadata, interface compilation, artifact packaging,
cross-language package mapping, policy/profile/adapter packaging, WASM/WASI package patterns.

Does **not** own: runtime command orchestration, terminal sessions, machine setup, shell
execution, tool detection, or session logs — those belong to the
[runtime controller (Takogami)](runtime-controller.md).

## Workflow

```mermaid
flowchart LR
  A[High-level interface] --> B["package-translator\nPolytope"]
  B --> C[Validate schema]
  C --> D[Resolve target]
  D --> E[Rust crate]
  D --> F[WASM component]
  D --> G[TS / Py package]
  D --> H[Policy bundle]
  E --> L[Artifact + registry entry]
  F --> L
  G --> L
  H --> L
```

## Package types

```txt
code-package        wraps source code or reusable libraries
pattern-package     a reusable architecture or workflow pattern
policy-package      rules, constraints, enforcement metadata
profile-package     a user/workspace/domain profile bundle
agent-package       prompts, skills, scopes, rails, gates
asset-package       a reusable asset bundle with provenance
component-package   a WASM/WASI component package
adapter-package     a toolchain or package-manager adapter
deployment-package  deployment/infrastructure targets (future)
```

## Relationship to native package managers

The package translator does not replace native package managers — it compiles, wraps, links,
validates, and describes packages across systems. A TypeScript target is still installed by
pnpm/bun/npm; a Rust crate is still built by Cargo; a WASM component is still run by the
[portable-component runtime (Wisp)](portable-component-runtime.md)/Wasmtime. The package translator
packages the system meaning; native tools execute native responsibilities.

## Command surface

```bash
takogami package validate <pkg>    takogami package inspect <pkg>
takogami package project <pkg> --target <typescript|rust|python|wasm|oci>
takogami package build <pkg>       takogami package pack <pkg>       takogami package publish <pkg>
```

The package translator does not initially need a standalone executable; its functions are
exposed through the [runtime controller (Takogami)](runtime-controller.md)'s `takogami package` surface.
The preferred high-level verb is `project` — a higher-dimensional package model projected into a
target ecosystem. Package contracts live in the
[metadata plane (Ontarch)](metadata-plane.md).
