//! Token-bucket rate limits.

use std::collections::HashMap;
use std::hash::Hash;
use std::time::{Duration, Instant};

/// Allows `capacity` events at once, refilling to `capacity` over `period`.
#[derive(Debug, Clone)]
pub(crate) struct TokenBucket {
    capacity: f64,
    per_sec: f64,
    tokens: f64,
    updated: Instant,
}

impl TokenBucket {
    pub fn new(capacity: u32, period: Duration, now: Instant) -> Self {
        let capacity = f64::from(capacity);
        TokenBucket {
            capacity,
            per_sec: capacity / period.as_secs_f64(),
            tokens: capacity,
            updated: now,
        }
    }

    /// Takes one token if there is one.
    pub fn take(&mut self, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    fn is_full(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= self.capacity
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.updated).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.per_sec).min(self.capacity);
        self.updated = now;
    }
}

/// One [`TokenBucket`] per key, such as an EndpointId or source IP.
#[derive(Debug)]
pub(crate) struct RateLimiter<K> {
    capacity: u32,
    period: Duration,
    buckets: HashMap<K, TokenBucket>,
}

impl<K: Hash + Eq> RateLimiter<K> {
    pub fn new(capacity: u32, period: Duration) -> Self {
        RateLimiter {
            capacity,
            period,
            buckets: HashMap::new(),
        }
    }

    pub fn allow(&mut self, key: K, now: Instant) -> bool {
        let (capacity, period) = (self.capacity, self.period);
        self.buckets
            .entry(key)
            .or_insert_with(|| TokenBucket::new(capacity, period, now))
            .take(now)
    }

    /// Forgets keys whose bucket has refilled, since a fresh bucket is the
    /// same. Keeps the map from growing with every key ever seen.
    pub fn prune(&mut self, now: Instant) {
        self.buckets.retain(|_, bucket| !bucket.is_full(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn bucket_allows_a_burst_then_refills() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(5, MINUTE, start);
        for _ in 0..5 {
            assert!(bucket.take(start));
        }
        assert!(!bucket.take(start));
        // One token comes back every 12 s.
        assert!(!bucket.take(start + Duration::from_secs(11)));
        assert!(bucket.take(start + Duration::from_secs(12)));
        assert!(!bucket.take(start + Duration::from_secs(12)));
        // Never more than the capacity, however long it waits.
        let later = start + 10 * MINUTE;
        for _ in 0..5 {
            assert!(bucket.take(later));
        }
        assert!(!bucket.take(later));
    }

    #[test]
    fn limiter_keys_are_independent() {
        let now = Instant::now();
        let mut limiter = RateLimiter::new(1, MINUTE);
        assert!(limiter.allow("a", now));
        assert!(!limiter.allow("a", now));
        assert!(limiter.allow("b", now));
    }

    #[test]
    fn prune_drops_only_refilled_buckets() {
        let now = Instant::now();
        let mut limiter = RateLimiter::new(2, MINUTE);
        limiter.allow("old", now);
        limiter.allow("new", now + MINUTE);
        limiter.prune(now + MINUTE + Duration::from_secs(1));
        assert_eq!(limiter.buckets.len(), 1);
        assert!(limiter.buckets.contains_key("new"));
        // A pruned key starts again with a full bucket.
        assert!(limiter.allow("old", now + MINUTE));
        assert!(limiter.allow("old", now + MINUTE));
        assert!(!limiter.allow("old", now + MINUTE));
    }
}
