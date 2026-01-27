//! Performance benchmarking utilities.
//!
//! Provides utilities for measuring and tracking performance
//! of registry operations.

use std::time::{Duration, Instant};

/// Result of a benchmark run.
#[derive(Clone, Debug)]
pub struct BenchmarkResult {
    /// Name of the benchmark.
    pub name: String,
    /// Number of iterations.
    pub iterations: usize,
    /// Total duration.
    pub total_duration: Duration,
    /// Average duration per iteration.
    pub avg_duration: Duration,
    /// Minimum duration.
    pub min_duration: Duration,
    /// Maximum duration.
    pub max_duration: Duration,
    /// Throughput in bytes per second (if applicable).
    pub throughput_bps: Option<f64>,
}

impl BenchmarkResult {
    /// Formats the result as a human-readable string.
    pub fn format(&self) -> String {
        let mut s = format!(
            "{}: {} iterations in {:?}\n  avg: {:?}, min: {:?}, max: {:?}",
            self.name,
            self.iterations,
            self.total_duration,
            self.avg_duration,
            self.min_duration,
            self.max_duration
        );

        if let Some(bps) = self.throughput_bps {
            let mbps = bps / 1_000_000.0;
            s.push_str(&format!("\n  throughput: {:.2} MB/s", mbps));
        }

        s
    }
}

/// A benchmark runner for measuring operation performance.
pub struct Benchmark {
    name: String,
    iterations: usize,
    warmup_iterations: usize,
    bytes_per_iteration: Option<usize>,
}

impl Benchmark {
    /// Creates a new benchmark.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            iterations: 10,
            warmup_iterations: 2,
            bytes_per_iteration: None,
        }
    }

    /// Sets the number of iterations.
    pub fn iterations(mut self, n: usize) -> Self {
        self.iterations = n;
        self
    }

    /// Sets the number of warmup iterations.
    pub fn warmup(mut self, n: usize) -> Self {
        self.warmup_iterations = n;
        self
    }

    /// Sets the bytes processed per iteration (for throughput calculation).
    pub fn bytes_per_iteration(mut self, n: usize) -> Self {
        self.bytes_per_iteration = Some(n);
        self
    }

    /// Runs the benchmark with a synchronous function.
    pub fn run<F>(&self, mut f: F) -> BenchmarkResult
    where
        F: FnMut(),
    {
        // Warmup
        for _ in 0..self.warmup_iterations {
            f();
        }

        // Benchmark
        let mut durations = Vec::with_capacity(self.iterations);
        let start = Instant::now();

        for _ in 0..self.iterations {
            let iter_start = Instant::now();
            f();
            durations.push(iter_start.elapsed());
        }

        let total_duration = start.elapsed();
        self.compute_result(durations, total_duration)
    }

    /// Runs the benchmark with an async function.
    pub async fn run_async<F, Fut>(&self, mut f: F) -> BenchmarkResult
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = ()>,
    {
        // Warmup
        for _ in 0..self.warmup_iterations {
            f().await;
        }

        // Benchmark
        let mut durations = Vec::with_capacity(self.iterations);
        let start = Instant::now();

        for _ in 0..self.iterations {
            let iter_start = Instant::now();
            f().await;
            durations.push(iter_start.elapsed());
        }

        let total_duration = start.elapsed();
        self.compute_result(durations, total_duration)
    }

    fn compute_result(&self, durations: Vec<Duration>, total_duration: Duration) -> BenchmarkResult {
        let min_duration = *durations.iter().min().unwrap_or(&Duration::ZERO);
        let max_duration = *durations.iter().max().unwrap_or(&Duration::ZERO);
        let avg_duration = total_duration / self.iterations as u32;

        let throughput_bps = self.bytes_per_iteration.map(|bytes| {
            let total_bytes = bytes * self.iterations;
            total_bytes as f64 / total_duration.as_secs_f64()
        });

        BenchmarkResult {
            name: self.name.clone(),
            iterations: self.iterations,
            total_duration,
            avg_duration,
            min_duration,
            max_duration,
            throughput_bps,
        }
    }
}

/// Generates test data of specified size.
pub fn generate_test_data(size: usize) -> Vec<u8> {
    // Generate deterministic pseudo-random data
    let mut data = Vec::with_capacity(size);
    let mut state: u64 = 12345;

    for _ in 0..size {
        // Simple LCG for reproducibility
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        data.push((state >> 56) as u8);
    }

    data
}

/// Common layer sizes for benchmarking.
pub mod layer_sizes {
    /// 1 KB layer.
    pub const TINY: usize = 1024;
    /// 1 MB layer.
    pub const SMALL: usize = 1024 * 1024;
    /// 10 MB layer.
    pub const MEDIUM: usize = 10 * 1024 * 1024;
    /// 100 MB layer.
    pub const LARGE: usize = 100 * 1024 * 1024;
    /// 1 GB layer.
    pub const HUGE: usize = 1024 * 1024 * 1024;
}

/// Tracks performance over time for regression detection.
#[derive(Clone, Debug, Default)]
pub struct PerformanceBaseline {
    /// Baseline results keyed by benchmark name.
    results: std::collections::HashMap<String, Duration>,
    /// Tolerance for regression detection (percentage).
    tolerance: f64,
}

impl PerformanceBaseline {
    /// Creates a new baseline tracker.
    pub fn new() -> Self {
        Self {
            results: std::collections::HashMap::new(),
            tolerance: 0.1, // 10% default tolerance
        }
    }

    /// Sets the tolerance for regression detection.
    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    /// Records a baseline result.
    pub fn record(&mut self, name: &str, duration: Duration) {
        self.results.insert(name.to_string(), duration);
    }

    /// Checks if a result regressed from baseline.
    pub fn check_regression(&self, name: &str, duration: Duration) -> Option<RegressionReport> {
        self.results.get(name).and_then(|baseline| {
            let baseline_ns = baseline.as_nanos() as f64;
            let current_ns = duration.as_nanos() as f64;
            let ratio = current_ns / baseline_ns;

            if ratio > 1.0 + self.tolerance {
                Some(RegressionReport {
                    name: name.to_string(),
                    baseline: *baseline,
                    current: duration,
                    regression_percent: (ratio - 1.0) * 100.0,
                })
            } else {
                None
            }
        })
    }

    /// Loads baseline from JSON.
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let map: std::collections::HashMap<String, u64> = serde_json::from_str(json)?;
        let results = map
            .into_iter()
            .map(|(k, v)| (k, Duration::from_nanos(v)))
            .collect();
        Ok(Self {
            results,
            tolerance: 0.1,
        })
    }

    /// Saves baseline to JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let map: std::collections::HashMap<&str, u64> = self
            .results
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_nanos() as u64))
            .collect();
        serde_json::to_string_pretty(&map)
    }
}

/// Report of a performance regression.
#[derive(Clone, Debug)]
pub struct RegressionReport {
    /// Name of the benchmark.
    pub name: String,
    /// Baseline duration.
    pub baseline: Duration,
    /// Current duration.
    pub current: Duration,
    /// Regression percentage.
    pub regression_percent: f64,
}

impl RegressionReport {
    /// Formats the report.
    pub fn format(&self) -> String {
        format!(
            "REGRESSION: {} - baseline {:?}, current {:?} ({:.1}% slower)",
            self.name, self.baseline, self.current, self.regression_percent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_benchmark_run() {
        let result = Benchmark::new("test")
            .iterations(5)
            .warmup(1)
            .run(|| {
                std::thread::sleep(Duration::from_millis(1));
            });

        assert_eq!(result.iterations, 5);
        assert!(result.avg_duration >= Duration::from_millis(1));
    }

    #[test]
    fn test_generate_test_data() {
        let data1 = generate_test_data(100);
        let data2 = generate_test_data(100);

        assert_eq!(data1.len(), 100);
        assert_eq!(data1, data2); // Deterministic
    }

    #[test]
    fn test_performance_baseline() {
        let mut baseline = PerformanceBaseline::new().with_tolerance(0.1);
        baseline.record("test", Duration::from_millis(100));

        // No regression
        assert!(baseline
            .check_regression("test", Duration::from_millis(105))
            .is_none());

        // Regression detected
        let report = baseline
            .check_regression("test", Duration::from_millis(150))
            .unwrap();
        assert!(report.regression_percent > 40.0);
    }

    #[tokio::test]
    async fn test_benchmark_run_async() {
        let result = Benchmark::new("async_test")
            .iterations(3)
            .warmup(1)
            .run_async(|| async {
                tokio::time::sleep(Duration::from_millis(1)).await;
            })
            .await;

        assert_eq!(result.iterations, 3);
    }
}
