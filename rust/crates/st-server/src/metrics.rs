//! Request / SQLite timing aggregates (no path or payload content).
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

const RING: usize = 4096;

#[derive(Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub errors: AtomicU64,
    pub sqlite_lock_wait_ns: AtomicU64,
    pub sqlite_ops: AtomicU64,
    pub buckets_accepted: AtomicU64,
    pub batches: AtomicU64,
    started: Mutex<Option<Instant>>,
    latencies_ns: Mutex<LatencyRing>,
}

#[derive(Default)]
struct LatencyRing {
    buf: Vec<u64>,
    i: usize,
}

impl Metrics {
    pub fn record_request(&self, latency_ns: u64, status: u16) {
        self.ensure_started();
        self.requests.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            self.errors.fetch_add(1, Ordering::Relaxed);
        }
        if let Ok(mut ring) = self.latencies_ns.lock() {
            if ring.buf.len() < RING {
                ring.buf.push(latency_ns);
            } else {
                let i = ring.i % RING;
                ring.buf[i] = latency_ns;
                ring.i = ring.i.wrapping_add(1);
            }
        }
    }

    pub fn record_sqlite_wait(&self, wait_ns: u64) {
        self.sqlite_lock_wait_ns
            .fetch_add(wait_ns, Ordering::Relaxed);
        self.sqlite_ops.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_batch(&self, accepted: u32) {
        self.ensure_started();
        self.batches.fetch_add(1, Ordering::Relaxed);
        self.buckets_accepted
            .fetch_add(accepted as u64, Ordering::Relaxed);
    }

    fn ensure_started(&self) {
        if let Ok(mut g) = self.started.lock() {
            if g.is_none() {
                *g = Some(Instant::now());
            }
        }
    }

    pub fn snapshot(&self) -> MetricsSnapshot {
        let mut samples = self
            .latencies_ns
            .lock()
            .map(|g| g.buf.clone())
            .unwrap_or_default();
        samples.sort_unstable();
        let n = samples.len();
        let pct = |p: f64| -> f64 {
            if n == 0 {
                return 0.0;
            }
            let idx = ((p / 100.0) * (n as f64 - 1.0)).round() as usize;
            samples[idx.min(n - 1)] as f64 / 1_000_000.0
        };
        let max_ms = samples.last().copied().unwrap_or(0) as f64 / 1_000_000.0;
        let ops = self.sqlite_ops.load(Ordering::Relaxed).max(1);
        let elapsed_s = self
            .started
            .lock()
            .ok()
            .and_then(|g| g.map(|t| t.elapsed().as_secs_f64()))
            .unwrap_or(0.0)
            .max(0.001);
        let buckets = self.buckets_accepted.load(Ordering::Relaxed);
        MetricsSnapshot {
            requests: self.requests.load(Ordering::Relaxed),
            errors: self.errors.load(Ordering::Relaxed),
            p50_ms: pct(50.0),
            p95_ms: pct(95.0),
            p99_ms: pct(99.0),
            max_ms,
            sqlite_ops: ops,
            sqlite_avg_lock_wait_ms: self.sqlite_lock_wait_ns.load(Ordering::Relaxed) as f64
                / ops as f64
                / 1_000_000.0,
            buckets_accepted: buckets,
            batches: self.batches.load(Ordering::Relaxed),
            req_per_sec: self.requests.load(Ordering::Relaxed) as f64 / elapsed_s,
            buckets_per_sec: buckets as f64 / elapsed_s,
            elapsed_s,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MetricsSnapshot {
    pub requests: u64,
    pub errors: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    pub max_ms: f64,
    pub sqlite_ops: u64,
    pub sqlite_avg_lock_wait_ms: f64,
    pub buckets_accepted: u64,
    pub batches: u64,
    pub req_per_sec: f64,
    pub buckets_per_sec: f64,
    pub elapsed_s: f64,
}

pub struct SqliteTimer<'a> {
    metrics: &'a Metrics,
    start: Instant,
}

impl<'a> SqliteTimer<'a> {
    pub fn start(metrics: &'a Metrics) -> Self {
        Self {
            metrics,
            start: Instant::now(),
        }
    }
}

impl Drop for SqliteTimer<'_> {
    fn drop(&mut self) {
        self.metrics
            .record_sqlite_wait(self.start.elapsed().as_nanos() as u64);
    }
}
