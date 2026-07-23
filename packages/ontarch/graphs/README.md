# metadata-plane (Ontarch) graphs — project relationship graph

The project graph: how units, capabilities, and policies relate. This is WfOS's local
**project graph**, analogous to the dependency graphs monorepo tools expose for AI agent
navigation. Canon: metadata-plane graphs.

## Edge contract

[`edges.schema.json`](edges.schema.json) defines the graph format (draft-07 JSON schema):

- **Nodes:** `{ id, kind }` where `kind` is one of `native-toolchain`, `workspace`, `package`,
  `app`, `capability`, `policy`, `actor`.
- **Edges:** `{ from, rel, to }` where `rel` is one of `provides`, `requires`, `uses`,
  `governed-by`, `blocked-by`, `packaged-by`, `runs-on`.

## Generation

The graph is **generated**, not hand-authored. `moon run ontarch:sync` derives it from the
unit descriptors' `capabilities.provides`/`capabilities.requires`, cross-unit `uses` edges
(when one unit's `requires` overlaps another's `provides`), and the policies that govern
each unit:

```txt
wfos        -> provides    -> capability:metadata.registry
wfos        -> requires    -> capability:proto
panoply        -> governed-by -> policy:panoply.agent
agent       -> blocked-by  -> policy:no-agent-git-push
```

Output (host-specific, gitignored under `registry/`):

- `registry/graph.json` — the schema-conformant JSON graph (nodes + edges).
- `registry/graph.dot`  — a Graphviz DOT rendering of the same edges.

## Querying

```bash
# all edges from a unit
jq -r '.edges[] | select(.from=="wfos") | "\(.from) -\(.rel)-> \(.to)"' registry/graph.json

# what depends on a capability
jq -r '.edges[] | select(.rel=="requires" and .to=="capability:proto") | .from' registry/graph.json

# render (if Graphviz is installed)
dot -Tsvg registry/graph.dot -o graph.svg
```

A future runtime-controller (Takogami) `takogami graph` (H09) consumes the same artifact.

## Related

- Descriptors (edge source): [`../descriptors/`](../descriptors/)
- Policies (edge source): [`../policies/`](../policies/)
- Navigation view: `Workstreams/.agents/graphs/README.md`
