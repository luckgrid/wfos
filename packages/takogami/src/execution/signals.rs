//! Unix signal forwarding and process-group lifecycle.

use async_trait::async_trait;
use tokio::signal::unix::{Signal, SignalKind, signal};

/// Source of termination signals for the controller process.
#[async_trait]
pub trait SignalSource: Send {
    async fn recv(&mut self) -> Option<i32>;
}

/// Live SIGINT / SIGTERM / SIGHUP listeners.
pub struct UnixSignalSource {
    sigint: Signal,
    sigterm: Signal,
    sighup: Signal,
}

impl UnixSignalSource {
    pub fn install() -> std::io::Result<Self> {
        Ok(Self {
            sigint: signal(SignalKind::interrupt())?,
            sigterm: signal(SignalKind::terminate())?,
            sighup: signal(SignalKind::hangup())?,
        })
    }
}

#[async_trait]
impl SignalSource for UnixSignalSource {
    async fn recv(&mut self) -> Option<i32> {
        tokio::select! {
            v = self.sigint.recv() => v.map(|_| libc::SIGINT),
            v = self.sigterm.recv() => v.map(|_| libc::SIGTERM),
            v = self.sighup.recv() => v.map(|_| libc::SIGHUP),
        }
    }
}

/// Test double that never yields a signal.
#[derive(Debug, Default)]
pub struct NullSignalSource;

#[async_trait]
impl SignalSource for NullSignalSource {
    async fn recv(&mut self) -> Option<i32> {
        std::future::pending::<()>().await;
        None
    }
}

/// RAII guard that SIGKILLs the child process group on drop unless disarmed.
#[derive(Debug)]
pub struct ProcessGroupGuard {
    pgid: Option<libc::pid_t>,
}

impl ProcessGroupGuard {
    pub fn new(pid: u32) -> Self {
        // After setpgid(0, 0) in the child, the process group id equals the child pid.
        Self {
            pgid: Some(pid as libc::pid_t),
        }
    }

    pub fn signal_group(&self, sig: i32) {
        if let Some(pgid) = self.pgid {
            // kill(-pgid, sig) targets the process group.
            unsafe {
                let _ = libc::kill(-pgid, sig);
            }
        }
    }

    pub fn disarm(&mut self) {
        self.pgid = None;
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid.take() {
            unsafe {
                let _ = libc::kill(-pgid, libc::SIGKILL);
            }
        }
    }
}

pub fn signal_name(sig: i32) -> &'static str {
    match sig {
        s if s == libc::SIGINT => "SIGINT",
        s if s == libc::SIGTERM => "SIGTERM",
        s if s == libc::SIGHUP => "SIGHUP",
        s if s == libc::SIGKILL => "SIGKILL",
        _ => "SIGTERM",
    }
}
