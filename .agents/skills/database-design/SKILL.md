---
name: database-design
description: Use whenever creating or migrating database schemas: normalization through BCNF with denormalization escape hatches, auto-increment vs UUID vs composite keys, FK actions, composite index column order, expand-contract zero-downtime migrations, pool sizing, isolation-level anomalies, join tables, soft deletes, audit trails, time-series partitioning, and read-replica routing. Reaches for it when a migration locks a large table, ORMs generate UUID keys that fragment indexes, pool exhaustion appears under load, or reporting needs conflict with OLTP writes.
---

# Database Design

Schema decisions outlive application code by years: tables are refactored under
live traffic, and every mistake is a future migration. Design for the queries you
actually run, the write rate you sustain, and the deploys you must not break.

## Normalization

- 1NF: atomic values, no repeating groups. `phone1, phone2, phone3` columns
  violate it; make a `phones` table.
- 2NF: no non-key attribute depending on part of a composite key. An
  `order_lines` keyed by `(order_id, product_id)` that stores `product_name`
  depends only on `product_id` - move it to `products`.
- 3NF: no transitive dependencies. `orders.zipcode -> orders.city` makes city
  depend on a non-key; reference a `zipcodes` table.
- BCNF: every determinant is a candidate key - catches 3NF edge cases when
  composite keys overlap.

Denormalize deliberately, not preemptively: duplicate a column only when a JOIN
shows up in profiling (e.g. `orders.total_amount` as a snapshot at purchase
time), write both sides in one transaction, and record why. Undocumented
denormalizations become "impossible" bugs two years later.

## Primary Key Strategies

| Strategy | Pros | Costs |
|---|---|---|
| Auto-increment `bigint` | 8 bytes, sequential inserts stay B-tree-friendly, readable | Leaks volume in URLs; creation is DB-bound |
| UUID v4 | Globally unique, no coordination | 16 bytes; random inserts fragment B-trees and bloat the buffer cache |
| UUID v7 / ULID | Unique + time-ordered, so inserts stay sequential | Newer; check driver support and storage type |
| Composite natural key | No surrogate indirection | Must never change; every FK referencing it gets wide |

Default to `bigint` identity; use UUIDv7 when IDs must be generated client-side
or merged across systems. Avoid v4 UUIDs as the clustered/primary key of
write-heavy tables - insert-order randomization is a measured tax on every write
and every index.

## Foreign Key Design

- `RESTRICT`/`NO ACTION` (default): the delete fails loudly. Safest; use for
  anything referenced by billing, audit, or external contracts.
- `CASCADE`: children die with the parent (`order_lines` with `orders`). Fine
  for true ownership; catastrophic when the "child" is also referenced from
  elsewhere and the cascade fans out further than expected.
- `SET NULL`: the child survives detached (`tickets.user_id` on user deletion).
  Requires a nullable column plus a policy for orphans.
- Skipping FKs "for performance" costs one insert-time index lookup and buys
  silent orphans whenever deletes race. Omit FKs only for bulk-load staging
  tables or cross-system references, then enforce in code with a cleanup job.

## Indexing Strategy

- Index what queries need: WHERE predicates, JOIN columns, ORDER BY/GROUP BY.
- Composite order: equality columns first, range/sort column last -
  `(tenant_id, created_at)` serves `WHERE tenant_id = ? ORDER BY created_at
  DESC LIMIT 20`; the reverse order forces a sort.
- Covering indexes (`INCLUDE (...)`) remove heap lookups on hot point queries.
- Indexes are write amplification: each one adds maintenance per INSERT/UPDATE
  of an indexed column; 10 indexes can roughly double write latency. Audit with
  `pg_stat_user_indexes` (`idx_scan = 0`) and drop the dead ones.
- Low-cardinality columns (boolean flags) are near-useless alone. A partial
  index (`WHERE archived_at IS NULL`, `WHERE status = 'open'`) turns them into
  high-selectiveness indexes over exactly the hot subset.

## Schema Evolution

Zero downtime means additive, backward-compatible steps - expand then contract:

1. Expand: add nullable column or new table; deploy code that writes both old
   and new shapes.
2. Backfill in batches (1k-10k rows per transaction, throttled). One giant
   UPDATE locks the table and stalls replication.
3. Flip reads behind a feature flag; deploy readers; then stop writing old.
4. Contract: drop the old column in a later release, never the same deploy.

Guard DDL: Postgres takes ACCESS EXCLUSIVE locks even for fast operations if
they queue behind a long transaction - set `lock_timeout` (e.g. 5s) and retry.
MySQL rewrites tables for many ALTERs; use online DDL or pt-online-schema-change.
Adding a nullable column without a default is metadata-only in both engines.

## Connection Pooling

- Budget: `pool_size <= (db_max_connections - reserved) / app_instances`, and
  per-server optimum near `cores * 2 + spindles`. Each Postgres backend is a
  process costing ~5-10 MB; `max_connections = 500` on an 8-core box is a
  memory and lock-contention problem, not capacity.
- Exhaustion symptoms: `too many clients already`, rising pool wait times,
  timeouts that vanish when traffic drops. Fix by shrinking per-instance pools
  and adding PgBouncer (transaction mode; beware session features - `SET`,
  prepared statements, advisory locks) - or by fixing code that holds a
  connection across an outbound call.
- Acquire late, release early, and never hold a connection over an HTTP call.

## Transaction Isolation

| Level | Prevents | Still allows | Cost |
|---|---|---|---|
| READ COMMITTED (PG default) | Dirty reads | Non-repeatable reads, phantoms, lost updates | Baseline |
| REPEATABLE READ | Non-repeatable reads (PG MVCC also blocks phantoms) | Write skew (PG aborts conflicts: retry) | Retry logic required |
| SERIALIZABLE | All, including write skew | Nothing - it aborts instead | ~10-30% throughput; retries |

Enforce multi-row invariants (balance checks, inventory reservation) with
SERIALIZABLE or REPEATABLE READ plus `SELECT ... FOR UPDATE`, scoped to those
transactions only, and build retry-on-abort into the transaction helper. Do not
raise isolation globally for one critical screen.

## Data Modeling Patterns

- Many-to-many: join table keyed on both FKs (composite PK is fine); put
  relationship attributes (`enrolled_at`) on the join row, never on either side.
- Polymorphic association (`subject_type` + `subject_id`, no real FK): trades
  referential integrity for flexibility. Prefer join tables per type or a base
  table with typed subtypes. If forced (audit logs), validate in triggers and
  accept that the DB cannot enforce it.
- EAV tables: an anti-pattern at scale - every retrieval becomes a self-join
  festival. Use a JSONB column with a GIN index for truly schema-optional
  attributes instead.
- Soft deletes (`deleted_at`): every query must filter it (default scope +
  partial indexes); unique constraints need partial form (`WHERE deleted_at IS
  NULL`); nothing actually disappears, so pair with a retention job. Use hard
  delete plus an audit table when data must truly go (GDPR erasure).
- Audit trails: append-only `audit_log(actor_id, entity, entity_id, before,
  after, at)` written in the same transaction as the change; UPDATE on audit
  rows is forbidden by design.

## Time-Series and Event Data

- Partition by time (Postgres declarative range partitions, daily/weekly) once
  a table passes ~100M rows or retention matters: `DROP PARTITION` is instant
  where mass DELETE churns, bloats, and keeps VACUUM busy forever.
- Include the time column in PK/unique constraints; index (partition key +
  usual filter) so the planner prunes partitions before scanning.
- Retention: drop old partitions on a schedule; batch off-peak DELETEs where
  partitions are impossible.
- Pre-aggregate reads: a `metrics_hourly` rollup maintained by a job or
  incremental refresh beats scanning 90 days of raw events per dashboard load.

## Read Replicas and Eventual Consistency

- Route writes to the primary and reads to replicas deliberately; lag is
  nonzero (ms to seconds). Monitor `pg_stat_replication` replay lag and alert.
- Read-your-writes: after a user's own write (profile update, payment), serve
  that user's next reads from the primary, or hold the session on a replica
  until its replay timestamp passes the write's commit LSN. Without this,
  users see their submission "disappear".
- Split read/write paths explicitly at the data-access layer
  (`primary.query()` vs `replica.query()`). Implicit "any node" reads produce
  heisenbugs that vanish while you debug, because debugging latency exceeds
  replication lag.

## Common Pitfalls

- UUIDv4 clustered keys on write-heavy tables fragmenting B-trees.
- Expand and contract shipped in one deploy, breaking rolling restarts.
- Raising `max_connections` instead of fixing pool math; no PgBouncer.
- Unique constraints that ignore soft-deleted rows.
- Reading your own write from a lagging replica right after a form submit.
- Lock-taking DDL without `lock_timeout`, queued behind a long transaction.

## Definition of Done

- [ ] Tables at 3NF/BCNF; every denormalization documented with the query that
      justified it.
- [ ] FK actions chosen per relationship; no orphan-producing deletes.
- [ ] Migrations additive, batched, guarded by lock timeouts and staged deploys.
- [ ] Pool budget written down: instances x pool size fits DB limits.
- [ ] Multi-row invariants run at SERIALIZABLE or with explicit locks, with
      retry-on-abort.
- [ ] Post-write screens obey a read-your-writes rule against replica lag.
