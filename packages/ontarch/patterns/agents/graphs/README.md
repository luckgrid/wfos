# .agents/graphs — workspace, tool, skill, and policy relationships

The navigation view of the project graph: how units, capabilities, and policies relate
(unit → provides/requires → capability, unit → governed-by → policy, agent → blocked-by →
policy). The graph is **generated**, not hand-authored.

- Edge contract (schema): `packages/ontarch/graphs/edges.schema.json`
- Generated graph (host-specific, gitignored): `packages/ontarch/registry/graph.json` and
  `graph.dot`, produced by `moon run ontarch:sync`.

Render or query the generated graph:

```bash
# nodes/edges as JSON
jq '.edges[] | "\(.from) -\(.rel)-> \(.to)"' \
  packages/ontarch/registry/graph.json

# Graphviz (if installed)
dot -Tsvg packages/ontarch/registry/graph.dot -o graph.svg
```

A future runtime-controller `takogami graph` consumes the same artifact.
