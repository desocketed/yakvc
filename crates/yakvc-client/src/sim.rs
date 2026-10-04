//! Deterministic network impairment for tests and load testing.
//!
//! Applied to voice datagrams after receipt and before the jitter buffer, so
//! it needs no hook into Iroh's transport.

use std::time::Duration;

use rand::rngs::StdRng;
use rand::{RngExt, SeedableRng};

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

/// Applies an [`Impairment`] to one peer's datagrams, in arrival order.
/// The same seed and arrival order always give the same decisions.
#[derive(Debug)]
pub(crate) struct Impairer {
    impairment: Impairment,
    rng: StdRng,
    /// In the "bad" state of the bursty loss model, where everything is lost.
    in_burst: bool,
}

impl Impairer {
    pub(crate) fn new(impairment: Impairment) -> Self {
        Impairer {
            rng: StdRng::seed_from_u64(impairment.seed),
            impairment,
            in_burst: false,
        }
    }

    /// When to deliver the next datagram, as delays from its arrival: none if
    /// it is lost, two if it is duplicated.
    pub(crate) fn delays(&mut self) -> Vec<Duration> {
        if self.lose() {
            return Vec::new();
        }
        let mut delays = vec![self.delay()];
        if self.rng.random::<f32>() < self.impairment.duplicate {
            delays.push(self.delay());
        }
        delays
    }

    fn lose(&mut self) -> bool {
        match self.impairment.loss {
            Loss::None => false,
            Loss::Uniform(p) => self.rng.random::<f32>() < p,
            Loss::Bursty { rate, mean_burst } => {
                // Leaving a burst with probability 1/mean_burst makes bursts
                // mean_burst long on average; entering with this probability
                // makes the long-run share of lost datagrams equal `rate`.
                let leave = 1.0 / mean_burst.max(1.0);
                let rate = rate.clamp(0.0, 0.99);
                let enter = rate * leave / (1.0 - rate);
                let roll = self.rng.random::<f32>();
                self.in_burst = if self.in_burst {
                    roll >= leave
                } else {
                    roll < enter
                };
                self.in_burst
            }
        }
    }

    fn delay(&mut self) -> Duration {
        let jitter = self.impairment.jitter.mul_f64(self.rng.random::<f64>());
        self.impairment.delay + jitter
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(impairment: &Impairment, datagrams: usize) -> Vec<Vec<Duration>> {
        let mut impairer = Impairer::new(impairment.clone());
        (0..datagrams).map(|_| impairer.delays()).collect()
    }

    #[test]
    fn same_seed_gives_same_decisions() {
        let impairment = Impairment {
            loss: Loss::Bursty {
                rate: 0.05,
                mean_burst: 3.0,
            },
            delay: Duration::from_millis(10),
            jitter: Duration::from_millis(30),
            duplicate: 0.01,
            seed: 7,
        };
        assert_eq!(run(&impairment, 5_000), run(&impairment, 5_000));

        let other_seed = Impairment {
            seed: 8,
            ..impairment.clone()
        };
        assert_ne!(run(&impairment, 5_000), run(&other_seed, 5_000));
    }

    #[test]
    fn no_impairment_delivers_everything_at_once() {
        let delays = run(&Impairment::default(), 100);
        assert!(delays.iter().all(|d| d == &[Duration::ZERO]));
    }

    #[test]
    fn uniform_loss_rate_is_close_to_configured() {
        let impairment = Impairment {
            loss: Loss::Uniform(0.2),
            ..Impairment::default()
        };
        let lost = run(&impairment, 20_000)
            .iter()
            .filter(|d| d.is_empty())
            .count();
        assert!((3_600..4_400).contains(&lost), "lost {lost}");
    }

    #[test]
    fn bursty_loss_matches_rate_and_burst_length() {
        let impairment = Impairment {
            loss: Loss::Bursty {
                rate: 0.05,
                mean_burst: 4.0,
            },
            seed: 1,
            ..Impairment::default()
        };
        let lost: Vec<bool> = run(&impairment, 100_000)
            .iter()
            .map(|d| d.is_empty())
            .collect();
        let total = lost.iter().filter(|&&l| l).count();
        let bursts = lost.windows(2).filter(|w| !w[0] && w[1]).count();
        let mean_burst = total as f64 / bursts as f64;
        assert!((4_000..6_000).contains(&total), "lost {total}");
        assert!((3.0..5.0).contains(&mean_burst), "mean burst {mean_burst}");
    }

    #[test]
    fn delay_and_jitter_stay_in_bounds_and_duplicates_appear() {
        let impairment = Impairment {
            delay: Duration::from_millis(20),
            jitter: Duration::from_millis(50),
            duplicate: 0.5,
            ..Impairment::default()
        };
        let delays = run(&impairment, 1_000);
        let all: Vec<Duration> = delays.iter().flatten().copied().collect();
        assert!(
            all.iter()
                .all(|d| (Duration::from_millis(20)..=Duration::from_millis(70)).contains(d))
        );
        let duplicated = delays.iter().filter(|d| d.len() == 2).count();
        assert!((400..600).contains(&duplicated), "duplicated {duplicated}");
    }
}
