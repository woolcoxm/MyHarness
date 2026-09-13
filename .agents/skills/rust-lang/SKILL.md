---
name: rust-lang
description: Use whenever writing, reviewing, or debugging Rust — choosing smart pointers, designing traits and generics, fixing borrow-checker or lifetime errors, structuring error handling with Result and ?, writing async tokio code, or running cargo check/clippy/test. Covers ownership reasoning, Send/Sync concurrency, collections, modules and workspaces, and feature-flag conditional compilation so changes compile clean and clippy-clean the first time.
---

# Rust: Ownership, Traits, and Safe Concurrency

Rust moves entire bug classes — data races, use-after-free, null dereferences — from production incidents to compile errors. Treat every fight with the compiler as a bug found before shipping, not as pedantry to work around.

## Ownership: The Mental Model

Every value has exactly one owner; assignment transfers ownership by move.

```rust
let s1 = String::from("hi");
let s2 = s1;          // s1 moved: using s1 below is a compile error
let s3 = s2.clone();  // explicit deep copy — only when you need both
```

Borrow rules: any number of `&T` readers OR exactly one `&mut T` writer, never both at once. WHY: aliased mutation is the definition of a data race and of iterator invalidation; the borrow checker is a compile-time aliasing proof. When it rejects code, restructure — narrow the mutable borrow's scope, split the function, return owned data — before reflexively adding `clone()`.

## Lifetimes

Lifetimes do not keep data alive; they let the compiler prove no reference outlives its owner. Annotate only when elision cannot infer the input→output relationship (elision: a sole input reference propagates to the output; `&self` methods propagate `&self`).

```rust
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str { // ties output to inputs
    if x.len() > y.len() { x } else { y }
}
```

`'static` means the reference is valid for the whole program (string literals, `Box::leak`) — it is rarely the fix for a lifetime error. Usually you want a generic `'a`, or an owned `String`/`Cow<'a, str>` instead of a borrow.

## Traits

Traits define shared behavior. Dispatch choice has real tradeoffs: `fn f<T: Trait>(x: T)` monomorphizes — one specialization per concrete type, inlinable and fast, at the cost of bigger binaries and compile times. `fn f(x: &dyn Trait)` goes through a vtable — one copy, runtime dispatch, and the only option for heterogeneous `Vec<Box<dyn Trait>>` or plugin-style boundaries.

Prefer associated types when the trait admits one output per type (`Iterator::Item`); use generic parameters when multiple implementations must coexist (`From<T>`, `Add<Rhs>`). Move complex bounds into `where` clauses for readability.

## Error Handling

Return `Result<T, E>` throughout and chain with `?`. Production code never calls `unwrap()` on a fallible path — a panic is a crash, and crashing is a policy decision made elsewhere. Use `thiserror` for libraries (a small derived enum; `#[from]` gives callers `?` for free) and `anyhow` for applications (type-erased errors plus `.context("...")`).

```rust
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("bad header: {0}")]
    Header(String),
    #[error(transparent)]
    Io(#[from] std::io::Error), // io::Error converts via ? for free
}
```

Error strings are API: state what failed and what to do next.

## Smart Pointers

| Type | Use when | Cost |
|---|---|---|
| `Box<T>` | Heap data, recursive types, huge values | single owner |
| `Rc<T>` | Shared ownership, single-threaded | not `Send`/`Sync` |
| `Arc<T>` | Shared ownership across threads | atomic refcounts |
| `Mutex<T>` | Shared mutation across threads | contention, poisoning |
| `RwLock<T>` | Many readers, rare writers | heavier than `Mutex` |

`Deref` makes `&Box<T>` coerce to `&T`, so smart pointers stay mostly transparent. Interior mutability (`RefCell`, `Mutex`) moves borrow checking to runtime — a double borrow panics instead of failing to compile.

## Async Rust

Use tokio unless something forbids it. An `async fn` returns a future; nothing runs until awaited or spawned.

```rust
let handle: tokio::task::JoinHandle<u32> = tokio::spawn(async move {
    fetch().await // spawned futures must be Send + 'static: own your data
});
tokio::select! { // first branch to finish wins; the rest are dropped
    v = handle => v,
    _ = token.cancelled() => return Err(Cancelled),
}
```

Spawning requires `Send + 'static` because the task can outlive the caller and hop threads — that is why borrowed data inside spawned tasks fails to compile. Cancellation drops the future at its next `.await` point: make loops check a `CancellationToken`, and put cleanup in `Drop` guards, because "code after the loop" never runs on cancel. Make `select!` branches cancel-safe — re-running them after cancellation must be harmless.

## Collections

| Collection | Ordering | Sweet spot |
|---|---|---|
| `Vec<T>` | insertion | default; contiguous, cache-friendly |
| `VecDeque<T>` | queue | O(1) push/pop at both ends |
| `HashMap<K,V>` | none | O(1) key lookup |
| `BTreeMap<K,V>` | sorted | range queries, deterministic iteration |

Choose `BTreeMap` when iteration order must be deterministic — snapshots, golden-file tests, anything serialized and diffed across runs.

## Modules and Crates

The module tree mirrors the filesystem; `mod` declares, `use` imports. Visibility is private by default: `pub(crate)` for internal APIs, `pub` only at real boundaries. Re-export to present a flat API: `pub use self::error::Result;`. Prefer one workspace over a nested-repo mess, and pin shared dependency versions in `[workspace.dependencies]` so crates never mismatch.

```toml
[workspace]
members = ["crates/core", "crates/cli"]
```

## Concurrency

`Send` = safe to move across threads; `Sync` = safe to share `&T` across threads. Both are auto-derived from fields — if a type isn't `Send`, it holds something that isn't. `Mutex` poisoning records "a holder panicked while holding the lock": `.lock()` returns `Result`, and unwrapping it is a policy choice, acceptable when a poisoned lock means shared state is untrustworthy anyway. Channels: `std::sync::mpsc` for multi-producer/single-consumer; `crossbeam-channel` for MPMC, `select!`, and throughput.

## Cargo Workflow

```bash
cargo check                 # fastest loop: type-check, no codegen
cargo clippy --all-targets  # lints; must stay warning-free
cargo test                  # unit + integration; doc tests run too
cargo bench                 # criterion-based perf harness
cargo doc --open            # rustdoc; examples are compiled and tested
```

Gate optional code with features and platforms with `cfg`. WHY cfg-gate: platform-specific code still has to compile on every platform, so Linux-only code must typecheck on the Windows build.

```toml
[features]
landlock = ["dep:landlock"]
```

```rust
#[cfg(feature = "landlock")]
#[cfg(not(windows))]
fn sandbox() { /* ... */ }
```

## Common Pitfalls

- Borrowing in a loop: collect into a `Vec` first, mutate after — do not interleave a `&mut` with reads over the same collection.
- Partial move: pulling a field by value kills the whole struct; use `std::mem::take(&mut self.field)` to move out while leaving a default.
- `Rc<RefCell<T>>` in threaded code: rejected by design — use `Arc<Mutex<T>>`.
- Blocking calls (sync IO, `std::thread::sleep`, CPU loops) inside async stall the executor; wrap them in `tokio::task::spawn_blocking`.
- Comparing a `MutexGuard<T>` to a `T`: dereference first (`*guard == value`).
- "Requires `'static`" errors from `tokio::spawn`: you moved a reference into the task — move owned data (`Arc`, `String`) instead.

## Definition of Done

- [ ] `cargo check` and `cargo clippy --all-targets` pass with zero warnings
- [ ] No `unwrap()`/`expect()` on fallible paths outside tests and lock poisons
- [ ] Errors are typed and actionable (what failed, what to do next)
- [ ] Async code is cancel-safe; blocking work uses `spawn_blocking`
- [ ] Shared state uses the narrowest sync primitive that works
- [ ] Public items documented; doc examples compile and run as tests
- [ ] Platform-specific code is cfg-gated and cross-compiles
