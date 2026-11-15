//! Counters used by the proxy and its metrics endpoint.
//!
//! Replacement authored in this chat on 2026-11-15 from the required public
//! field/method contract and Rust's documented atomic operations. No upstream
//! implementation was pasted into this replacement. The previous implementation
//! was visible during review, so this is not a formal clean-room claim or a legal
//! clearance. Its archived copy remains in `NU/RadProxy/src/metrics/mod.rs`.
//!
//! API reference: https://doc.rust-lang.org/std/sync/atomic/

pub mod http;

use std::sync::atomic::{AtomicU64, Ordering};

// Declare each event once so the live counter and exported reading stay aligned.
macro_rules! counter_schema {
    ($($field:ident => $description:literal),+ $(,)?) => {
        /// Independently updated event totals belonging to one proxy instance.
        #[derive(Debug, Default)]
        pub struct ProxyMetrics {
            $(#[doc = $description] pub $field: AtomicU64,)+
        }

        /// Observed totals. Individual reads are atomic; the whole set is not a transaction.
        #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
        pub struct MetricsSnapshot {
            $(#[doc = $description] pub $field: u64,)+
        }

        impl ProxyMetrics {
            /// Reads the totals without using them to synchronize application state.
            #[must_use]
            pub fn snapshot(&self) -> MetricsSnapshot {
                MetricsSnapshot { $($field: self.$field.load(Ordering::Relaxed),)+ }
            }
        }
    };
}

counter_schema! {
    total_connections => "Accepted TCP connections.",
    blocked_by_filter => "Rejected connections attributed to the IP filter.",
    blocked_by_radon => "Rejected connections attributed to extensions.",
    status_pings => "Handled server-list requests.",
    server_switches => "Completed changes of backend server.",
}

impl ProxyMetrics {
    /// Adds one accepted connection to this instance's total.
    pub fn record_connection(&self) {
        let _ = self.total_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Adds one handled server-list request to this instance's total.
    pub fn record_status_ping(&self) {
        let _ = self.status_pings.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_events_immediately_and_keeps_instances_separate() {
        let active = ProxyMetrics::default();
        let idle = ProxyMetrics::default();
        active.record_connection();
        active.record_status_ping();
        assert_eq!(
            active.snapshot(),
            MetricsSnapshot {
                total_connections: 1,
                status_pings: 1,
                ..MetricsSnapshot::default()
            }
        );
        assert_eq!(idle.snapshot(), MetricsSnapshot::default());
    }

    #[test]
    fn exports_counters_updated_by_existing_callers() {
        let counters = ProxyMetrics::default();
        counters.blocked_by_filter.fetch_add(3, Ordering::Relaxed);
        counters.blocked_by_radon.fetch_add(5, Ordering::Relaxed);
        counters.server_switches.fetch_add(7, Ordering::Relaxed);
        assert_eq!(
            counters.snapshot(),
            MetricsSnapshot {
                blocked_by_filter: 3,
                blocked_by_radon: 5,
                server_switches: 7,
                ..MetricsSnapshot::default()
            }
        );
    }

    #[test]
    fn retains_events_from_threads_after_they_exit() {
        let counters = ProxyMetrics::default();
        std::thread::scope(|workers| {
            for _ in 0..6 {
                workers.spawn(|| {
                    for _ in 0..37 {
                        counters.record_connection();
                        counters.record_status_ping();
                    }
                });
            }
        });
        assert_eq!(counters.snapshot().total_connections, 222);
        assert_eq!(counters.snapshot().status_pings, 222);
    }
}
