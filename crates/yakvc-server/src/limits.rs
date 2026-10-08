//! Token-bucket rate limits and caps on concurrent connections.

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
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

    /// Whole seconds until the next token, at least one.
    pub fn secs_to_next_token(&mut self, now: Instant) -> u32 {
        self.refill(now);
        let secs = ((1.0 - self.tokens) / self.per_sec).ceil();
        // `as` saturates, so even an empty bucket refilling over years fits.
        (secs as u32).max(1)
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

/// The key per-IP limits count `ip` under. An IPv6 host is usually given a
/// whole /64 and can pick any address in it, so IPv6 addresses count per
/// /64; an IPv4-mapped address counts as its IPv4 address.
pub(crate) fn ip_key(ip: IpAddr) -> IpAddr {
    match ip.to_canonical() {
        IpAddr::V6(v6) => {
            let prefix = v6.to_bits() & !(u128::from(u64::MAX));
            IpAddr::V6(Ipv6Addr::from_bits(prefix))
        }
        v4 => v4,
    }
}

/// A cap on how many of something may exist at once.
#[derive(Debug)]
pub(crate) struct Slots {
    max: usize,
    used: Arc<AtomicUsize>,
}

/// One taken slot, given back when dropped.
#[derive(Debug)]
pub(crate) struct Slot(Arc<AtomicUsize>);

impl Slots {
    pub fn new(max: usize) -> Self {
        Slots {
            max,
            used: Arc::default(),
        }
    }

    pub fn take(&self) -> Option<Slot> {
        self.used
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                (used < self.max).then_some(used + 1)
            })
            .ok()?;
        Some(Slot(self.used.clone()))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

/// [`Slots`] for each source IP, counted under [`ip_key`].
#[derive(Debug)]
pub(crate) struct IpSlots {
    max: usize,
    used: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

/// One taken [`IpSlots`] slot, given back when dropped.
#[derive(Debug)]
pub(crate) struct IpSlot {
    key: IpAddr,
    used: Arc<Mutex<HashMap<IpAddr, usize>>>,
}

impl IpSlots {
    pub fn new(max: usize) -> Self {
        IpSlots {
            max,
            used: Arc::default(),
        }
    }

    pub fn take(&self, ip: IpAddr) -> Option<IpSlot> {
        let key = ip_key(ip);
        let mut used = self.used.lock().unwrap();
        let count = used.entry(key).or_default();
        if *count >= self.max {
            return None;
        }
        *count += 1;
        Some(IpSlot {
            key,
            used: self.used.clone(),
        })
    }
}

impl Drop for IpSlot {
    fn drop(&mut self) {
        let mut used = self.used.lock().unwrap();
        let count = used.get_mut(&self.key).expect("held slots are counted");
        *count -= 1;
        // Keeps the map from growing with every IP ever seen.
        if *count == 0 {
            used.remove(&self.key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipv6_addresses_count_per_64() {
        let key = |ip: &str| ip_key(ip.parse().unwrap());
        assert_eq!(key("2001:db8:1:2:aaaa::1"), key("2001:db8:1:2:bbbb::2"));
        assert_eq!(key("2001:db8:1:2:aaaa::1"), key("2001:db8:1:2::"));
        assert_ne!(key("2001:db8:1:2::1"), key("2001:db8:1:3::1"));
        assert_eq!(key("192.0.2.1"), key("::ffff:192.0.2.1"));
        assert_ne!(key("192.0.2.1"), key("192.0.2.2"));
    }

    #[test]
    fn ip_slots_are_per_ip_and_given_back_when_dropped() {
        let slots = IpSlots::new(1);
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        let first = slots.take(ip("2001:db8::1")).unwrap();
        assert!(slots.take(ip("2001:db8::2")).is_none(), "same /64");
        let _other = slots.take(ip("192.0.2.1")).unwrap();
        drop(first);
        assert!(slots.take(ip("2001:db8::2")).is_some());
        assert_eq!(slots.used.lock().unwrap().len(), 1);
    }

    #[test]
    fn slots_are_given_back_when_dropped() {
        let slots = Slots::new(2);
        let first = slots.take().unwrap();
        let _second = slots.take().unwrap();
        assert!(slots.take().is_none());
        drop(first);
        assert!(slots.take().is_some());
    }

    #[test]
    fn time_to_the_next_token() {
        let start = Instant::now();
        let mut bucket = TokenBucket::new(6, MINUTE, start);
        for _ in 0..6 {
            bucket.take(start);
        }
        // One token every 10 s, rounded up to whole seconds.
        assert_eq!(bucket.secs_to_next_token(start), 10);
        assert_eq!(
            bucket.secs_to_next_token(start + Duration::from_millis(500)),
            10
        );
        assert_eq!(bucket.secs_to_next_token(start + Duration::from_secs(9)), 1);
        // With a token waiting it is still at least one second.
        assert_eq!(bucket.secs_to_next_token(start + MINUTE), 1);
    }

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
