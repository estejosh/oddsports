//! Per-user rate limiting. PRD 5.3 #3 — protects API costs and abuse.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(60);
const MAX_PER_WINDOW: usize = 20;

static BUCKETS: Mutex<Option<HashMap<i64, Vec<Instant>>>> = Mutex::new(None);

pub fn allow(user_id: i64) -> bool {
    let mut guard = BUCKETS.lock().unwrap();
    let buckets = guard.get_or_insert_with(HashMap::new);
    let now = Instant::now();

    let hits = buckets.entry(user_id).or_default();
    hits.retain(|t| now.duration_since(*t) < WINDOW);
    if hits.len() >= MAX_PER_WINDOW {
        return false;
    }
    hits.push(now);

    // Opportunistic cleanup so the map doesn't grow unbounded.
    if buckets.len() > 10_000 {
        buckets.retain(|_, v| v.iter().any(|t| now.duration_since(*t) < WINDOW));
    }
    true
}
