//! Stable controller exit codes (distinct from native child exit codes).
//!
//! Native child exit codes pass through unchanged in later stories (S6).
//! Controller codes occupy a reserved range and must not be silently remapped.

/// Success.
pub const SUCCESS: u8 = 0;
/// Unexpected internal failure.
pub const INTERNAL: u8 = 1;
/// Invalid usage (flags, arguments, unknown command).
pub const USAGE: u8 = 2;
/// Contract / schema-version failure.
pub const CONTRACT: u8 = 3;
/// Resolution failure (reserved for S4).
pub const RESOLUTION: u8 = 4;
/// Policy deny (reserved for S5).
pub const POLICY_DENY: u8 = 5;
/// Policy gate fail-closed (reserved for S5).
pub const POLICY_GATE: u8 = 6;
/// Command recognized but not yet implemented.
pub const NOT_IMPLEMENTED: u8 = 10;

pub fn exit_code_name(code: u8) -> &'static str {
    match code {
        SUCCESS => "success",
        INTERNAL => "internal",
        USAGE => "usage",
        CONTRACT => "contract",
        RESOLUTION => "resolution",
        POLICY_DENY => "policy_deny",
        POLICY_GATE => "policy_gate",
        NOT_IMPLEMENTED => "not_implemented",
        _ => "unknown",
    }
}
