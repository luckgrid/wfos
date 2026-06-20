# Archon agent guide

Archon is data and contracts. [`README.md`](README.md) and
[`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) are the source of truth.

## Rules

- **Reading is safe.** Descriptors, schemas, and policies are meant to be read by agents to
  understand routing, contracts, and rails.
- **`registry/tools.json` is generated** by `dust doctor` and host-specific (gitignored).
  Never hand-edit it; regenerate it instead.
- **Policies define the rails you operate under.** `policies/dust.agent.policy.toml` is what
  blocks mutating Dust commands in agent mode — treat it as authoritative, not advisory.
- **Keep contracts honest.** When adding metadata, follow the Dust example: a descriptor for
  how a product connects, a schema for any generated artifact, a policy for its agent rails.
  Generated output goes under `registry/` (gitignored); contracts and policies stay tracked.
- **Native manifests stay authoritative** — do not duplicate or override `Cargo.toml`,
  `package.json`, or lockfile data in Archon.
