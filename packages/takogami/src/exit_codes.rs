//! Stable controller exit codes (distinct from native child exit codes).

pub const SUCCESS: u8 = 0;
pub const INTERNAL: u8 = 1;
pub const USAGE: u8 = 2;
pub const CONTRACT: u8 = 3;
pub const RESOLUTION: u8 = 4;
pub const POLICY_DENY: u8 = 5;
pub const POLICY_GATE: u8 = 6;
pub const STATE_IO: u8 = 7;
pub const EXECUTION_IO: u8 = 8;
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
        STATE_IO => "state_io",
        EXECUTION_IO => "execution_io",
        NOT_IMPLEMENTED => "not_implemented",
        _ => "unknown",
    }
}

pub fn exit_from_signal_number(signal: i32) -> u8 {
    let code = 128i32.saturating_add(signal);
    code.clamp(0, 255) as u8
}

pub fn exit_from_signal_name(name: &str) -> u8 {
    let n = match name {
        "SIGINT" => libc::SIGINT,
        "SIGTERM" => libc::SIGTERM,
        "SIGHUP" => libc::SIGHUP,
        "SIGKILL" => libc::SIGKILL,
        _ => 15,
    };
    exit_from_signal_number(n)
}
