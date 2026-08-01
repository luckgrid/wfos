# Worker guidance

WfOS uses the same authored entrypoint for humans, automated agents, scripts, and future worker
interfaces: the nearest maintained `README.md`.

This document holds repository-wide conventions that are too detailed for the root README but
apply across packages. It does not replace package documentation, native manifests, selected
agent profiles, or executable policy enforcement.

## Entry and precedence

When entering the repository:

1. Read the root [`README.md`](../README.md).
2. Read the nearest README for the package, app, docs collection, or nested namespace being
   changed.
3. Follow links to the detailed source for the concern at hand.
4. Inspect current native manifests, source, tests, schemas, and policies before modifying them.
5. For an automated session, resolve and obey the selected profile and policies before taking
   action.

When guidance appears to conflict, use this order:

1. executable safety boundary or selected policy;
2. native manifest, schema, or source contract;
3. nearest namespace README and its linked detailed docs;
4. generated registry or projection;
5. historical session record.

Generated state explains authored sources; it does not silently override them.

## Shared repository rules

- Run workspace commands from the repository root unless a package README documents a narrower
  working directory.
- Keep packages modular. Do not move wider-system capability into WfOS merely because WfOS can
  integrate with it.
- Keep native manifests authoritative. Ontarch adds semantic metadata and policy relationships;
  it does not replace Cargo, package, task, or lock files.
- Prefer stable archetype identifiers in architecture and interfaces. Use product names when the
  document or code intentionally addresses a concrete distribution package.
- Keep public WfOS documentation self-contained. Do not require private Workstreams documents to
  understand a package or public contract.
- Treat example Workstreams paths as conventions with documented override points, not as a
  mandatory filesystem layout.
- Keep setup, quick-start, status, and package-inventory information deduplicated. Put each detail
  in one maintained home and link to it.
- Target macOS and Linux first. Include Windows support when it is low-effort and covered by the
  relevant interface or test.
- Preserve historical session and evidence records. Correct current docs and code rather than
  rewriting old records as though they were authored under the new model.

## Automated-worker model

README files provide shared orientation. They must not duplicate a complete automated-agent
policy.

Automated-worker behavior is split across:

```text
.agents/profiles/*.toml
  session scope, allowed and blocked paths, command classes, validators, isolation,
  skill loading, output compression, and log targets

packages/ontarch/policies/*.toml
  reusable allow, gate, and block intent

packages/ontarch/patterns/agents/
  reusable profile, skill, tool, and graph navigation contracts

packages/ontarch/registry/
  generated projections plus tracked session provenance

runtime-controller enforcement
  command-time policy checks and durable execution records
```

The selected profile and policy are authoritative for an automated session. App-specific config
may consume that data but must not become a second policy source of truth or contain duplicated
secrets.

See [agent configs](agent-configs.md) and [agent rails](agent-rails.md).

## Safe default posture

Before an automated worker performs a mutation, it should be able to answer:

- Which README defines this namespace?
- Which authored source owns the behavior being changed?
- Which profile bounds the session?
- Which policy applies to the command or resource?
- Which validation task proves the change?
- Which generated outputs must be regenerated rather than edited?
- Which state or session record owns the handoff?

Stop on security-boundary ambiguity instead of inferring permission from prose.

## Package-specific entrypoints

- [`packages/panoply/README.md`](../packages/panoply/README.md) — native-toolchain commands,
  manifest authority, bootstrap boundary, and validation.
- [`packages/ontarch/README.md`](../packages/ontarch/README.md) — metadata contracts, generated
  registry, profiles, policies, and validation.
- [`packages/takogami/README.md`](../packages/takogami/README.md) — runtime-controller commands,
  policy enforcement, graph/bin behavior, and records.
- [`docs/README.md`](README.md) — detailed documentation map.

## Change checklist

Before completing a repository change:

- [ ] The nearest README still provides an accurate entrypoint.
- [ ] Detailed rules live in docs, manifests, schemas, policies, or tests rather than being
      duplicated in multiple READMEs.
- [ ] Generated outputs were regenerated, not hand-edited.
- [ ] Relevant Moon or Cargo validation tasks passed.
- [ ] Public docs do not depend on private Workstreams paths or documents.
- [ ] Historical records remain historically accurate.
