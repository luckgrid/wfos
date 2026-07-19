# Wisp — portable component runtime (planned)

Wisp is the WASM/WASI binary interface layer: portable, sandboxed, componentized execution.
Where [Panoply](../panoply/README.md) is local-native execution, Wisp is portable sandboxed
execution.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Targets WASI and the WebAssembly Component Model; default runtime is
  [Wasmtime](https://wasmtime.dev/) (the Panoply `wisp` module).
- Components declare capabilities (filesystem/network/secret scope); the runtime enforces them.
- [Polytope](../polytope/README.md) packages components, [Cthulhu](../cthulhu/README.md) runs
  them (`cth portable run …`), and [Ontarch](../ontarch/README.md) defines what they may touch.

Design: [`../../docs/portable-component-runtime.md`](../../docs/portable-component-runtime.md).
