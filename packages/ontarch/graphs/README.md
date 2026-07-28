# metadata-plane (Ontarch) graphs — project relationship graph

The project graph: how units, capabilities, and policies relate. This is WfOS's local
**project graph**, analogous to the dependency graphs monorepo tools expose for AI agent
navigation. Canon: metadata-plane graphs.

## Edge contract

[`edges.schema.json`](edges.schema.json) defines the graph format (draft-07 JSON schema):

- **Nodes:** `{ id, kind }` where `kind` is one of `workspace`, `native-toolchain`,
  `metadata-plane`, `runtime-controller`, `package-translator`, `portable-component-runtime`,
  `package`, `app`, `site`, `pattern`, `tool`, `runtime`, `agent`, `policy`, `capability`,
  `actor`, `profile`, `skill`.
- **Edges:** `{ from, rel, to }` where `rel` is one of `provides`, `requires`, `uses`,
  `governed-by`, `blocked-by`, `packaged-by`, `runs-on`, `selects`, `can-invoke`.

## Generation

The graph is **generated**, not hand-authored. `moon run ontarch:sync` derives it from the
unit descriptors' `capabilities.provides`/`capabilities.requires`, cross-unit `uses` edges
(when one unit's `requires` overlaps another's `provides`), policies that govern each unit,
profile `selects` edges, and profile `can-invoke` skill edges:

```txt
wfos        -> provides    -> capability:metadata.registry
wfos        -> requires    -> capability:proto
panoply     -> governed-by -> policy:panoply.agent
agent       -> blocked-by  -> policy:no-agent-git-push
```

Output (host-specific, gitignored under `registry/`):

- `registry/graph.json` — the schema-conformant JSON graph (nodes + edges +
  `registry_generation.source_fingerprints`). This is the **trusted** artifact.
- `registry/graph.dot`  — a Graphviz DOT rendering of the same edges. Generated for humans;
  **not** trusted by Takogami.

## Freshness

Generated `graph.json` embeds `registry_generation.source_fingerprints` over authored inputs.
Consumers recompute those fingerprints and label reads `hit` / `miss` / `stale`. Missing
generation metadata is `stale`. Refresh requires an explicit `moon run ontarch:sync` (or
`ontarch sync`); read-only queries never sync as a side effect.

## Querying

```bash
# all edges from a unit
jq -r '.edges[] | select(.from=="wfos") | "\(.from) -\(.rel)-> \(.to)"' registry/graph.json

# what depends on a capability
jq -r '.edges[] | select(.rel=="requires" and .to=="capability:proto") | .from' registry/graph.json

# render (if Graphviz is installed)
dot -Tsvg registry/graph.dot -o graph.svg
```

## Runtime-controller projection

`takogami graph [--format text|dot|json] [--json]` is implemented. It:

- reads bounded, no-follow `registry/graph.json` only;
- validates schema, semantics, endpoints, and source fingerprints;
- reports hit / miss / stale;
- never runs implicit Ontarch sync;
- never starts a child process;
- never creates an operational command record;
- never trusts sibling `graph.dot`;
- projects deterministic text, DOT, or JSON; global `--json` wraps one `CommandEnvelope`.

## Related

- Descriptors (edge source): [`../descriptors/`](../descriptors/)
- Policies (edge source): [`../policies/`](../policies/)
- Runtime consumer: [`../../takogami/README.md`](../../takogami/README.md)
- Navigation view: `Workstreams/.agents/graphs/README.md`
