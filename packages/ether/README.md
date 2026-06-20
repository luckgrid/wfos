# Ether — portable runtime (planned)

Ether is the WASM/WASI binary interface layer: portable, sandboxed, componentized execution.
Where [Dust](../dust/README.md) is local-native execution, Ether is portable sandboxed
execution.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Targets WASI and the WebAssembly Component Model; default runtime is
  [Wasmtime](https://wasmtime.dev/) (the Dust `ether` module).
- Components declare capabilities (filesystem/network/secret scope); the runtime enforces them.
- [Hypercube](../hypercube/README.md) packages components, [Kraken](../kraken/README.md) runs
  them (`krk ether run …`), and [Archon](../archon/README.md) defines what they may touch.

Design: [`../../docs/portable-runtime.md`](../../docs/portable-runtime.md).
