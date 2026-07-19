# portable-component-runtime — Wisp (planned)

The portable-component-runtime (Wisp) is the WASM/WASI binary interface layer: portable, sandboxed, componentized execution.
Where the [native-toolchain (Panoply)](../panoply/README.md) is local-native execution, Wisp is portable sandboxed
execution.

**Status: planned.** This is a placeholder; no crate exists yet.

## Plan

- Targets WASI and the WebAssembly Component Model; default runtime is
  [Wasmtime](https://wasmtime.dev/) (the native-toolchain `wisp` module — distinct from this product).
- Components declare capabilities (filesystem/network/secret scope); the runtime enforces them.
- [package-translator (Polytope)](../polytope/README.md) packages components, [runtime-controller (Cthulhu)](../cthulhu/README.md) runs
  them (`cth portable run …`), and [metadata-plane (Ontarch)](../ontarch/README.md) defines what they may touch.

Design: [`../../docs/portable-component-runtime.md`](../../docs/portable-component-runtime.md).
