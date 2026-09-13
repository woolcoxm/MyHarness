---
name: system-design
description: Apply distributed-systems design fundamentals — architecture tradeoffs, messaging, caching, replication, scaling, and resilience — whenever designing a new service, evaluating an architecture, or reasoning about a system under load or failure.
---

# System Design: Architecture and Tradeoffs

Design is choosing tradeoffs explicitly. Every section here gives a decision rule, not a
default. Write down which tradeoff you are making and why; if you cannot name the cost
of a choice, you have not made it yet.

## Monolith vs Microservices

Use a monolith until a forcing function appears. Microservices trade development
velocity for organizational scaling — they are a team-structure tool first, a technical
tool second.

| Factor | Monolith | Microservices |
|---|---|---|
| Team size | < ~30 engineers, one codebase works | Multiple teams owning separate domains |
| Deploy frequency | One deploy train; atomic releases | Independent deploys per service |
| Domain boundaries | Unclear or still evolving | Stable, well-understood seams |
| Data consistency | ACID transactions across modules | Distributed transactions / sagas |
| Ops cost | One process, one log | Needs service discovery, tracing, orchestration |
| Failure blast radius | Whole app deploys together | Isolated per service (if boundaries hold) |

Start with a modular monolith: enforce module boundaries in-process (separate packages,
no cross-module DB writes), then extract a service only when a module needs independent
scaling, independent deploys, or a different runtime.

## Event-Driven Architecture

Classify every message before choosing infrastructure:

- **Command** — "do this"; exactly one consumer; the sender expects an effect. `CancelOrder`.
- **Event** — "this happened"; zero or many consumers; the sender does not care who listens. `OrderCancelled`.
- **Query** — "tell me"; expects a response. Never put queries on a bus that has no reply path.

| Delivery | Use when | Cost |
|---|---|---|
| At-least-once | Default. Money, state changes, anything that must not be lost | Consumers must be idempotent |
| At-most-once | Losing a message is cheaper than duplicating it (metrics, telemetry) | Silent data loss on crash |

Prefer at-least-once plus idempotent consumers over at-most-once. Handle poison
messages with a retry limit plus a dead-letter queue — never retry forever.

## CQRS

Split read and write models only when their shapes genuinely diverge.

- **Split when**: reads vastly outnumber writes (100:1+), the read projection needs
  denormalized/joined shapes, or different storage fits each side (Postgres writes,
  Elasticsearch reads).
- **Do not split when**: CRUD covers it. A CQRS layer over identical models is pure overhead.

Eventual consistency is the price: after a write, reads may serve stale data for the
replication lag (ms to seconds). Do not paper over it — bound it, measure the lag, and
decide per query whether stale reads are acceptable. Never CQRS the part of the domain
that needs read-your-own-write consistency (e.g., payment confirmation) unless you add
a write-through read or version fencing.

## CAP Theorem in Practice

During a network partition a distributed system must choose C or A — you cannot have both.
Choose per subsystem, not per system:

- **Choose CP** (reject requests during partition): inventory reservation, account
  balances, anything where a wrong answer is worse than no answer.
- **Choose AP** (serve possibly-stale data): carts, recommendations, social feeds.

CP does not mean "always consistent" — it means consistent once the partition heals.
AP does not mean "wrong data" — it means available with the last known good state.
Design the merge/heal path explicitly; that is where AP systems break.

## Message Queue Selection

| Need | Kafka | RabbitMQ | SQS |
|---|---|---|---|
| Throughput | 100k+/s, partition-ordered | ~10k/s, flexible routing | ~3k/s per queue, easy |
| Ordering | Per-partition, strict | Best-effort | FIFO queues only, lower limit |
| Replay | Yes — offset rewind, retention days/forever | No — message consumed is gone | Limited — visibility timeout, redrive |
| Model | Log (consumers track position) | Queue (broker tracks delivery) | Queue (managed) |
| Ops | Heavy (ZooKeeper/KRaft, tuning) | Moderate | None |

Choose Kafka when multiple consumers must independently replay the same history
(event sourcing, analytics, audit). Choose RabbitMQ for complex routing of commands to
specific consumers. Choose SQS when it is a plain work queue and you do not want ops.

## Caching Layers

Layers, in order of request flow: browser -> CDN -> app cache (in-process or Redis) -> database.
Cache at the layer closest to the caller that can hold the data with acceptable staleness.

| Strategy | Meaning | Use for |
|---|---|---|
| Cache-aside | Read DB on miss, fill cache; invalidate on write | Default for read-heavy entities |
| Write-through | Write cache + store together | When misses are unacceptable |
| Write-behind | Write cache, async flush to store | Write-heavy; risks loss on crash |
| TTL-only | Let entries expire, never invalidate | Tolerant data (feeds, rankings) |

Invalidate aggressively on write (`DEL key`) and always pair with a TTL as a safety
net. Prefer versioned keys (`user:42:v7`) over fine-grained invalidation when a
dependency graph gets hairy. Never cache the result of an unauthenticated request.

## Load Balancing

- **Round-robin** — even, stateless backends. Cheapest; ignores load.
- **Least-connections** — heterogeneous backends or long-lived requests (websockets).
- **Consistent hashing** — session/state affinity without sticky infrastructure; a node
  leaving remaps only ~1/N of keys. Use for cache-key routing and sharded lookups.
- **Random two-choice (power of two)** — near-least-connections at round-robin cost.

Health checks must test real capacity (a TCP accept is not "healthy"); eject
consistently failing nodes and re-admit them gradually.

## Database Replication

- **Read replicas** scale reads, not writes. Route analytics and stale-tolerant reads
  to replicas; keep read-your-own-write flows on the primary.
- **Lag**: monitor `replay_lag`; if a user must read their own write, pin that user to
  the primary for a short window after their write, or wait for the LSN.
- **Primary election**: automate failover (Patroni, RDS multi-AZ) and rehearse it.
  Split-brain is the failure mode — prefer a system that refuses writes over one that
  accepts two primaries.

## Scaling Decision Framework

Exhaust the cheap axis before paying for the expensive one:

1. **Measure** — find the actual bottleneck (CPU, IO, lock contention, connection pool).
2. **Tune** — indexes, connection pooling, N+1 queries. Often 10x, nearly free.
3. **Scale vertically** — bigger box. Linear cost, zero code change, buys time.
4. **Scale horizontally** — only after the app is stateless (sessions and files moved
   to Redis/S3) and the database is the remaining shared bottleneck (then: replicas,
   sharding, or CQRS reads).

## API Gateway vs Direct Service Calls

Use a gateway when clients are external or numerous and cross-cutting concerns
(auth, rate limiting, TLS, request shaping) would otherwise be duplicated per service.
Use direct service-to-service calls (with mTLS or internal auth) inside the trust
boundary — a gateway hop on the internal path adds latency and a failure point for
every request. Do not put business logic in the gateway; it becomes the new monolith.

## Resilience: Circuit Breakers and Bulkheads

- **Circuit breaker**: stop calling a failing dependency after an error-rate/latency
  threshold trips it; probe occasionally (half-open) before restoring. Prevents
  cascading failure and lets the downstream recover.
- **Bulkhead**: isolate resource pools per dependency (separate connection pools,
  thread pools, semaphores) so one slow service cannot starve the others.
- **Timeouts everywhere**: no call without a deadline, shorter than the caller's own.

```text
state: closed -> (error rate > 50% in window) -> open
open  -> (after cool-down) -> half-open -> one trial call
half-open -> success -> closed ; failure -> open
```

## Idempotency Keys for Safe Retries

Make every mutating endpoint accept a client-supplied `Idempotency-Key` header.
Store `(key, request_hash, response)` for 24h+; on retry with the same key and same
hash, return the stored response; on same key with a different body, return `409`.

```sql
CREATE TABLE idempotency (
  key TEXT PRIMARY KEY,
  request_hash TEXT NOT NULL,
  response_status INT,
  response_body TEXT,
  created_at TIMESTAMPTZ DEFAULT now()
);
```

This is the mechanism that makes at-least-once delivery and client retries safe. Pair
it with idempotent consumers on every queue listener.

## Common Pitfalls

- **Distributed monolith** — microservices that call each other synchronously for every
  request. Worse than a monolith: same coupling, plus network failure modes.
- **Sharing a database between services** — the strongest possible coupling; extract
  the schema or go back to a monolith.
- **At-most-once "for simplicity"** — you are choosing silent data loss; a crash in the
  wrong window drops messages with no trace.
- **Caching without TTL safety nets** — one missed invalidation serves stale data
  forever.
- **No dead-letter queue** — poison messages block or spam the queue forever.
- **Idempotency checked after the effect** — store the key before executing, or two
  concurrent retries both run the charge.
- **Assuming read replicas are current** — showing a user anything but their own
  just-written data is the classic lag bug.
