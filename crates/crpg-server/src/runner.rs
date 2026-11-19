//! The background runner (T154): the only module that reads a clock, owns a
//! thread or blocks (Reading 9).
//!
//! [`spawn`] drives a [`HostLoop`] on a background thread at its cadence; the
//! [`RunnerHandle`] is how callers rendezvous with the loop ([`call`]) and
//! stop it. Tests drive the runner with an injected, manually stepped clock,
//! never wall time, so they are deterministic.
//!
//! [`call`]: RunnerHandle::call

use std::fmt;
use std::io;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::run::{HostLoop, Hosting, LoopError};

/// A millisecond clock driving the runner; never decreases.
pub trait Clock: Send + 'static {
    /// The current time in milliseconds.
    fn now_ms(&self) -> u64;
}

/// The wall-clock [`Clock`]: `origin_ms` plus whole milliseconds since
/// construction.
#[derive(Debug, Clone, Copy)]
pub struct SystemClock {
    /// When the clock was constructed; the elapsed time is read from here.
    start: Instant,
    /// The time read at construction: pass the `start_ms` of the loop this
    /// clock will drive (Reading 13).
    origin_ms: u64,
}

impl SystemClock {
    /// A clock reading whole milliseconds since construction.
    pub fn new() -> Self {
        Self::starting_at(0)
    }

    /// Reads `origin_ms` at construction: pass the `start_ms` of the loop it
    /// will drive (Reading 13).
    pub fn starting_at(origin_ms: u64) -> Self {
        Self {
            start: Instant::now(),
            origin_ms,
        }
    }
}

impl Default for SystemClock {
    /// [`SystemClock::new`].
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        let elapsed = u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.origin_ms.saturating_add(elapsed)
    }
}

/// A caller closure waiting for its pass: runs on `&mut HostLoop` after the
/// next step, carrying its result sender inside.
type Task<H> = Box<dyn FnMut(&mut HostLoop<H>) + Send>;

/// One message to the loop thread: a closure to run after the next step, or
/// the order to end with no further step.
enum Msg<H: Hosting> {
    /// A caller closure with its result channel inside; a dropped receiver is
    /// ignored when the closure runs.
    Task(Task<H>),
    /// End with `error: None`, running no further step.
    Stop,
}

/// Drives `host_loop` on a background thread at its cadence.
///
/// `where H: Hosting + Send + 'static, C: Clock`: the thread repeats (a)
/// `host_loop.step(clock.now_ms())`, ending with that step's error; (b) the
/// one pending closure on `&mut host_loop`, sending its result; (c) a wait of
/// `min(next_due_ms − clock.now_ms() (saturating), period_ms)` for the next
/// closure. A closure received while waiting runs at (b) of the next pass,
/// after its step (R-2, R-3).
pub fn spawn<H, C>(host_loop: HostLoop<H>, clock: C) -> io::Result<RunnerHandle<H>>
where
    H: Hosting + Send + 'static,
    C: Clock,
{
    // Named shape to keep the `HostLoop<H>` bound beside the message type.
    fn run<H, C>(mut host_loop: HostLoop<H>, clock: C, control: Receiver<Msg<H>>) -> RunnerExit<H>
    where
        H: Hosting + Send + 'static,
        C: Clock,
    {
        let mut pending: Option<Task<H>> = None;
        let error = loop {
            let now = clock.now_ms();
            if let Err(error) = host_loop.step(now) {
                break Some(error);
            }
            if let Some(mut task) = pending.take() {
                task(&mut host_loop);
            }
            let wait = host_loop
                .next_due_ms()
                .saturating_sub(clock.now_ms())
                .min(host_loop.period_ms());
            match control.recv_timeout(Duration::from_millis(wait)) {
                Ok(Msg::Task(task)) => pending = Some(task),
                Ok(Msg::Stop) => break None,
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break None,
            }
        };
        RunnerExit { host_loop, error }
    }

    let (control_send, control_recv) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("crpg-server-loop".to_owned())
        .spawn(move || run(host_loop, clock, control_recv))?;
    Ok(RunnerHandle {
        control: control_send,
        thread: Some(thread),
    })
}

/// The loop thread's owner: rendezvous with the loop (`call`) and stop it.
///
/// `Send`; dropping an unjoined handle sends `Stop` and joins, discarding
/// the result (R-4).
pub struct RunnerHandle<H: Hosting> {
    /// The loop thread's orders; the thread owns the other end.
    control: Sender<Msg<H>>,
    /// The loop thread, taken by `join` so `Drop` after `join` is a no-op.
    thread: Option<JoinHandle<RunnerExit<H>>>,
}

impl<H: Hosting + Send + 'static> RunnerHandle<H> {
    /// Sends `f` to run on `&mut HostLoop<H>` after the next step and returns
    /// its result channel (R-3).
    pub fn submit<R, F>(&self, f: F) -> Receiver<R>
    where
        R: Send + 'static,
        F: FnOnce(&mut HostLoop<H>) -> R + Send + 'static,
    {
        let (result_send, result_recv) = mpsc::channel();
        let mut f = Some(f);
        let task: Task<H> = Box::new(move |host_loop| {
            if let Some(f) = f.take() {
                let _ = result_send.send(f(host_loop));
            }
        });
        // A failed send means the thread is gone; the task is dropped with
        // its sender, so the returned receiver disconnects (`RunnerGone`).
        let _ = self.control.send(Msg::Task(task));
        result_recv
    }

    /// Runs `f` on the loop thread after the next step and waits for its
    /// result; a disconnect (the runner ended) is [`RunnerGone`] (R-3, R-5).
    pub fn call<R, F>(&self, f: F) -> Result<R, RunnerGone>
    where
        R: Send + 'static,
        F: FnOnce(&mut HostLoop<H>) -> R + Send + 'static,
    {
        self.submit(f).recv().map_err(|_| RunnerGone)
    }

    /// Orders the thread to end with `error: None` after its current wait;
    /// a send error (the thread is gone) is ignored (R-4).
    pub fn stop(&self) {
        let _ = self.control.send(Msg::Stop);
    }

    /// Whether the loop thread has finished.
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_some_and(JoinHandle::is_finished)
    }

    /// Orders the thread to end and returns how it left: its loop as the
    /// last step left it with the step error that ended it, or
    /// [`RunnerPanicked`] when the thread died (R-4, R-5).
    pub fn join(mut self) -> Result<RunnerExit<H>, RunnerPanicked> {
        let _ = self.control.send(Msg::Stop);
        let thread = self
            .thread
            .take()
            .expect("a handle holds its thread until joined");
        thread.join().map_err(|_| RunnerPanicked)
    }
}

impl<H: Hosting> Drop for RunnerHandle<H> {
    /// Sends `Stop` and joins, discarding the result (R-4).
    fn drop(&mut self) {
        let _ = self.control.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// How the loop thread left: its loop as the last step left it (hosting,
/// links and schedule, so it can be taken apart and respawned), with the
/// step error that ended it, if any (R-2, R-4).
pub struct RunnerExit<H: Hosting> {
    /// The loop as its last step left it.
    pub host_loop: HostLoop<H>,
    /// The step error that ended the runner, or `None` for a stop.
    pub error: Option<LoopError<H::Error>>,
}

/// `call` after the runner ended: the step error (R-5) or a panic dropped
/// the closure before it ran (R-2 (a)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerGone;

impl fmt::Display for RunnerGone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RunnerGone at run/runner")
    }
}

impl std::error::Error for RunnerGone {}

/// `join` after the loop thread died: a `Hosting` stage or a caller closure
/// panicked (R-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunnerPanicked;

impl fmt::Display for RunnerPanicked {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RunnerPanicked at run/runner")
    }
}

impl std::error::Error for RunnerPanicked {}
