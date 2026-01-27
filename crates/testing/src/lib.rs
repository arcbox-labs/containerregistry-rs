//! Test fixtures and parity testing utilities.
//!
//! This crate provides fixtures, golden data helpers, parity test
//! runners, fault injection, and benchmarking utilities for verifying
//! behavior against go-containerregistry.

pub mod bench;
pub mod fault;
pub mod fixture;
pub mod golden;
pub mod parity;

pub use bench::{Benchmark, BenchmarkResult, PerformanceBaseline};
pub use fault::{Fault, FaultConfig, FaultInjectionServer};
pub use fixture::{FixtureBuilder, LayerCompression, TestImage};
pub use golden::{GoldenComparator, assert_digest_eq, assert_manifest_eq};
pub use parity::ParityRunner;
