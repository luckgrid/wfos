# .agents/tools — tool contracts and command surfaces

The navigation view of the local toolkit. Tool *meaning* (manifest, detection, install) lives
in the native-toolchain (Panoply); the machine registry lives in Ontarch (`registry/tools.json`).
This directory is the operator-facing summary derived from both.

## `local-toolkit.yml` (generated)

`moon run ontarch:sync` writes `local-toolkit.yml` from the Panoply manifest +
`registry/tools.json`. It is host-specific (gitignored). Each tool is classified into exactly
one bucket:

| Status | Meaning |
|--------|---------|
| `present` | installed on this host |
| `missing` | a module-default tool that is absent (should be installed via `panoply bootstrap`) |
| `candidate` | an optional tool (`default: false`) not installed — available to adopt |
| `deprecated` | flagged for removal (taxonomy slot; none today) |

Shape:

```yaml
generated_at: "…"
manifest_version: "0.1.0"
host: "Darwin arm64"
summary: { present: 42, missing: 0, candidate: 2, deprecated: 0 }
present:
  - { id: ripgrep, module: nav, default: true }
missing: []
candidate:
  - { id: aube, module: js, default: false }
deprecated: []
```

## Related

- Panoply manifest: `packages/panoply/manifest/panoply.tools.toml`
- Ontarch tools registry: `packages/ontarch/registry/tools.json`
