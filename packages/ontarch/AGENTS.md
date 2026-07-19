# metadata-plane (Ontarch) agent guide

The metadata-plane is data and contracts. [`README.md`](README.md) and
[`../../docs/metadata-plane.md`](../../docs/metadata-plane.md) are the source of truth.

**Profiles:** agent operating profiles are authored under
[`Workstreams/.agents/profiles/`](../../../../../.agents/profiles/README.md) (tracked TOML);
`ontarch validate` checks them against `schemas/profile.schema.json` and `ontarch sync` flattens
them into `registry/profiles.json`. See [agent-configs.md](../../docs/agent-configs.md).

## Rules

- **Reading is safe.** Descriptors, schemas, and policies are meant to be read by agents to
  understand routing, contracts, and rails.
- **The registry is generated, never hand-edited.** `registry/tools.json` comes from `panoply
  doctor`; `registry/{units,skills,profiles,policies}.json` and the graph come from
  `moon run ontarch:sync`. All are host-specific and gitignored — regenerate, don't edit.
- **`moon run ontarch:validate` is the gate.** It validates every descriptor, policy, profile,
  skill record, and the generated graph against its JSON schema (`schemas/*.schema.json`,
  `graphs/edges.schema.json`).
- **Skill records** are authored under [`Workstreams/.agents/skills/`](../../../../../.agents/skills/README.md);
  `ontarch skills resolve|scan|map` are report-only on-demand tools. See [agent-skills.md](../../docs/agent-skills.md).
  `validate` and `sync` are agent-safe: they read contracts and write only generated output.
- **Policies define the rails you operate under.** `policies/panoply.agent.policy.toml` is enforced
  by the native-toolchain when `PANOPLY_AGENT=1` (mutating substrate commands exit non-zero). `policies/no-agent-git-push.policy.toml`
  is metadata-plane policy metadata for publish actions (push, release, merge) — authoritative intent and
  graph edges today; runtime command blocking deferred to the runtime-controller (Cthulhu), same boundary as direct `pass`/`git`
  invocation on `PATH`.
- **Keep contracts honest.** When adding metadata, follow the native-toolchain example: a descriptor for
  how a product connects, a schema for any generated artifact, a policy for its agent rails.
  Generated output goes under `registry/` (gitignored); contracts and policies stay tracked.
- **Native manifests stay authoritative** — do not duplicate or override `Cargo.toml`,
  `package.json`, or lockfile data in the metadata-plane.
