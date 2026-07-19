# Portable component runtime — Wisp (planned)

The `portable-component-runtime` (Wisp) is the WASM/WASI binary interface layer. It makes
low-level workflow capabilities portable, sandboxed, and componentized. Where the
[native toolchain (Panoply)](native-toolchain.md) is local-native execution, Wisp is portable
sandboxed execution. Status: **planned.**

## Scope

```txt
WASM · WASI · WIT · Component Model · Wasmtime
portable components · sandboxed command components · capability declarations
```

Wisp owns component execution, WASI permissions, WIT contracts, portable workflow modules,
and language-independent binary interfaces. It does not replace the native toolchain.

## Capability model

Components declare exactly what they may touch; the runtime enforces it. Example contract:

```toml
id = "descriptor-validator"
kind = "wisp-component"
version = "0.1.0"

[component]
file = "components/descriptor-validator.wasm"
wit  = "schemas/wit/descriptor-validator.wit"

[capabilities]
requires = ["filesystem.read:packages/ontarch"]

[policy]
network = false
secret_read = false
write_scope = ["packages/ontarch/registry"]
```

Example components: a descriptor validator (checks metadata-plane descriptors against schemas),
a policy checker, a prompt linter, a session summarizer, an asset hasher with provenance.

## Relationships

```txt
package-translator packages components.   portable-component-runtime runs components.
runtime-controller controls when and how. metadata-plane defines what they may touch.
```

The default runtime is [Wasmtime](https://wasmtime.dev/) (the native-toolchain `wisp` module),
invoked by the [runtime controller (Cthulhu)](runtime-controller.md) as
`cth portable run <component> --scope <path>`.

## Broader WASM landscape

Wisp targets the [WebAssembly Component Model](https://component-model.bytecodealliance.org/)
and [WASI](https://wasi.dev/). For heavier server-side or multi-tenant component hosting, these
projects are worth tracking as the field matures:

| Project | Niche |
|---------|-------|
| [Spin](https://github.com/spinframework/spin) | Build/run event-driven WASM apps without a container layer |
| [wasmCloud](https://github.com/wasmcloud/wasmcloud) | Distributed actor platform for WASM components |
| [Hyperlight Wasm](https://opensource.microsoft.com/blog/2025/03/26/hyperlight-wasm-fast-secure-and-os-free/) | Micro-VM isolation for WASM, fast and OS-free |

The recurring theme — lighter isolation and faster cold starts than Linux containers for
many workloads — is exactly why WASM/WASI is the portable execution target for WfOS.
