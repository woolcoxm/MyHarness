---
name: golang
description: Use whenever writing, reviewing, or debugging Go — goroutine and channel design, select with timeouts and cancellation, interface sizing and type switches, error wrapping with %w and errors.Is/As, context plumbing, generics constraints, table-driven tests, or go.mod/go.sum module questions. Covers worker pools, fan-in/fan-out pipelines, and nil-map, slice-aliasing, loop-capture pitfalls so concurrent code stays idiomatic and race-detector clean.
---

# Go: Concurrency-First Development

Go's motto is share memory by communicating: channels, goroutines, and context are the core of the language — master them and concurrency problems become composition problems. Write code that looks like Go wrote it: small interfaces, explicit errors, no cleverness.

## Goroutines and Channels

A goroutine costs a few KB, so spawn freely — but every goroutine needs a visible exit path, or unbounded spawning becomes a memory leak.

```go
ch := make(chan int)      // unbuffered: send blocks until a receiver is ready
buf := make(chan int, 10) // buffered: sends block only when full
for v := range ch {       // receives until the channel is closed
    process(v)
}
```

Only the sender closes a channel — a receiver closing it races with sends and panics with "send on closed channel". Closing broadcasts "no more values" to every receiver. A nil channel blocks forever, which is a feature: set a channel variable to nil inside `select` to disable that branch.

## Select

```go
select {
case v := <-results:                // first ready channel wins
    process(v)
case <-ctx.Done():                  // cancellation
    return ctx.Err()
case <-time.After(5 * time.Second): // per-operation timeout
    return ErrTimeout
default:                            // non-blocking: taken when nothing is ready
    return ErrWouldBlock
}
```

`default` exists for polling and try-send without spawning extra goroutines. In loops prefer `context.WithTimeout` over `time.After`: each `time.After` call allocates a timer that is not collected until it fires.

## Interfaces

Implementation is implicit — no `implements` keyword. Keep interfaces to one or two methods and declare them where they are consumed; that keeps packages decoupled and testable with fakes. Compose bigger interfaces from small ones (`io.ReadWriteCloser`). `any` erases type safety — use it only at true boundaries like `fmt` and `json`. Use the two-value assertion `v, ok := x.(T)` (the single-value form panics on mismatch); use a type switch when several types are possible.

```go
type Storer interface {
    Get(ctx context.Context, key string) ([]byte, error)
}
```

## Error Handling

Wrap with `%w` so callers can inspect causes; check with `errors.Is` (identity, sentinels) and `errors.As` (extract a typed cause). Combine multiple failures with `errors.Join`.

```go
if err := load(path); err != nil {
    return fmt.Errorf("read config %q: %w", path, err) // wrap, don't discard
}
if errors.Is(err, fs.ErrNotExist) { /* missing file */ }
var ae *net.AddrError
if errors.As(err, &ae) { /* inspect the address error */ }
```

Sentinel errors (`var ErrNotFound = errors.New("not found")`) work with `errors.Is`; custom error types implement `error` and are found by `errors.As`. Comparing with `==` only works when you know there is no wrapping.

## Structs and Methods

Value receivers copy the struct; pointer receivers mutate it and share one instance. If any method needs a pointer receiver, use pointer receivers for the whole type. Method sets: `T` methods live on `T` and `*T`; `*T` methods live only on `*T` — that decides interface satisfaction. Embedding (`struct { *Base }`) forwards methods but is not inheritance: the forwarded method sees the embedded value, not the outer struct, so there is no override dispatch. Prefer a named field plus explicit delegation unless pure forwarding is exactly what you mean.

## Generics

```go
func Map[T, U any](in []T, f func(T) U) []U {
    out := make([]U, len(in))
    for i, v := range in {
        out[i] = f(v)
    }
    return out
}

func Sum[T constraints.Integer | constraints.Float](xs []T) T { // constrained
    var sum T
    for _, x := range xs {
        sum += x
    }
    return sum
}
```

Use generics when the algorithm is identical across types (containers, algorithms); use interfaces when behavior differs per implementation. Do not add type parameters to code that handles exactly one type.

## Context

`context.Context` carries cancellation and deadlines. Pass it as the first parameter named `ctx`, never store it in a struct, and never pass `context.Background()` from a request deep into a call stack.

```go
ctx, cancel := context.WithTimeout(ctx, 2*time.Second)
defer cancel() // releases the timer immediately; skipping leaks it
result, err := fetch(ctx)
```

Use `WithCancel` for manual stop signals, `WithTimeout` for a duration, `WithDeadline` for an absolute time. Check `ctx.Err()` in long loops; any IO call worth writing accepts a ctx and aborts when Done fires.

## Concurrency Patterns

```go
// Worker pool: bound concurrency with a buffered-channel semaphore
sem := make(chan struct{}, concurrency)
var wg sync.WaitGroup
for _, job := range jobs {
    wg.Add(1)
    sem <- struct{}{} // acquire a slot
    go func(j Job) {
        defer wg.Done()
        defer func() { <-sem }() // release the slot
        process(ctx, j)
    }(job)
}
wg.Wait()
```

Fan-out: many goroutines consume one jobs channel. Fan-in: many producers feed one collector. Pipelines: each stage owns its output channel and closes it when its inputs are drained. Whatever the shape, verify with `go test -race`.

## Module System

`go.mod` declares the module path, Go version, and dependencies; `go.sum` pins content hashes — commit both. `go mod tidy` drops unused requirements and fetches missing ones. A major bump from v2 on must change the module path (`example.com/tool/v2`). `go mod vendor` creates a hermetic `vendor/` tree that builds with `-mod=vendor`.

## Testing

```go
func TestParse(t *testing.T) {
    tests := []struct {
        name, in string
        want     int
        wantErr  bool
    }{
        {"decimal", "10", 10, false},
        {"empty", "", 0, true},
    }
    for _, tt := range tests {
        t.Run(tt.name, func(t *testing.T) { // subtests run and filter alone
            got, err := Parse(tt.in)
            if tt.wantErr != (err != nil) {
                t.Fatalf("Parse(%q) err = %v, wantErr %v", tt.in, err, tt.wantErr)
            }
            if got != tt.want {
                t.Errorf("Parse(%q) = %d, want %d", tt.in, got, tt.want)
            }
        })
    }
}
```

Benchmarks use `func BenchmarkX(b *testing.B)` with `b.N`; run `go test -bench=. -benchmem`. Mock by defining a small interface at the consumer and writing a fake — no mock framework needed. Test helpers take `t *testing.T` and call `t.Helper()` so failures point at the caller.

## Common Pitfalls

- Writing to a nil map panics: `make(map[string]int)` before assignment; reads from a nil map are legal and return zero values.
- Loop variable capture (pre-1.22): every closure in the loop shared one variable. Pass the value as an argument (`go func(j Job) { ... }(job)`) or declare `go 1.22`+ in go.mod for per-iteration variables.
- Slice aliasing: `append` may return a slice sharing the backing array, so two "copies" mutate each other. Copy explicitly with `copy` or a full slice expression when independence matters.
- `:=` shadowing: `if err := f(); err != nil` inside a block hides an outer `err`; go vet and careful review catch assignments that discard values you meant to check.
- Closing a channel twice, or sending after close, panics — senders close, exactly once, after the last send.
- Ignoring `ctx.Done()` in long loops makes deadlines and cancellation toothless.

## Definition of Done

- [ ] `go build ./...`, `go vet ./...`, and `go test -race ./...` all pass
- [ ] Every blocking call takes a ctx and aborts on `ctx.Done()`
- [ ] Wrapped errors use `%w`; causes checked with `errors.Is`/`errors.As`
- [ ] Every goroutine has a visible exit; channels closed only by senders
- [ ] Interfaces declared at the consumer, one or two methods each
- [ ] `go.mod`/`go.sum` tidy and committed; `gofmt -l` prints nothing
