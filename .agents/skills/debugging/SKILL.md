---
name: debugging
description: Systematic debugging methodology - hypothesis-driven investigation, binary search isolation, strategic logging, and common bug pattern recognition across languages and environments.
---

# Debugging: A Systematic Method

Debugging is not guessing. It's hypothesis-driven investigation: form a hypothesis, design an experiment that can falsify it, run it, and narrow the search space by half each time. The model that "just tries things" wastes your time; the model that follows this method finds bugs in minutes.

## The Method

### Step 1: Reproduce reliably
Before anything else, get a deterministic reproduction. If it's intermittent:
- Add a loop: `for i in $(seq 1 100); do ./test; done` — does it fail every time?
- Check for race conditions: add sleep/timing variation
- Check environment: does it fail in CI but not locally? Docker but not bare metal?
- If you cannot reproduce it, you cannot fix it. Stop and make it reproducible first.

### Step 2: Understand the expected behavior
Write down what SHOULD happen. Not "it should work" — specifically: "given input X, function Y returns Z because [reasoning]." This is your hypothesis.

### Step 3: Bisect the search space
```
Where is the bug?
├── Input parsing?     → Test with known-good input, does the bug persist?
├── Business logic?    → Add a breakpoint/print at the boundary, inspect intermediate values
├── Output rendering?  → Check what the logic produced vs what was displayed
├── State mutation?    → Dump state before/after each mutation
└── Concurrency?       → Single-thread it; does the bug disappear?
```

Each test should eliminate ~50% of the possible causes. If you've done 3 tests and haven't narrowed meaningfully, you're testing the wrong things.

### Step 4: Isolate to a minimal test case
Strip everything non-essential until the bug reproduces in the smallest possible snippet. This often reveals the actual cause (a type coercion, an off-by-one, a nil that propagated).

### Step 5: Fix, then verify the fix doesn't break anything else
- Write a test that reproduces the bug (it should fail before the fix, pass after)
- Run the full test suite
- Check for the same pattern elsewhere: `grep -rn "similar_pattern" src/`

## Strategic Logging

Don't scatter prints randomly. Place them at boundaries:

```python
# BAD: random prints
print("here")
print(x)
print("after")

# GOOD: boundary logging with context
print(f"BEFORE transform: input={input!r}, type={type(input).__name__}")
result = transform(input)
print(f"AFTER transform: result={result!r}, expected={expected!r}")
```

Log these four things at every boundary:
1. **Input** (what came in, full representation)
2. **Output** (what came out)  
3. **Expected** (what you thought would come out)
4. **Delta** (how they differ — if input == expected_input but output != expected_output, the bug is between these two points)

## Binary Search with Git

When a bug appeared "sometime in the last N commits":

```bash
git bisect start
git bisect bad  # current HEAD is broken
git bisect good v1.2.0  # this version worked
# git checks out the midpoint; test it
cargo test  # or whatever your test command is
git bisect good  # or git bisect bad
# repeat ~log2(N) times
git bisect reset
```

This finds the exact commit in O(log N) tests instead of O(N).

## Common Bug Patterns (check these first)

### Off-by-one
```python
# Suspicious: range boundaries
for i in range(len(items)):      # OK but prefer enumerate
for i in range(1, len(items)):   # skipped index 0?
items[len(items)]                 # out of bounds (Python: IndexError, JS: undefined)
items[-1] == items[len(items)-1]  # these are the same; know which you mean
```

### Type coercion (JavaScript)
```javascript
"1" + 1 === "11"  // string concatenation, not addition
1 == "1"          // true (loose equality coerces)
1 === "1"         // false (strict equality — always use ===)
[] + {}           // "[object Object]"
NaN !== NaN       // true — use Number.isNaN(), not isNaN()
```

### Null/undefined propagation
The bug is rarely where the null appears; it's where it was *created*. Trace backward:
- Where was this value set? (constructor, function return, API response, DB query)
- What condition allowed null to reach here?
- Fix the source, not the symptom. Adding `if (x === null) return` at 12 call sites is treating symptoms.

### State mutation you didn't expect
```python
# Classic: mutable default argument
def add_item(item, items=[]):  # BUG: the list persists between calls!
    items.append(item)
    return items

add_item("a")  # ["a"]
add_item("b")  # ["a", "b"] — expected ["b"]
```

### Race conditions
Symptoms: works single-threaded, fails under load; intermittent; timing-dependent.
Quick check: add `time.sleep(0.1)` between operations — does the bug disappear? If yes, you have a race. Fix with proper locking/atomics, not more sleeps.

### Async/await forgotten
```javascript
// Bug: returns a Promise, not the value
function getUser() {
    return fetchUser();  // forgot await
}
const user = getUser();  // user is a Promise
user.name  // undefined
```

## Debugging Tools by Language

| Language | Breakpoint | Stack trace | Profiler |
|---|---|---|---|
| Rust | `rust-gdb`/`rust-lldb` | `RUST_BACKTRACE=1` | `cargo flamegraph` |
| Python | `breakpoint()` / pdb | `traceback.print_exc()` | `cProfile`, `line_profiler` |
| JavaScript | `debugger` / DevTools | `Error().stack` | Chrome Performance tab |
| Go | `dlv debug` | `panic()` stack | `pprof` |
| Java | IDE debugger / `jdb` | `printStackTrace()` | `jprofiler`, `async-profiler` |

## When You're Stuck

1. **Rubber-duck it**: explain the expected behavior line by line. The act of explaining often reveals the wrong assumption.
2. **Check the environment**: env vars, config files, feature flags, database state — is the code running against what you think it is?
3. **Read the error message**: actually read it. What file, what line, what type, what value?
4. **Check the version**: is the library version what you think? `pip show`, `npm ls`, `cargo tree`
5. **Simplify the data**: does the bug happen with simple input? If not, the bug is in the complex-data handling path.
