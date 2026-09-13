---
name: performance-optimization
description: Use measurement-driven optimization whenever a hot path, slow query, memory blowup, startup delay, or throughput ceiling needs work - profile first, pick the right data structure, cut allocations, batch I/O, design caches and invalidation, and tune queries with EXPLAIN ANALYZE. Triggers on latency budgets, suspected N+1 queries, GC pauses, profiling-tool choice (flamegraph, cProfile, pprof, DevTools), and any investigation that starts with "this is slow".
---

# Performance Optimization

Performance work without measurement is superstition. The method: state a budget, measure with a profiler, find the 20% of code taking 80% of the time, fix it, and prove the fix with the same measurement.

## Profiling Methodology

1. **Set a performance budget first**: "p95 API latency < 200 ms", "startup < 1 s", "frame < 16 ms". Without a target you cannot say "done" - or know whether the work mattered.
2. **Measure with a profiler, not intuition**: developers guess the hotspot wrong most of the time; cold paths get optimized while the real one runs untouched.
3. **Apply 80/20**: roughly 80% of time sits in 20% of code. The profile shows you which 20%.
4. **Record a before/after benchmark with every change** - otherwise the decision gets re-litigated in every review, and regressions slip back in.

## Profiling Tools by Language

```bash
# Rust: where cycles go, plus statistically sound micro-benchmarks
cargo flamegraph --bin app
cargo bench            # criterion: "change: -34.2% +/- 1.1%, p < 0.05"

# Python: function-level, then line-level on the hot function, then allocations
python -m cProfile -s cumulative app.py
kernprof -l -v app.py          # line_profiler: which LINE burns the time
python -m memory_profiler app.py

# JavaScript: Chrome DevTools Performance tab (record, read the flame chart)
node --prof app.js && node --prof-process isolate*.log

# Go: import _ "net/http/pprof", then
go tool pprof http://localhost:6060/debug/pprof/profile?seconds=30
go test -bench .
```

## Algorithmic Complexity

Pick the data structure that makes the common operation cheap - this beats any micro-optimization:

| Need | Structure | Cost |
|---|---|---|
| lookup by key | hash map | O(1) average |
| range / sorted access | sorted array + binary search | O(log n) |
| prefix matching | trie | O(k) in key length |
| ordered mutation while iterating | B-tree / skip list | O(log n) per op |

```python
# O(n^2): for each user, scan all users again
for u in users:
    partner = next(x for x in users if x.id == u.partner_id)   # n * n

# O(n): one pass builds the index, one pass uses it
by_id = {u.id: u for u in users}
for u in users:
    partner = by_id[u.partner_id]
```

Nested iteration over the same collection is the tell. At n = 10,000 that is 100 million comparisons versus 20,000 - no amount of micro-tuning rescues the wrong complexity.

## Memory Optimization

- **Reduce allocations in hot paths**: reuse buffers, hoist allocations out of loops, prefer APIs that write into caller-provided storage.
- **Object pooling** for hot paths holding expensive resources (DB connections, large buffers, parser arenas); a bounded pool doubles as backpressure.
- **GC awareness**: generational collectors bet that most objects die young. Allocation churn - rapid allocate-then-die - forces frequent young-gen collections, and every allocation is hidden CPU work. Allocation pressure *is* CPU time; GC pauses are just where it becomes visible.
- **Memory layout**: a contiguous array of structs walks cache lines sequentially; a linked list of heap nodes pointer-chases and misses cache on every hop. Same Big-O, wildly different wall time - the CPU cache is the memory model that actually matters.

## I/O Optimization

- **Batch**: one query writing 1,000 rows beats 1,000 queries writing one. Per-call overhead - round trip, protocol, fsync - dominates small operations.
- **Connection pooling**: establishing connections is slow (TCP + TLS + auth); reuse them, and cap the pool because the database is the shared resource you are protecting.
- **Async I/O for concurrency**: thousands of waiting sockets on few threads. Async buys *waiting* concurrency, not compute - CPU-bound work in async code still occupies the thread.
- **Stream instead of loading**: process datasets as chunks (keyset pagination, `serde_json::StreamDeserializer`, Node streams). Constant memory beats buffering a 10 GB file to serve one row.

## Caching Strategy

| Layer | Right when | Tradeoff |
|---|---|---|
| In-process (moka, lru, a Map) | single instance | zero network cost; dies with the process |
| Distributed (Redis) | multiple instances | shared truth; adds a network hop and a failure mode |

- **Invalidation is the hard part**. TTL is simple but serves stale data up to the window. Event-driven invalidation is precise but only as reliable as your events. Version-based keys (bump a version to invalidate a family at once) is blunt but certain.
- **Cache stampede prevention**: when a hot key expires, N concurrent requests simultaneously recompute the expensive thing. Fix with a per-key lock (one recomputes, the rest wait) or stale-while-revalidate (serve stale, refresh in the background).

## Database Query Optimization

```sql
EXPLAIN ANALYZE
SELECT * FROM orders WHERE customer_id = 42 ORDER BY created_at DESC LIMIT 10;
-- "Seq Scan + sort" -> missing index; "Index Scan using ..." -> good

CREATE INDEX orders_customer_created ON orders (customer_id, created_at DESC);
```

- Read the plan: Seq Scan over a big table, nested loops with inflated row estimates, and outliers in "actual time" are the smoking guns.
- **Index strategy**: index the columns in WHERE/JOIN/ORDER BY; composite order matters (equality columns first, range column last); every index taxes every write, so do not index speculatively.
- **N+1 elimination**: ORMs lazily load related rows inside loops. Force a join or batch-fetch (`select_related`, `includes`, `joins + group_by` in memory).
- **Pool tuning**: total max connections must respect the database's limit shared across all services; too small queues requests, too big exhausts the database.

## Common Pitfalls (Anti-Patterns)

- **Premature optimization without measurement** - intuition names the wrong hotspot; you pay complexity for nothing.
- **Micro-optimizations that hurt readability** - the next reader pays every day for nanoseconds nobody measured.
- **Caching everything indiscriminately** - every cache is staleness and invalidation debt; cache only what is expensive AND reused.
- **Async everywhere, even for CPU-bound work** - you moved the bottleneck to the thread pool and added colored functions.
- **Rewriting in a faster language before fixing the algorithm** - an O(n^2) loop in Rust is still O(n^2).
- **Skipping the after-benchmark** - no proof of improvement, and no regression guard for the future.

## Definition of Done

- [ ] Budget stated and measured before starting; a profiler (not intuition) identified the hotspot
- [ ] Algorithm and data-structure fixes considered before micro-optimizations
- [ ] Before/after benchmark recorded (criterion, pprof, or equivalent) showing the improvement
- [ ] Memory changes justified by the profile: allocations reduced or pooling added where it counted
- [ ] Caches have an explicit invalidation strategy and stampede protection
- [ ] Slow queries verified with EXPLAIN ANALYZE; N+1s eliminated; connection pools sized deliberately
