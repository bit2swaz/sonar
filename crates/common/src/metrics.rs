//! Prometheus metrics for the Sonar ZK coprocessor.
//!
//! Create one [`Metrics`] instance per process via [`Metrics::new`], then
//! optionally start the HTTP scrape endpoint with [`Metrics::start_server`].

use std::sync::atomic::AtomicU64;

use anyhow::Context;
use prometheus_client::{
    encoding::text::encode,
    metrics::{
        counter::Counter,
        family::Family,
        gauge::Gauge,
        histogram::{exponential_buckets, Histogram},
    },
    registry::Registry,
};

// ---------------------------------------------------------------------------
// Metrics struct
// ---------------------------------------------------------------------------

/// All Prometheus metrics for Sonar, each registered in their own [`Registry`].
pub struct Metrics {
    /// Total computation requests submitted on-chain.
    pub requests_submitted: Counter,
    /// Total proofs successfully verified on-chain.
    pub proofs_verified: Counter,
    /// Total proofs that failed, labelled by `reason`.
    pub proofs_failed: Family<Vec<(String, String)>, Counter>,
    /// Total fees earned in lamports (f64 counter so sub-lamport accrual works).
    pub total_fees_earned_lamports: Counter<f64, AtomicU64>,
    /// Current fraction of prover capacity in use (0.0 – 1.0).
    pub prover_utilization: Gauge<f64, AtomicU64>,
    /// End-to-end request latency distribution in seconds.
    pub request_latency_seconds: Histogram,
    /// Compute-units consumed per on-chain verification.
    pub verification_cu_used: Histogram,
    /// Number of active prover processes.
    pub active_provers: Gauge,
    /// The registry that owns all of the above metrics.
    registry: Registry,
}

impl Metrics {
    /// Create a new [`Metrics`] instance with a fresh, independent [`Registry`].
    pub fn new() -> anyhow::Result<Self> {
        let mut registry = Registry::default();

        let requests_submitted = Counter::default();
        registry.register(
            "sonar_requests_submitted",
            "Total computation requests submitted on-chain",
            requests_submitted.clone(),
        );

        let proofs_verified = Counter::default();
        registry.register(
            "sonar_proofs_verified",
            "Total proofs successfully verified on-chain",
            proofs_verified.clone(),
        );

        let proofs_failed: Family<Vec<(String, String)>, Counter> = Family::default();
        registry.register(
            "sonar_proofs_failed",
            "Total proofs that failed, labelled by reason",
            proofs_failed.clone(),
        );

        let total_fees_earned_lamports: Counter<f64, AtomicU64> = Counter::default();
        registry.register(
            "sonar_total_fees_earned_lamports",
            "Cumulative fees earned in lamports",
            total_fees_earned_lamports.clone(),
        );

        let prover_utilization: Gauge<f64, AtomicU64> = Gauge::default();
        registry.register(
            "sonar_prover_utilization",
            "Fraction of prover capacity currently in use (0.0-1.0)",
            prover_utilization.clone(),
        );

        // Latency buckets: 0.1s … 300s
        let request_latency_seconds = Histogram::new(exponential_buckets(0.1, 2.0, 12));
        registry.register(
            "sonar_request_latency_seconds",
            "End-to-end request latency distribution in seconds",
            request_latency_seconds.clone(),
        );

        // CU buckets: 1k … ~4M
        let verification_cu_used = Histogram::new(exponential_buckets(1_000.0, 2.0, 12));
        registry.register(
            "sonar_verification_cu_used",
            "Compute-units consumed per on-chain verification",
            verification_cu_used.clone(),
        );

        let active_provers: Gauge = Gauge::default();
        registry.register(
            "sonar_active_provers",
            "Number of active prover processes",
            active_provers.clone(),
        );

        Ok(Self {
            requests_submitted,
            proofs_verified,
            proofs_failed,
            total_fees_earned_lamports,
            prover_utilization,
            request_latency_seconds,
            verification_cu_used,
            active_provers,
            registry,
        })
    }

    /// Borrow the underlying [`Registry`].
    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    /// Encode all metrics in the OpenMetrics text format.
    pub fn render(&self) -> anyhow::Result<String> {
        render_registry(&self.registry)
    }

    /// Start a minimal HTTP server that serves `GET /metrics` on the given
    /// port.  The caller must keep the returned future alive (e.g. via
    /// `tokio::spawn`).
    pub async fn start_server(registry: Registry, port: u16) -> anyhow::Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind(format!("0.0.0.0:{port}"))
            .await
            .with_context(|| format!("Failed to bind metrics server to port {port}"))?;

        loop {
            let (mut stream, _) = listener.accept().await?;
            let body = render_registry(&registry)?;

            // Read and discard the incoming HTTP request.
            let mut req = [0u8; 1024];
            let _ = stream.read(&mut req).await;

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: \
                 application/openmetrics-text; version=1.0.0; charset=utf-8\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await?;
        }
    }
}

fn render_registry(registry: &Registry) -> anyhow::Result<String> {
    let mut buf = String::new();
    encode(&mut buf, registry).context("failed to encode Prometheus metrics")?;
    Ok(buf)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_metrics_new_registers_all() {
        let m = Metrics::new().unwrap();
        let rendered = m.render().unwrap();
        for name in &[
            "sonar_requests_submitted",
            "sonar_proofs_verified",
            "sonar_proofs_failed",
            "sonar_total_fees_earned_lamports",
            "sonar_prover_utilization",
            "sonar_request_latency_seconds",
            "sonar_verification_cu_used",
            "sonar_active_provers",
        ] {
            assert!(
                rendered.contains(name),
                "Expected metric '{name}' in rendered output:\n{rendered}"
            );
        }
    }

    #[test]
    fn test_counter_increments() {
        let m = Metrics::new().unwrap();
        m.requests_submitted.inc();
        m.requests_submitted.inc();
        m.requests_submitted.inc();
        let rendered = m.render().unwrap();
        // OpenMetrics appends _total to counter names.
        assert!(
            rendered.contains("sonar_requests_submitted_total 3"),
            "Expected counter value 3 in:\n{rendered}"
        );
    }

    #[test]
    fn test_failed_counter_with_label() {
        let m = Metrics::new().unwrap();
        let label = vec![("reason".to_owned(), "InvalidProof".to_owned())];
        m.proofs_failed.get_or_create(&label).inc();
        m.proofs_failed.get_or_create(&label).inc();
        let rendered = m.render().unwrap();
        assert!(
            rendered.contains("InvalidProof"),
            "Expected label 'InvalidProof' in:\n{rendered}"
        );
        assert!(
            rendered.contains("} 2"),
            "Expected counter value 2 in:\n{rendered}"
        );
    }

    #[test]
    fn test_histogram_record() {
        let m = Metrics::new().unwrap();
        m.request_latency_seconds.observe(1.5);
        let rendered = m.render().unwrap();
        assert!(
            rendered.contains("sonar_request_latency_seconds"),
            "Expected histogram in:\n{rendered}"
        );
    }

    #[test]
    fn test_gauge_set() {
        let m = Metrics::new().unwrap();
        m.prover_utilization.set(0.75);
        let rendered = m.render().unwrap();
        assert!(
            rendered.contains("sonar_prover_utilization"),
            "Expected gauge in:\n{rendered}"
        );
        assert!(
            rendered.contains("0.75"),
            "Expected gauge value 0.75 in:\n{rendered}"
        );
    }

    #[test]
    fn test_independent_registries() {
        let m1 = Metrics::new().unwrap();
        let m2 = Metrics::new().unwrap();
        m1.requests_submitted.inc();
        m1.requests_submitted.inc();
        // m2 counter should still be zero — registries are independent.
        assert_eq!(m2.requests_submitted.get(), 0);
    }

    #[tokio::test]
    async fn test_metrics_server_responds() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpStream;

        // Build a minimal registry for the server under test.
        let mut server_registry = Registry::default();
        let c: Counter = Counter::default();
        server_registry.register("sonar_test_metric", "test", c.clone());
        c.inc();

        let port: u16 = 19_090;
        tokio::spawn(Metrics::start_server(server_registry, port));

        // Give the server a moment to bind.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let mut stream = TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .expect("connect to metrics server");
        stream
            .write_all(b"GET /metrics HTTP/1.0\r\n\r\n")
            .await
            .unwrap();

        let mut resp = String::new();
        stream.read_to_string(&mut resp).await.unwrap();

        assert!(resp.contains("200 OK"), "Expected 200 OK, got:\n{resp}");
        assert!(
            resp.contains("sonar_test_metric"),
            "Expected metric name in body:\n{resp}"
        );
    }
}
