//! Cancellable, progress-estimating process handle for the interactive
//! `get()`/`put()` transfer path in `worker/protondriveworker.cpp` —
//! deliberately separate from [`crate::cli`]'s `CommandRunner` (used by every
//! other call, including the daemon's own fire-and-forget pinned-file sync
//! via `cli::upload`/`cli::download`, which has no interactive cancel button
//! and stays on the old blocking path unchanged).
//!
//! The `proton-drive` CLI has no stable progress API (no `--progress` flag,
//! no incremental JSON), and its `-v`/`--verbose` flag was confirmed live to
//! move error detail onto stdout and reduce stderr to an unhelpful separator
//! line (the exact symptom tracked in #38) — so this deliberately never
//! spawns with `-v`. Progress is instead a rough estimate: elapsed time times
//! a running average throughput measured from this process's own completed
//! transfers, capped below the real total until the transfer is actually
//! done. `stdout`/`stderr` capture is unchanged from
//! `cli::RealCommandRunner` (buffered via `wait_with_output()`, never read
//! line-by-line), so `cli::ensure_success`'s error classification is
//! completely unaffected by any of this.

use std::io;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use crate::cli::{
    is_transient_lock_contention, CommandOutput, DriveError, LOCK_CONTENTION_RETRIES,
    LOCK_CONTENTION_RETRY_DELAY,
};

/// How long a single [`TransferHandle::poll`] call blocks waiting for the
/// current attempt to finish before returning control to the caller — the
/// cadence at which `worker/protondriveworker.cpp` re-checks `wasKilled()`
/// and updates `processedSize()`. Short enough to feel responsive to a
/// Cancel click, long enough not to busy-loop.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// A rough starting guess (bytes/sec) used only until this process has
/// completed at least one real transfer in that direction.
const DEFAULT_THROUGHPUT_BYTES_PER_SEC: f64 = 3.0 * 1024.0 * 1024.0;

/// Never let the time-based estimate claim more than this fraction of the
/// real total before the transfer has actually finished.
const ESTIMATE_CAP_FRACTION: f64 = 0.95;

struct Throughput {
    total_bytes: u64,
    total_secs: f64,
}

impl Throughput {
    const fn new() -> Self {
        Self {
            total_bytes: 0,
            total_secs: 0.0,
        }
    }

    fn bytes_per_sec(&self) -> f64 {
        if self.total_secs <= 0.0 {
            DEFAULT_THROUGHPUT_BYTES_PER_SEC
        } else {
            self.total_bytes as f64 / self.total_secs
        }
    }

    fn record(&mut self, bytes: u64, secs: f64) {
        if secs > 0.0 {
            self.total_bytes += bytes;
            self.total_secs += secs;
        }
    }
}

static UPLOAD_THROUGHPUT: Mutex<Throughput> = Mutex::new(Throughput::new());
static DOWNLOAD_THROUGHPUT: Mutex<Throughput> = Mutex::new(Throughput::new());

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

impl Direction {
    fn throughput(self) -> &'static Mutex<Throughput> {
        match self {
            Direction::Upload => &UPLOAD_THROUGHPUT,
            Direction::Download => &DOWNLOAD_THROUGHPUT,
        }
    }

    /// Recovers from a poisoned mutex instead of propagating the panic — a
    /// worker process crashing because a *different*, unrelated transfer
    /// panicked while holding this lock would be a much worse outcome than
    /// occasionally reading/writing slightly-off throughput bookkeeping.
    fn lock(self) -> std::sync::MutexGuard<'static, Throughput> {
        self.throughput()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// Result of one [`TransferHandle::poll`] call.
pub enum TransferPoll {
    Pending { estimated_bytes: u64 },
    Done(Result<CommandOutput, DriveError>),
}

struct Attempt {
    pid: u32,
    rx: mpsc::Receiver<io::Result<std::process::Output>>,
}

fn spawn(program: &str, args: &[String]) -> Result<Attempt, DriveError> {
    let child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Its own process group (pgid == its own pid), not this worker's —
        // confirmed live that `proton-drive`/a plain `sh -c` can fork a
        // child of its own (e.g. `sh -c "sleep 30"` spawns a real `sleep`
        // child rather than exec-replacing itself on this system), and
        // killing only the direct pid then leaves that grandchild running
        // as an orphan. `kill()` below signals the whole group instead.
        .process_group(0)
        .spawn()
        .map_err(|err| DriveError::Spawn(err.to_string()))?;
    let pid = child.id();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    Ok(Attempt { pid, rx })
}

/// A single in-flight (or just-finished) transfer, driven by repeated
/// [`poll`](Self::poll) calls from the KIO worker's synchronous `get()`/
/// `put()` — there is no background thread here beyond the one draining the
/// child's stdout/stderr (same shape as `cli::RealCommandRunner::run_once`),
/// so nothing outlives the caller that isn't already reaped by [`Drop`].
pub struct TransferHandle {
    program: String,
    args: Vec<String>,
    direction: Direction,
    total_bytes: u64,
    started_at: Instant,
    bytes_per_sec_estimate: f64,
    attempt: Attempt,
    attempts_made: u32,
    cancelled: bool,
    pid: AtomicU32,
}

impl TransferHandle {
    pub fn start(
        direction: Direction,
        args: Vec<String>,
        total_bytes: u64,
    ) -> Result<Self, DriveError> {
        Self::start_program("proton-drive".to_string(), direction, args, total_bytes)
    }

    /// Same as [`start`](Self::start) but with the spawned program
    /// parameterized — lets tests exercise the real spawn/poll/cancel
    /// mechanics against a plain `/bin/sh`, without needing a live,
    /// authenticated `proton-drive` installation the way `cli.rs`'s own
    /// `CommandRunner`-injecting tests avoid it.
    pub(crate) fn start_program(
        program: String,
        direction: Direction,
        args: Vec<String>,
        total_bytes: u64,
    ) -> Result<Self, DriveError> {
        let attempt = spawn(&program, &args)?;
        let pid = attempt.pid;
        let bytes_per_sec_estimate = direction.lock().bytes_per_sec();
        Ok(Self {
            program,
            args,
            direction,
            total_bytes,
            started_at: Instant::now(),
            bytes_per_sec_estimate,
            attempt,
            attempts_made: 1,
            cancelled: false,
            pid: AtomicU32::new(pid),
        })
    }

    fn estimate(&self) -> u64 {
        let elapsed = self.started_at.elapsed().as_secs_f64();
        let raw = elapsed * self.bytes_per_sec_estimate;
        let cap = self.total_bytes as f64 * ESTIMATE_CAP_FRACTION;
        raw.min(cap).max(0.0) as u64
    }

    fn record_throughput(&self) {
        let elapsed = self.started_at.elapsed().as_secs_f64();
        self.direction.lock().record(self.total_bytes, elapsed);
    }

    /// Waits up to [`POLL_INTERVAL`] for the current attempt to finish.
    /// Transparently retries on a transient lock-contention failure (#38,
    /// same bounded-retry policy as `CommandRunner::run`) — the caller never
    /// sees that intermediate failure, only ever a genuine `Done`.
    pub fn poll(&mut self) -> TransferPoll {
        match self.attempt.rx.recv_timeout(POLL_INTERVAL) {
            Ok(Ok(output)) => {
                let out = CommandOutput {
                    stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                    stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
                    success: output.status.success(),
                };
                if !self.cancelled
                    && is_transient_lock_contention(&out)
                    && self.attempts_made < LOCK_CONTENTION_RETRIES
                {
                    thread::sleep(LOCK_CONTENTION_RETRY_DELAY * self.attempts_made);
                    match spawn(&self.program, &self.args) {
                        Ok(attempt) => {
                            self.pid.store(attempt.pid, Ordering::Relaxed);
                            self.attempt = attempt;
                            self.attempts_made += 1;
                            TransferPoll::Pending {
                                estimated_bytes: self.estimate(),
                            }
                        }
                        Err(err) => TransferPoll::Done(Err(err)),
                    }
                } else {
                    if out.success {
                        self.record_throughput();
                    }
                    TransferPoll::Done(Ok(out))
                }
            }
            Ok(Err(err)) => TransferPoll::Done(Err(DriveError::Spawn(err.to_string()))),
            Err(mpsc::RecvTimeoutError::Timeout) => TransferPoll::Pending {
                estimated_bytes: self.estimate(),
            },
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                TransferPoll::Done(Err(DriveError::Spawn(
                    "proton-drive's output thread vanished without a result".to_string(),
                )))
            }
        }
    }

    /// Which throughput bucket this handle contributes to — exposed so
    /// `crate::bridge` can decide whether a finished transfer's
    /// [`CommandOutput`] should be validated as a download or an upload
    /// (`crate::transfer` itself has no notion of "download" vs "upload",
    /// only of a spawned command being polled/cancelled/estimated).
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// Best-effort kill of whatever's currently running. Safe to call
    /// multiple times, or after the transfer already finished on its own.
    pub fn cancel(&mut self) {
        self.cancelled = true;
        kill(self.pid.load(Ordering::Relaxed));
    }
}

/// Negative pid: signals the whole process group spawned with
/// `process_group(0)` above, not just its leader.
fn kill(pid: u32) {
    let _ = Command::new("kill")
        .args(["-9", &format!("-{pid}")])
        .status();
}

impl Drop for TransferHandle {
    /// Defensive: kills any still-running child even if `cancel()` was never
    /// called explicitly (e.g. the handle is dropped after a `finish_*`
    /// parse error). A `kill -9` on an already-exited pid is a harmless
    /// no-op, so this is safe to run unconditionally.
    fn drop(&mut self) {
        kill(self.pid.load(Ordering::Relaxed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poll_until_done(handle: &mut TransferHandle) -> Result<CommandOutput, DriveError> {
        loop {
            if let TransferPoll::Done(result) = handle.poll() {
                return result;
            }
        }
    }

    #[test]
    fn a_quick_command_completes_successfully() {
        let mut handle = TransferHandle::start_program(
            "/bin/sh".to_string(),
            Direction::Upload,
            vec!["-c".to_string(), "exit 0".to_string()],
            1024,
        )
        .unwrap();

        let out = poll_until_done(&mut handle).unwrap();
        assert!(out.success);
    }

    #[test]
    fn cancel_kills_a_slow_process_promptly() {
        let mut handle = TransferHandle::start_program(
            "/bin/sh".to_string(),
            Direction::Download,
            vec!["-c".to_string(), "sleep 30".to_string()],
            1024,
        )
        .unwrap();

        // Give it a moment to actually be running, then cancel.
        assert!(matches!(handle.poll(), TransferPoll::Pending { .. }));
        handle.cancel();

        let started = Instant::now();
        let out = poll_until_done(&mut handle).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel() should kill the process almost immediately, not wait out the full sleep"
        );
        assert!(!out.success);
    }

    #[test]
    fn exhausts_retries_instead_of_looping_forever_on_persistent_lock_contention() {
        let mut handle = TransferHandle::start_program(
            "/bin/sh".to_string(),
            Direction::Upload,
            vec![
                "-c".to_string(),
                "echo 'database is locked' 1>&2; exit 1".to_string(),
            ],
            1024,
        )
        .unwrap();

        let out = poll_until_done(&mut handle).unwrap();
        assert!(!out.success);
        assert_eq!(handle.attempts_made, LOCK_CONTENTION_RETRIES);
    }

    #[test]
    fn estimate_is_monotonic_and_stays_below_the_cap_until_done() {
        let mut handle = TransferHandle::start_program(
            "/bin/sh".to_string(),
            Direction::Download,
            vec!["-c".to_string(), "sleep 2".to_string()],
            10_000_000,
        )
        .unwrap();

        let mut previous = 0u64;
        for _ in 0..5 {
            match handle.poll() {
                TransferPoll::Pending { estimated_bytes } => {
                    assert!(estimated_bytes >= previous);
                    assert!(estimated_bytes <= (10_000_000f64 * ESTIMATE_CAP_FRACTION) as u64);
                    previous = estimated_bytes;
                }
                TransferPoll::Done(_) => break,
            }
        }
        handle.cancel();
        poll_until_done(&mut handle).ok();
    }
}
