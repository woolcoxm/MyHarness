---
name: api-design
description: Design and review HTTP and GraphQL APIs — resource modeling, method semantics, status codes, pagination, versioning, error shapes, webhooks, and authentication — whenever creating a new API, adding endpoints, or evaluating an API contract.
---

# API Design: HTTP and GraphQL Contracts

An API is a promise: every endpoint, status code, and error shape becomes someone
else's dependency. Optimize for the client that arrives in two years holding only the
docs. Predictability beats cleverness in every choice below.

## REST Resource Modeling

Model nouns, not verbs. URLs identify things; HTTP methods supply the verbs.

```text
GOOD                          BAD
GET    /users/42              GET /getUser?id=42
POST   /orders                GET /createOrder
GET    /users/42/orders       (ownership = nesting; filtering = query param)
DELETE /orders/9/line-items/3 POST /orders/9/removeLineItem
```

Nesting rules:
- Nest only for ownership/existence dependence (`/users/42/orders`); never nest for filtering.
- Cap nesting at two levels; deeper paths get brittle — use a top-level resource plus a filter.
- Use a sub-resource noun for state transitions: `POST /orders/9/cancellation` — one endpoint, explicit idempotency.
- Actions that fit no resource semantics (search, compute, export) get a noun sub-resource: `POST /orders/search`.

## HTTP Method Semantics

| Method | Safe | Idempotent | Use |
|---|---|---|---|
| GET | Yes | Yes | Read; never mutate (no logging-in-via-GET) |
| PUT | No | Yes | Full replace of the resource at a known, client-owned URI |
| PATCH | No | No* | Partial update (JSON Patch / merge patch) |
| POST | No | No | Create (server assigns ID), process, trigger |
| DELETE | No | Yes | Remove; further DELETEs return 404/204 |

(*PATCH is idempotent only if the patch format is — document your guarantee.)
PUT requires the client to own the identity (`PUT /users/42`); POST lets the server
assign it (`POST /users`). Choose on that basis.

## Status Codes That Matter

| Code | Return when |
|---|---|
| 200 | Success with a body (GET/PUT/PATCH) |
| 201 | Resource created; set `Location: /things/9` |
| 202 | Accepted for async processing; return a status URL |
| 204 | Success, empty body (DELETE, no-content update) |
| 400 | Malformed request — bad JSON, missing field; client must change the request |
| 401 | Unauthenticated — no or invalid credentials |
| 403 | Authenticated but not permitted — do not leak whether the resource exists |
| 404 | Not found (also for forbidden-but-hidden resources) |
| 409 | State conflict — duplicate key, stale version, idempotency mismatch |
| 422 | Well-formed but semantically invalid (validation failure) |
| 429 | Rate limited — MUST include `Retry-After: 30` (seconds or HTTP-date) |

400 vs 422: 400 = server could not parse it; 422 = parsed but failed validation.
Pick one convention, document it, never mix.

## Pagination

Offset pagination (`?page=3&per_page=50`) is fine below ~10k rows. It degrades badly:
`OFFSET 100000` scans and discards rows, and inserts during iteration shift pages,
skipping or duplicating items. Cursor wins at scale:

```text
GET /orders?limit=50                          -> first page, return next_cursor
GET /orders?limit=50&cursor=eyJpZCI6MTQyfQ==  -> stable, index-friendly, no skips

Cursor = opaque, signed encoding of the sort key + unique id (tie-breaker).
Return next_cursor: null at the end; treat cursors as opaque — clients never construct them.
```

## Versioning

| Strategy | URL path `/v1/` | Header `Accept: ...;v=1` | Content negotiation |
|---|---|---|---|
| Visibility | Obvious in logs, docs, cache keys | Invisible in URL | Invisible |
| Client effort | Trivial | Medium | Medium |
| Purity | Purists object | Correct per HTTP spec | Correct per spec |
| Precedent | Stripe, GitHub (path) | GitHub (media type) | Rare |

Use URL path versioning: cache-friendly, debuggable, greppable; operational clarity
beats purity. Additive changes (new optional fields, new endpoints) are not breaking —
do not bump the version. Breaking = removing/renaming fields, changing types or
semantics, tightening validation.

## Error Response Format

One shape for every error, everywhere:

```json
{
  "error": {
    "code": "order_already_shipped",
    "message": "Order 9 has already shipped and can no longer be cancelled.",
    "details": [{ "field": "order_id", "issue": "state is 'shipped', expected 'pending'" }],
    "documentation_url": "https://api.example.com/docs/errors#order_already_shipped",
    "request_id": "req_8f3a2b"
  }
}
```

Rules: `code` is a stable machine-readable string (never just the HTTP phrase);
`message` is human-readable and says what to do next; `details` carries per-field
validation errors; include `request_id` in body and header so support can trace it.
Never leak stack traces or internal identifiers.

## GraphQL Schema Design

Design types around client views, not database tables.

- Use the Connections spec for lists (edges/nodes/pageInfo) — pagination is built in.
- Mutations are verb-noun (`cancelOrder(input: CancelOrderInput!): CancelOrderPayload`); return a payload type with the mutated object plus a `userErrors` list.
- Reserve subscriptions for push semantics (live updates); do not emulate polling.
- N+1: every list resolver that fetches per-item is a guaranteed N+1 — dataload every relation:

```js
const userLoader = new DataLoader(async (ids) => {
  const users = await db.users.whereIn("id", ids);   // 1 query, not N
  return ids.map((id) => users.find((u) => u.id === id));
});
// resolver: (post) => userLoader.load(post.author_id)
```

Return errors via the `errors` array with `extensions.code`; keep HTTP at 200 except
for transport-level failures.

## API Documentation

Write OpenAPI first or generate it from code — the spec is the source of truth and
lives in the repo. Keep docs in sync mechanically: CI validates the spec's examples
against real responses (contract tests). An example request+response per endpoint is
worth more than any prose. Document per endpoint: auth required, rate limit class,
idempotency support, pagination style.

## Rate Limiting

| Algorithm | Behavior | Cost |
|---|---|---|
| Token bucket | Burst up to bucket size, sustained refill rate | O(1) per key |
| Sliding window log | Exact, no boundary spikes | O(requests) — avoid raw |
| Sliding window counter | Approximate, cheap, smooths boundaries | O(1) |

Use token bucket for per-user limits (natural bursts); fixed/sliding counter for
global shingles. Always communicate the limits — undocumented limits get discovered
by outages in production. Document burst vs sustained explicitly.

```text
RateLimit-Limit: 100
RateLimit-Remaining: 37
RateLimit-Reset: 14
Retry-After: 14        (on 429 only)
```

## Webhook Design

```json
{
  "id": "evt_7c1d",
  "type": "invoice.paid",
  "created_at": "2026-09-12T10:00:00Z",
  "data": { "invoice_id": "in_92", "amount": 4900, "currency": "usd" }
}
```

- Retries: assume the receiver is down; exponential backoff + jitter for 24h, then dead-letter. Delivery is therefore at-least-once — receiver-side idempotency on the event `id` is mandatory.
- Signature: HMAC-SHA256 over the *raw* body, header `X-Signature: t=<ts>, v1=<hmac>`, timestamp included; receivers reject stale timestamps (replay protection) and verify over raw bytes, never a re-serialized body.
- Return 2xx fast and work async; a receiver that processes inline times out and triggers duplicate deliveries.

## Authentication Patterns

| Pattern | Use when | Weakness |
|---|---|---|
| API key | Server-to-server, scripts, simple | No user context; manual rotation |
| JWT | Stateless service-to-service, short-lived sessions | Unrevocable before expiry; keep TTL minutes |
| OAuth2 / OIDC | Third-party delegated access to user data | Complex; use a library, never hand-roll |
| Session cookie | First-party browser apps | Needs sticky/shared session store |

First-party web: session cookie (HttpOnly, Secure, SameSite). Machine-to-machine
inside the trust boundary: short-lived JWTs or mTLS. Third-party developers: API keys
with scopes. "Act as this user on another service": OAuth2. Never put bearer tokens
in URLs or logs.

## Common Pitfalls

- Verbs in URLs (`/getUser`, cancel-via-GET) — breaks caching, proxies, and every HTTP-aware tool.
- 200 with `{"success": false}` — status codes are the contract; use 4xx/5xx.
- 401 vs 403 confusion — 401 is "who are you", 403 is "I know you; no".
- Unbounded lists — every collection endpoint paginates from day one; retrofitting pagination breaks clients.
- Breaking changes without a version bump — removing a field breaks clients that defensively read it.
- Different error shapes per endpoint — clients special-case each one; enforce one envelope with a shared serializer.
- Webhooks without receiver idempotency — at-least-once delivery means duplicates will happen; the duplicate must be a no-op.
- JWTs that live for days — a leaked token is unrevocable; use short TTL + refresh, or accept sessions.
