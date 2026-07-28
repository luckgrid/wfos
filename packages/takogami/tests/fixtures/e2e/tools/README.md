# E2E tool shims

Tracked tree documents the hermetic PATH root. The Phase 4 harness writes
executable shims (`cargo`, `rustc`, `moon`, `rg`, `git`, `demo-bin`) into a
temp copy of this directory at test time. Do not rely on host PATH tools.
