---
name: code-review
description: Review code systematically - a correctness/security/performance/maintainability checklist, actionable feedback patterns, approve-vs-request-changes criteria, and self-review method.
---

# Code Review

Review in passes, not all at once. Read for intent first, then correctness, then the checklist. A review that only catches style issues has failed; a review that misses an unvalidated input has failed worse.

## Before You Review

Read the PR description, linked issue, and any design doc first. Note the diff size: a 2,000-line diff gets a request to split, not a rubber stamp - review quality decays sharply past ~400 lines. Ask for the tests-first or smallest-diff version when the change mixes refactor + feature + fix.

## The Systematic Checklist

Run every diff through these six gates in order. Stop and flag at the first gate that fails hard (security > correctness > the rest).

### Correctness
- Trace each new branch with a concrete input; does the output match the stated intent?
- Check boundaries: empty collection, zero, one, max, negative, null/None/undefined.
- Check error paths the author forgot: what happens when the network call fails, the DB row is missing, the parse returns an error?
- Check resource ownership: who closes the file/connection/goroutine? What leaks on early return?

### Security
Covered in detail below - but at minimum ask: where does this input come from, and who is allowed to do this?

### Performance
Covered below - but at minimum ask: does this code run once, per request, or in a loop over N rows?

### Maintainability
- Can a new hire understand this without asking the author? Naming, function size, layering.
- Does the diff make the next change easier or harder? Count new public API surface: every addition is a permanent commitment.
- Is dead code, commented-out code, or debug logging being added?

### Tests
- Does the diff change behavior without changing tests? Reject unless the change is untestable trivia.
- Do the new tests actually fail if the new code regresses? Check assertions, not just that tests exist.

### API design
- Is the new function's contract obvious from its signature (parameter types, return type, error type)?
- Are optional parameters accumulating? Three booleans in a row means missing types or an options struct.
- Is this the smallest public surface that does the job? Prefer private until a second caller exists.

## Giving Actionable Feedback

Every comment must contain three parts: the trigger (what input/state), the consequence (what breaks), and the direction (what to do). Never write "this is bad."

| Bad | Actionable |
|---|---|
| "This could fail." | "This will return an empty list when the user has no saved cards, and the caller at checkout.rs:88 then divides by `cards.len()`. Return an error or handle the empty case there." |
| "Not thread-safe." | "Two concurrent requests can read-then-write `self.count` between the check at line 12 and the update at line 14; use the existing `AtomicUsize` like `Session::hits` does." |
| "SQL injection risk." | "This string-concats `user_query` into SQL; a user typing `' OR 1=1 --` reads every row. Use the bound-parameter form already used in `find_user_by_email`." |

Separate blocking issues from optional ones: label comments "blocking" or "nit" explicitly. Only blocking issues block merge. Tone: critique the code, never the author - "the loop does X" not "you did X wrong."

## Security Review

Work through this order - it matches how attacks actually happen:

1. **Input validation**: every input from outside the trust boundary (HTTP params, file contents, env vars, messages from other services) is parsed and validated at the boundary, not deep in business logic. Check for: type confusion, length limits missing (unbounded reads = DoS), path traversal (`../` in filenames), and deserialization of untrusted data.
2. **Auth checks**: does every new endpoint/handler verify identity and permission? Look for routes registered before the auth middleware, internal RPC handlers assumed to be "trusted," and IDOR - object IDs from the request used without checking ownership (`GET /invoice/123` must verify 123 belongs to the caller).
3. **Injection**: SQL (string-built queries vs bound parameters), shell (`system(cmd)` with interpolated input), LDAP/XPath, template injection, and header injection (unsanitized input into `Location` or `Set-Cookie`).
4. **Data exposure**: secrets in logs, error messages that return stack traces or internal IDs to clients, PII written to analytics, secrets committed in the diff (`grep` for `password`, `token`, `BEGIN PRIVATE KEY`), and debug endpoints left enabled.
5. **Crypto**: no homegrown crypto, no `rand()` for tokens (use the CSPRNG), no MD5/SHA1 for anything security-relevant, constant-time comparison for secrets.

Treat any security finding as blocking regardless of "we'll fix it later."

## Performance Review

Look for the multiplicative patterns, not micro-optimizations:

- **N+1 queries**: a query inside a loop, or a lazy-loaded relation accessed per item. `SELECT` in a `for` loop over results = N+1; batch it or join it.
```python
# BAD: 1 + N queries
users = db.query(User).all()
names = [u.profile.display_name for u in users]  # query per user

# GOOD: 2 queries
users = db.query(User).options(joinedload(User.profile)).all()
```
- **Unnecessary allocations**: clones of large structures in loops, building intermediate collections just to take `.len()`, string concatenation in a loop instead of a builder, collecting an iterator to immediately iterate it again.
- **Missing indexes**: new queries filtered on a column with no index (check the WHERE/JOIN/ORDER BY columns against the schema/migrations in the diff).
- **Repeated work in hot paths**: recomputing constants per call, re-parsing per iteration, unbounded caches vs no cache on an expensive pure function called per request.
- **Locking**: locks held across I/O, long transactions spanning network calls.

Require a measurement before accepting any optimization comment you make: profile or benchmark data decides, not vibes. Conversely, block changes that add per-request latency without the author stating the cost.

## Approve vs Request Changes

| Situation | Verdict |
|---|---|
| Blocking correctness or security issue | Request changes |
| Tests missing or do not cover new behavior | Request changes |
| Only nits, naming, style | Approve with comments |
| You are unsure and cannot resolve in a day of study | Approve a follow-up issue, do not block |
| Author explicitly deferred an agreed item | Approve, verify the issue exists |
| Refactor mixed with behavior change | Request changes (ask to split) |

When you request changes, list every issue in the first round. Drip-feeding five rounds of one comment each is disrespectful; re-review the fixed diff against your own list before responding.

## Reviewing Your Own Code (Rubber-Duck Method)

Never push directly after writing. Before opening the PR:

1. Re-read the full diff top to bottom as if a stranger wrote it.
2. Explain each hunk out loud (literally, or written in the PR description): "this loop does X because Y." The moment the explanation stalls, that hunk is wrong or unclear - fix it now, not after review.
3. Check your own failure modes first: the clever one-liner, the "temporary" debug print, the TODO without an issue link, tests you wrote by copying the implementation's logic.
4. Run the diff against the six checklist gates yourself and comment-inline before others do.

## Code Smells That Indicate Deeper Problems

Each smell is a pointer, not a defect itself. Investigate what it points at:

| Smell | Deeper problem it usually means |
|---|---|
| Function > 60 lines or deep nesting | Missing abstraction; mixed responsibilities |
| Boolean parameters (`process(x, true, false)`) | Two functions wearing one name |
| Comments explaining what code does | Code does not express intent; restructure |
| Same 5-line check at many call sites | Missing type/guard at the boundary |
| One test needs 200 lines of setup | Design is hard to construct; too much coupling |
| `instanceof`/downcast chains | Polymorphism or enum is the actual model |
| Catching errors just to re-raise or ignore | Unclear error contract |
| Data clumps (same 3 params everywhere) | A missing domain concept |

## Common Pitfalls

- **Nitpicking style that the formatter should own** - configure the formatter, stop commenting on it.
- **Approving because the author is senior or in a hurry** - the checklist applies to everyone.
- **Reviewing only the diff** - open the surrounding file; the diff can be correct and the integration wrong.
- **Blocking on preferences** (framework choice, taste) - block on the checklist, suggest preferences as nits.
- **Silent approvals** - say what you checked so the next reviewer knows the coverage.
- **Arguing in comments past two rounds** - take it to a call or spike, post the decision back to the PR.
