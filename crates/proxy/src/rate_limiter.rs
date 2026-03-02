//! Token-bucket rate limiter per client IP.
//!
//! Equivalent Python: proxy_base._check_rate_limit()
//!
//! Each IP has a bucket of tokens that refills at RATE_RPS tokens/second,
//! capped at BURST_MAX. Each request costs 1 token; requests are rejected
//! (→ 429) when the bucket is empty.
//!
//! Stale buckets are purged every PURGE_INTERVAL_SECS to prevent memory leaks.

use parking_lot::Mutex;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

const RATE_RPS: f64 = 100.0; // tokens added per second per IP
const BURST_MAX: f64 = 200.0; // max burst size
const PURGE_INTERVAL: Duration = Duration::from_secs(30);
const STALE_AFTER: Duration = Duration::from_secs(60);
const MAX_IPS: usize = 10_000;

struct Bucket {
    tokens: f64,
    last: Instant,
}

struct Inner {
    buckets: HashMap<String, Bucket>,
    last_purge: Instant,
}

impl Inner {
    /// Conditionally purge stale buckets given a caller-supplied `now`.
    ///
    /// Returns `true` if a purge actually happened (useful for testing).
    fn purge_if_needed(&mut self, now: Instant) -> bool {
        if now.duration_since(self.last_purge) >= PURGE_INTERVAL
            && self.buckets.len() >= MAX_IPS / 2
        {
            self.buckets.retain(|_, b| now.duration_since(b.last) < STALE_AFTER);
            self.last_purge = now;
            true
        } else {
            false
        }
    }
}

pub struct RateLimiter {
    inner: Mutex<Inner>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(Inner {
                buckets: HashMap::new(),
                last_purge: Instant::now(),
            }),
        }
    }

    /// Returns `true` if the request from `ip` is allowed; `false` → 429.
    pub fn check(&self, ip: &str) -> bool {
        let mut g = self.inner.lock();
        let now = Instant::now();

        // Periodic purge of stale entries (time-based, not count-based → O(1) amortized)
        g.purge_if_needed(now);

        let bucket = g.buckets.entry(ip.to_owned()).or_insert_with(|| Bucket {
            tokens: BURST_MAX - 1.0, // first request always passes
            last: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * RATE_RPS).min(BURST_MAX);
        bucket.last = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -------------------------------------------------------------------------
    // Helper: build an Inner with a fake last_purge set far in the past and
    // buckets whose `last` timestamp is also set far in the past.
    //
    // We use `Instant::now() - duration` via checked_sub to back-date
    // timestamps so we can exercise the purge logic without sleeping.
    // -------------------------------------------------------------------------

    fn instant_ago(dur: Duration) -> Instant {
        Instant::now().checked_sub(dur).expect("checked_sub failed")
    }

    fn make_inner_with_old_purge(purge_age: Duration) -> Inner {
        Inner {
            buckets: HashMap::new(),
            last_purge: instant_ago(purge_age),
        }
    }

    fn insert_stale_bucket(inner: &mut Inner, ip: &str, age: Duration) {
        inner.buckets.insert(
            ip.to_string(),
            Bucket {
                tokens: BURST_MAX,
                last: instant_ago(age),
            },
        );
    }

    fn insert_fresh_bucket(inner: &mut Inner, ip: &str) {
        inner.buckets.insert(
            ip.to_string(),
            Bucket {
                tokens: BURST_MAX,
                last: Instant::now(),
            },
        );
    }

    // -------------------------------------------------------------------------
    // test_purge_removes_stale_buckets
    //
    // Condition: last_purge is old (>= PURGE_INTERVAL), bucket count >=
    // MAX_IPS/2, and some buckets have last > STALE_AFTER ago.
    // Expected: stale buckets are removed; fresh ones survive.
    // -------------------------------------------------------------------------
    #[test]
    fn test_purge_removes_stale_buckets() {
        let mut inner = make_inner_with_old_purge(PURGE_INTERVAL + Duration::from_secs(1));

        // Fill to just above the half-capacity threshold
        let threshold = MAX_IPS / 2;
        for i in 0..threshold {
            // All stale (last seen 2 × STALE_AFTER ago)
            insert_stale_bucket(&mut inner, &format!("10.0.{}.{}", i / 256, i % 256), STALE_AFTER + Duration::from_secs(1));
        }
        // Add a few fresh buckets that must survive
        insert_fresh_bucket(&mut inner, "192.168.1.1");
        insert_fresh_bucket(&mut inner, "192.168.1.2");

        let total_before = inner.buckets.len();
        assert!(total_before >= threshold, "pre-condition: need >= MAX_IPS/2 buckets");

        let now = Instant::now();
        let purged = inner.purge_if_needed(now);

        assert!(purged, "purge should have run");
        // All stale entries must be gone; the two fresh ones remain
        assert_eq!(inner.buckets.len(), 2, "only fresh buckets should survive");
        assert!(inner.buckets.contains_key("192.168.1.1"));
        assert!(inner.buckets.contains_key("192.168.1.2"));
    }

    // -------------------------------------------------------------------------
    // test_purge_condition_requires_half_capacity
    //
    // If bucket count < MAX_IPS / 2, purge must NOT run even when the interval
    // has elapsed.
    // -------------------------------------------------------------------------
    #[test]
    fn test_purge_condition_requires_half_capacity() {
        let mut inner = make_inner_with_old_purge(PURGE_INTERVAL + Duration::from_secs(1));

        // Insert fewer than MAX_IPS / 2 stale buckets
        let below_threshold = MAX_IPS / 2 - 1;
        for i in 0..below_threshold {
            insert_stale_bucket(&mut inner, &format!("10.0.{}.{}", i / 256, i % 256), STALE_AFTER + Duration::from_secs(1));
        }

        let count_before = inner.buckets.len();
        let now = Instant::now();
        let purged = inner.purge_if_needed(now);

        assert!(!purged, "purge must NOT run when count < MAX_IPS/2");
        assert_eq!(inner.buckets.len(), count_before, "bucket count must be unchanged");
    }

    // -------------------------------------------------------------------------
    // test_no_purge_before_interval
    //
    // If last_purge is recent (< PURGE_INTERVAL), purge must NOT run even when
    // bucket count exceeds MAX_IPS / 2.
    // -------------------------------------------------------------------------
    #[test]
    fn test_no_purge_before_interval() {
        // last_purge just 1 second ago — well within the 30s interval
        let mut inner = make_inner_with_old_purge(Duration::from_secs(1));

        // Fill above half-capacity with stale buckets
        let threshold = MAX_IPS / 2;
        for i in 0..threshold {
            insert_stale_bucket(&mut inner, &format!("10.0.{}.{}", i / 256, i % 256), STALE_AFTER + Duration::from_secs(1));
        }

        let count_before = inner.buckets.len();
        let now = Instant::now();
        let purged = inner.purge_if_needed(now);

        assert!(!purged, "purge must NOT run before PURGE_INTERVAL has elapsed");
        assert_eq!(inner.buckets.len(), count_before, "bucket count must be unchanged");
    }

    // -------------------------------------------------------------------------
    // Additional: basic allow/deny behaviour (smoke test for check())
    // -------------------------------------------------------------------------
    #[test]
    fn test_first_request_always_allowed() {
        let rl = RateLimiter::new();
        assert!(rl.check("1.2.3.4"), "first request must be allowed");
    }

    #[test]
    fn test_burst_exhaustion_triggers_429() {
        let rl = RateLimiter::new();
        let ip = "5.6.7.8";
        // Drain the burst (BURST_MAX tokens; first request consumes 1 and inserts
        // the bucket with BURST_MAX - 1 remaining)
        let burst = BURST_MAX as usize;
        for _ in 0..burst {
            rl.check(ip);
        }
        // Next request must be rejected
        assert!(!rl.check(ip), "request after burst exhaustion must be denied");
    }

    #[test]
    fn test_different_ips_are_independent() {
        let rl = RateLimiter::new();
        assert!(rl.check("1.1.1.1"));
        assert!(rl.check("2.2.2.2"));
    }
}
