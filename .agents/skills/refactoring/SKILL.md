---
name: refactoring
description: Restructure code without changing behavior - the safe verify-each-step workflow, core refactoring moves (extract, inline, move, rename, polymorphism), strangler fig for legacy systems, and when not to refactor.
---

# Refactoring

Refactoring changes structure without changing behavior. Every step must keep the program observably identical - if tests can tell the difference, it is not a refactor, it is a rewrite. Treat that rule as the definition.

## The Safe Workflow

1. **Test first.** Characterization tests pin current behavior before touching anything. No suite, no refactor. If tests are missing, write them for the exact code you will change - not the whole module - before step 2.
2. **Small step.** One refactoring per edit: one extraction, one rename, one move. Never "while I'm here" fixes; log those as TODOs or issues.
3. **Verify.** Run the tests plus build/lint after every step.
```bash
cargo test && cargo clippy --all-targets && git commit -m "refactor: extract parse_header from read_config"
```
4. **Commit.** Each verified step gets its own commit. When something breaks three steps later, revert to green in seconds instead of debugging a 600-line diff.

If you cannot describe the step as "extract X" or "rename A to B", it is too big. Split it.

## Core Refactorings

### Extract Method / Function

Pull a cohesive block into a named function when the block has one purpose, appears duplicated, or needs a comment to explain.

```python
# Before
def process_order(order):
    # apply discount
    if order.customer.tier == "gold":
        order.total = order.total * 0.85
    elif order.customer.tier == "silver":
        order.total = order.total * 0.95

# After
def process_order(order):
    order.total = discounted_total(order)

def discounted_total(order):
    if order.customer.tier == "gold":
        return order.total * 0.85
    if order.customer.tier == "silver":
        return order.total * 0.95
    return order.total
```

The extraction pays off only if the new name replaces the need for the comment. Too many parameters after extraction means you cut in the wrong place - extract from a different boundary.

### Inline Variable / Method

Remove indirection that adds no meaning. If every reader jumps through a name to the expression anyway, collapse it.

```rust
// Before: the variable adds one line and zero meaning
let is_eligible = user.age >= 18;
if is_eligible { ... }

// Inline when used once and obvious; keep when used 3+ times or when the
// name documents non-obvious intent (e.g. `was_recently_verified`).
if user.age >= 18 { ... }
```

Inline a method when it is called from one place, is one line, and its name says no more than its body.

### Move Method Between Classes

Move a method to the class that owns the data it uses most. The tell: a method on `Order` that reads `customer.tier`, `customer.signup_date`, and `customer.tier_discounts` and touches no order data - it belongs on `Customer`.

1. Add the method to the target class (may need to pass the original object as a parameter).
2. Delegate the old method to the new one; run tests.
3. Update callers one by one; delete the delegator when no callers remain.

### Rename Safely

Renaming is the highest-value, highest-risk refactor - names are the documentation readers actually use.

| Situation | Method |
|---|---|
| IDE/language server knows all references | Rename symbol in IDE; verify zero manual edits |
| Dynamic language, exported symbol | `grep` all usages first; keep a deprecated alias for one release |
| Public API used by other teams | Add new name, deprecate old, migrate callers, remove in a scheduled release |
| Local/private symbol | Rename in one commit; grep the same name to catch shadowed copies |

Never sed-replace across a codebase without grep-previewing every hit: the same word appears in strings, comments, columns, and unrelated identifiers.

### Replace Conditional with Polymorphism

Use when the same type-switch appears in multiple places - each new variant then means one new class/variant, not N more match arms.

```rust
// Before: every new bird edits every match
fn speed(bird: &Bird) -> f64 {
    match bird.kind {
        Kind::European => 35.0,
        Kind::African => 40.0 - bird.load_factor(),
        Kind::Norwegian => grounded_speed(bird),
    }
}

// After: behavior lives with the variant
trait Bird { fn speed(&self) -> f64; }
struct European;
impl Bird for European { fn speed(&self) -> f64 { 35.0 } }
```

Do not use polymorphism for a single switch - a single exhaustive `match` is clear and sufficient. The refactoring pays when the same discriminator branches 2+ times.

### Extract Class from a God Class

A class doing parsing, validation, and I/O needs splitting. Find the subsets of fields that are used together but never interact with the rest; each subset is a candidate class.

1. Pick the most independent subset of fields + methods.
2. Create the new class; move the fields first, compile - errors show every accessor to redirect.
3. Move the methods one at a time, running tests between moves.
4. Leave a delegating method on the old class; migrate callers; delete the delegation.

Aim for the god class shrinking monotonically over a series of commits, never a big-bang rewrite.

### Introduce Parameter Object

When the same group of parameters travels together (start/end/timezone, or 5+ params), make it a type. Bonus: the type becomes the place to validate the group once.

```typescript
// Before - every call site can get the order wrong
function report(start: Date, end: Date, tz: string, format: string, includeDrafts: boolean)

// After - one concept, validated once, extendable without touching call sites
interface ReportRange { start: Date; end: Date; tz: string }
function report(range: ReportRange, opts: ReportOptions)
```

### Replace Magic Numbers with Named Constants

```rust
const MAX_LOGIN_ATTEMPTS: u32 = 5;
const DAY_MS: u64 = 24 * 60 * 60 * 1000;

if attempts > MAX_LOGIN_ATTEMPTS { lock_account() }  // meaning is in the code
```

Exception: `0` and `1` in obvious arithmetic, and values only used once next to their only meaning (`.take(1)`). If the number appears twice or a reader could ask "why this value?", name it.

## Large Refactors

### Feature Flags

For a refactor too big for one release, route around it:

1. Put the new implementation behind a flag defaulting to old behavior.
2. Migrate call sites or data incrementally; both paths must work.
3. Flip the flag in staging; run both paths' tests against shared cases.
4. Delete the old path and the flag in the next release - a flag that lives forever doubles maintenance permanently.

### Strangler Fig (Legacy Systems)

When you cannot rewrite a legacy system in place, grow the replacement around it:

1. Put an anti-corruption facade in front of the legacy entry point; all traffic now flows through your seam.
2. Implement one slice of functionality behind the facade (new code, tested independently); route that slice's requests to the new implementation by feature flag or router rule.
3. Measure: new slice must match old behavior (characterization tests against the old system define "match").
4. Repeat per slice until the legacy core has no traffic, then delete it.

Never data-migrate and code-switch in the same step - one reversible change at a time, so a rollback is a flag flip, not a restore.

## When NOT to Refactor

| Situation | Decision |
|---|---|
| Code works, is tested, and nothing will change near it | Leave it; refactoring carries nonzero risk |
| The module is scheduled for deletion or replacement this quarter | Do not polish code you are about to remove |
| No tests and no time to write characterization tests | Stabilize first; otherwise every "improvement" is a gamble |
| Deadline is close | Ship, then refactor in a calm window with tests |
| The "ugly" code encodes a workaround for a real external bug | Document it, do not clean it - you will reintroduce the bug |

## Common Pitfalls

- **Behavior drift**: sneaking a bug fix into a refactor commit. Fix and refactor in separate commits, or the bisect history becomes useless.
- **Big-bang rewrite**: an "improve structure" PR of 2,000+ lines cannot be reviewed or reverted. Ship the same outcome as 20 small commits.
- **Refactoring the wrong code**: polishing code that is about to be replaced, or optimizing structure in code with no future change pressure.
- **Test-verified-but-unobserved behavior**: passing tests do not prove identical behavior unless they covered the changed paths - check the coverage of the suite on the exact lines you moved.
- **Intermediate refactors**: half-finished renames across branch boundaries - complete each refactoring before merging.
- **Abstraction from one example**: do not extract a generic framework the first time; wait for the rule of three.
