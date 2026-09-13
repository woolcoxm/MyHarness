---
name: design-patterns
description: Apply the right design pattern to a real problem — creational, structural, behavioral, functional, and concurrency patterns, plus selection guidance and anti-patterns — whenever structuring new code or refactoring existing structure.
---

# Design Patterns: Select by Problem, Not by Catalog

A pattern is a named solution to a recurring problem. Use one when you have the
problem it names — the test is "what concrete pain does this remove today?", never
"this code feels too simple". Reach for patterns when flexibility is demonstrably
needed, and prefer the simplest construct that survives the next likely change.

## Creational Patterns

### Factory Method vs Abstract Factory vs Builder

| Pattern | Problem it solves | Signal to use |
|---|---|---|
| Factory method | Creation varies by a single axis | One `match` on type hiding a constructor |
| Abstract factory | A *family* of related objects must stay consistent | Two+ products that must match (theme: button+menu) |
| Builder | Telescoping constructors / multi-step assembly | 4+ constructor params or optional-heavy construction |

Factory method — worth it once creation logic is more than `Struct::new`:

```rust
fn parser_for(fmt: &Format) -> Box<dyn Parser> {
    match fmt {
        Format::Json => Box::new(JsonParser),
        Format::Yaml => Box::new(YamlParser),
    }
}
```

Abstract factory — only when products must pair up; otherwise it is ceremony:

```rust
trait UiFactory {
    fn button(&self) -> Box<dyn Button>;
    fn menu(&self) -> Box<dyn Menu>;   // must belong to the same theme as button
}
```

Builder — pays for itself the moment a second optional field appears:

```rust
let server = Server::builder()
    .port(8080)
    .tls_cert("cert.pem")        // optional
    .max_connections(256)        // optional
    .build()?;
```

Do not introduce a factory for one implementation "for later" — add it at the second
implementation; modern IDEs make that extraction cheap.

## Structural Patterns

### Adapter vs Facade vs Decorator vs Proxy

These get confused constantly. Separate them by intent:

| Pattern | Intent | Tells itself apart by |
|---|---|---|
| Adapter | Make an incompatible interface fit the one callers expect | Fixed target trait; the adaptee cannot change |
| Facade | Give a complicated subsystem one simple front door | *Simplifying*, not converting — callers see a subset |
| Decorator | Add responsibilities dynamically, same interface | Wrapping is transparent; caller cannot tell it apart |
| Proxy | Control *access* (lazy, guarded, remote), same interface | Wrapping is about interception, not added behavior |

```rust
// Adapter: legacy logger now implements the new trait — translate, do not simplify
trait Log { fn write(&self, msg: &str); }
struct LegacyAdapter(LegacyLog);
impl Log for LegacyAdapter { fn write(&self, msg: &str) { self.0.log_message(msg) } }

// Decorator: same interface, adds behavior, chainable
struct Timestamped<L: Log>(L);
impl<L: Log> Log for Timestamped<L> {
    fn write(&self, msg: &str) { self.0.write(&format!("[{now}] {msg}")) }
}

// Proxy: same interface, controls access (connect on first use)
struct LazyDb(Option<Connection>);
```

Rule of thumb: converting = adapter, simplifying = facade, stacking behavior = decorator, intercepting access = proxy.

## Behavioral Patterns

### Strategy vs Command vs Observer vs Mediator

| Pattern | Real-world scenario |
|---|---|
| Strategy | Shipping cost varies by carrier; swap algorithms behind one interface |
| Command | Undo/redo, job queues — an *operation* becomes a value you store and replay |
| Observer | "Tell me when the order status changes" — one event, unknown listeners |
| Mediator | Chat room, UI event bus — peers talk through a hub, not to each other |

```rust
// Strategy: pick pricing at runtime, hot-swappable (Standard, MemberDiscount{pct}, ...)
trait Pricing { fn total(&self, cart: &Cart) -> Cents; }

// Command: the operation is data — queue it, serialize it, undo it
trait Command { fn execute(&self, doc: &mut Doc); fn undo(&self, doc: &mut Doc); }
struct InsertText { pos: usize, text: String }

// Observer: subject stays ignorant of listeners
emitter.on("order.paid", |e| email.send(&e));
emitter.on("order.paid", |e| warehouse.reserve(&e));
```

Use mediator when peer-to-peer references have grown into a maze (A calls B calls
A...). It centralizes interaction at god-object risk — keep the mediator a router, not a brain.

## Functional Patterns

Reach for these before the OO catalog — they are usually cheaper.

- **Maybe/Result** — make absence and failure part of the type instead of null checks
  and exceptions; the compiler then enforces handling.
- **Pipelines** — replace nested transforms with a left-to-right chain; each step independently testable.

```rust
let total = cart.lines()
    .iter()
    .filter(|l| !l.removed)
    .map(|l| l.unit_price * l.qty)
    .sum::<Cents>();
```

- **Partial application** — fix arguments now, finish later; kills the boolean-flag parameter (`fetch(url, true, false)`) by producing named intermediate functions.
- **Lazy evaluation** — build a computation now, run it only if needed; right for expensive defaults that are often skipped, not a general style.

```ts
const withAuth = (token: string) => (url: string) => fetch(url, { headers: { token } });
const adminFetch = withAuth(ADMIN_TOKEN);   // partial application
```

## Concurrency Patterns

| Pattern | Use when |
|---|---|
| Producer-consumer | Arrival rate differs from processing rate; a bounded queue decouples them |
| Worker pool | Parallelize homogeneous tasks under a fixed cost cap (N workers, M jobs) |
| Reader-writer lock | Reads dominate writes on shared state; readers coexist, writers exclude |
| Actor model | State must stay isolated per entity; messages are the only interface |

```rust
// Producer-consumer: bounded channel applies natural backpressure
let (tx, rx) = mpsc::channel(1024);
for _ in 0..workers {
    let rx = rx.clone();
    spawn(async move { while let Some(job) = rx.recv().await { process(job).await } });
}
```

Prefer bounded queues: an unbounded queue converts overload into memory exhaustion.
The reader-writer lock is not a free upgrade over a mutex — under write-heavy loads it is slower and can starve readers or writers. Measure.

## Anti-Patterns

- **God object** — one type accumulating every responsibility. Split along change axes: things that change together stay together.
- **Spaghetti code** — control flow with no discernible structure. Impose layers: parse -> validate -> compute -> persist, each independently testable.
- **Golden hammer** — one pattern applied everywhere (everything is a singleton, everything is an event). Re-read the selection table below.
- **Premature optimization** — optimizing without a profile. Measure first; the bottleneck is never where you guess.
- **Singleton abuse** — hidden global state, untestable, lifetime lies. Pass the dependency explicitly; constructor injection is honest state.

## When NOT to Use Patterns

- The pattern must solve a problem you *have*, not a hypothetical future one. "We might need multiple implementations" is one extraction away — add the pattern at the second implementation.
- Indirection with one implementation and no second caller is worse than concrete code: it costs a file, a trait, and a hop of comprehension.
- If removing the pattern changes no caller and enables no test, it was decoration.
- Do not pattern-match on code-smell anxiety ("no interfaces yet"). Match on friction.

## Pattern Selection Guide

| Problem | Reach for |
|---|---|
| Construction varies by type/axis | Factory method |
| A family of related objects must match | Abstract factory |
| Many optional constructor params | Builder (or plain struct with defaults) |
| Interface mismatch you cannot fix in the adaptee | Adapter |
| Subsystem too complicated for callers | Facade |
| Add behavior in stackable layers | Decorator |
| Lazy/guarded/remote access, same interface | Proxy |
| Swap algorithm at runtime | Strategy (or a plain function value) |
| Operations as storable/undoable values | Command |
| Notify unknown numbers of listeners | Observer (event bus = mediator) |
| Peers tangled in N-to-N calls | Mediator |
| Null checks everywhere / error paths invisible | Maybe/Result |
| Nested data transforms | Pipeline |
| Arrival rate != processing rate | Producer-consumer (bounded) |
| Shared state, mostly reads | Reader-writer lock (measure first) |
| Isolated state per entity, message-driven | Actor model |

## Common Pitfalls

- Pattern-driven design — choosing from the catalog first and bending the problem to fit; always start from the friction you are feeling.
- Strategy with one strategy — indirection with no payoff; delete it until the second implementation exists.
- Facade that leaks — the "simplified" front door still requires callers to know subsystem details; fix it or remove it.
- Observer chains — events triggering events triggering events; nobody can trace a flow. Cap the depth; use explicit calls for critical flows.
- Decorator stacks as architecture — six wrappers deep is invisible behavior; prefer composition with named steps.
- Singletons for convenience — every `getInstance()` is a hidden parameter that breaks parallel tests; inject instead.
- Copy-pasted pattern boilerplate — if the shape survives but the intent does not (a Builder that requires every field), you added ceremony, not a pattern.
