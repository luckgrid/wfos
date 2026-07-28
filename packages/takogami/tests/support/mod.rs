//! Shared hermetic Ontarch fixture helpers for E09.S7 tests (S7-R01/R02).
//!
//! Integration tests include this via:
//! `#[path = "support/mod.rs"] mod support;`

#![allow(dead_code)] // shared across integration test crates with different subsets
#![allow(unused_imports)]

mod e2e_harness;
mod ontarch_fixture;
mod payloads;
mod schemas;

pub use e2e_harness::*;
pub use ontarch_fixture::*;
pub use payloads::*;
pub use schemas::*;
