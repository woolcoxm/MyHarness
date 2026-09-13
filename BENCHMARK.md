# Benchmark: myharness vs ZCode, same complex task

Task: "build a playable three.js game in a single index.html" (Neon Drift —
an endless hover-racer with collision, particles, HUD, overlays; ~45 KB of
code). ZCode arm run in a fresh-context subagent; metrics read from ZCode's
own per-request model-I/O rollout log. myharness arm is projected from its
measured payload floor (pinned by a regression test) applied to the
identical conversation, since the coding-plan endpoint is client-bound
(signature + proof-of-work headers) and standard API keys were not
available at benchmark time.

## Measured: ZCode (17 requests, real usage from rollout)

| Metric | Value |
|---|---|
| Input tokens billed | 843,263 |
| — of which served from cache | 780,864 (92.6%) |
| Fresh input | 62,399 |
| Output tokens | 49,482 |
| Payload floor per request (system+tools) | 26,217 chars ≈ **6,554 tok** |
| Conversation messages growth | 15.7k chars → 240.7k chars |
| Mean input per request | 49,604 tok |

## Measured: myharness cost structure

| Metric | Value |
|---|---|
| Payload floor per request (system + 16 tool schemas) | 15,328 chars ≈ **3,832 tok** |
| Floor vs ZCode | **41% smaller** |
| Cache behavior | byte-stable prefix (same caching economics) |

## Head-to-head (projected for the identical conversation)

Per-request overhead delta = 6,554 − 3,832 = **2,722 tok/request**.

- Gross input tokens: 843k (ZCode) vs ~797k (myharness) → **~5.5% fewer**
- At cache pricing (cached input ~10% of base): input cost-equivalents
  140.5k vs 134.4k → **~4.3% cheaper input**
- Output: identical (same model, same task) → 49.5k both sides
- Blended bill (output at ~4x input price): ~338 vs ~332 cost-units →
  **~2% cheaper overall for this task shape**

## Reading the numbers honestly

1. For **long, tool-heavy builds** the conversation and the model's own
   output dominate the bill; the harness's payload floor is second-order
   (a few percent). No harness changes that — the model's output is the
   model's output.
2. For **short interactions** (quick questions, small edits — the most
   common session shape) the floor dominates, and myharness's 3.8k floor
   vs ~6.5k (subagent profile; the main-agent profile is larger still)
   makes each trivial request **~1.7x cheaper** before caching and more
   with it.
3. The levers that actually move a token budget, in order of magnitude:
   **model choice** (glm-4.7-air vs glm-5.3 pricing gap dwarfs everything
   here), **output-token discipline** (myharness's v0.14 prompt round
   attacks exactly this), then payload floor, then caching hygiene
   (both harnesses cache well — 92.6% of ZCode's input was cache-served;
   myharness's cache-stability discipline achieves the same effect).

## Reproduce

ZCode arm: run the game prompt in a ZCode subagent; parse
`~/.zcode/cli/rollout/model-io-sess_subagent_agent_*.jsonl`
(`response.usage.{inputTokens,outputTokens,cacheReadTokens}` and
`request.body.{system,tools}` sizes). myharness arm: with a standard API
key set, `myharness -p "<same prompt>"` prints the same usage line to
stderr; the floor is asserted by `cargo test prompt_budget -- --nocapture`.
