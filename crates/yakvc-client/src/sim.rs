//! Deterministic network impairment for tests and load testing.
//!
//! Applied to voice datagrams after receipt and before the jitter buffer, so
//! it needs no hook into Iroh's transport.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Impairment {
    pub loss: Loss,
    /// Fixed added delay.
    pub delay: Duration,
    /// Random extra delay, uniform in `0..=jitter`. Reorders packets when it
    /// exceeds the frame interval.
    pub jitter: Duration,
    /// Probability a datagram is delivered twice.
    pub duplicate: f32,
    /// Seed for every random choice, so runs are repeatable.
    pub seed: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Loss {
    #[default]
    None,
    /// Each datagram dropped independently with this probability.
    Uniform(f32),
    /// Gilbert–Elliott bursts: `rate` overall loss, `mean_burst` average
    /// consecutive drops.
    Bursty { rate: f32, mean_burst: f32 },
}
