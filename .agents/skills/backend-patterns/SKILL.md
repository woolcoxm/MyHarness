---
name: backend-patterns
description: Use whenever designing or hardening server-side request handling: middleware and validation chains, session vs JWT vs API-key auth, domain error classes and HTTP status mapping, DB pooling, background jobs with retries and dead letters, streaming uploads, rate limiting, health probes, 12-factor config, and structured logging with correlation IDs. Reaches for it when endpoints leak stack traces, the pool times out under load, uploads exhaust memory, jobs retry forever, or logs cannot answer what a request did.
---

# Backend Patterns

Every request crosses the same surfaces: transport, middleware, validation,
handler, dependencies, response. Harden them in that order - a fix at the
validation boundary protects every handler; a fix inside one handler protects one
route. Each rule below states why it exists so it survives refactors.

## Request Lifecycle

Order the chain deliberately: request ID + logging, then error handler
(registered last so it runs first on failure), then auth, rate limit, body parse
(with size cap), route validation, handler, serializer.

- Middleware must either call the next stage or respond - never both.
- Handlers return domain objects; exactly one layer maps domain errors to HTTP.
  Handlers that set status codes directly make error behavior unfindable.
- Bound the whole request (server timeout, e.g. 30s) and each outbound call
  (3-5s). A hanging upstream otherwise pins connections until pools exhaust.

## Authentication: Decision Matrix

| Pattern | State | Best for | Weakness / mitigation |
|---|---|---|---|
| Session cookie | Server-side store | Browsers, logout, instant revocation | Lookup per request; needs CSRF token; set `HttpOnly; Secure; SameSite=Lax` |
| JWT (Authorization header) | Stateless | Services, mobile, cross-domain | Unrevocable before expiry; keep access TTL 5-15 min + server-side refresh token |
| API key | Revocable table | Server-to-server, scripts, public APIs | Rotate on schedule; scope per key; store hashed |

- Never store tokens in `localStorage` - any XSS reads them. Cookies with
  `HttpOnly` are not readable from scripts, which is the point.
- API keys: embed a visible prefix (`sk_live_ab12...`) so leaks are searchable,
  store only a SHA-256 hash, show the plaintext once at creation.

## Input Validation

Validate at the boundary with a schema (zod, Joi, Pydantic, class-validator):
inside the handler is too late for routing concerns, and ad-hoc checks do not
compose.

- Whitelist: declare expected fields and types; reject or strip unknown fields.
  Blacklists fail on the eleventh variant of the payload you did not predict.
- Validation asks "is this acceptable input"; sanitization asks "is it safe to
  embed in HTML/SQL". Do both, separately: validate structure and ranges up
  front; neutralize by construction at the output (parameterized queries,
  auto-escaping templates), not by scrubbing input.
- Parse-then-freeze: coerce once into a validated object, freeze it, and pass it
  downstream so no layer re-reads raw input.

## Error Architecture

Define error classes per domain (`PaymentError`, `NotFoundError`) carrying a
status code and a safe message; one top-level middleware maps them to responses:

```js
class DomainError extends Error {
  constructor(message, status, code) { super(message); this.status = status; this.code = code; }
}
// mapper: DomainError -> its status+code; anything else -> 500 + generic message
```

- 4xx means the caller made a mistake; 5xx means we did. 401 unauthenticated,
  403 unauthorized, 404 also hides existence, 409 conflict, 422 validation,
  429 throttled.
- In production return `{ "error": { "code", "message" } }` - never stack traces,
  SQL, or file paths; they hand attackers a map. Log the full error keyed by
  request ID instead.

## Database Connections

- Pool always; acquire late, release early. A connection is held across `await`s,
  so a 2s outbound HTTP call inside a transaction holds a slot for 2s - that is
  the usual cause of exhaustion, not pool size.
- Size from `core_count * 2 + spindles` (HikariCP heuristic) as the server-side
  optimum; the real ceiling is `instances * pool_size` vs DB `max_connections`.
  Raising `max_connections` past ~2x cores buys contention, not throughput.
- Queue waiters with a timeout, alert on wait time, and wrap multi-statement
  invariants in one transaction.

## Background Jobs

- Queue selection: DB-backed (`SELECT ... FOR UPDATE SKIP LOCKED`) to start;
  Redis-backed (BullMQ, Sidekiq) past ~1k jobs/min or when you need
  delayed/scheduled jobs; Kafka for stream-scale volume and replay.
- Make every job idempotent: at-least-once delivery means the same job runs
  twice. Dedup on a natural key (`orderId`), never on a message auto-ID.
- Retry with exponential backoff plus jitter (e.g. 30s, 2m, 8m, 30m, 2h), cap
  attempts at ~5, then dead-letter with an alert. Uncapped un-jittered retries
  re-create the outage you are retrying through.

## File Uploads

- Stream multipart bodies to disk or object storage. Buffering a 2GB upload is
  self-inflicted DoS; enforce a global cap below the reverse proxy's limit.
- Check `Content-Length` first, then count bytes while streaming and abort past
  the cap - the header alone lies.
- Validate by magic bytes, never the client-supplied MIME or extension. Scan
  with ClamAV when files will be served to other users; store outside the web
  root; serve from a separate domain or signed URLs (uploads + cookies on one
  origin turn a stored SVG into session theft).

## Rate Limiting

Use a sliding window (Redis `ZADD` + `ZREMRANGEBYSCORE` per key, O(log n)) -
fixed windows allow a 2x burst exactly at each boundary. Key by, in preference
order: API key, user ID, then IP; trust `X-Forwarded-For` only behind proxies you
control. Return `429` with `Retry-After`. Split limit classes: strict on auth
(e.g. 5/min per account), generous on reads (e.g. 600/min).

## Health Checks

- Liveness (`/healthz`): process is up. Must not touch the DB - a DB outage
  would otherwise restart healthy pods in a loop.
- Readiness (`/readyz`): dependencies reachable; failing it removes the instance
  from the load balancer without killing in-flight work.
- Startup probe for slow bootstraps so the platform does not kill a process
  still warming caches.

## Configuration

12-factor: config lives in the environment; the build artifact is identical
across environments. Validate every env var at boot and fail fast listing all
missing keys (one-at-a-time errors force 10 restarts to find 10 problems). Keep
feature flags in one registry evaluated per request; flags baked into build
artifacts cannot be flipped without a redeploy.

## Logging

- Emit structured JSON, one object per line: `ts, level, requestId, userId,
  route, status, duration_ms`. String logs cannot be queried or aggregated.
- Levels that matter: `error` pages someone; `warn` means degraded but
  self-recovering; `info` is request summaries and state changes; `debug` is off
  by default. If `info` floods enough to hide `warn`, it is wrong.
- Propagate one request ID from the edge through logs, outbound headers, and job
  messages - without it, cross-service debugging is guesswork.
- Scrub PII and secrets in the serializer (blocklist for tokens, emails, card
  numbers), not by trusting every call site to remember.

## Common Pitfalls

- Registering the error middleware before routes, so it never fires.
- Auth checks copied per handler: the one forgotten route is the breach.
- Holding a pooled connection across an outbound HTTP call.
- Retrying non-idempotent jobs (charges, sends) without dedup keys.
- Trusting client `Content-Type` or file extension for upload validation.
- Rate limiting by `remoteAddress` behind a proxy: every user shares one IP.

## Definition of Done

- [ ] All input validated at the boundary with whitelisted, frozen schemas.
- [ ] Errors mapped in one place; 5xx responses leak no internals.
- [ ] DB access pooled; no connection held across outbound calls.
- [ ] Jobs idempotent, retries capped and jittered, dead-letter path alerting.
- [ ] Liveness and readiness probes fail independently and correctly.
- [ ] Structured logs carry a correlation ID from edge to job queue.
