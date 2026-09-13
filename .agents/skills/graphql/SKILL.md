---
name: graphql
description: Use whenever designing or evolving a GraphQL API: schema-first design from UI needs, mutation and argument naming, subscriptions over WebSockets, resolver anatomy, defeating N+1 with dataloaders, cursor pagination, partial-error shapes, field- and directive-level auth, complexity and depth limits, and federation boundaries. Reaches for it when queries trigger hundreds of database round trips, the resolver tree duplicates JOIN logic, pagination skips rows on live data, or a subgraph split is proposed for a simple CRUD service.
---

# GraphQL

Clients shape the query; the server owns cost control and compatibility. Every rule
below exists because client freedom to nest and branch turns small design choices
into production incidents (N+1, unbounded depth) or silent breaking changes
(renames, nullability flips) for clients you cannot see.

## Schema Design Principles

- Design from the UI screens backward, not from tables forward. A table-shaped
  schema leaks joins and surrogate IDs the client never wanted; the schema is the
  client's vocabulary for years.
- Model the domain (`Order`, `OrderLineItem`), not REST wrappers
  (`OrderResponse { data: [...] }`, `orderCreate` returning `{ success, message }`).
- Version by adding optional fields or same-meaning new names (`phoneV2`). Never
  rename, repurpose, or tighten nullability of an existing field - old clients
  keep sending old queries indefinitely. Deprecate with
  `@deprecated(reason: "...")`, monitor field usage, remove only at zero.

## Queries and Mutations

- Mutations are domain verbs: `createOrder`, `cancelOrder` - never table verbs
  (`updateOrdersTable`). Queries are nouns: `orders`, `orderById`.
- Take one `input: CancelOrderInput!` object per mutation. Adding an optional
  field inside the input is non-breaking; adding a top-level argument is not.
- Return a payload type with the affected object plus typed errors, not a bare
  `Boolean` - the client must know what state the server now believes in.
- Non-null (`!`) means "never absent under any circumstance". Inside a list,
  a non-null element that errors nulls the entire list. Make list elements and
  anything fetched from another service nullable unless truly guaranteed.

## Subscriptions

- Transport over WebSocket with the `graphql-ws` library/protocol;
  `subscriptions-transport-ws` is unmaintained legacy.
- Authenticate at `CONNECTION_INIT` once per socket - per-event HTTP headers do
  not exist on a WebSocket, and unauthenticated subscriptions leak data.
- Name events as facts: `orderPlaced`, `paymentFailed`. Send deltas when the
  client can apply them; send IDs when the client should refetch.
- Handle reconnection client-side: resubscribe with a last-event cursor and
  reconcile the gap. Servers hold no socket state across reconnects.

## Resolvers

Signature: `fieldName(parent, args, context, info)`.

- The default resolver reads `parent[fieldName]` synchronously - write a custom
  resolver only when the value is computed, fetched, or guarded.
- `context` carries per-request state (auth user, loaders, DB handles) and is
  built once per request at the server layer. `info` exposes selection sets;
  touch it only when profiling proves a real win from lookahead.
- Resolvers may be async anywhere; the executor awaits each level. Keep them
  thin: resolve keys, delegate batching to loaders, keep branching in the
  schema, not in resolver trees.

## N+1 and Dataloaders

A list of 50 orders each resolving `customer` fires 51 queries (1 + 50). Batch
with DataLoader, which collects keys within one tick and issues one query:

```js
const customerLoader = new DataLoader(async (ids) => {
  const rows = await db.customers.whereIn('id', ids);   // exactly 1 query
  return ids.map((id) => rows.find((r) => r.id === id)); // aligned to input keys
});
```

- Build loaders per request, never at module scope: they cache, and a shared
  loader hands one user's data to the next request.
- The batch function must return results aligned to input keys, using
  `undefined` (not `null`) for misses - that is how cache hits work.
- Dataloaders are overkill when the parent resolver already JOINed the child
  data, or when a resolver runs once per request (no batch to form). They add
  a microtask tick of latency; pay it only when N is real.

## Pagination

Use cursor pagination (Relay connection spec: `edges { node cursor } pageInfo
{ hasNextPage endCursor }`) for anything that changes under pagination: with
offsets, two inserts between page fetches duplicate or skip rows. Offsets also
degrade - `OFFSET 100000` reads and discards 100k rows, while a keyset cursor
(`WHERE (created_at, id) < ($1, $2) ORDER BY created_at DESC, id DESC LIMIT 20`)
stays index-fast. Reserve offsets for small, static, page-numbered admin tables.

## Error Handling

- Field failures belong in the top-level `errors` array with `path` pointing at
  the field and `extensions.code` (`UNAUTHENTICATED`, `FORBIDDEN`,
  `BAD_USER_INPUT`); surviving `data` is still delivered - partial success is a
  feature, not a bug.
- For mutation outcomes the client must branch on, return typed errors in
  `data` (`payload.userErrors: [UserError!]!`) and reserve top-level `errors`
  for failures the client can only display.
- In production, never put stack traces, SQL, or file paths in `message`.

## Authentication and Authorization

- Authenticate once per request into `context` (token -> user). Authorize:
  resolver-level guards for scattered per-field rules; directive-based auth
  (`@auth(role: ADMIN)`) when the rule is uniform and you want it visible in
  the schema instead of buried in code.
- Enforce authorization at the data-fetch layer too (loaders, `info` filtering)
  - nested selections bypass shallow top-field checks.
- Propagate identity explicitly to subgraphs and backing services; context does
  not cross process boundaries by itself.

## Performance

- Depth limit (e.g. max 10 levels) and complexity limit (score each field,
  reject above ~1000). Without them, one client's nested query is a
  denial-of-service against your database.
- Persisted queries / APQ: clients send a hash, the server stores the document.
  This removes per-request parsing cost and blocks arbitrary attacker-authored
  queries in production.
- Set per-type/per-field cache hints; mark per-user data private. Measure with
  tracing before touching resolvers - the expensive resolver is usually not the
  suspected one.

## Federation and Stitching

Split into subgraphs when separate teams own disjoint domains and need
independent deploys; the gateway comes at the cost of a network hop plus entity
resolution, so do not split for "performance". Prefer federation (`@key`
entities, composed schema) over ad-hoc schema stitching, which has no governance
and drifts silently.

## When NOT to Use GraphQL

- Plain CRUD over a few resources: REST with OpenAPI is less machinery.
- Fixed server-to-server internal APIs: the consumer never varies its queries,
  so schema flexibility buys nothing and costs a gateway.
- File-heavy workflows: multipart over GraphQL is awkward; serve files from
  object storage via signed URLs and keep only metadata in the graph.

## Common Pitfalls

- Non-null fields inside lists nulling whole lists on one bad row.
- Module-scoped DataLoaders caching data across users.
- Mutations returning `Boolean` with machine-readable errors nowhere.
- No depth/complexity limits or persisted queries in production.
- Tightening nullability or renaming fields and breaking unseen clients.

## Definition of Done

- [ ] No N+1: batched loaders or JOINed parents, verified by query counting.
- [ ] Cursor pagination wherever the data changes during paging.
- [ ] Errors carry `extensions.code`; partial success preserved where intended.
- [ ] Auth checks cover nested fields, not just top-level resolvers.
- [ ] Depth and complexity limits active; persisted queries enabled in production.
