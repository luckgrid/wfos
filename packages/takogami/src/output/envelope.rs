use crate::contracts::CommandEnvelope;
use std::io::{self, Write};

/// Emit exactly one JSON document on stdout (no prose, no RTK).
pub fn emit_json(envelope: &CommandEnvelope) -> io::Result<()> {
    let line = serde_json::to_string(envelope)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    writeln!(io::stdout(), "{line}")
}
