---
name: nodejs-runtime
description: Use whenever building, debugging, or optimizing Node.js services and tooling: event loop internals (microtasks vs macrotasks), streams and backpressure, buffers for binary protocols, worker_threads vs child_process vs cluster, lockfile discipline, dotenv and config layering, CommonJS/ESM interop, async error handling, graceful shutdown, and streaming filesystem work. Reaches for it when the server stalls under load, memory climbs on large files, imports fail with ERR_REQUIRE_ESM, or unhandled rejections crash the process.
---

# Node.js Runtime

Node runs JavaScript on one thread over an event loop, with a libuv thread pool
(default 4, override with `UV_THREADPOOL_SIZE`) for fs, DNS, and crypto. Every rule
below exists because breaking it does one of two things: blocks the loop (every
connected client stalls together) or buffers a whole payload in memory (heap grows
until the process dies). Optimize for those two failure modes first.

## Event Loop

Phase order each iteration: timers (`setTimeout`/`setInterval`), pending I/O
callbacks, poll, check (`setImmediate`), close callbacks. Microtasks (promises,
`queueMicrotask`) drain after every macrotask and between phases; `process.nextTick`
drains before even promises. Recursive `nextTick` starves I/O entirely - that is
why user-facing code never queues it unboundedly.

| API | Runs | Use for |
|---|---|---|
| `process.nextTick` | Before promise microtasks | Framework internals only |
| `Promise.then` / `queueMicrotask` | After nextTick, before timers | Default async sequencing |
| `setImmediate` | Check phase, same iteration | Continue after current I/O |
| `setTimeout(fn, 0)` | Timers phase, next iteration | Real delays, never ordering |

A 100ms synchronous operation (multi-MB `JSON.parse`, bcrypt, image resize) caps the
entire process at 10 req/s no matter how many connections wait. Move CPU work over
~10ms off the loop (see workers below) or chunk it with `setImmediate` between
slices. Diagnose with `node --cpu-prof` or `clinic doctor`: one stack pinned at
100% inside a request handler is the signature of loop blocking.

## Streams

Four types: Readable, Writable, Duplex (independent both directions, e.g. a TCP
socket), Transform (output derived from input, e.g. gzip, line splitting).

- Pipe to bound memory: `readable.pipe(writable)` pauses the source when the
  sink's buffer fills and resumes on `drain`. `highWaterMark` (64KB default for fs
  streams) is the tripwire that triggers pause, not a suggestion.
- Never load a large file just to send it:

  ```js
  // Bad: entire file sits in heap before the first byte is sent
  res.end(await fs.promises.readFile(path));
  // Good: ~64KB chunks, backpressure respected
  const { size } = await fs.promises.stat(path);
  res.setHeader('Content-Length', size);
  fs.createReadStream(path).pipe(res);
  ```

- Use `for await (const chunk of stream)` for ordered processing (parsers, CSV);
  it manages pause/resume for you.
- When writing manually, stop when `write()` returns `false` and resume on
  `'drain'`. Ignoring that boolean is how heaps grow to file size.

## Buffers

Use `Buffer` (a `Uint8Array` subclass) for anything binary; converting to strings
silently re-encodes and corrupts. Three rules prevent most encoding hell:

- Split binary data on byte markers (`buf.lastIndexOf(0x0a)`), never on a decoded
  string's `indexOf`, because a multi-byte UTF-8 character can span a chunk
  boundary and string-splitting cuts it in half.
- Call `toString('utf8')` only at field boundaries; reverse with
  `Buffer.from(s, 'utf8')`. Always pass an explicit encoding.
- Track byte lengths with `Buffer.byteLength(s)`, not `s.length`: 'e-acute' is 1
  character, 2 UTF-16 code units, 2 UTF-8 bytes. Protocol headers carry byte
  counts, so compute those, not character counts.

## worker_threads vs child_process vs cluster

| Tool | Isolation | Use when | Cost |
|---|---|---|---|
| `worker_threads` | Thread in same process | CPU-bound JS over ~10ms | postMessage copies or SharedArrayBuffer |
| `child_process` | Separate process | Shell out, run non-JS binaries | Spawn 10-50ms; stdout serialization |
| `cluster` | N full processes | Scale an HTTP server across cores | N heaps, no shared state; needs sticky sessions for sessions/WebSockets |

Do not reach for `cluster` to fix a slow handler - more processes multiply a slow
path, they do not fix it. Profile first (event loop section above).

## npm, pnpm, yarn

- Commit exactly one lockfile (`package-lock.json`, `pnpm-lock.yaml`, or
  `yarn.lock`). CI installs with `npm ci` (or `pnpm i --frozen-lockfile`), which
  fails on lockfile drift; plain `install` in CI hides range drift and makes
  builds unreproducible.
- Pin exact versions in applications (the lockfile is the truth); publish
  libraries with caret ranges (`^1.2.3`) so consumers can patch. Use tilde (`~`)
  when a dependency's minor releases break things.
- Peer dependency conflicts: run `npm ls <pkg>` to see the duplicated tree, then
  align the consumer's range with the host's requirement. Never `--force` an
  install - it moves the failure from install time to runtime, where nobody is
  looking.

## Environment and Configuration

Layer config so each level overrides the last: defaults in code, then `.env` (dev
only), then real environment variables, then CLI flags. Read `process.env` once at
startup, coerce and validate it there, and export a typed object - scattering
`process.env.X` across handlers hides what config exists and moves crashes from
boot time to request time. Never commit secrets; commit `.env.example` documenting
keys without values. In production, inject env vars from the secret manager rather
than shipping `.env` files into images.

## CommonJS vs ESM

Target ESM (`"type": "module"`) for new code: static imports enable tree-shaking
and top-level await, and it is where the ecosystem is moving. Interop gotchas:

- `require()` of an ESM module throws `ERR_REQUIRE_ESM` (until Node 22+ with
  `--experimental-require-module`). If a CJS dependency must load ESM, load it
  with dynamic `import()` from async code.
- CJS default-vs-named: `import pkg from 'cjs-pkg'` gives `module.exports`;
  named imports work only when cjs-module-lexer can detect them statically
  (`module.exports = { a, b }` detects; `exports[key]` does not - destructure
  from the default import instead).
- `__dirname`/`__filename` do not exist in ESM; derive them with
  `fileURLToPath(new URL('.', import.meta.url))`.
- Windows trap: ESM specifiers must be `file://` URLs, not bare `C:\...` paths,
  or Node throws `ERR_UNSUPPORTED_ESM_URL_SCHEME`. This repo's AGENTS.md also
  warns about `\U` in double-quoted TOML - same class of bug, same fix.

## Async Errors and Shutdown

- Every promise chain is either `await`ed or has a `.catch`. One floating promise
  is one random crash later, far from its cause.
- Register process handlers - they are the airbag, not the strategy:

  ```js
  process.on('unhandledRejection', (err) => { logger.error({ err }); shutdown(1); });
  process.on('uncaughtException', (err) => {   // state may be corrupt: log and exit
    logger.error({ err }); shutdown(1);
  });
  ```

- On SIGTERM: stop accepting connections, finish in-flight requests (cap at 10s),
  close DB pools, then exit. Kubernetes sends SIGTERM, waits the grace period
  (default 30s), then SIGKILLs mid-request if you ignored it.

## File System

- Use `fs/promises` for small files; never the sync APIs (`readFileSync`) in
  request handlers - they block the loop by definition.
- Stream anything over ~1MB and prefer `pipeline` from `stream/promises`: unlike
  `.pipe()`, it propagates errors from every stage and destroys the chain:
  `await pipeline(fs.createReadStream(src), createGzip(), fs.createWriteStream(dst))`.
- `fs.watch` is not recursive everywhere and coalesces events; debounce and
  verify with `stat` - one editor save commonly emits 2-5 events.

## Common Pitfalls

- Treating `setTimeout(0)` as "after the current tick" - both microtasks and
  `setImmediate` beat it.
- `JSON.parse` on request bodies with no size cap: a 200MB body blocks the loop
  for seconds. Cap the body and reject early.
- `cluster` without sticky sessions, then WebSocket handoffs fail for every
  second connection.
- Bare `.pipe()` with no error listeners on either stream: one `error` event
  takes the process down. Use `pipeline`.
- String-splitting chunked binary data and corrupting multi-byte characters.

## Definition of Done

- [ ] No request handler contains sync CPU work over ~10ms or sync fs calls.
- [ ] Large payloads stream with backpressure; no unbounded buffering anywhere.
- [ ] Exactly one committed lockfile; CI installs with `npm ci` or equivalent.
- [ ] `unhandledRejection` and `uncaughtException` handlers log and exit nonzero.
- [ ] Config validated once at startup from layered env; secrets not committed.
- [ ] SIGTERM drains in-flight work and closes pools within the grace period.
