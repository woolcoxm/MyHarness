# myharness — Design

*A terminal coding-agent harness for GLM, in Rust. This document is the "who /
what / when / where / why / how" the project was designed against, including the
research it is built on.*

---

## The one-paragraph version

A harness is the scaffolding around a language model that turns "a model that
can predict text" into "an agent that can do work": the tool contracts it
calls, the loop that feeds results back, the context budget that keeps it
coherent over hours, the permission model that keeps it from doing damage, and
the session log that lets work survive a crash. myharness is a from-scratch
Rust implementation of that stack, deliberately shaped around **what a GLM
coding model is trained to do well** (Claude-Code-style tool calling,
`cat -n` reads, exact-string edits) rather than around any novel interaction
invention. Every default was chosen to make the model's failure modes
self-correcting instead of catastrophic.

---

## WHO

A harness has **two users with different needs**, and most design tension comes
from serving both:

1. **The model (GLM-5.3) is the primary user.** It "reads" the tool schemas,
   the system prompt, and every error string. It never sees the UI. Everything
   it touches must be: deterministic (same input → same output), loud on
   failure (errors that say exactly what to fix), and bounded (no result can
   eat the context window). The model is a brilliant pattern-matcher wrapped
   around a noisy channel — the harness's job is to make the channel quiet.
2. **The developer is the safety and UX user.** They need to know what the
   agent is about to do before it does it, to interrupt it, to resume after a
   crash, and to script it in CI (`-p` mode) without a TUI in the way.

Who built this: designed and implemented autonomously by the same class of
model that runs inside it — which is the point. The harness encodes what a
coding agent knows it needs.

## WHAT

A single static Rust binary (`myharness`) providing:

| Layer | What it does |
|---|---|
| `llm/` | Unified message IR + `Provider` trait. Streaming clients for **Anthropic Messages** (`/v1/messages`, SSE) and **OpenAI chat-completions** (`/chat/completions`, SSE tool-call accumulation), plus a scripted `mock` provider for offline tests. Retries with backoff on 429/5xx/network errors, `Retry-After` honored. Tool-result **images** ride the IR as base64 blocks (anthropic protocol; openai degrades to a text note). |
| `tools/` | Fourteen tools: `read_file` (text + images), `write_file`, `edit_file`, `bash` (persistent cwd, timeouts, **background tasks**), `bash_output` (poll background tasks), `glob`, `grep`, `ls`, `todo_write`, `skill` (SKILL.md packs), `task` (subagents), `web_fetch`, `web_search` (keyless DDG), `repo_map` (symbol digest). Contracts detailed below. |
| `agent/` | The turn loop (request → stream → tool calls → results → repeat), system prompt (static contract + dynamic state: cwd, todos), token estimation, threshold compaction with handoff summaries (optionally routed to a cheaper `model_fast`). Tools collect **side-effects** that the agent merges after execution, which lets concurrency-safe calls fan out **in parallel**. |
| `perms/` | Modes `plan / ask / auto-edit / yolo`; deny→allow→prompt rule engine with glob patterns; **compound-command decomposition** for bash rules; non-interactive fail-closed. |
| `session.rs` | JSONL event-sourced transcripts (message / compaction / todos / **files_read** / cwd / clear events); replay-based resume including the read-before-edit guard set. |
| `config.rs`, `cli.rs`, `ui.rs` | Layered config (defaults ← `myharness.toml` ← env ← CLI), clap CLI (`-p`, `--mode`, `--yolo`, `-c`, `sessions`, `config`, `resume`), and a line-oriented ASCII REPL with slash commands. |

### Tool contracts (the part the model actually experiences)

- **`read_file`** → `cat -n` format (`     1\ttext`), 2000-line default with
  offset/limit, binary detection, long-line truncation. **Image files**
  (png/jpg/gif/webp, ≤4 MB) come back as image blocks the model can see —
  screenshots, diagrams, UI verification. Line numbers anchor every later
  conversation about a file.
- **`edit_file`** → exact-string replacement. Fails loudly with counts:
  0 matches (check whitespace / re-read), N>1 matches (add context or
  `replace_all`). Requires the file was read this session.
- **`write_file`** → refuses to overwrite an unread file (the #1 way agents
  destroy work). New files create parent directories.
- **`bash`** → persistent working directory (a `__MH_CWD__` marker line reports
  the post-command cwd back to the harness; `cd` sticks), timeouts with
  process-tree kills (`taskkill /T /F` on Windows), 30k head+tail truncation,
  exit code always reported, user-interrupt kills mid-command.
  **`run_in_background`** starts the command without waiting; the model keeps
  working and polls with **`bash_output`** (state, exit code, output tail).
  Shell resolution on Windows: git-bash (WSL's `bash.exe` deliberately
  excluded) → PowerShell → cmd.
- **`glob` / `grep` / `ls`** → find-before-read. `grep` uses the `ignore`
  crate (ripgrep's traversal engine): gitignore-aware, skips
  `target/node_modules/.git/...`, glob filters, 200-match cap, and
  `output_mode` = `content` (path:line:text) / `files` (paths + counts) /
  `count` (path:N) so broad searches can answer "where" without paying for
  line text. Both search tools run on the blocking pool.
- **`web_fetch`** → http(s) GET, HTML→text (scripts/styles dropped, entities
  decoded), JSON/plain passthrough, 2 MB download cap, 20k char result.
  With the optional `prompt` parameter the page text is handed to the
  configured `model_fast` and only the answer comes back — query-focused
  extraction instead of a 20k-char page dump. Documentation and
  error-message lookups without leaving the harness.
- **`todo_write`** → full-list replacement; injected into every system prompt
  so the plan survives context growth; one-`in_progress` rule enforced.
- **`skill`** → loads a discovered SKILL.md instruction pack by exact name
  (frontmatter `name`/`description`, markdown body). Skills are discovered
  from the user's `~/.agents/skills/` and the workspace's
  `.agents/skills/` (ZCode-compatible locations; workspace wins on name
  collisions, `MYHARNESS_SKILLS_DIR` overrides the user dir). Only the
  name + description list rides the system prompt — the full body arrives
  through this tool on demand, so a skill costs context only when used.
  Re-discovered per call, so editing a SKILL.md needs no restart; the REPL
  invokes one directly with `/<name> [args]`.
- **`task`** → subagent with a fresh context, `agent_type` presets
  (`explore`: read-only + `web_fetch`, default; `build`: adds
  `bash`/`bash_output` for running builds and tests), a `tools` override,
  turn cap, no recursion. Only the final report returns to the parent —
  exploration noise stays out of the main thread. Multiple subagents in
  one message run in parallel.

### Parallel tool execution

Tools never mutate agent state directly: they run against an owned snapshot
(`ToolCtx`) and emit a `ToolEffects` diff (files read, cwd change, todos,
background registrations) that the agent merges afterwards. Calls are
permission-checked sequentially (prompts must respect the model's ordering),
then concurrency-safe tools (`read_file`, `grep`, `glob`, `ls`, `web_fetch`,
`todo_write`, `task`) fan out on the runtime while state-touching tools
(`write_file`, `edit_file`, `bash`) run in order. Results are assembled in
the model's original order regardless of completion order.

### Context management

Heuristic token estimate (ASCII ≈ 0.3/char, other scripts ≈ 0.8) guards a
compaction threshold (default 80% of the window). Compaction asks the model
itself for a structured handoff brief (goal, decisions, files touched, next
steps, verbatim must-survive snippets), keeps a 6-message tail — with
boundaries adjusted so a `tool_result` is never separated from its
`tool_use` — and persists a `compaction` event so resume rebuilds the same
state. API-reported usage is tracked separately and shown per turn.

## WHEN

- **When to use it:** any coding task where you want the model driving —
  multi-file refactors, test-driven changes, exploration with subagents,
  scripted CI runs (`myharness -p "fix the failing test" --yolo`).
- **When it compacts:** automatically at the threshold, or `/compact` on
  demand.
- **When it asks permission:** mode-dependent (see `perms`), always
  fail-closed when a prompt is impossible (`-p` without allow rules).
- **When to reach for something else:** when you need OS-level sandboxing
  (Codex), a full TUI (OpenCode), or MCP extensions (Goose) — see Roadmap;
  v1 is intentionally the reliable core, not the feature frontier.

## WHERE

Your terminal, in a repository. Sessions live in
`%LOCALAPPDATA%\myharness\sessions\*.jsonl` (overridable with
`MYHARNESS_DATA_DIR`); config is discovered from the cwd upward
(`myharness.toml`) then the user config dir. `-p` mode streams nothing but the
final answer to stdout, so it pipes cleanly into CI scripts.

## WHY (each decision, and where it came from)

1. **Exact-string edit with uniqueness enforcement** — the single
   highest-leverage reliability contract across every harness surveyed
   (Claude Code). Malformed edits become self-correcting because the error
   tells the model exactly what to do next. Aider's whole-file and udiff
   formats exist to accommodate weaker edit reliability; we skip that
   complexity and encode the strict contract.
2. **Read-before-write/edit gating** — Claude Code. Prevents blind clobbers;
   the failure direction is "annoying retry", never "lost work". The guard
   set (files_read) is persisted as a session event so it survives resume.
3. **`cat -n` reads** — the format GLM-class models are most heavily trained
   on; line numbers make edit conversations unambiguous.
4. **Head+tail truncation with explicit markers** (30k bash, 60k read) —
   Claude Code. One tool result must never eat the context window.
5. **JSONL event-sourced sessions** — Claude Code / Codex rollout files.
   Append-only, crash-safe (torn tail lines are skipped on replay), resume
   for free, and a stable documented schema (the anti-pattern Claude Code
   documented: internal formats that users parse anyway — ours is small and
   versioned by simplicity).
6. **Threshold compaction ~80% + persisted compaction events** — Goose
   (auto-compact at 80%) and Claude Code (`/compact`); avoiding Gemini CLI's
   documented anti-pattern where compaction state is lost across resume.
   Compaction can route to a cheaper `model_fast` (Aider's tiered-model
   lesson) since summaries don't need the flagship model.
7. **Subagents with scoped tools and final-report-only return** — Claude Code
   Task / OpenCode `task`. Preserves the main thread's budget on broad
   exploration; multiple subagents run in parallel.
8. **Compound-command splitting for bash rules** — Claude Code's documented
   permission semantics: `ls && rm -rf /` must not ride an `ls` allow rule.
   Our splitter over-splits quoted separators, which fails safe (an extra
   prompt, never a silent allow).
9. **Fail-closed non-interactive mode** — Codex's sandbox/approval orthogonality
   distilled: when no human can answer, "needs approval" means "denied with
   instructions", not "allowed".
10. **Mock provider as a first-class citizen** — the only way to test the
    *whole* harness (loop, tools, permissions, compaction, subagents) in CI
    without network, keys, or cost.
11. **Rust, static binary, no runtime deps** — Codex's stated motivations for
    codex-rs: zero-dependency install, no GC pauses while streaming, native
    process/signal control. Also: one binary to copy to a machine and run.
12. **ASCII line-oriented UI, no TUI** — correctness in every console, pipe,
    and CI log; a TUI can be added as a frontend later without touching the
    core (see Roadmap).

## v0.2 — what the model asked for (and got)

Built after asking "is anything missing?". Each item maps to a concrete
model-side failure mode in v0.1:

| Feature | Why the model needs it |
|---|---|
| **Images in read_file** | No visual verification meant front-end/UI work was blind; screenshots are how agents close the loop on rendering. Anthropic-protocol native; openai protocol degrades to a note. |
| **Background bash + bash_output** | A 3-minute `cargo test` used to freeze the whole agent. Now the model starts it, keeps reading/editing, and polls — the single biggest wall-clock win. |
| **Parallel tool calls** | Models naturally batch independent reads; executing them serially wasted round-trips. Effects-collection architecture makes it safe. |
| **web_fetch** | Docs, error messages, and API references without leaving the harness. |
| **files_read persistence** | Post-resume editing required re-reads; the guard set now rides the session log. |
| **model_fast** | Compaction summaries don't need the flagship model — route them cheaper. |

## v0.3 — the second honest audit

Asked the same question again and found a different class of gaps:

| Feature | Why |
|---|---|
| **Thinking/reasoning streams** | v0.2 *silently discarded* GLM reasoning output (`reasoning_content` on the OpenAI protocol, `thinking_delta` on the Anthropic one). Now streamed to the operator with a `~` prefix, and deliberately never fed back into the context (providers reject it on input). A correctness fix, not a feature. |
| **Prompt caching** | Anthropic-protocol `cache_control` breakpoints on tools + system + last message (3 of 4 allowed). Long sessions stop re-paying the full input cost every request. On by default, configurable. |
| **AGENTS.md injection** | Repo instructions (nearest AGENTS.md upward, capped at 8k chars) are appended to the system prompt — per-project conventions finally reach the model. |
| **Auto-verify after edits** | Opt-in `verify_cmd` (e.g. `cargo check`) runs after any write/edit turn and the result is injected as the next user message, so the model sees compile/test failures immediately instead of forgetting to check. Synchronous and bounded by the bash timeout — long checks belong in background bash. |
| **Workspace write-scoping** | In unattended modes (auto-edit/yolo), write_file/edit_file outside the workspace root are denied with guidance; ask-mode still allows explicit user approval. The remaining sandbox hole is bash, which needs OS-level work (Roadmap #1). |

## HOW

### The loop

```
user input ──▶ push User message (persist)
                 │
                 ▼
        ┌────────────────────────────────────────────┐
        │ estimate tokens; if over threshold:        │
        │   summarize → handoff msg + 6-msg tail     │  (persist compaction)
        │ build request (system + todos, tools)      │
        │ stream response; print text deltas live    │
        │ collect text blocks + tool_use blocks      │  (persist assistant)
        │ no tool calls? ──▶ done: final message     │
        │ for each tool_use:                         │
        │   permission check (deny/allow/prompt)     │
        │   dispatch → ToolOutput (errors are data)  │
        │ push User(tool_results)          (persist) │
        │ turn cap? notice → grace → hard stop       │
        └────────────────────────────────────────────┘
```

Everything is an `append` to the JSONL after it happens — the transcript can
only ever be a prefix of what truly occurred, so resume is always consistent.

### Module map

```
src/
  main.rs            binary: CLI dispatch, provider construction, resume
  cli.rs             clap surface
  config.rs          layered config + shell resolution
  llm/mod.rs         IR (Message/ContentBlock), Provider trait, SSE splitter, retries
  llm/anthropic.rs   /v1/messages streaming client
  llm/openai.rs      /chat/completions streaming client (tool_call delta assembly)
  llm/mock.rs        scripted provider (tests, --provider mock)
  agent/mod.rs       the loop, permission prompting, tool dispatch
  agent/state.rs     mutable session state (messages, todos, cwd, files_read, usage)
  agent/system_prompt.rs   the model contract (static + dynamic sections)
  agent/compact.rs   compaction (summary + tail, pair-safe boundaries)
  agent/tokens.rs    estimation, transcript rendering
  tools/*.rs         one file per tool; ToolCtx = state + cfg + provider
  perms.rs           modes, rules, compound splitting
  skills.rs          SKILL.md discovery (user + workspace .agents/skills)
  session.rs         JSONL events, replay, listing
  ui.rs              REPL, slash commands, streaming printer
tests/integration.rs 11 end-to-end scenarios on the mock provider
```

### Testing strategy

- **80 automated tests**, zero network: 41 unit + 39 integration (the v0.13
  addition drives three serve processes over stdio: implicit session
  creation with session_id in the turn.run response, attach-by-id resume
  with the replayed message count, same-session continuation, and the
  unknown-id error), plus two
  opt-in live AppContainer tests (`MYHARNESS_LIVE_AC=1`, verified on the
  dev machine: launcher-level containment denial, and the bash tool round
  trip under `sandbox = "appcontainer"` with both cmd and git-bash). The v0.11
  addition drives the real `serve` binary over stdio: initialize, a full
  turn.run with streamed mh/* notifications and a final_text response, and
  an unknown-method error — the MCP smoke pattern applied to the server.
  The v0.10
  additions cover URI round-trips, Content-Length framing helpers, and a
  live LSP round trip through the real built mock server (write → didOpen →
  publishDiagnostics → injected message, client kept alive across turns).
  The v0.9
  additions cover per-language import parsing, import resolution, and the
  PageRank ordering (a file referenced by two siblings outranks a newer
  unreferenced leaf). The v0.8
  additions cover the schema-subset validator (types/required/enum/items,
  fence-stripped parsing) and the corrective-round loop (invalid output is
  fed back to the model and repaired; exhausted retries return the text and
  the CLI exits non-zero). The v0.7
  additions cover the DDG result parser (fixture + dedupe + percent
  round-trip + an opt-in live test validated against the real endpoint),
  background completion notices (delivered exactly once, exit code and
  output included), and repo_map symbol extraction per language plus the
  end-to-end tool call. The v0.6
  additions cover skill discovery/parsing/override precedence, the skill
  tool round trip, the system-prompt skill list (main agent only), task
  agent_type presets (build runs bash, explore does not, unknown types
  fail loudly), background subagents (detached run + report delivered
  through bash_output), command-file parsing/expansion/override
  precedence, grep output modes, and the web_fetch extraction request.
  The v0.5/v0.4 rounds added the Windows Job Object kill-on-close wiring
  (live: close-the-handle, watch the child die), the full hook contract
  (a deny-with-exit-2 `.cmd` blocking bash end-to-end), an MCP round trip
  through the real built example server (`mcp__mock__echo` → `ECHO: ...`),
  and the repository-layout digest. Earlier coverage: write→read→finish
  flows, the edit error ladder, blind-overwrite refusal, bash execution,
  plan-mode gating, non-interactive denial, permission rules incl.
  compound smuggle, compaction + persistence, subagent isolation, session
  replay, todos, parallel batches, background bash, images through the IR,
  files_read replay, out-of-workspace denial, verify_cmd injection,
  AGENTS.md injection.
- **Binary smoke tests** with `--provider mock` against the real executable:
  `-p` mode, REPL with permission prompts (including MCP tools being
  gated), `sessions`, `config` (parsing hooks/mcp/sandbox from a real
  `myharness.toml`), resume, and four mock scripts
  (`examples/smoke-script*.json`).
- **Cross-target verification**: the Landlock path compiles for
  `x86_64-unknown-linux-gnu` (validated in a scratch crate against the
  crate's canonical API, since the full tree needs a Linux cross-compiler
  for `ring`'s assembly).
- Live-API testing is a one-liner once a key is set (see README), but nothing
  in the suite depends on it.

---

## Research appendix: what other harnesses taught this design

Surveyed: **Claude Code** (tool contracts, permissions, sessions, hooks),
**OpenAI Codex CLI / codex-rs** (Rust architecture, sandbox × approval
orthogonality, rollout files, exec mode), **OpenCode** (client/server split,
LSP feedback, shadow-git snapshots), **Aider** (edit formats, tree-sitter
repo map, git conventions), **Goose** (compaction thresholds, MCP-first,
scheduler), **Gemini CLI** (free tier, compression service, `/compress`).

**Adopted (v1):** items 1–10 in WHY above.

**Deliberately deferred (Roadmap):** tree-sitter repo map,
LSP diagnostics after edits, git auto-commit/shadow snapshots, JSON-RPC
server split, `--output-schema`, `web_search`. (OS sandboxing, hooks, and
the MCP client were deferred here once too — v0.4 shipped them.)

**Anti-patterns actively avoided:**
- whole-file rewrite as default edit format (slow, costly, error-prone);
- naive bash prefix matching without compound decomposition;
- compaction that doesn't persist across resume (Gemini CLI issue #21335) or
  triggers so early it trades context quality for headroom;
- unstable internal session formats users can't safely build on;
- runtime dependencies (ship one static binary);
- API keys in transcripts (keys are only read from env; never logged).

## v0.4 — the research round (sandboxing, hooks, MCP)

The roadmap items that needed platform research, researched and shipped:

| Feature | What it actually is (and isn't) |
|---|---|
| **Windows sandbox (`job`, default)** | Every bash child joins a process-wide Job Object with `KILL_ON_JOB_CLOSE` — the entire command tree dies with the harness, no orphaned builds/servers can outlive a crash. Verified by a live kill-on-close test. This is *containment*, not filesystem isolation; real FS sandboxing on Windows is AppContainer (roadmap). |
| **Linux sandbox (`strict`, opt-in)** | Landlock (ABI V1, best-effort) applied in the child before exec: read-everywhere, writes only under workspace + temp. Opt-in because cargo/npm/pip legitimately write to home caches. Compile-verified against `x86_64-unknown-linux-gnu` with the landlock crate's canonical API (verified via a cross-target scratch crate, since the full dep tree needs a Linux cross-compiler for `ring`). Runtime enforcement needs kernel 5.13+. |
| **Hooks** | Claude-Code-compatible contract: JSON payload on stdin (`session_id`, `tool_name`, `tool_input`, ...), exit code 2 blocks, or `{"decision":"deny","reason":...}` on stdout (`permissionDecision` accepted as alias). `PreToolUse` runs before permissions (hook deny always wins); `PostToolUse` observes results; `Stop` with exit 2 pushes the reason back to the model and forces continuation (bounded to 3 rounds). A broken/failing hook fails closed. |
| **MCP client** | stdio JSON-RPC: initialize handshake → `tools/list` → each server tool bridged as `mcp__<server>__<tool>` (the naming convention models already know). Calls serialized per server; failures degrade to warnings; MCP tools go through the normal permission engine. Live round trip verified through the real binary with `examples/mock-mcp-server.rs`. |
| **Repository layout digest** | The interim repo map: top-level dirs with recursive file counts + root files, injected into the system prompt (build/dependency dirs skipped, 40-entry cap). Not symbol-level (tree-sitter stays on the roadmap) but answers "what lives where" for one directory walk's cost. |

Platform note discovered the hard way: `cmd /C /D script` is broken on
current Windows builds (`/D` must precede `/C`) — fixed in both the hook
runner and the cmd shell path, with a regression test that exercises the
hook contract end-to-end.

## Roadmap (v8 candidates, in value order)

1. **AppContainer sandboxing** on Windows — the real filesystem isolation
   that Job Objects don't provide.
2. **Tree-sitter symbol precision** (optional upgrade) — the regex
   extractor + import-graph ranking shipped in v0.9; grammars would add
   precision, not ranking.

## v0.5 — the hostile self-review round

Instead of adding features first, this round re-read the whole codebase
looking for defects. Findings — and what each became:

| Finding | Resolution |
|---|---|
| **Windows guard-key mismatch (real bug, since v0.1)** | `canon()` used `canonicalize()`, which returns `\\?\`-prefixed verbatim paths for existing files but lexical paths otherwise. `write_file` computed its read-guard key *before* creating a file; `edit_file` computed it *after* — so edit-after-create always failed the guard on Windows. No earlier test covered create-then-edit; the new journal test caught it. Fixed by stripping the verbatim prefix (dunce-style) so keys are existence-independent; regression test added. |
| **web_fetch exfiltration gap** | `is_read_only=true` auto-allowed network egress in *every* mode — a prompt-injected model could exfiltrate repo data via URL with zero gating. New `Tool::requires_approval()` category: gated read-only tools stay plan-allowed (research) but prompt in ask/auto-edit like mutations; `-p` fails closed without an allow rule. |
| **`-p` without a task** | Silently fell into the REPL. Now a clear error. |
| **No connect timeout** | A hung TCP connect stalled the agent forever (retries only cover errors). 10s connect timeout on both provider clients. |
| **No undo** | New **edit journal**: every write_file/edit_file backs up the prior file state to `data_dir/checkpoints/<session>/` and records a journal entry (persisted in the session log, surviving resume). `/undo [n]` restores newest-first — backups copied back, agent-created files deleted. Verified end-to-end at the binary level across a session restart. Bash-driven changes are not journaled (documented limitation). |
| **Test gaps** | Added: thinking streamed but never persisted; Stop-hook forcing exactly one continuation (one-shot hook); web-fetch gating matrix; journal + undo round trip including replay semantics; canon key stability; plan-mode regression. |
| **Docs drift** | lib.rs module docs, README, AGENTS.md updated; CI workflow added (`.github/workflows/ci.yml`: Windows + Ubuntu, tests + clippy -D warnings + release build + offline smoke). |

Review items inspected and deliberately left alone: parallel-batch
snapshot semantics (inherent race between concurrent reads and sequential
writes — documented behavior), session FilesRead event growth (bounded by
file count), MCP strict per-server serialization (correctness over
throughput), AppContainer sandboxing (still roadmap — genuinely large).

## v0.6 — the ZCode round (reverse-engineered patterns)

Built by observing ZCode — the harness the building model runs inside —
from the model's seat (its tool contracts and prompt structure) and from
its on-disk artifacts (`~/.agents/skills`, plugin cache layout). The
interesting finding: ZCode independently converged on nearly the same core
ABI this harness designed (cat -n reads, exact-string edits, read-gating,
background bash, todos, subagents). What it has that we lacked:

| Ported | The ZCode behavior it came from |
|---|---|
| **Skills system** | SKILL.md packs with `---` frontmatter (`name`, `description`), discovered from the *same directories* (`~/.agents/skills` + workspace `.agents/skills`), progressive disclosure (name+description in the prompt, full body loaded by a `skill` tool on demand), and `/<name>` REPL invocation. Existing ZCode skills work in myharness unchanged. |
| **Query-focused web_fetch** | ZCode's WebFetch answers a `prompt` against a page using a small fast model instead of returning raw text. Ported: optional `prompt` parameter routes page text through `model_fast` and returns just the answer (errors tell the model to retry with the prompt omitted). |
| **Task agent_type presets** | ZCode subagents are typed (Explore = read-only, general-purpose = full). Ported as `agent_type`: `explore` (default, unchanged) / `build` (+ `bash`, `bash_output`) — the most common subagent want is "run the tests and tell me what broke", which read-only couldn't serve. |
| **Background subagents** | ZCode runs agents asynchronously with completion notifications. Ported onto the existing background-task registry: `task run_in_background=true` registers a `BgTask` and returns immediately; the subagent's final report lands in the task entry and `bash_output` delivers it — one polling contract for builds *and* research, no new tool. (v1 is poll-based; push notifications are the roadmap refinement.) |
| **Command files** | ZCode's `commands/*.md`: frontmatter (`description`, `argument-hint`) plus a prompt body with a `$ARGUMENTS` placeholder (format verified against a real plugin-cache command). Ported as `.agents/commands/*.md` (user `~/.agents/commands` + workspace, workspace wins): `/name args` expands the template into a turn; `/commands` lists. |
| **grep output_mode** | content / files / count — broad "where does X live" searches stop paying for line text they don't need. |
| **Prompt-layer guidance** | ZCode's context-management note (summarization is automatic; *keep working, don't wrap up early*) — a one-line addition that stops pre-emptive wind-downs before compaction; "final message must contain everything" sharpened; "don't re-read a file to verify an edit — the tool errors loudly on failure" (saves a round trip per edit). |

Deliberately NOT ported: the TUI (line-oriented ASCII stays), scheduled
automations, plugin marketplace (skills directories give the 90% case),
and ask-the-user structured questions (tools never prompt here — the
architecture routes prompts through the permission layer only).

## v0.7 — the roadmap round

Roadmap items that needed no external service, shipped:

| Feature | What it is |
|---|---|
| **`web_search`** | Keyless search via DuckDuckGo's HTML endpoint: query → ranked `title / url / snippet` results (≤8), urls unwrapped from the `uddg=` redirect, entities decoded, dupes dropped. Parser is fixture-tested plus an opt-in live test (`MYHARNESS_LIVE_DDG=<saved page>`, validated against the real endpoint); a layout change degrades to a loud error that points at web_fetch. Egress-gated exactly like web_fetch — a query string can carry data away too. Fills the "#6, needs a backend" roadmap slot without a key. |
| **Background completion notices** | The polling contract got its push-feel: at the top of every loop iteration the agent checks for finished-but-unheard background tasks (commands and subagents alike) and injects one dedup'd system notice ahead of the next request. No new mechanism — a message, persisted like any other, so resume history stays complete. The operator gets a hint line at turn end. |
| **`repo_map`** | The symbol-level repo map (roadmap #2, interim): per-language line-regex extraction (Rust, Python, JS/TS, Go, Java-like) of `kind name` definitions, files ranked by mtime (git-less proxy for Aider's commit ranking), minified/generated files skipped, 10k-char budget. One call answers "where is X defined"; grep/read_file still do the detail work. Tree-sitter stays on the roadmap for precision and call-graph ranking. |

## v0.8 — structured output (--output-schema)

`myharness -p --output-schema <inline-json|file> "task"` turns the final
message into a pipeline artifact: it must be JSON (bare or fenced) matching
a supported schema subset — `type`, `properties`, `required`, `items`,
`enum`, nested arbitrarily, validated by a hand-rolled checker whose errors
name the exact property path and mismatch (the model can act on them; a
`jsonschema` dependency bought nothing for this subset). The agent loop
gives an invalid final message up to two corrective rounds — the validation
error and the schema are pushed back as a system message — and the `-p`
layer exits 2 if the printed output still fails, so pipe consumers can
trust stdout and test exit codes.

## v0.9 — repo map v2 (dependency-ranked)

The map's file order now comes from a simplified PageRank (damping 0.85,
20 iterations) over the repository's import graph — Aider's core insight,
implemented git-less: imports are parsed per language (rust `use`, python
`import`/`from`, JS/TS relative imports + `require`, Go import blocks,
Java-like), resolved to files by module-path suffix matching, and files
rank most-referenced-first with mtime as the tiebreaker. The effect: the
files the codebase itself leans on (core types, shared utilities) surface
at the top of the budget instead of being buried by whatever was touched
last.

Deliberate scope call, documented: **no tree-sitter**. Five grammar crates
of C dependencies buy symbol-extraction precision the ranking doesn't
need; the reference graph is what makes a repo map useful. Tree-sitter
stays on the roadmap as an optional precision upgrade, not a ranking one.

## v0.10 — LSP diagnostics after edits

`[[lsp]]` config entries declare language servers; after any write/edit
turn the edited files are opened (didOpen, then full-text didChange) and
the errors/warnings the server publishes are injected as the next user
message — same feedback slot as `verify_cmd`, but per-file and usually
sub-second. The client (`lsp.rs`) is deliberately minimal: Content-Length
JSON-RPC framing, initialize handshake, lazy start on first relevant
edit, `kill_on_drop` so the server dies with the harness, and a
collect-with-timeout that stops as soon as every awaited file has been
heard from. Server crashes degrade to a warning and a skip, never a
failed turn. Verified end-to-end against `examples/mock-lsp-server.rs`
(the MCP mock-server pattern, applied to LSP).

```toml
[[lsp]]
name = "rust"
languages = ["rs"]
command = "rust-analyzer"
```

## v0.11 — JSON-RPC serve mode

`myharness serve` turns the harness into a line-framed JSON-RPC 2.0 service
over stdio — the Codex app-server pattern, one JSON object per line, no
Content-Length ceremony, so `jq` and netcat can drive it as easily as a
TUI can. Methods: `initialize`, `session.list`, `turn.run {input}`
(response `{final_text, interrupted}`), `turn.cancel`. While a turn runs,
the agent's UI layer emits `mh/turn.delta`, `mh/turn.thinking`,
`mh/tool.start`, `mh/tool.end`, `mh/info`, `mh/warn` notifications — the
existing Ui grew a third sink (stdout / quiet / channel) instead of a new
event bus, so REPL, `-p`, and serve share one code path.

Concurrency: requests are handled concurrently (`turn.cancel` must land
mid-turn) but turns serialize on the agent lock; notifications and
responses flow through one writer channel so stdout lines never tear.
Shutdown detail that cost an hour: the agent's channel-Ui owns a Sender,
so serve swaps the Ui out before draining the writer — otherwise the
process hangs on exit. Permission prompts fail closed (no human on this
UI); allow rules are the only way a gated tool runs.

## v0.12 — AppContainer sandboxing (the real filesystem isolation)

`sandbox = "appcontainer"` on Windows: every bash command runs inside a
dedicated `myharness-sandbox` AppContainer profile. Inside the container a
process can read what "ALL APPLICATION PACKAGES" ACEs allow (Windows and
Program Files trees), write ONLY the directories we granted an ACE for
(the workspace root and the system temp dir), and has **no network at all**
(no capabilities are requested — cargo fetch will fail loudly; that is the
point of a sandbox; use `job`/`strict` when a workflow needs egress).

Mechanics (`sandbox/appcontainer.rs`): the profile is created once and its
SID derived; write dirs get an appended ACE (existing DACL entries are
copied — nothing is replaced; the ACE persists for the session and is
harmless: that SID is only usable by processes launched inside the
container, i.e. by us); the child is created via raw `CreateProcessW` with
a `SECURITY_CAPABILITIES` thread attribute (std's Command cannot express
this), stdout/stderr pipes, NUL stdin, and it still joins the process-wide
Job Object. bash.rs now runs every shell through one `Spawned` abstraction
(tokio child or container child) with non-blocking `try_wait` polling —
one code path for foreground timeout/interrupt, background tasks, and both
sandbox modes. Verified live: writes outside the workspace are DENIED,
writes inside succeed, and the full tool round trip works with both cmd
and git-bash as the session shell. Known limits, documented: no container
networking, temp-dir writes are allowed (build tools need it), and
profile/ACL management is per-session best-effort.

## v0.13 — serve session attach/resume

The JSON-RPC server is no longer stateless-per-process: `session.attach
{id}` (prefix match; omitted = latest) replays an existing session into
the serve agent — messages, todos, files_read, cwd, journal, exactly the
CLI `resume` path — and `session.new` starts a fresh one explicitly;
`turn.run` responses now carry the `session_id` so thin clients can
persist it. Without either, the first turn still creates a session
implicitly. Attach detaches any current agent under the same lock that
turns serialize on, so the two can't interleave.

Two concurrency bugs found and fixed by the new multi-process test, both
worth remembering: (1) the agent-builder closure captured a notification
Sender clone that lived on serve's stack below `writer.await` — the
writer could never see the channel close, so the process hung on exit;
(2) the lazy-session ensure used two lock acquisitions, and shutdown
could swap the UI to quiet between them, silently eating the turn's
delta notifications. The fix is one lock held from ensure through run.
