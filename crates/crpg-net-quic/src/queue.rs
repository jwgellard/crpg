//! Shared endpoint state, bounded frame queues, wakeups, and the lane tasks
//! (reader, writer, control) that both endpoints run on their I/O thread.
//!
//! The sync side (the caller's thread) and the async side (the endpoint's
//! I/O thread) meet only here: one `std::sync::Mutex<State>`, a `Condvar`
//! for sync waiters, and `tokio::sync::Notify` wakeups for async tasks. The
//! mutex is never held across an `.await`, and every queue is bounded in
//! frames and bytes (E§6, E§7).

use std::collections::{BTreeMap, VecDeque};
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::task::Poll;
use std::time::{Duration, Instant};

use crpg_net::sim::QueueCaps;
use quinn::{
    Connection, ConnectionError, ReadError, ReadExactError, RecvStream, SendStream, VarInt,
    WriteError,
};
use tokio::sync::Notify;

use crate::close::{CloseCode, CloseReason, ConnectionStats, SendError};
use crate::handshake::{frame_header, parse_frame_header, FrameError, Welcome, FRAME_HEADER_BYTES};
use crate::server::{ConnectionId, ServerEvent};

/// Extra time a sync waiter allows beyond a close deadline before it stops
/// waiting for the I/O thread (a liveness guard, not a protocol timer).
pub(crate) const SYNC_GRACE: Duration = Duration::from_secs(1);

/// One of two futures' outputs.
pub(crate) enum Either<A, B> {
    Left(A),
    Right(B),
}

/// Polls `a` then `b`, returning whichever completes first (no macros).
pub(crate) async fn race<A: Future, B: Future>(a: A, b: B) -> Either<A::Output, B::Output> {
    let mut a = pin!(a);
    let mut b = pin!(b);
    std::future::poll_fn(move |cx| {
        if let Poll::Ready(out) = a.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(out));
        }
        if let Poll::Ready(out) = b.as_mut().poll(cx) {
            return Poll::Ready(Either::Right(out));
        }
        Poll::Pending
    })
    .await
}

/// `Instant::now() + ms`, saturating far in the future on overflow.
pub(crate) fn deadline_after(ms: u32) -> Instant {
    let now = Instant::now();
    now.checked_add(Duration::from_millis(u64::from(ms)))
        .unwrap_or(now)
}

/// The 4-byte header followed by `payload`, checked against `cap`.
pub(crate) fn framed(payload: &[u8], cap: usize) -> Result<Vec<u8>, FrameError> {
    let header = frame_header(payload.len(), cap)?;
    let mut out = Vec::with_capacity(FRAME_HEADER_BYTES + payload.len());
    out.extend_from_slice(&header);
    out.extend_from_slice(payload);
    Ok(out)
}

/// The E§8.1 mapping from quinn's connection error to a close reason.
pub(crate) fn reason_from(error: &ConnectionError) -> CloseReason {
    match error {
        ConnectionError::ApplicationClosed(close) => CloseReason::Peer {
            code: close.error_code.into_inner(),
        },
        ConnectionError::TimedOut => CloseReason::IdleTimeout,
        ConnectionError::Reset => CloseReason::Reset,
        // Local closes record their own code before calling into quinn, so
        // this fallback is only reached by a close nothing recorded.
        ConnectionError::LocallyClosed => CloseReason::Local(CloseCode::Normal),
        ConnectionError::TransportError(_)
        | ConnectionError::ConnectionClosed(_)
        | ConnectionError::VersionMismatch
        | ConnectionError::CidsExhausted => CloseReason::Transport,
    }
}

/// Sends an application CONNECTION_CLOSE with the code's name as reason.
pub(crate) fn close_quinn(connection: &Connection, code: CloseCode) {
    connection.close(VarInt::from_u32(code.as_u32()), code.name().as_bytes());
}

/// Where a connection is in the E§5 sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    /// TLS done, hello not yet read: invisible to the API.
    Hello,
    /// `HelloReceived` emitted; awaiting `accept`/`refuse`.
    Pending,
    /// Welcome queued or written; lane frames flow.
    Accepted,
}

/// Wakeups for one connection's async tasks.
#[derive(Default)]
pub(crate) struct ConnWake {
    pub(crate) control: Notify,
    pub(crate) reader: Notify,
    pub(crate) writer: Notify,
}

/// A `Flush` close in progress.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlushRequest {
    pub(crate) code: CloseCode,
    pub(crate) deadline: Instant,
    pub(crate) started: bool,
}

/// One connection's record.
pub(crate) struct Conn {
    pub(crate) phase: Phase,
    pub(crate) hello_version: u8,
    /// Set by `accept`; taken by the control task, which writes it first.
    pub(crate) welcome: Option<Welcome>,
    inbound: VecDeque<Vec<u8>>,
    inbound_bytes: usize,
    parked: Option<Vec<u8>>,
    /// Header-prefixed frames; accounting uses payload bytes.
    outbound: VecDeque<Vec<u8>>,
    outbound_bytes: usize,
    /// Outbound frames ever discarded (closes); reports take deltas.
    pub(crate) discarded: usize,
    /// A local `Flush` close has begun with this code.
    pub(crate) closing: Option<CloseCode>,
    /// First recorded reason the connection ended (first wins).
    pub(crate) reason: Option<CloseReason>,
    /// The lane reader may still add frames; publication waits for it.
    pub(crate) reader_active: bool,
    /// The close is visible to the API (event pushed, `try_recv` may end).
    pub(crate) published: bool,
    pub(crate) event_polled: bool,
    pub(crate) closed_reported: bool,
    /// Its `Closed` was polled and reported: the API no longer sees it. The
    /// record stays only until the control task has taken `quinn_close`.
    pub(crate) released: bool,
    /// The control task must send this application close.
    pub(crate) quinn_close: Option<CloseCode>,
    pub(crate) flush: Option<FlushRequest>,
    pub(crate) writer_finished: bool,
    pub(crate) close_done: bool,
    pub(crate) control_done: bool,
    pub(crate) connection: Connection,
    pub(crate) wake: Arc<ConnWake>,
}

impl Conn {
    pub(crate) fn new(phase: Phase, connection: Connection, wake: Arc<ConnWake>) -> Conn {
        Conn {
            phase,
            hello_version: 0,
            welcome: None,
            inbound: VecDeque::new(),
            inbound_bytes: 0,
            parked: None,
            outbound: VecDeque::new(),
            outbound_bytes: 0,
            discarded: 0,
            closing: None,
            reason: None,
            reader_active: false,
            published: false,
            event_polled: false,
            closed_reported: false,
            released: false,
            quinn_close: None,
            flush: None,
            writer_finished: false,
            close_done: false,
            control_done: false,
            connection,
            wake,
        }
    }

    /// Closing or closed, as `try_send`/`accept`/`refuse`/`close` report it.
    pub(crate) fn close_state(&self) -> Option<CloseReason> {
        self.reason.or(self.closing.map(CloseReason::Local))
    }

    /// No inbound frame queued or parked.
    pub(crate) fn drained(&self) -> bool {
        self.inbound.is_empty() && self.parked.is_none()
    }

    /// Frames queued plus the parked one.
    pub(crate) fn held_inbound_frames(&self) -> usize {
        self.inbound.len() + usize::from(self.parked.is_some())
    }

    pub(crate) fn stats(&self, keepalive_pings_sent: u64) -> ConnectionStats {
        ConnectionStats {
            inbound_frames: self.inbound.len(),
            inbound_bytes: self.inbound_bytes,
            outbound_frames: self.outbound.len(),
            outbound_bytes: self.outbound_bytes,
            reader_parked: self.parked.is_some(),
            keepalive_pings_sent,
        }
    }
}

/// An endpoint stop request (shutdown, drop or client close).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Stop {
    pub(crate) code: CloseCode,
    pub(crate) deadline: Instant,
}

/// What the reader's offer of one complete frame did.
enum Offer {
    Queued,
    Parked,
    Gone,
}

/// Everything both sides share, behind one mutex.
pub(crate) struct State {
    pub(crate) conns: BTreeMap<u64, Conn>,
    inbound_caps: QueueCaps,
    outbound_caps: QueueCaps,
    pub(crate) inbound_frames: usize,
    pub(crate) inbound_bytes: usize,
    pub(crate) outbound_frames: usize,
    pub(crate) outbound_bytes: usize,
    emit_events: bool,
    pub(crate) events: VecDeque<ServerEvent>,
    /// Connection slots held (`max_connections` accounting).
    pub(crate) live: usize,
    pub(crate) refused_handshakes: u64,
    pub(crate) refused_at_limit: u64,
    pub(crate) next_id: u64,
    /// No new connection is admitted.
    pub(crate) stopping: bool,
    pub(crate) stop: Option<Stop>,
}

impl State {
    pub(crate) fn new(
        inbound_caps: QueueCaps,
        outbound_caps: QueueCaps,
        emit_events: bool,
    ) -> State {
        State {
            conns: BTreeMap::new(),
            inbound_caps,
            outbound_caps,
            inbound_frames: 0,
            inbound_bytes: 0,
            outbound_frames: 0,
            outbound_bytes: 0,
            emit_events,
            events: VecDeque::new(),
            live: 0,
            refused_handshakes: 0,
            refused_at_limit: 0,
            next_id: 1,
            stopping: false,
            stop: None,
        }
    }

    /// Queues or parks one complete inbound frame.
    fn offer_inbound(&mut self, id: u64, frame: Vec<u8>) -> Offer {
        let caps = self.inbound_caps;
        let (host_frames, host_bytes) = (self.inbound_frames, self.inbound_bytes);
        let Some(conn) = self.conns.get_mut(&id) else {
            return Offer::Gone;
        };
        let len = frame.len();
        if inbound_room(&caps, conn, host_frames, host_bytes, len) {
            conn.inbound.push_back(frame);
            conn.inbound_bytes += len;
            self.inbound_frames += 1;
            self.inbound_bytes += len;
            Offer::Queued
        } else {
            conn.parked = Some(frame);
            Offer::Parked
        }
    }

    /// Takes the next inbound frame, then lets parked frames in (ascending
    /// connection order) while room allows.
    pub(crate) fn pop_inbound(&mut self, id: u64) -> Option<Vec<u8>> {
        let conn = self.conns.get_mut(&id)?;
        let frame = conn.inbound.pop_front()?;
        conn.inbound_bytes -= frame.len();
        self.inbound_frames -= 1;
        self.inbound_bytes -= frame.len();
        self.promote_parked();
        Some(frame)
    }

    fn promote_parked(&mut self) {
        let caps = self.inbound_caps;
        for conn in self.conns.values_mut() {
            let Some(len) = conn.parked.as_ref().map(Vec::len) else {
                continue;
            };
            if !inbound_room(&caps, conn, self.inbound_frames, self.inbound_bytes, len) {
                continue;
            }
            if let Some(frame) = conn.parked.take() {
                conn.inbound.push_back(frame);
                conn.inbound_bytes += len;
                self.inbound_frames += 1;
                self.inbound_bytes += len;
                conn.wake.reader.notify_one();
            }
        }
    }

    /// Queues one outbound payload, or `QueueFull` with nothing changed
    /// (per-connection frames, then bytes, then host-wide frames, then bytes).
    pub(crate) fn queue_outbound(&mut self, id: u64, payload: &[u8]) -> Result<(), SendError> {
        let caps = self.outbound_caps;
        let len = payload.len();
        let host_frames_ok = self.outbound_frames < caps.host_frames;
        let host_bytes_ok = self
            .outbound_bytes
            .checked_add(len)
            .is_some_and(|total| total <= caps.host_bytes);
        let conn = self
            .conns
            .get_mut(&id)
            .ok_or(SendError::UnknownConnection)?;
        let room = conn.outbound.len() < caps.per_peer_frames
            && conn
                .outbound_bytes
                .checked_add(len)
                .is_some_and(|total| total <= caps.per_peer_bytes)
            && host_frames_ok
            && host_bytes_ok;
        if !room {
            return Err(SendError::QueueFull);
        }
        // The caller checked the send cap, so the header cannot fail; if it
        // ever did, nothing is queued.
        let frame = framed(payload, usize::MAX).map_err(|_| SendError::FrameTooLarge)?;
        conn.outbound.push_back(frame);
        conn.outbound_bytes += len;
        self.outbound_frames += 1;
        self.outbound_bytes += len;
        conn.wake.writer.notify_one();
        Ok(())
    }

    /// The writer takes the next framed outbound frame; it leaves the
    /// accounting here, when it is handed to QUIC.
    fn pop_outbound(&mut self, id: u64) -> Option<Vec<u8>> {
        let conn = self.conns.get_mut(&id)?;
        let frame = conn.outbound.pop_front()?;
        let len = frame.len() - FRAME_HEADER_BYTES;
        conn.outbound_bytes -= len;
        self.outbound_frames -= 1;
        self.outbound_bytes -= len;
        Some(frame)
    }

    fn discard_outbound(&mut self, id: u64) {
        let Some(conn) = self.conns.get_mut(&id) else {
            return;
        };
        let frames = conn.outbound.len();
        let bytes = conn.outbound_bytes;
        conn.outbound.clear();
        conn.outbound_bytes = 0;
        conn.discarded += frames;
        self.outbound_frames -= frames;
        self.outbound_bytes -= bytes;
    }

    /// Records why the connection ended (first wins), discards outbound
    /// frames that can no longer be sent, wakes its tasks, and publishes
    /// the close once the reader is done. `true` if this call recorded it.
    pub(crate) fn record_close(&mut self, id: u64, reason: CloseReason) -> bool {
        let Some(conn) = self.conns.get_mut(&id) else {
            return false;
        };
        if conn.reason.is_some() {
            return false;
        }
        conn.reason = Some(reason);
        conn.wake.reader.notify_one();
        conn.wake.writer.notify_one();
        conn.wake.control.notify_one();
        self.discard_outbound(id);
        self.try_publish(id);
        true
    }

    /// Makes a recorded close visible once no reader can add frames: pushes
    /// `Closed` for connections that produced `HelloReceived`.
    pub(crate) fn try_publish(&mut self, id: u64) {
        let Some(conn) = self.conns.get_mut(&id) else {
            return;
        };
        let Some(reason) = conn.reason else {
            return;
        };
        if conn.reader_active || conn.published {
            return;
        }
        conn.published = true;
        if self.emit_events && conn.phase != Phase::Hello {
            self.events.push_back(ServerEvent::Closed {
                conn: ConnectionId::from_raw(id),
                reason,
            });
        }
    }

    /// Forgets a connection for the API. The record itself is removed at
    /// once unless the control task still has to send its close, in which
    /// case the control task removes it after taking `quinn_close`.
    pub(crate) fn release(&mut self, id: u64) {
        let Some(conn) = self.conns.get_mut(&id) else {
            return;
        };
        conn.released = true;
        if conn.quinn_close.is_none() {
            self.conns.remove(&id);
        }
    }

    /// Ends the reader's claim on the connection and publishes if closed.
    pub(crate) fn reader_finished(&mut self, id: u64) {
        if let Some(conn) = self.conns.get_mut(&id) {
            conn.reader_active = false;
        }
        self.try_publish(id);
    }
}

fn inbound_room(
    caps: &QueueCaps,
    conn: &Conn,
    host_frames: usize,
    host_bytes: usize,
    len: usize,
) -> bool {
    conn.inbound.len() < caps.per_peer_frames
        && conn
            .inbound_bytes
            .checked_add(len)
            .is_some_and(|total| total <= caps.per_peer_bytes)
        && host_frames < caps.host_frames
        && host_bytes
            .checked_add(len)
            .is_some_and(|total| total <= caps.host_bytes)
}

/// The state, the sync waiters' condvar, the failure flag and the
/// endpoint task's wakeup.
pub(crate) struct Shared {
    state: Mutex<State>,
    pub(crate) cond: Condvar,
    pub(crate) failed: AtomicBool,
    pub(crate) endpoint_wake: Notify,
}

impl Shared {
    pub(crate) fn new(state: State) -> Shared {
        Shared {
            state: Mutex::new(state),
            cond: Condvar::new(),
            failed: AtomicBool::new(false),
            endpoint_wake: Notify::new(),
        }
    }

    /// The sync API's lock: `None` once the endpoint failed or the mutex
    /// is poisoned, which callers report as `CloseReason::EndpointFailed`.
    pub(crate) fn lock(&self) -> Option<MutexGuard<'_, State>> {
        if self.failed.load(Ordering::SeqCst) {
            return None;
        }
        self.state.lock().ok()
    }

    /// The lock regardless of the failure flag (I/O tasks, event draining);
    /// `None` only when poisoned.
    pub(crate) fn lock_any(&self) -> Option<MutexGuard<'_, State>> {
        self.state.lock().ok()
    }

    pub(crate) fn is_failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }

    /// Wakes every sync waiter to re-check its condition.
    pub(crate) fn notify_sync(&self) {
        self.cond.notify_all();
    }

    /// Waits on the condvar until `done(state)` or `deadline`; `None` if the
    /// lock is lost (poisoned) meanwhile.
    pub(crate) fn wait_until<'a>(
        &'a self,
        mut guard: MutexGuard<'a, State>,
        deadline: Instant,
        mut done: impl FnMut(&State) -> bool,
    ) -> Option<MutexGuard<'a, State>> {
        loop {
            if done(&guard) || self.is_failed() {
                return Some(guard);
            }
            let now = Instant::now();
            if now >= deadline {
                return Some(guard);
            }
            guard = self.cond.wait_timeout(guard, deadline - now).ok()?.0;
        }
    }
}

/// Marks the endpoint failed if its I/O thread ends without `clean()`
/// (panic or runtime failure): every connection closes with
/// `EndpointFailed`, and sync waiters wake.
pub(crate) struct FailGuard {
    shared: Arc<Shared>,
    clean: bool,
}

impl FailGuard {
    pub(crate) fn new(shared: Arc<Shared>) -> FailGuard {
        FailGuard {
            shared,
            clean: false,
        }
    }

    pub(crate) fn clean(mut self) {
        self.clean = true;
    }
}

impl Drop for FailGuard {
    fn drop(&mut self) {
        if self.clean {
            return;
        }
        if let Some(mut state) = self.shared.lock_any() {
            let ids: Vec<u64> = state.conns.keys().copied().collect();
            for id in ids {
                state.record_close(id, CloseReason::EndpointFailed);
                state.reader_finished(id);
            }
            // No task is left to send a close, so no record waits for one.
            for conn in state.conns.values_mut() {
                conn.quinn_close = None;
            }
            state.conns.retain(|_, conn| !conn.released);
        }
        self.shared.failed.store(true, Ordering::SeqCst);
        self.shared.notify_sync();
    }
}

/// Resolves once a stop has been requested.
pub(crate) async fn wait_stop(shared: &Shared) -> Stop {
    loop {
        match shared.lock_any() {
            Some(state) => {
                if let Some(stop) = state.stop {
                    return stop;
                }
            }
            None => {
                return Stop {
                    code: CloseCode::Shutdown,
                    deadline: Instant::now(),
                }
            }
        }
        shared.endpoint_wake.notified().await;
    }
}

/// Why reading one frame stopped.
pub(crate) enum ReadFail {
    /// Close the connection with this code.
    Code(CloseCode),
    /// The connection is gone.
    Lost,
}

/// Reads one length-prefixed frame: `Ok(None)` on a FIN exactly at a frame
/// boundary. The header is checked against `cap` before any allocation, and
/// a header over the cap is never followed by a body read.
pub(crate) async fn read_frame(
    recv: &mut RecvStream,
    cap: usize,
) -> Result<Option<Vec<u8>>, ReadFail> {
    let mut header = [0u8; FRAME_HEADER_BYTES];
    match recv.read_exact(&mut header).await {
        Ok(()) => {}
        Err(ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(error) => return Err(read_fail(error)),
    }
    let len = parse_frame_header(header, cap).map_err(|error| match error {
        FrameError::Empty => ReadFail::Code(CloseCode::ProtocolViolation),
        FrameError::TooLarge => ReadFail::Code(CloseCode::FrameTooLarge),
    })?;
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(read_fail)?;
    Ok(Some(body))
}

fn read_fail(error: ReadExactError) -> ReadFail {
    match error {
        // FIN mid-frame or a RESET_STREAM on the lane.
        ReadExactError::FinishedEarly(_) | ReadExactError::ReadError(ReadError::Reset(_)) => {
            ReadFail::Code(CloseCode::ProtocolViolation)
        }
        ReadExactError::ReadError(_) => ReadFail::Lost,
    }
}

/// An I/O-side local close: record it, then tell quinn (only if this call
/// recorded it, so a close already decided elsewhere keeps its code).
pub(crate) fn local_close(shared: &Shared, id: u64, connection: &Connection, code: CloseCode) {
    let recorded = shared
        .lock_any()
        .is_some_and(|mut state| state.record_close(id, CloseReason::Local(code)));
    shared.notify_sync();
    if recorded {
        close_quinn(connection, code);
    }
}

/// Reads lane frames into the inbound queue until FIN, close or violation.
pub(crate) async fn lane_reader(
    shared: Arc<Shared>,
    id: u64,
    connection: Connection,
    mut recv: RecvStream,
    cap: usize,
    wake: Arc<ConnWake>,
) {
    loop {
        match read_frame(&mut recv, cap).await {
            Ok(Some(frame)) => {
                if !deliver(&shared, id, frame, &wake).await {
                    break;
                }
            }
            Ok(None) | Err(ReadFail::Lost) => break,
            Err(ReadFail::Code(code)) => {
                local_close(&shared, id, &connection, code);
                break;
            }
        }
    }
    if let Some(mut state) = shared.lock_any() {
        state.reader_finished(id);
    }
    shared.notify_sync();
}

/// Queues `frame`, or parks it and stops reading until it is let in.
/// `false` when reading should stop (the record is gone).
///
/// A close does not end the wait: quinn keeps stream data it already
/// received readable after the connection ends, so once room frees the
/// reader goes on reading it, and the close is published only when the
/// reader finds nothing more (E§7: frames fully received before a close stay
/// drainable after it).
async fn deliver(shared: &Shared, id: u64, frame: Vec<u8>, wake: &ConnWake) -> bool {
    let offer = match shared.lock_any() {
        Some(mut state) => state.offer_inbound(id, frame),
        None => return false,
    };
    shared.notify_sync();
    match offer {
        Offer::Queued => return true,
        Offer::Gone => return false,
        Offer::Parked => {}
    }
    loop {
        wake.reader.notified().await;
        let Some(state) = shared.lock_any() else {
            return false;
        };
        let Some(conn) = state.conns.get(&id) else {
            return false;
        };
        if conn.parked.is_none() {
            return true;
        }
    }
}

enum Next {
    Frame(Vec<u8>),
    Finish,
    Wait,
    Exit,
}

/// Hands queued outbound frames to QUIC in order; on a `Flush` close,
/// finishes the stream once the queue is empty and waits for the peer's ack.
pub(crate) async fn lane_writer(
    shared: Arc<Shared>,
    id: u64,
    connection: Connection,
    mut send: SendStream,
    wake: Arc<ConnWake>,
) {
    loop {
        let next = match shared.lock_any() {
            Some(mut state) => match state.conns.get(&id) {
                None => Next::Exit,
                Some(conn) if conn.reason.is_some() => Next::Exit,
                Some(conn) => {
                    let flushing = conn.flush.is_some();
                    match state.pop_outbound(id) {
                        Some(frame) => Next::Frame(frame),
                        None if flushing => Next::Finish,
                        None => Next::Wait,
                    }
                }
            },
            None => Next::Exit,
        };
        match next {
            Next::Frame(frame) => match send.write_all(&frame).await {
                Ok(()) => {}
                Err(WriteError::Stopped(_)) => {
                    // STOP_SENDING on the lane.
                    local_close(&shared, id, &connection, CloseCode::ProtocolViolation);
                    break;
                }
                Err(_) => break,
            },
            Next::Finish => {
                let _ = send.finish();
                let _ = send.stopped().await;
                break;
            }
            Next::Wait => wake.writer.notified().await,
            Next::Exit => break,
        }
    }
    if let Some(mut state) = shared.lock_any() {
        if let Some(conn) = state.conns.get_mut(&id) {
            conn.writer_finished = true;
        }
    }
    wake.control.notify_one();
}

/// Runs one accepted connection's control loop: sends requested closes,
/// runs `Flush` closes, and records the reason when the connection ends.
pub(crate) async fn lane_control(
    shared: Arc<Shared>,
    id: u64,
    connection: Connection,
    wake: Arc<ConnWake>,
) {
    loop {
        enum Act {
            Close(CloseCode),
            Flush(FlushRequest),
            Nothing,
        }
        let act = match shared.lock_any() {
            Some(mut state) => match state.conns.get_mut(&id) {
                Some(conn) => {
                    if let Some(code) = conn.quinn_close.take() {
                        // A released record waited only for this close.
                        if conn.released {
                            state.conns.remove(&id);
                        }
                        Act::Close(code)
                    } else {
                        match conn.flush.as_mut() {
                            Some(flush) if !flush.started => {
                                flush.started = true;
                                Act::Flush(*flush)
                            }
                            _ => Act::Nothing,
                        }
                    }
                }
                None => Act::Nothing,
            },
            None => Act::Nothing,
        };
        match act {
            Act::Close(code) => close_quinn(&connection, code),
            Act::Flush(flush) => run_flush(&shared, id, &connection, &wake, flush).await,
            Act::Nothing => {}
        }
        match race(wake.control.notified(), connection.closed()).await {
            Either::Left(()) => continue,
            Either::Right(error) => {
                if let Some(mut state) = shared.lock_any() {
                    state.record_close(id, reason_from(&error));
                }
                break;
            }
        }
    }
    if let Some(mut state) = shared.lock_any() {
        if let Some(conn) = state.conns.get_mut(&id) {
            conn.control_done = true;
            // A close requested after the last check is never sent now.
            conn.quinn_close = None;
            if conn.released {
                state.conns.remove(&id);
            }
        }
    }
    shared.notify_sync();
}

/// E§3 `CloseMode::Flush`: let the writer drain and FIN, wait for the ack
/// up to the deadline, discard what is left, then CONNECTION_CLOSE.
async fn run_flush(
    shared: &Shared,
    id: u64,
    connection: &Connection,
    wake: &ConnWake,
    flush: FlushRequest,
) {
    wake.writer.notify_one();
    let deadline = tokio::time::Instant::from_std(flush.deadline);
    loop {
        let finished = match shared.lock_any() {
            Some(state) => state
                .conns
                .get(&id)
                .is_none_or(|conn| conn.writer_finished || conn.reason.is_some()),
            None => true,
        };
        if finished {
            break;
        }
        match race(wake.control.notified(), tokio::time::sleep_until(deadline)).await {
            Either::Left(()) => continue,
            Either::Right(()) => break,
        }
    }
    let recorded = match shared.lock_any() {
        Some(mut state) => {
            let recorded = state.record_close(id, CloseReason::Local(flush.code));
            if let Some(conn) = state.conns.get_mut(&id) {
                conn.close_done = true;
            }
            recorded
        }
        None => false,
    };
    shared.notify_sync();
    if recorded {
        close_quinn(connection, flush.code);
    }
}
