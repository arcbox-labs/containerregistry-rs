//! Metrics and observability utilities for registry operations.
//!
//! Provides utilities for tracking request counts, latencies, and errors.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Operation types for metrics tracking.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Operation {
    /// Ping the registry.
    Ping,
    /// Get a manifest.
    GetManifest,
    /// Head a manifest.
    HeadManifest,
    /// Put a manifest.
    PutManifest,
    /// Get a blob.
    GetBlob,
    /// Head a blob.
    HeadBlob,
    /// Put a blob.
    PutBlob,
    /// List tags.
    ListTags,
    /// Get catalog.
    Catalog,
}

impl Operation {
    /// Returns the operation name as a string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Operation::Ping => "ping",
            Operation::GetManifest => "get_manifest",
            Operation::HeadManifest => "head_manifest",
            Operation::PutManifest => "put_manifest",
            Operation::GetBlob => "get_blob",
            Operation::HeadBlob => "head_blob",
            Operation::PutBlob => "put_blob",
            Operation::ListTags => "list_tags",
            Operation::Catalog => "catalog",
        }
    }
}

/// Counters for a single operation type.
#[derive(Debug, Default)]
pub struct OperationMetrics {
    /// Total number of requests.
    pub requests: AtomicU64,
    /// Number of successful requests.
    pub successes: AtomicU64,
    /// Number of failed requests.
    pub failures: AtomicU64,
    /// Number of retries.
    pub retries: AtomicU64,
    /// Total bytes uploaded.
    pub bytes_uploaded: AtomicU64,
    /// Total bytes downloaded.
    pub bytes_downloaded: AtomicU64,
    /// Total latency in microseconds.
    total_latency_us: AtomicU64,
    /// Minimum latency in microseconds.
    min_latency_us: AtomicU64,
    /// Maximum latency in microseconds.
    max_latency_us: AtomicU64,
}

impl OperationMetrics {
    /// Creates new operation metrics.
    pub fn new() -> Self {
        Self {
            min_latency_us: AtomicU64::new(u64::MAX),
            ..Default::default()
        }
    }

    /// Records a successful request.
    pub fn record_success(&self, latency: Duration, bytes: u64) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.successes.fetch_add(1, Ordering::Relaxed);
        self.bytes_downloaded.fetch_add(bytes, Ordering::Relaxed);
        self.record_latency(latency);
    }

    /// Records a failed request.
    pub fn record_failure(&self, latency: Duration) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.failures.fetch_add(1, Ordering::Relaxed);
        self.record_latency(latency);
    }

    /// Records a retry.
    pub fn record_retry(&self) {
        self.retries.fetch_add(1, Ordering::Relaxed);
    }

    /// Records uploaded bytes.
    pub fn record_upload(&self, bytes: u64) {
        self.bytes_uploaded.fetch_add(bytes, Ordering::Relaxed);
    }

    fn record_latency(&self, latency: Duration) {
        let us = latency.as_micros() as u64;
        self.total_latency_us.fetch_add(us, Ordering::Relaxed);
        self.min_latency_us.fetch_min(us, Ordering::Relaxed);
        self.max_latency_us.fetch_max(us, Ordering::Relaxed);
    }

    /// Returns the average latency.
    pub fn avg_latency(&self) -> Duration {
        let total = self.total_latency_us.load(Ordering::Relaxed);
        let count = self.requests.load(Ordering::Relaxed);
        if count == 0 {
            Duration::ZERO
        } else {
            Duration::from_micros(total / count)
        }
    }

    /// Returns the minimum latency.
    pub fn min_latency(&self) -> Option<Duration> {
        let min = self.min_latency_us.load(Ordering::Relaxed);
        if min == u64::MAX {
            None
        } else {
            Some(Duration::from_micros(min))
        }
    }

    /// Returns the maximum latency.
    pub fn max_latency(&self) -> Option<Duration> {
        let max = self.max_latency_us.load(Ordering::Relaxed);
        if max == 0 {
            None
        } else {
            Some(Duration::from_micros(max))
        }
    }

    /// Returns the success rate (0.0 to 1.0).
    pub fn success_rate(&self) -> f64 {
        let total = self.requests.load(Ordering::Relaxed);
        if total == 0 {
            1.0
        } else {
            let successes = self.successes.load(Ordering::Relaxed);
            successes as f64 / total as f64
        }
    }
}

/// Collector for all registry metrics.
#[derive(Debug)]
pub struct MetricsCollector {
    /// Metrics per operation type.
    operations: HashMap<Operation, Arc<OperationMetrics>>,
    /// Total request count across all operations.
    total_requests: AtomicU64,
    /// Start time for rate calculations.
    start_time: Instant,
}

impl Default for MetricsCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsCollector {
    /// Creates a new metrics collector.
    pub fn new() -> Self {
        let mut operations = HashMap::new();
        for op in [
            Operation::Ping,
            Operation::GetManifest,
            Operation::HeadManifest,
            Operation::PutManifest,
            Operation::GetBlob,
            Operation::HeadBlob,
            Operation::PutBlob,
            Operation::ListTags,
            Operation::Catalog,
        ] {
            operations.insert(op, Arc::new(OperationMetrics::new()));
        }

        Self {
            operations,
            total_requests: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }

    /// Gets the metrics for an operation.
    pub fn operation(&self, op: Operation) -> Arc<OperationMetrics> {
        self.operations.get(&op).cloned().unwrap_or_default()
    }

    /// Records a successful operation.
    pub fn record_success(&self, op: Operation, latency: Duration, bytes: u64) {
        if let Some(metrics) = self.operations.get(&op) {
            metrics.record_success(latency, bytes);
        }
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a failed operation.
    pub fn record_failure(&self, op: Operation, latency: Duration) {
        if let Some(metrics) = self.operations.get(&op) {
            metrics.record_failure(latency);
        }
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Records a retry attempt.
    pub fn record_retry(&self, op: Operation) {
        if let Some(metrics) = self.operations.get(&op) {
            metrics.record_retry();
        }
    }

    /// Records a successful upload operation with bytes uploaded.
    pub fn record_upload(&self, op: Operation, latency: Duration, bytes: u64) {
        if let Some(metrics) = self.operations.get(&op) {
            metrics.requests.fetch_add(1, Ordering::Relaxed);
            metrics.successes.fetch_add(1, Ordering::Relaxed);
            metrics.bytes_uploaded.fetch_add(bytes, Ordering::Relaxed);
            metrics.record_latency(latency);
        }
        self.total_requests.fetch_add(1, Ordering::Relaxed);
    }

    /// Returns the total request count.
    pub fn total_requests(&self) -> u64 {
        self.total_requests.load(Ordering::Relaxed)
    }

    /// Returns the request rate (requests per second).
    pub fn request_rate(&self) -> f64 {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        if elapsed == 0.0 {
            0.0
        } else {
            self.total_requests() as f64 / elapsed
        }
    }

    /// Returns a summary of all metrics.
    pub fn summary(&self) -> MetricsSummary {
        let mut ops = Vec::new();
        for (op, metrics) in &self.operations {
            let requests = metrics.requests.load(Ordering::Relaxed);
            if requests > 0 {
                ops.push(OperationSummary {
                    operation: *op,
                    requests,
                    successes: metrics.successes.load(Ordering::Relaxed),
                    failures: metrics.failures.load(Ordering::Relaxed),
                    retries: metrics.retries.load(Ordering::Relaxed),
                    bytes_uploaded: metrics.bytes_uploaded.load(Ordering::Relaxed),
                    bytes_downloaded: metrics.bytes_downloaded.load(Ordering::Relaxed),
                    avg_latency: metrics.avg_latency(),
                    min_latency: metrics.min_latency(),
                    max_latency: metrics.max_latency(),
                    success_rate: metrics.success_rate(),
                });
            }
        }

        MetricsSummary {
            total_requests: self.total_requests(),
            elapsed: self.start_time.elapsed(),
            request_rate: self.request_rate(),
            operations: ops,
        }
    }

    /// Resets all metrics.
    pub fn reset(&mut self) {
        for metrics in self.operations.values() {
            metrics.requests.store(0, Ordering::Relaxed);
            metrics.successes.store(0, Ordering::Relaxed);
            metrics.failures.store(0, Ordering::Relaxed);
            metrics.retries.store(0, Ordering::Relaxed);
            metrics.bytes_uploaded.store(0, Ordering::Relaxed);
            metrics.bytes_downloaded.store(0, Ordering::Relaxed);
            metrics.total_latency_us.store(0, Ordering::Relaxed);
            metrics.min_latency_us.store(u64::MAX, Ordering::Relaxed);
            metrics.max_latency_us.store(0, Ordering::Relaxed);
        }
        self.total_requests.store(0, Ordering::Relaxed);
        self.start_time = Instant::now();
    }
}

/// Summary of metrics for a single operation.
#[derive(Debug, Clone)]
pub struct OperationSummary {
    /// The operation type.
    pub operation: Operation,
    /// Total requests.
    pub requests: u64,
    /// Successful requests.
    pub successes: u64,
    /// Failed requests.
    pub failures: u64,
    /// Retry attempts.
    pub retries: u64,
    /// Bytes uploaded.
    pub bytes_uploaded: u64,
    /// Bytes downloaded.
    pub bytes_downloaded: u64,
    /// Average latency.
    pub avg_latency: Duration,
    /// Minimum latency.
    pub min_latency: Option<Duration>,
    /// Maximum latency.
    pub max_latency: Option<Duration>,
    /// Success rate.
    pub success_rate: f64,
}

/// Summary of all metrics.
#[derive(Debug, Clone)]
pub struct MetricsSummary {
    /// Total requests across all operations.
    pub total_requests: u64,
    /// Time elapsed since metrics collection started.
    pub elapsed: Duration,
    /// Request rate (requests per second).
    pub request_rate: f64,
    /// Per-operation summaries.
    pub operations: Vec<OperationSummary>,
}

impl MetricsSummary {
    /// Formats the summary as a human-readable string.
    pub fn format(&self) -> String {
        let mut s = format!(
            "Registry Metrics\n  Total requests: {}\n  Elapsed: {:?}\n  Request rate: {:.2} req/s\n",
            self.total_requests, self.elapsed, self.request_rate
        );

        for op in &self.operations {
            s.push_str(&format!(
                "\n  {}:\n    requests: {} (success: {}, failures: {})\n",
                op.operation.as_str(),
                op.requests,
                op.successes,
                op.failures
            ));
            if op.retries > 0 {
                s.push_str(&format!("    retries: {}\n", op.retries));
            }
            s.push_str(&format!(
                "    latency: avg {:?}, min {:?}, max {:?}\n",
                op.avg_latency, op.min_latency, op.max_latency
            ));
            s.push_str(&format!("    success rate: {:.1}%\n", op.success_rate * 100.0));
            if op.bytes_downloaded > 0 || op.bytes_uploaded > 0 {
                s.push_str(&format!(
                    "    bytes: {} up, {} down\n",
                    format_bytes(op.bytes_uploaded),
                    format_bytes(op.bytes_downloaded)
                ));
            }
        }

        s
    }
}

/// Formats bytes as human-readable.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// A timer for measuring operation duration.
pub struct OperationTimer {
    start: Instant,
    operation: Operation,
}

impl OperationTimer {
    /// Creates a new timer for the given operation.
    pub fn new(operation: Operation) -> Self {
        Self {
            start: Instant::now(),
            operation,
        }
    }

    /// Returns the operation.
    pub fn operation(&self) -> Operation {
        self.operation
    }

    /// Returns the elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Completes the timer and returns the elapsed time.
    pub fn finish(self) -> Duration {
        self.start.elapsed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operation_metrics() {
        let metrics = OperationMetrics::new();

        metrics.record_success(Duration::from_millis(100), 1000);
        metrics.record_success(Duration::from_millis(200), 2000);
        metrics.record_failure(Duration::from_millis(50));

        assert_eq!(metrics.requests.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.successes.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.failures.load(Ordering::Relaxed), 1);
        assert_eq!(metrics.bytes_downloaded.load(Ordering::Relaxed), 3000);
    }

    #[test]
    fn test_success_rate() {
        let metrics = OperationMetrics::new();
        assert_eq!(metrics.success_rate(), 1.0); // No requests = 100% success

        metrics.record_success(Duration::from_millis(10), 0);
        metrics.record_success(Duration::from_millis(10), 0);
        metrics.record_failure(Duration::from_millis(10));
        metrics.record_failure(Duration::from_millis(10));

        assert!((metrics.success_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_metrics_collector() {
        let collector = MetricsCollector::new();

        collector.record_success(Operation::GetManifest, Duration::from_millis(100), 1000);
        collector.record_failure(Operation::GetBlob, Duration::from_millis(50));
        collector.record_retry(Operation::GetBlob);

        assert_eq!(collector.total_requests(), 2);

        let summary = collector.summary();
        assert_eq!(summary.total_requests, 2);
        assert_eq!(summary.operations.len(), 2);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.00 GB");
    }

    #[test]
    fn test_operation_timer() {
        let timer = OperationTimer::new(Operation::Ping);
        std::thread::sleep(Duration::from_millis(10));
        let elapsed = timer.finish();
        assert!(elapsed >= Duration::from_millis(10));
    }
}
