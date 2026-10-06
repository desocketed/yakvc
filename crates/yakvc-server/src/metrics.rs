//! Prometheus metrics, served as plain text over a minimal HTTP endpoint.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use crate::server::Shared;

#[derive(Debug, Default)]
pub(crate) struct Metrics {
    /// Tickets issued after a challenge (or in dev mode).
    pub auth_ok: Counter,
    /// Sessions registered from a cached ticket.
    pub auth_cached: Counter,
    /// Unverified tickets issued to clients that declined the challenge.
    pub auth_unverified: Counter,
    pub auth_failed: Counter,
    /// Auths answered with `RetryAfter`.
    pub auth_retry_after: Counter,
    pub mojang_calls: Counter,
    pub mojang_micros: Counter,
}

#[derive(Debug, Default)]
pub(crate) struct Counter(AtomicU64);

impl Counter {
    pub fn inc(&self) {
        self.add(1);
    }

    pub fn add(&self, n: u64) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }

    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

impl Metrics {
    pub fn observe_mojang(&self, elapsed: Duration) {
        self.mojang_calls.inc();
        self.mojang_micros
            .add(u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX));
    }
}

/// Renders every metric in the Prometheus text format.
pub(crate) fn render(shared: &Shared, relay: Option<&iroh_relay::server::Metrics>) -> String {
    let m = &shared.metrics;
    let stats = shared.sessions.stats();
    let mut out = String::new();
    let mut metric = |name: &str, kind: &str, help: &str, value: u64| {
        let _ = writeln!(
            out,
            "# HELP {name} {help}\n# TYPE {name} {kind}\n{name} {value}"
        );
    };
    metric(
        "yakvc_sessions",
        "gauge",
        "Live rendezvous sessions.",
        stats.sessions as u64,
    );
    metric(
        "yakvc_matches",
        "gauge",
        "Pairs currently matched.",
        stats.matches as u64,
    );
    metric(
        "yakvc_auth_ok_total",
        "counter",
        "Tickets issued.",
        m.auth_ok.get(),
    );
    metric(
        "yakvc_auth_cached_total",
        "counter",
        "Sessions registered from a cached ticket.",
        m.auth_cached.get(),
    );
    metric(
        "yakvc_auth_unverified_total",
        "counter",
        "Unverified tickets issued to clients without a Mojang proof.",
        m.auth_unverified.get(),
    );
    metric(
        "yakvc_auth_failed_total",
        "counter",
        "Failed auths.",
        m.auth_failed.get(),
    );
    metric(
        "yakvc_auth_retry_after_total",
        "counter",
        "Auths deferred because Mojang was rate-limiting or down.",
        m.auth_retry_after.get(),
    );
    metric(
        "yakvc_mojang_calls_total",
        "counter",
        "hasJoined calls.",
        m.mojang_calls.get(),
    );
    let _ = writeln!(
        out,
        "# HELP yakvc_mojang_seconds_total Time spent in hasJoined calls.\n\
         # TYPE yakvc_mojang_seconds_total counter\n\
         yakvc_mojang_seconds_total {}",
        m.mojang_micros.get() as f64 / 1e6
    );
    if let Some(relay) = relay {
        let mut metric = |name: &str, help: &str, value: u64| {
            let _ = writeln!(
                out,
                "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}"
            );
        };
        metric(
            "yakvc_relay_bytes_recv_total",
            "Bytes the relay received from clients.",
            relay.bytes_recv.get(),
        );
        metric(
            "yakvc_relay_bytes_sent_total",
            "Bytes the relay sent to clients.",
            relay.bytes_sent.get(),
        );
    }
    out
}

/// Answers every HTTP request on `listener` with the current metrics.
pub(crate) async fn serve(
    listener: TcpListener,
    shared: Arc<Shared>,
    relay: Option<Arc<iroh_relay::server::Metrics>>,
) {
    while let Ok((mut stream, _)) = listener.accept().await {
        let body = render(&shared, relay.as_deref());
        tokio::spawn(async move {
            // The request itself doesn't matter; read it so the client isn't
            // reset, then answer.
            let mut request = [0; 1024];
            let _ = stream.read(&mut request).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\n\
                 Content-Type: text/plain; version=0.0.4\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        });
    }
}
