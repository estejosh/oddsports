/** Per-user rate limiting. PRD 5.3 #3 — protects API costs and abuse. */

const WINDOW_MS = 60_000;
const MAX_PER_WINDOW = 20;

const buckets = new Map<number, number[]>();

export function allow(userId: number): boolean {
  const now = Date.now();
  const hits = (buckets.get(userId) ?? []).filter((t) => now - t < WINDOW_MS);
  if (hits.length >= MAX_PER_WINDOW) {
    buckets.set(userId, hits);
    return false;
  }
  hits.push(now);
  buckets.set(userId, hits);
  return true;
}

// Periodic cleanup so the map doesn't grow unbounded.
setInterval(() => {
  const now = Date.now();
  for (const [id, hits] of buckets) {
    const live = hits.filter((t) => now - t < WINDOW_MS);
    if (live.length === 0) buckets.delete(id);
    else buckets.set(id, live);
  }
}, WINDOW_MS).unref();
