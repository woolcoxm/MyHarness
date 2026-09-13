---
name: sql-mastery
description: Use whenever writing or tuning nontrivial SQL: window functions (ROW_NUMBER, PARTITION BY, frames), recursive CTEs for trees, anti-joins, correlated subqueries, reading EXPLAIN ANALYZE, B-tree vs GIN vs GiST index choice, FILTER and GROUPING SETS, JSONB containment and indexing, and PostgreSQL vs MySQL vs SQLite divergence. Reaches for it when a report query takes minutes, an index exists but is not used, deduplication keeps ties, gaps must be found in sequences, or the same query behaves differently across engines.
---

# SQL Mastery

Write set-based queries, then prove their plans. Every rule exists because the
optimizer is excellent at shapes it recognizes (sargable predicates, narrow
indexes) and quietly degrades to sequential scans on shapes it does not - usually
silently, discovered at 1000x the row count.

## Window Functions

- Ranking: `ROW_NUMBER()` is unique (ties broken by ORDER BY); `RANK()` shares
  the rank and skips (1,1,3); `DENSE_RANK()` shares without gaps (1,1,2).
  Deduplicate with `ROW_NUMBER() OVER (PARTITION BY natural_key ORDER BY
  updated_at DESC)` and keep `rn = 1` - that is the deterministic winner.
- `PARTITION BY` scopes the window; omit it and every function frames the whole
  result set (grand totals instead of per-customer running totals).
- Frames: with ORDER BY, the default is `RANGE ... UNBOUNDED PRECEDING TO CURRENT
  ROW`, and RANGE includes peer rows (ties). For running totals that silently
  double-counts tied timestamps - state `ROWS` explicitly:

  ```sql
  SELECT id, account_id,
         SUM(amount) OVER (PARTITION BY account_id ORDER BY posted_at
                           ROWS UNBOUNDED PRECEDING) AS running_balance
  FROM ledger;
  ```

- First/last per group: `ROW_NUMBER()` + filter (portable), or
  `(ARRAY_AGG(x ORDER BY ts DESC))[1]` in Postgres to fetch more columns.
- Gap detection (sessions, missing sequence values):

  ```sql
  WITH s AS (SELECT id, created_at,
                    LAG(created_at) OVER (ORDER BY created_at) AS prev)
  SELECT * FROM s WHERE created_at - prev > INTERVAL '5 minutes';
  ```

## CTEs

- Non-recursive CTEs name pipeline stages; Postgres 12+ inlines them, so
  readability is free. Pre-12, every CTE was an optimization fence - an indexed
  lookup inside one became a seq scan. Use `NOT MATERIALIZED` / `MATERIALIZED`
  explicitly when the fence matters either way.
- Recursive CTEs traverse trees and graphs:

  ```sql
  WITH RECURSIVE tree AS (
      SELECT id, parent_id, 1 AS depth FROM nodes WHERE id = $1
    UNION ALL
      SELECT n.id, n.parent_id, t.depth + 1
      FROM nodes n JOIN tree t ON n.parent_id = t.id
  )
  SELECT * FROM tree;
  ```

  `UNION ALL` allows duplicates but never loops-guard against cycles by
  limiting `depth` or tracking visited IDs; plain `UNION` dedupes every
  iteration and is the slow safe option on cyclic data.

## Complex JOINs

- `LEFT JOIN ... WHERE right.col = 'x'` is an INNER JOIN in disguise: the WHERE
  discards the NULL-extended rows the LEFT JOIN produced. Put right-side
  filters in `ON`; use `WHERE right.id IS NULL` deliberately as an anti-join.
- Anti-joins: prefer `NOT EXISTS` over `NOT IN`. `NOT IN` against a set
  containing NULL returns zero rows forever (NULL comparison semantics), and
  plans worse on many engines:

  ```sql
  SELECT o.* FROM orders o
  WHERE NOT EXISTS (SELECT 1 FROM refunds r WHERE r.order_id = o.id);
  ```

- Self-joins model hierarchies (employee/manager) and adjacency (previous/next
  event per user). Alias both sides; index the join column.

## Subqueries vs JOINs

Pick whichever states the question, then check the plan. Correlated subqueries
in the SELECT list are fine when the planner unnests them; in WHERE against
large sets they can execute per-row - rewrite to `EXISTS` or a JOIN after
reading EXPLAIN. Keep scalar subqueries (MAX/MIN lookup) where a JOIN would fan
out rows: a JOIN that multiplies rows then aggregates them is a correctness bug
(inflated SUMs/COUNTs), not merely slow.

## Query Optimization

Read `EXPLAIN (ANALYZE, BUFFERS)` top-down:

- Compare `estimated rows` vs `actual rows`; off by 10x or more means stale
  statistics (run `ANALYZE`) or a shape the planner misprices.
- Nodes: Seq Scan reads the whole table - fine for small tables or when the
  query touches most rows; Index Scan for selective lookups; Bitmap Heap Scan
  for medium selectivity. Nested Loop + inner index scan beats Hash Join only
  on small outer sides; a Hash Join over 2 rows is a mispricing smell.
- `rows removed by filter` close to total = index exists but is useless for
  this predicate, or is missing the right column.
- Sargability: keep columns bare in predicates. `WHERE created_at >=
  DATE '2026-01-01'` can use an index; `WHERE DATE(created_at) = '2026-01-01'`
  cannot, so it scans everything.

## Indexing Strategy

| Type | Serves | Use for |
|---|---|---|
| B-tree (default) | `=`, `<`, `>`, `BETWEEN`, `ORDER BY`, `LIKE 'abc%'` | Almost everything |
| GIN | `@>`, `?`, `?|`, tsvector, arrays | JSONB containment, full-text, trigram |
| GiST | ranges, geometry, KNN | exclusion constraints, geo queries |

- Partial indexes cover the hot subset at a fraction of the size:
  `CREATE INDEX ON orders (customer_id) WHERE status = 'open'`.
- Covering: `CREATE INDEX ON orders (customer_id) INCLUDE (status, total)`
  enables index-only scans (verify heap fetches drop to ~0 after VACUUM).
- Composite order: equality columns first, then the range/sort column -
  `(tenant_id, created_at)` serves `tenant_id = ? ORDER BY created_at DESC
  LIMIT 20`; the reverse order cannot serve the sort.

## Aggregation Patterns

- Use `COUNT(*) FILTER (WHERE status = 'paid')` (Postgres, SQL standard) to
  compute per-status counts in one pass instead of N subqueries or scans.
- Subtotals: `GROUP BY ROLLUP (region, country)` returns per-country,
  per-region, and grand-total rows in one scan - replacing a UNION of three
  queries. `CUBE` for full cross-grouping, `GROUPING SETS` to pick subsets.
- Check for HashAggregate in the plan; a sort spilling to disk on wide
  distinct sets means raising `work_mem` or pre-aggregating.

## JSONB (Postgres)

- Containment: `payload @> '{"tags": ["urgent"]}'` (GIN-indexable). Existence:
  `payload ? 'key'`. Extraction: `payload #>> '{a,b}'` yields text, `#>` yields
  JSON, `->>' yields text for one key.
- Index with `CREATE INDEX ON events USING GIN (payload jsonb_path_ops)` -
  smaller and faster than the default GIN when you only need `@>`.
- `->` compares JSON to JSON; comparing its result to a string literal yields
  NULL. Use `->>` for text comparisons. A hot `->>` path in WHERE deserves a
  generated column + plain B-tree index instead.

## CTE vs Temp Table vs Subquery

CTE: scoped to one statement, inlined or materialized per planner/version.
Temp table: gets statistics, can be indexed, persists across statements in the
session - use it when an intermediate result is large, reused 2+ times, or when
the plan shows a CTE recomputed. Subquery/derived table: inline, unnamed, fine
for one-shot filters. Default to CTEs for readability; switch deliberately.

## Engine Gotchas

- Window functions: full support in Postgres; MySQL 8.0+ only (5.7 has none);
  SQLite 3.25+. `FILTER` is Postgres/SQLite; on MySQL emulate with
  `SUM(CASE WHEN ... )`.
- CTE optimization: Postgres 12+ inlines (older fences everything); MySQL 8
  materializes always; SQL Server inlines from 2019.
- JSON: MySQL uses path syntax `payload->>'$.a.b'`; Postgres uses
  `payload #>> '{a,b}'`. `NOT IN` + NULL traps apply everywhere; MySQL adds
  utf8mb4 collations that compare case- and accent-insensitively by default.

## Common Pitfalls

- A WHERE on the right table silently converting LEFT JOIN to INNER JOIN.
- `NOT IN` against a nullable column returning nothing, forever.
- Function-wrapped columns in WHERE defeating every index on that column.
- Default `RANGE` frame double-counting peer rows in running totals.
- Reading estimated plans only - `EXPLAIN ANALYZE` shows what actually ran.

## Definition of Done

- [ ] `EXPLAIN (ANALYZE, BUFFERS)` inspected; estimates within ~10x of actuals.
- [ ] Selective queries hit indexes; every seq scan is justified by row share.
- [ ] Anti-joins use `NOT EXISTS`; no `NOT IN` on nullable columns.
- [ ] Window frames stated explicitly wherever ties are possible.
- [ ] Composite indexes ordered equality-first, range/sort column last.
