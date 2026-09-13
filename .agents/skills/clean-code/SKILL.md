---
name: clean-code
description: Write readable code - intent-revealing names, small pure functions, comments that explain why, SOLID with concrete fixes, DRY vs deliberate duplication, and Result-based error flow.
---

# Clean Code

Code is read 10x more than it is written. Every rule below serves one goal: minimize the time a reader needs to understand and safely change the code. Apply the rules to code you are writing or touching anyway - do not drive-by-rewrite untouched files.

## Naming

- **Variables reveal intent**: name the concept, not the type or the storage. `days_since_last_login`, not `d` or `int_val`. For loops, `i`/`j` are fine only when the body is under 3 lines; otherwise `row_index`, `retry_attempt`.
- **Functions are verbs**: `calculate_tax`, `parse_header`, `is_expired`. A function named with a noun (`user_data`) hides that it does work.
- **Classes/types are nouns**: `Invoice`, `RetryPolicy`, `HttpClient`. A class named `DoStuff` or `ManagerUtils` is doing something else wrong.
- **Booleans read as questions**: `is_valid`, `has_children`, `can_retry`, `should_retry`. Then call sites read as English: `if should_retry { retry() }`.
- **Avoid encodings and disinformation**: no Hungarian prefixes (`strName`), no `data2`, and never let a name promise what the value is not - `account_list` holding a map is a lie readers will trip over.

```javascript
// BAD: the reader must reconstruct the meaning from context
const d = new Date() - u.last;
if (d > 86400000) { ... }

// GOOD: the meaning is in the code
const DAY_MS = 24 * 60 * 60 * 1000;
const isStale = Date.now() - user.lastActivityAt > DAY_MS;
if (isStale) { deactivate(user); }
```

Rename freely early in a file's life; use the IDE rename, never find-and-replace.

## Function Design

One function does one thing - you can describe it in a sentence without "and". Heuristics: under ~20 lines, at most ~3 parameters, one level of abstraction (do not mix file parsing with string trimming).

```rust
// BAD: two jobs, hidden side effect, mixed abstractions
fn get_config(path: &str) -> Config {
    let mut s = String::new();
    File::open(path).unwrap().read_to_string(&mut s);  // panics hidden in a "get"
    CONFIG_COUNT += 1;                                  // side effect hidden in a "get"
    parse(&s)
}

// GOOD: one job each, failures are return values, no hidden writes
fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let raw = fs::read_to_string(path)?;
    parse_config(&raw)
}
```

- **No side effects hidden behind "get"/"query"/"is" names.** A reader skips those calls when reasoning about state changes; the bug reports write themselves.
- **No flag arguments.** `render(doc, true)` splits into `render_html(doc)` and `render_pdf(doc)` - the boolean is two functions wearing one name.
- **Prefer fewer parameters**: bundle related ones into a struct (also enables named access at the definition), derive what can be derived, default what can default.
- **Return early for guard clauses**; keep the happy path at the lowest indentation.

## Comments

- **Explain WHY, not WHAT.** The code already says what it does; say why it must.
```python
# BAD: translates the code
# increment i by one
i += 1

# GOOD: explains a decision the code cannot
# 3 retries because the upstream gateway drops ~0.1% of requests
# under load; beyond 3 the queue backs up faster than it drains
for attempt in range(3):
    ...
```
- **Delete commented-out code.** Version control remembers it; dead comments rot and lie within weeks.
- **Good code needs few comments.** A comment you would write above a block is usually a name: extract the function and make the comment its name.
- **Do write**: warnings about non-obvious constraints, links to the spec/issue behind a workaround, units and invariants on tricky math, and doc comments on public API contracts.

## SOLID

### S - Single Responsibility
One reason to change per module. Violation: `ReportGenerator` that formats, computes tax, and sends email - every new currency AND every new email template change it. Fix: `TaxCalculator`, `ReportFormatter`, `MailSender`, coordinated by a thin caller.

### O - Open/Closed
Extend by adding code, not editing a switch that every feature re-edits.
```rust
// BAD: every new payment type edits this function
fn process(kind: &str, amount: u32) { match kind { "card" => ..., "paypal" => ... } }
// Fix: trait PaymentMethod { fn process(&self, amount: u32); } - new type = new impl,
// existing code untouched. But apply only when variants actually accumulate;
// a match with two stable arms is fine.
```

### L - Liskov Substitution
A subtype must honor its parent's contract. Violation: `Square extends Rectangle` where `set_width` also changes height - code calling `set_width` through a `Rectangle` breaks. Fix: model them as sibling types sharing an interface, or make shapes immutable.

### I - Interface Segregation
No client forced to depend on methods it never calls. Violation: a 10-method `Printer` interface with one `print` method used by 95% of callers. Fix: `Printer` (print), `Scanner` (scan), `Fax` (burn it).

### D - Dependency Inversion
High-level policy depends on abstractions, not concrete infrastructure. Violation: `Checkout` constructs `StripeClient` internally - untestable without Stripe. Fix: accept a `PaymentGateway` trait; tests pass a fake, main wires the real one.

Apply SOLID where change pressure exists. An 8-line script with one future does not need five interfaces.

## DRY vs WET (and the Rule of Three)

Duplication is cheaper than the wrong abstraction. DRY means "one source of truth for each piece of knowledge" - not "any repeated characters get a helper."

| Situation | Choose | Why |
|---|---|
| Same rule in two places, will drift | DRY it now (one constant, one function) | Divergence is a latent bug |
| Similar-looking code solving different problems | Leave duplicated | Premature merging couples unrelated change rates |
| Third occurrence of a pattern | Extract (rule of three) | Two data points are not a pattern; three are |
| "General" helper used once | Inline it | Speculative generality is complexity with zero users |

```javascript
// Wrong abstraction: two callers, different semantics, forever coupled
function handleData(data, { isUser = true } = {}) {
    if (isUser) { validateEmail(data); } else { validateSku(data); }
    save(data);
}
// Better until a real third case appears: two functions that read clearly
function saveUser(user) { validateEmail(user); save(user); }
function saveProduct(product) { validateSku(product); save(product); }
```

## Error Handling as Control Flow

Errors are values in the domain, not surprises. Handle each error exactly once, at the layer with enough context to decide.

| Mechanism | Use for | Do not use for |
|---|---|---|
| Result/Option types | Recoverable, expected failures (parse, IO, not-found) | Panics as lazy exits |
| Exceptions | Truly exceptional, rare conditions (in languages without Result) | Regular control flow |
| Panics/throws on programmer error | Broken invariants that cannot be recovered (index bug) | Bad input from users or files |

```rust
// BAD: error swallowed; caller sees "0" and reports a wrong bill
fn tax_due(invoice: &Invoice) -> u32 {
    match compute_tax(invoice) { Ok(t) => t, Err(_) => 0 }
}

// GOOD: propagate; the UI layer decides how to show "tax unavailable"
fn tax_due(invoice: &Invoice) -> Result<u32, TaxError> {
    compute_tax(invoice)
}
```

Rules: never ignore an error silently (log or propagate, pick one); wrap with context at boundaries ("open config: permission denied" beats "permission denied"); do not catch-and-rethrow unchanged; and reserve top-level handlers for the last line of defense, not a routine layer.

## Code Organization

**Prefer cohesion over coupling**: things that change together live together. The test - when you add a typical feature, how many directories do you touch? One means the structure fits; five means layers are slicing through concepts.

**Feature folders over layer folders**:

```
# Layer folders: every feature edits 4 far-apart directories
src/
  controllers/  services/  repositories/  models/

# Feature folders: one feature = one place; shared kernels are explicit
src/
  billing/    # api.rs, service.rs, store.rs, tests
  accounts/
  shared/     # genuinely cross-feature: money, ids, errors
```

Feature folders keep unrelated modules from growing tentacles into each other; the `shared/` directory is a budget - every addition there couples everyone, so it must earn its place. Public surface of one feature = its thin API module; reaching into another feature's internals is a layering violation, enforce with visibility modifiers or lint rules.

## Common Pitfalls

- **Cleverness over clarity**: one-liner reduce chains that need a minute to parse. Write the boring loop; the reader (and the debugger) thanks you.
- **Extracting functions that need 6 parameters**: the cut is at the wrong boundary - find the cohesion seam instead.
- **Comments as deodorant**: comments masking bad names or long functions. Fix the code, delete the comment.
- **SOLID cargo-culting**: interfaces with one implementation "for flexibility" - delete them until the second implementation exists.
- **DRY-ing constants that coincidentally match**: `TWO` different concepts both equal to 60 is not duplication; merging `SECONDS_PER_MINUTE` and `FPS_LIMIT` couples them forever.
- **Deep nesting pyramids**: guard clauses flatten; more than 3 indent levels is a missing early return or a missing function.
