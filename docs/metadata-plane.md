# Metadata plane — Ontarch 📐

The `metadata-plane` (Ontarch) stores WfOS's machine-readable meaning: descriptors, registries,
schemas, policies, profiles, graphs, patterns, models, and package contracts. It exposes no
end-user runtime of its own. Build-time tasks validate authored contracts and generate compact
projections consumed by other packages.

Package entrypoint: [`../packages/ontarch/README.md`](../packages/ontarch/README.md).

## Concepts

```text
Descriptors  describe how units connect.
Registries   index what exists.
Schemas      define contracts for authored and generated data.
Policies     define reusable allow, gate, and block intent.
Profiles     scope policies to an automated session.
Graphs       expose capability, dependency, policy, and profile relationships.
Patterns     seed reusable machine-readable navigation contracts.
Models       define domain meaning (planned).
Packages     define package-translator interfaces (planned).
```

## Authored and generated data

| Path | Kind | Purpose |
|---|---|---|
| `descriptors/` and colocated `*.descriptor.toml` | authored | unit identity, paths, capabilities, and relationships |
| `schemas/` | authored | descriptor, policy, profile, skill, command, runtime-record, and bin contracts |
| `policies/` | authored | native-toolchain, Git, runtime, secret, publish, and bin rails |
| `$AGENTS_HOME/profiles/*.toml` | authored | applied automated-session scope and policy selection |
| `$AGENTS_HOME/skills/*.toml` | authored | curated skill, template, and pattern records |
| `patterns/agents/` | authored | reusable profile, skill, tool, and graph navigation seed |
| `registry/*.json`, `registry/graph.*` | generated | host-specific indexes and relationship projections |
| `registry/BIN-INVENTORY.md` | generated | human-readable bin projection |
| `registry/QUERIES.md`, `registry/queries/*.jq` | authored | registry query cookbook |
| `registry/sessions/*.json` | authored record | tracked build-session provenance |

Generated registry and graph files are host-specific and gitignored. Regenerate them; do not
hand-edit them.

## Tasks

```bash
moon run ontarch:validate
moon run ontarch:sync
moon run ontarch:scan
moon run ontarch:bin-report
moon run ontarch:bin-cleanup
moon run ontarch:agents-init
```

- `validate` checks descriptors, policies, profiles, skills, graph contracts, and bin contracts.
- `sync` emits unit, skill, profile, policy, tool, and graph projections.
- `scan` emits a read-only polyrepo report.
- `bin-report` and `bin-cleanup` produce bounded bin inventory and cleanup contracts.
- `agents-init` materializes the reusable agents-navigation seed into `$AGENTS_HOME`.

The tasks use bash, `awk`, and `jq`. They are read-only over authored source except for generated
registry output and the explicit `$AGENTS_HOME` materialization target.

## README-first workspace discovery

Repository and package instructions belong in maintained README files. Ontarch does not seed or
require a parallel instruction file.

Workspace discovery uses this precedence:

1. explicit `AGENTS_HOME`;
2. explicit `WS_ROOT`;
3. ancestor containing `.agents/`;
4. applied Workstreams root identified by `README.md` plus `Build/src/workspaces/`;
5. standalone WfOS fallback at `$WFOS_ROOT/.agents`.

The name `AGENTS_HOME` remains the environment variable for the machine-readable navigation layer;
it does not imply an `AGENTS.md` file.

## Agents-navigation pattern

`packages/ontarch/patterns/agents/` contains:

- profile contracts and generic examples;
- skill contracts, examples, and reusable template bodies;
- generated-toolkit and graph pointer contracts;
- pattern identity and version.

It intentionally does not contain or generate repository instructions. The owning repository and
nested namespaces provide shared human/automated guidance through README files. Profiles and
policies provide the narrower automated-session rules.

`ontarch agents-init` copies contracts and example records, writes `.pattern-lock`, skips existing
files unless `--force` is used, and leaves reusable skill bodies in the pattern for resolver
fallback.

## Generation and validation

`ontarch sync` reads authored descriptors, policies, profiles, curated skill records, and generated
tool facts. It emits compact JSON plus a graph containing unit dependencies, capabilities, policy
application, profile selection, and skill invocation edges.

`ontarch validate` reads required keys and enums from the schemas so contracts remain the source
of truth. It also verifies:

- profile-to-policy references;
- allowed skill IDs and external-skill scan gates;
- generated graph structure;
- command-output and runtime-record contracts;
- bin inventory and cleanup-plan contracts.

Generated unit and scan documents include source fingerprints. The runtime controller labels reads
as hit, miss, or stale by recomputing those fingerprints. Refresh remains explicit; read-only
queries do not silently regenerate source projections.

## Registry queries

The registry is a precomputed context cache:

```bash
jq -r --arg kind workspace -f packages/ontarch/registry/queries/by-kind.jq \
  packages/ontarch/registry/units.json

jq -r --arg cap proto -f packages/ontarch/registry/queries/requires.jq \
  packages/ontarch/registry/units.json
```

A worker may use one filtered query to orient to machine-readable units and policies, but should
still enter the authored namespace through its README and follow the owning manifest or document
before changing behavior.

## Consumer boundaries

- [Panoply](../packages/panoply/README.md) produces tool facts and consumes Ontarch policy.
- [Takogami](../packages/takogami/README.md) consumes trusted graph and bin contracts, applies
  runtime policy, and records command execution.
- Ontarch remains the metadata owner and never becomes the runtime executor.
- Native manifests remain authoritative for package, dependency, and task behavior.

## Adding metadata

When adding a machine-readable concern:

1. identify the authored authority;
2. add or update the relevant descriptor, schema, policy, profile, or pattern contract;
3. generate projections rather than editing registry output;
4. run `moon run ontarch:validate`;
5. run relevant consumer tests when a contract crosses into Takogami or Panoply.

See [agent configs](agent-configs.md), [agent rails](agent-rails.md), and
[worker guidance](worker-guidance.md).
