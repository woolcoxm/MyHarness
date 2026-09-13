---
name: javascript-modern
description: Load when writing or reviewing JavaScript to apply destructuring, spread, async combinator patterns, modules, iterables, closures, event-loop reasoning, and structured error handling.
---

# Modern JavaScript

Use these patterns to keep JS terse without becoming clever. Prefer the
construct that states intent: combinators for concurrency, destructuring
for shape, custom errors for failure taxonomy.

## Destructuring

```js
const { name, email: contact, role = "viewer" } = user;
//                            ^ rename      ^ default
const [first, , third = 0] = results;   // skip elements positionally
function draw({ x = 0, y = 0, color = "black" } = {}) {} // named params
const { profile: { org = "personal" } = {} } = user;     // nested defaults
```

- Defaults apply only to `undefined` — `null` passes through. Normalize
  with `??` first when an API sends `null`.
- Destructure in function signatures for named, defaultable parameters;
  keep the `= {}` fallback so callers can omit the object entirely.
- Return objects, not arrays, so call sites can pick fields by name.

## Spread and Rest

```js
const settings = { ...defaults, ...userPrefs };    // merge, later wins
const unique = [...new Set(items)];                // dedupe
const next = { ...state, items: [...state.items, item] }; // immutable add
function log(tag, ...args) {}                      // rest: collect args
```

- Treat spread as immutable update syntax: copy the parent, replace the
  changed branch; never mix with deep mutation.
- Spread is one level deep — nested objects stay shared. Clone
  explicitly (`structuredClone(obj)`) when independence is required.

## Template Literals

```js
const msg = `Hello ${name}, your total is ${(qty * price).toFixed(2)}.`;
const ids = rows.map((r) => `row-${r.id}`).join(", ");
```

Use tagged templates when interpolation needs policy (escaping,
streaming); prefer library tag functions over casual hand-rolled ones.

## Async Patterns

`async/await` is syntax over promises; the combinators are the real
concurrency tools.

| Combinator | Resolves when | Rejects when | Use for |
|------------|---------------|--------------|---------|
| `Promise.all` | All fulfill | First rejection | Parallel loads where all must succeed |
| `Promise.allSettled` | All settle | Never | Independent best-effort work |
| `Promise.race` | First settles | First rejection | Timeouts, first answer wins |
| `Promise.any` | First fulfills | All reject (`AggregateError`) | Redundant sources, fastest success |

```js
// Parallel: independent calls — never await them one by one
const [user, orders] = await Promise.all([
  api.getUser(id),
  api.getOrders(id),
]);

// Sequential: only when step N+1 needs step N's result
const profile = await api.getUser(id);
const prefs = await api.getPrefs(profile.theme);

// Best-effort fan-out
const results = await Promise.allSettled(urls.map((u) => fetch(u)));
```

Handle errors with `try/catch` around the `await`, or `.catch` on the
composed promise. Every `await` lives in a function whose rejection is
handled here or deliberately propagated to a caller that handles it.

## Modules

```js
import { formatMoney } from "./money.js";   // named import
import Invoice, { calcTax } from "./invoice.js"; // default + named
export const RATE = 0.08;                   // named export
const heavy = await import("./chart.js");   // dynamic import:
heavy.openChart(el);                        // code-split point
```

- Use named exports as the default style; default exports rename
  silently at every import site and hurt greppability.
- Dynamic-import user-invisible code (charts, editors, modals) to
  shrink the initial bundle; load on first interaction, not hover.
- Keep modules side-effect free so bundlers can tree-shake; avoid
  barrel files that re-export the world.

## Iterables and Generators

```js
function* fib() {
  let [a, b] = [0, 1];
  while (true) { yield a; [a, b] = [b, a + b]; } // lazy, infinite
}
for (const n of fib()) { if (n > 1000) break; console.log(n); }
```

- `for...of` works on anything with `Symbol.iterator`: arrays, strings,
  Maps, Sets, NodeLists. Use it when indexes are not needed.
- Use generators instead of huge intermediate arrays for pipelines, and
  `yield*` to delegate to another iterable.
- Generators power `async function*` streams: `for await (const chunk
  of stream)`.

## Closures and Scope

A closure is a function plus the variables it captured from its
defining scope, alive as long as the function is.

```js
function makeCounter() {
  let count = 0;                 // captured, private
  return { inc: () => ++count, get: () => count };
}
```

- Use closures for private state instead of underscore conventions.
- Watch memory: a closure stored globally (listener, cache, interval)
  pins everything it captured, including detached DOM nodes. Clear the
  reference when done (`clearInterval`, `removeEventListener`).
- `let`/`const` are block-scoped; a `var` in a loop shares one binding
  across iterations, which is why loop callbacks want `let`.

## The Event Loop

JS runs one task at a time. Script, timers, and events queue as
macrotasks; promise callbacks queue as microtasks, which drain fully
after each task and before rendering.

```js
console.log("1: sync");
setTimeout(() => console.log("4: macrotask"), 0);
Promise.resolve().then(() => console.log("3: microtask"));
console.log("2: sync");
// Output: 1, 2, 3, 4
```

- `setTimeout(fn, 0)` means "next macrotask, after microtasks and often
  after rendering" — never "immediately".
- Long synchronous work blocks both queues and the render: the page
  freezes. Chunk big loops or move them to a Worker.
- Infinite microtask chains starve rendering exactly like
  `while (true)`.

## Maps, Sets, WeakMap, WeakSet

| Structure | Reach for it when |
|-----------|-------------------|
| `Map` | Non-string keys, frequent add/delete, insertion order, `.size` |
| `Set` | Unique values, O(1) membership, union/intersection |
| `WeakMap` | Per-object metadata that should die with its key |
| `WeakSet` | Marking objects (visited, subscribed) without blocking GC |

```js
const seen = new WeakSet();
function process(node) {
  if (seen.has(node)) return;
  seen.add(node);
}
```

Prefer `Map` over plain objects as dictionaries: no prototype
pollution, direct key iteration, no key-coercion surprises.

## Optional Chaining and Nullish Coalescing

```js
const city = user?.address?.city;   // undefined instead of TypeError
const port = config.port ?? 3000;   // 0 and "" are kept!
fn?.(arg);                          // call only if fn exists
```

Use `??` for defaults, never `||`, whenever `0`, `""`, `false`, or
`NaN` are legitimate values. Do not chain ten `?.`s as armor — one
guard plus validation at the boundary reads better and fails louder.

## Error Handling

```js
class HttpError extends Error {
  constructor(status, message, options) {
    super(message, options);        // options.cause preserves the chain
    this.name = "HttpError";
    this.status = status;
  }
}

try {
  const res = await fetch(url);
  if (!res.ok) throw new HttpError(res.status, `GET ${url} failed`);
} catch (err) {
  throw new Error("Failed to load dashboard", { cause: err });
}
```

- Subclass `Error` per failure domain and set `name`; catch sites then
  branch on type, not message strings.
- Always forward `cause` when wrapping so logs keep the original stack.
- Attach structured data to errors instead of parsing their messages.

## Common Pitfalls

- `await` inside `forEach` does not wait: use `for...of` for
  sequential, `Promise.all` for parallel.
- Floating promises reject later as unhandled rejections: await them
  or attach `.catch`.
- Shallow spread assumed deep: nested objects are still shared.
- `||` for defaults clobbers `0`/`""`: use `??`.
- `typeof null === "object"`: check for `null` explicitly.
- Returning from `catch` or `finally` swallows the in-flight error.
