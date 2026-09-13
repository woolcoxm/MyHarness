---
name: testing-strategy
description: Decide what to test and how - test pyramid ratios, TDD workflow, property-based testing, test double selection, flaky test elimination, and coverage interpretation for any codebase.
---

# Testing Strategy

Tests are a design tool first and a regression net second. Write the tests that catch real bugs and skip the ones that only assert what the compiler already guarantees.

## The Test Pyramid

Aim for this ratio by count of tests:

```
        /  E2E  \        ~5%   slow, brittle, catch integration gaps
       / Integration \   ~15%  real collaborators, fake boundaries
      /    Unit      \   ~80%  fast, isolated, run in milliseconds
```

| Layer | Scope | Doubles | Runtime budget | What it proves |
|---|---|---|---|---|
| Unit | One function or type | Most collaborators doubled | < 10 ms each | Logic is correct |
| Integration | A subsystem (module + DB, HTTP handler + router) | Only external boundaries faked | < 1 s each | Parts work together |
| E2E | Whole system through the user entry point | None | Seconds | The user flow works |

Invert the pyramid only for thin glue code over heavy frameworks (most CRUD web apps end up hourglass-shaped: many unit tests, few integration, a handful of E2E smoke tests). Never make E2E the biggest layer - it is the slowest to run and the flakiest to maintain.

## What to Test vs What Not to Test

| Do test | Do not test |
|---|---|
| Business logic and branching (pricing, permissions, parsing) | Getters/setters, struct construction |
| Boundary conditions (empty, one, many, max, off-by-one) | Generated code, framework internals |
| Error paths (bad input, network failure, timeout) | Third-party libraries (their job) |
| Regression tests for every fixed bug | Trivial configuration wiring |
| Concurrency invariants if the code is concurrent | "It compiles" or "it serializes" |

Also test behavior, not implementation. A test that calls `calculate_discount` and asserts the price tests behavior and survives refactors. A test that inspects private state or counts function calls tests implementation and breaks on every refactor.

## TDD: Red-Green-Refactor

1. **Red**: write the smallest failing test that describes the next bit of behavior.
2. **Green**: write the minimum code to pass it. Ugly code is fine at this step.
3. **Refactor**: clean up test and production code with the suite green, then repeat.

```rust
// Red: fails because validate_username does not exist yet
#[test]
fn rejects_username_shorter_than_three_chars() {
    assert!(validate_username("ab").is_err());
    assert!(validate_username("abc").is_ok());
}
```

Use TDD for pure logic and bug fixes (write the failing repro test first, always). Skip strict TDD for exploratory code, throwaway scripts, and UI layout - there is no behavior to pin yet.

## Property-Based Testing

Instead of enumerating examples, state an invariant and let a generator (QuickCheck, Hypothesis, proptest) attack it.

Good properties: round-trips (`decode(encode(x)) == x`), idempotence (`f(f(x)) == f(x)`), ordering (sorted output is a permutation of input), invariants (total never goes negative), oracle (your result equals a slow-but-obvious implementation).

```python
from hypothesis import given, strategies as st

@given(st.lists(st.integers()))
def test_sort_is_permutation_and_ordered(xs):
    result = sorted(xs)
    assert sorted(result) == sorted(xs)        # permutation
    assert all(a <= b for a, b in zip(result, result[1:]))  # ordered
```

When a property fails, the shrinker produces a minimal counterexample. Promote that counterexample into a named unit test so it stays pinned forever.

## Test Doubles

| Double | Behavior | Use when |
|---|---|---|
| Stub | Returns canned data | SUT needs an input, not interaction |
| Fake | Working lightweight impl (in-memory repo) | Tests need realistic behavior across calls |
| Mock | Fails if expected calls do not happen | The interaction IS the requirement |
| Spy | Records calls, asserts afterward | Verify a side effect happened, optionally |

```python
# Stub: fills in an answer
clock = StubClock(now=datetime(2026, 1, 1))

# Mock: the call itself is the contract
mailer = Mock()
activate_account(user, mailer)
mailer.send_welcome.assert_called_once_with(user.email)
```

Prefer stubs and fakes; reach for mocks only when the protocol is the point (cache invalidation, event publishing). A test riddled with mock setup is coupling to implementation - replace mocks with a fake or restructure the code.

## Test Naming

Name tests after the behavior in the domain language, not the method name:

```rust
// BAD: fn test_parse_2
// GOOD:
#[test]
fn parse_rejects_input_with_trailing_garbage()
```

Pattern: `unit_condition_expected_result` or a sentence readable by a non-programmer ("transfer fails when balance is insufficient"). In a failure listing, the name alone must tell you what broke.

## Flaky Test Elimination

A flaky test is worse than no test - it trains the team to ignore red builds.

1. Quarantine it immediately (skip + issue), do not leave it failing randomly in the suite.
2. Find the nondeterminism source: shared mutable state between tests, wall-clock time, random seeds, network, filesystem, thread scheduling, ordering dependence.
3. Fix the source: inject the clock, seed the RNG explicitly, give each test isolated state, await explicit conditions instead of sleeps.
4. Verify by looping: `for i in $(seq 1 200); do cargo test suspect_test || break; done`.

Never fix flakiness with retries or longer sleeps. Retries hide races; sleeps slow the suite and still race.

## Coverage Metrics

Line/branch coverage is a floor, not a goal: it counts executed lines, not asserted behavior. A test with zero assertions counts as covered.

- Meaningful: coverage of critical modules (payment, auth, parsing) trended over time; coverage dropped by a diff.
- Not meaningful: a single project-wide percentage target, 100% chased with vacuous tests.

Treat mutation testing (mutmut, Stryker, cargo-mutants) as the honest metric - it kills a mutant only if a test would actually fail on a real logic change.

## Testing Async Code

Do not sleep for results; await the completion signal. Every async test needs a runtime and a timeout.

```rust
#[tokio::test]
async fn debounce_emits_only_after_quiet_period() {
    let mut debounced = Debounce::new(Duration::from_millis(50));
    debounced.push(1);
    debounced.push(2);
    let out = tokio::time::timeout(Duration::from_secs(1), debounced.next()).await;
    assert_eq!(out.unwrap(), vec![1, 2]);  // coalesced, not two events
}
```

For timers, run on a mocked clock (`tokio::time::pause`) so a 30-second retry policy tests in milliseconds. For concurrency, test the invariant directly: run N concurrent operations against one shared resource and assert the final state, plus assert no deadlock under a test timeout.

## Snapshot Testing

Snapshot tests (golden files, inline snapshots, approval tests) shine for outputs where a human must eyeball shape: rendered CLI output, error message text, serialized config, generated code.

They lie when the snapshot is a full deep dump of a structure you do not control - the test "passes after regenerating," and regeneration becomes a habit that approves anything. Rules:

- Review every snapshot diff like a code review; never bulk-accept `--update` output.
- Keep snapshots minimal: assert the fields that matter, not the entire blob.
- If a snapshot fails and you cannot explain each diff line from memory, do not update it blindly.

## Common Pitfalls

- **Testing the mock**: the test passes, the real integration is broken. Keep one integration test per real boundary.
- **Assertion roulette**: one assertion per logical claim; 40 assertions in one test means the first failure hides the rest.
- **Shared mutable fixtures**: tests pass in isolation, fail in parallel. Build fresh state per test.
- **Logic in tests**: loops and conditionals in a test hide which case failed - unroll into named cases.
- **Chasing coverage**: deleting an assertion raises speed, keeps coverage. Audit for assertion-free tests.
- **Mystery guests**: a test that uses setup state defined far away. Inline the values that matter.
