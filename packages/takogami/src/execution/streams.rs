//! Concurrent stream capture, human streaming, and encoding classification.

use std::io::{self, ErrorKind, Write};

use tokio::io::{AsyncRead, AsyncReadExt};

/// Default per-stream capture retained for JSON / RTK buffering.
pub const DEFAULT_CAPTURE_LIMIT: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEncoding {
    Utf8,
    LossyUtf8,
    Binary,
}

impl StreamEncoding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::LossyUtf8 => "lossy-utf-8",
            Self::Binary => "binary",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamCapture {
    pub bytes: Vec<u8>,
    pub total_bytes: u64,
    pub truncated: bool,
    pub encoding: StreamEncoding,
    pub broken_pipe: bool,
    pub read_error: Option<String>,
}

impl StreamCapture {
    pub fn empty() -> Self {
        Self {
            bytes: Vec::new(),
            total_bytes: 0,
            truncated: false,
            encoding: StreamEncoding::Utf8,
            broken_pipe: false,
            read_error: None,
        }
    }
}

/// Deterministic classification: NUL ⇒ binary; else valid UTF-8 ⇒ utf-8; else lossy.
pub fn classify_bytes(bytes: &[u8]) -> StreamEncoding {
    if bytes.contains(&0) {
        return StreamEncoding::Binary;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => StreamEncoding::Utf8,
        Err(_) => StreamEncoding::LossyUtf8,
    }
}

fn is_broken_pipe(err: &io::Error) -> bool {
    err.kind() == ErrorKind::BrokenPipe || err.raw_os_error() == Some(libc::EPIPE)
}

/// Drain a pipe concurrently-safe: retain at most `limit`, count all raw bytes.
pub async fn capture_pipe<R: AsyncRead + Unpin>(mut reader: R, limit: usize) -> StreamCapture {
    let mut retained = Vec::new();
    let mut total: u64 = 0;
    let mut truncated = false;
    let mut buf = [0u8; 8192];
    let mut read_error = None;

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                total = total.saturating_add(n as u64);
                if retained.len() < limit {
                    let room = limit - retained.len();
                    let take = n.min(room);
                    retained.extend_from_slice(&buf[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }
            }
            Err(err) => {
                read_error = Some(format!("stream read failed: {err}"));
                break;
            }
        }
    }

    let encoding = classify_bytes(&retained);
    StreamCapture {
        bytes: retained,
        total_bytes: total,
        truncated,
        encoding,
        broken_pipe: false,
        read_error,
    }
}

/// Destination for human streaming. Locks are taken per write so awaits stay `Send`.
#[derive(Debug, Clone, Copy)]
pub enum StreamDest {
    None,
    Stdout,
    Stderr,
}

/// Human-mode reader: optionally buffer first (RTK), else stream immediately.
///
/// When buffering and the limit is exceeded, flushes retained bytes to `dest` and continues
/// raw streaming without duplication. At EOF while still buffering, flushes retained bytes
/// when `flush_at_eof` is true (raw path); leave them for the caller when false (RTK path).
pub async fn stream_or_buffer<R: AsyncRead + Unpin>(
    mut reader: R,
    dest: StreamDest,
    limit: usize,
    buffer_first: bool,
    flush_at_eof: bool,
) -> StreamCapture {
    let mut retained = Vec::new();
    let mut total: u64 = 0;
    let mut truncated = false;
    let mut buffering = buffer_first;
    let mut broken_pipe = false;
    let mut read_error = None;
    let mut buf = [0u8; 8192];

    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                total = total.saturating_add(n as u64);
                let chunk = &buf[..n];

                if buffering {
                    if retained.len().saturating_add(n) <= limit {
                        retained.extend_from_slice(chunk);
                        continue;
                    }
                    // Overflow: keep a capture prefix, flush once, then stream the rest.
                    let room = limit.saturating_sub(retained.len());
                    if room > 0 {
                        retained.extend_from_slice(&chunk[..room.min(n)]);
                    }
                    write_dest(dest, &retained, &mut broken_pipe);
                    if room < n {
                        write_dest(dest, &chunk[room..], &mut broken_pipe);
                    }
                    truncated = true;
                    buffering = false;
                    retained.truncate(limit);
                    continue;
                }

                if retained.len() < limit {
                    let room = limit - retained.len();
                    let take = n.min(room);
                    retained.extend_from_slice(&chunk[..take]);
                    if take < n {
                        truncated = true;
                    }
                } else {
                    truncated = true;
                }

                write_dest(dest, chunk, &mut broken_pipe);
            }
            Err(err) => {
                read_error = Some(format!("stream read failed: {err}"));
                break;
            }
        }
    }

    if buffering && flush_at_eof {
        write_dest(dest, &retained, &mut broken_pipe);
    }

    let encoding = classify_bytes(&retained);
    StreamCapture {
        bytes: retained,
        total_bytes: total,
        truncated,
        encoding,
        broken_pipe,
        read_error,
    }
}

fn write_dest(dest: StreamDest, bytes: &[u8], broken_pipe: &mut bool) {
    if *broken_pipe || bytes.is_empty() {
        return;
    }
    let result = match dest {
        StreamDest::None => return,
        StreamDest::Stdout => {
            let mut out = io::stdout().lock();
            write_all_ignore_closed(&mut out, bytes)
        }
        StreamDest::Stderr => {
            let mut err = io::stderr().lock();
            write_all_ignore_closed(&mut err, bytes)
        }
    };
    match result {
        Ok(()) => {}
        Err(err) if is_broken_pipe(&err) => *broken_pipe = true,
        Err(_) => {}
    }
}

fn write_all_ignore_closed(writer: &mut dyn Write, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(bytes)?;
    let _ = writer.flush();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use tokio::io::BufReader;

    #[test]
    fn classify_prefers_binary_on_nul() {
        assert_eq!(classify_bytes(b"a\0b"), StreamEncoding::Binary);
        assert_eq!(classify_bytes(b"ok"), StreamEncoding::Utf8);
        assert_eq!(classify_bytes(&[0xff, 0xfe]), StreamEncoding::LossyUtf8);
    }

    #[tokio::test]
    async fn buffer_overflow_marks_truncated_and_keeps_prefix() {
        let input = vec![b'x'; 20];
        let capture = stream_or_buffer(
            BufReader::new(Cursor::new(input)),
            StreamDest::None,
            8,
            true,
            true,
        )
        .await;
        assert!(capture.truncated);
        assert_eq!(capture.bytes.len(), 8);
        assert_eq!(capture.total_bytes, 20);
    }
}
