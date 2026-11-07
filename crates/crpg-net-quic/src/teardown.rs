//! One close budget split between the flush and the CONNECTION_CLOSE, and the
//! I/O thread's exit sequence (T098g, D43a).

use std::time::{Duration, Instant};

use crate::close::CloseCode;

/// Ceiling of the slice of `close_flush_timeout_ms` kept for CONNECTION_CLOSE.
pub(crate) const CLOSE_RESERVE_CAP_MS: u32 = 200;

/// `min(CLOSE_RESERVE_CAP_MS, budget_ms / 4)`.
pub(crate) fn close_reserve_ms(budget_ms: u32) -> u32 {
    CLOSE_RESERVE_CAP_MS.min(budget_ms / 4)
}

/// One close budget's two instants, both taken from one `Instant::now()`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CloseDeadlines {
    /// A `Flush` stops waiting: now + (budget − reserve).
    pub(crate) flush_end: Instant,
    /// The endpoint stops waiting: now + budget.
    pub(crate) overall: Instant,
}

/// Overflow falls back to `now` for that instant, as `deadline_after` does.
pub(crate) fn close_deadlines(budget_ms: u32) -> CloseDeadlines {
    let now = Instant::now();
    let flush_ms = budget_ms - close_reserve_ms(budget_ms);
    CloseDeadlines {
        flush_end: after(now, flush_ms),
        overall: after(now, budget_ms),
    }
}

fn after(now: Instant, ms: u32) -> Instant {
    now.checked_add(Duration::from_millis(u64::from(ms)))
        .unwrap_or(now)
}

/// I/O-thread exit: `Endpoint::close(code)`, one `yield_now`, then
/// `wait_idle` bounded by `overall`.
pub(crate) async fn finish_endpoint_close(
    endpoint: &quinn::Endpoint,
    code: CloseCode,
    overall: Instant,
) {
    endpoint.close(
        quinn::VarInt::from_u32(code.as_u32()),
        code.name().as_bytes(),
    );
    tokio::task::yield_now().await;
    let overall = tokio::time::Instant::from_std(overall);
    let _ = tokio::time::timeout_at(overall, endpoint.wait_idle()).await;
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use quinn::{EndpointConfig, TokioRuntime};

    use super::{close_deadlines, close_reserve_ms, finish_endpoint_close};
    use crate::close::{CloseCode, CloseReason};
    use crate::handshake::ALPN_LANE0_V1;
    use crate::identity::{CertificatePin, ServerIdentity};
    use crate::limits::{ClientLimits, ServerLimits};
    use crate::queue::{close_quinn, reason_from};
    use crate::tls;

    const CERT: &[u8] = include_bytes!("../tests/fixtures/server_a.cert.der");
    const KEY: &[u8] = include_bytes!("../tests/fixtures/server_a.key.pk8.der");
    const SURNAME: &str = crate::handshake::TLS_SERVER_NAME;

    #[test]
    fn close_reserve_split_is_exact() {
        for (budget, reserve) in [
            (2_000, 200),
            (800, 200),
            (799, 199),
            (100, 25),
            (4, 1),
            (3, 0),
            (0, 0),
            (u32::MAX, 200),
        ] {
            assert_eq!(
                close_reserve_ms(budget),
                reserve,
                "reserve of budget {budget}"
            );
        }
        for budget in [
            0, 1, 2, 3, 4, 5, 7, 8, 99, 100, 101, 799, 800, 801, 1_999, 2_000,
        ] {
            let reserve = close_reserve_ms(budget);
            let before = Instant::now();
            let d = close_deadlines(budget);
            let after = Instant::now();
            assert_eq!(
                d.overall.duration_since(d.flush_end),
                Duration::from_millis(u64::from(reserve)),
                "split of budget {budget}"
            );
            assert!(d.flush_end <= d.overall, "order of budget {budget}");
            assert!(
                d.overall >= before + Duration::from_millis(u64::from(budget)),
                "overall starts at now for budget {budget}"
            );
            assert!(
                d.overall <= after + Duration::from_millis(u64::from(budget)),
                "overall ends at now for budget {budget}"
            );
        }
    }

    #[test]
    fn goodbye_sent_after_the_endpoint_deadline_passed() {
        let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind server socket");
        let addr = socket.local_addr().expect("server address");
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel::<()>();
        let server = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_io()
                .enable_time()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let identity = ServerIdentity::new(CERT.to_vec(), KEY.to_vec());
                let config = tls::server_config(&identity, &[ALPN_LANE0_V1], &ServerLimits::v1())
                    .expect("server config");
                let endpoint = quinn::Endpoint::new(
                    EndpointConfig::default(),
                    Some(config),
                    socket,
                    Arc::new(TokioRuntime),
                )
                .expect("server endpoint");
                let incoming = endpoint.accept().await.expect("incoming");
                let connection = incoming.await.expect("server connection");
                let _ = ready_tx.send(());
                tokio::time::timeout(Duration::from_secs(30), connection.closed())
                    .await
                    .ok()
                    .map(|error| reason_from(&error))
            })
        });
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .expect("client runtime");
        runtime.block_on(async move {
            let socket = std::net::UdpSocket::bind("127.0.0.1:0").expect("bind client socket");
            let endpoint = quinn::Endpoint::new(
                EndpointConfig::default(),
                None,
                socket,
                Arc::new(TokioRuntime),
            )
            .expect("client endpoint");
            let config = tls::client_config(
                CertificatePin::of_certificate(CERT),
                Arc::new(AtomicBool::new(false)),
                ALPN_LANE0_V1,
                &ClientLimits::v1(),
            )
            .expect("client config");
            let connection = endpoint
                .connect_with(config, addr, SURNAME)
                .expect("connect")
                .await
                .expect("client connection");
            ready_rx.await.expect("server ready");
            let passed = tokio::time::Instant::now() + Duration::from_millis(20);
            tokio::time::sleep_until(passed).await;
            close_quinn(&connection, CloseCode::Normal);
            finish_endpoint_close(&endpoint, CloseCode::Normal, passed.into_std()).await;
        });
        drop(runtime);
        let closed = server.join().expect("server thread");
        assert_eq!(closed, Some(CloseReason::Peer { code: 0 }));
    }
}
