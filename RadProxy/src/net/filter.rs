//! Anti-bot connection rate limiter and per-IP concurrency guard.

use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SHARD_COUNT: usize = 64;

#[derive(Debug)]
struct IpEntry {
    active: usize,
    window_start: Instant,
    window_count: u32,
}

/// Thread-safe per-IP connection filter and rate limiter.
pub struct IpFilter {
    max_connections_per_ip: usize,
    rate_limit_per_sec: u32,
    shards: Vec<Mutex<HashMap<IpAddr, IpEntry>>>,
}

impl IpFilter {
    /// Creates a new [`IpFilter`] with the given per-IP limits (`0` disables a limit).
    #[must_use]
    pub fn new(max_connections_per_ip: usize, rate_limit_per_sec: u32) -> Self {
        let shards = (0..SHARD_COUNT)
            .map(|_| Mutex::new(HashMap::new()))
            .collect();
        Self {
            max_connections_per_ip,
            rate_limit_per_sec,
            shards,
        }
    }

    #[inline]
    fn shard_for(&self, ip: IpAddr) -> &Mutex<HashMap<IpAddr, IpEntry>> {
        let mut hasher = DefaultHasher::new();
        ip.hash(&mut hasher);
        &self.shards[(hasher.finish() as usize) % SHARD_COUNT]
    }

    /// Attempts to admit a new connection from `ip`.
    ///
    /// Returns `Some(IpConnectionGuard)` on success (which automatically releases the
    /// active connection slot on drop), or `None` if rate-limited or over the concurrency cap.
    pub fn try_acquire(self: &Arc<Self>, ip: IpAddr) -> Option<IpConnectionGuard> {
        if self.max_connections_per_ip == 0 && self.rate_limit_per_sec == 0 {
            return Some(IpConnectionGuard { filter: None, ip });
        }

        let now = Instant::now();
        let mut map = self.shard_for(ip).lock().expect("ip filter mutex poisoned");

        // Periodically prune stale idle entries to prevent unbounded memory growth under flood.
        if map.len() > 256 {
            map.retain(|_, v| {
                v.active > 0 || now.duration_since(v.window_start) < Duration::from_secs(5)
            });
        }

        let entry = map.entry(ip).or_insert(IpEntry {
            active: 0,
            window_start: now,
            window_count: 0,
        });

        if now.duration_since(entry.window_start) >= Duration::from_secs(1) {
            entry.window_start = now;
            entry.window_count = 0;
        }

        if self.rate_limit_per_sec > 0 && entry.window_count >= self.rate_limit_per_sec {
            return None;
        }

        if self.max_connections_per_ip > 0 && entry.active >= self.max_connections_per_ip {
            return None;
        }

        entry.window_count += 1;
        entry.active += 1;

        Some(IpConnectionGuard {
            filter: Some(Arc::clone(self)),
            ip,
        })
    }

    fn release(&self, ip: IpAddr) {
        let mut map = self.shard_for(ip).lock().expect("ip filter mutex poisoned");
        if let Some(entry) = map.get_mut(&ip) {
            entry.active = entry.active.saturating_sub(1);
            if entry.active == 0
                && Instant::now().duration_since(entry.window_start) >= Duration::from_secs(5)
            {
                map.remove(&ip);
            }
        }
    }
}

/// RAII guard that releases an IP's active connection slot when dropped.
pub struct IpConnectionGuard {
    filter: Option<Arc<IpFilter>>,
    ip: IpAddr,
}

impl Drop for IpConnectionGuard {
    fn drop(&mut self) {
        if let Some(filter) = &self.filter {
            filter.release(self.ip);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_and_concurrency_enforced() {
        let filter = Arc::new(IpFilter::new(2, 3));
        let ip: IpAddr = "127.0.0.1".parse().unwrap();

        let g1 = filter.try_acquire(ip).unwrap();
        let g2 = filter.try_acquire(ip).unwrap();
        // Exceeds max_connections_per_ip = 2.
        assert!(filter.try_acquire(ip).is_none());

        drop(g1);
        drop(g2);

        // 3rd attempt in same second succeeds, 4th hits rate_limit_per_sec = 3.
        let _g3 = filter.try_acquire(ip).unwrap();
        assert!(filter.try_acquire(ip).is_none());
    }
}
