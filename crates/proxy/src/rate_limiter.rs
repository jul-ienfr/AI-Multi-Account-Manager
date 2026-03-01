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
        if now.duration_since(g.last_purge) >= PURGE_INTERVAL
            && g.buckets.len() >= MAX_IPS / 2
        {
            g.buckets.retain(|_, b| now.duration_since(b.last) < STALE_AFTER);
            g.last_purge = now;
        }

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
