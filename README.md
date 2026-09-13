# myharness

**A terminal coding-agent harness for GLM — designed and built autonomously by the same class of model that runs inside it, across 20+ research-driven rounds.**

One static Rust binary, no runtime dependencies, Windows-first. Point it at a repo, give it a task, watch it work — through a line-oriented REPL, a custom full-screen TUI with mid-turn steering, a headless pipeline mode with structured output, an autonomous overnight mode, or a JSON-RPC server.

```
$ myharness
myharness 0.18.0 | model glm-5.3 | mode ask
session 20260913-212234-494afa | /help for commands

mh> add a --verbose flag to the CLI
Parsing the CLI first...
  * read_file(src/cli.rs)
    ok        1	use clap::{Parser, Subcommand};
  * edit_file(src/cli.rs)
    ok  Replaced 1 occurrence(s) in src/cli.rs (now 64 lines)
  * bash(cargo check)
    ok  Exit code: 0
Done. `--verbose` is now accepted and threaded through the config.
-- turn 3 | 31.9k in (29.1k cache read) / 1.1k out (cumulative)
```

---

## How this project was created

The unusual part first. myharness was **written autonomously by a GLM-5.3 coding agent** — the same class of model the harness exists to serve. The human collaborator set direction, approved each round, and contributed one research project of his own ([zero-mem](#zero-token-long-term-memory-zero-mem), below); the model wrote every line of Rust, the design document, and the 161-test suite.

The method was research-driven. Every version round began from one of three sources:

1. **A literature survey.** The v0.1 core design came from studying what working harnesses had already converged on: Claude Code's tool contracts and permission semantics, OpenAI Codex CLI's Rust architecture and sandbox/approval orthogonality, OpenCode's client/server split, Aider's edit formats and tree-sitter repo map, Goose's compaction thresholds, Gemini CLI's documented failure modes. [DESIGN.md](DESIGN.md) records, for every feature, which harness taught it — and which anti-pattern each decision avoids.
2. **A self-audit.** Several rounds started by re-reading the whole codebase looking for defects (v0.5) or asking "what does each request *cost*?" (v0.14). These found real bugs: a Windows path-key mismatch that had broken create-then-edit since v0.1, and a system prompt that was invalidating the entire prompt cache on every `cd` or todo update.
3. **Reverse engineering.** The biggest rounds (v0.6, v0.15–v0.18) came from systematically mining other harnesses for ideas — the open-source ones (**Codex CLI, OpenCode, pi, Cline, Aider, Gemini CLI, Goose**) by reading their source, and commercial ones by studying their observable behavior and locally inspecting their distributed bundles. Every idea was **reimplemented from scratch** in Rust: no code copied, no prompts vendored. The research clones are deliberately outside this repository.

### What the research found

**Finding 1: serious harnesses independently converge on the same model-facing ABI.** `cat -n` file reads, exact-string edits with uniqueness-enforced loud errors, read-before-write gating, background bash, todo lists, subagents that return only a final report — every system that works at scale arrived at the same contracts, because these are the contracts coding models are trained on. That convergence *is* the design validation for this harness's v0.1.

**Finding 2: harness quality is contract quality.** The systems that feel best differ less in features than in the discipline of their model-facing surfaces — error strings that say exactly what to do next, outputs that can never eat the context window, deterministic tools, bounded everything. myharness treats its tool schemas and error strings as a versioned ABI for this reason (see [AGENTS.md](AGENTS.md)).

**Finding 3: the best individual ideas are scattered and mostly small.** The full provenance table of adopted ideas is in [DESIGN.md](DESIGN.md); the headline adoptions: pi's **steering** (Enter mid-turn injects after the tool batch), the user's own **zero-mem** (zero-LLM-call long-term memory), Cline's **plan-mode read-only command guard** and **stale-context tracking**, Aider's **repo-map budget math** and **reflect loop**, Goose's **turn-context block**, Codex's **actor-shaped TUI frontend**, and ZCode's **goal-loop** pattern (adapted for autonomous mode below).

### The benchmark

[BENCHMARK.md](BENCHMARK.md) has the full three-way comparison — the same three.js game prompt through ZCode, pi, and myharness. Measured: ZCode 843k input / 49.5k output / ~6.5k floor; pi 852k input-processed / 58.3k output / ~2.4k floor / $0.58; myharness **3.8k floor (41% smaller than ZCode, 58% larger than pi)** with a leaner tool surface. The honest reading: for long builds, conversation depth and output dominate; the harness's payload floor is second-order. For short interactions (the most common shape), myharness is **~1.7× cheaper per request**. The first-order budget lever is model choice (glm-4.7-air vs glm-5.3).

---

## What's in the harness

### The model-facing ABI: 17 tools

| Tool | Contract highlights |
|---|---|
| `read_file` | `cat -n` format, 2000-line default with offset/limit, images (png/jpg/gif/webp ≤4 MB) returned as vision blocks |
| `write_file` | Refuses to overwrite a file not read this session; creates parent dirs |
| `edit_file` | Exact-string replace; 0-match and N>1-match errors say exactly what to do; stale-context guard refuses if the file changed on disk since it was read |
| `bash` | Persistent cwd, timeouts with process-tree kills, background mode, sandboxing; oversized output spills whole to an artifact file — head+tail in context, `read_file` for the middle |
| `bash_output` | Poll background tasks (commands, subagents, monitors) |
| `glob` / `grep` / `ls` | Find-before-read; grep uses ripgrep's traversal engine with `content/files/count` output modes |
| `todo_write` | Full-list replacement; rides the message stream (cache-stable), folded into compaction handoffs |
| `task` | Subagents: `explore` (read-only) / `build` (+bash) presets, `tools` override, turn cap, no recursion, background mode, parallel batches |
| `web_fetch` | HTML→text, query-focused extraction via `model_fast`, SSRF-guarded (credentials denied, private/loopback denied, redirects returned not followed) |
| `web_search` | Keyless DuckDuckGo; egress-gated |
| `repo_map` | PageRank over the import graph; files-in-context boosted, important files pinned |
| `skill` | Loads SKILL.md packs on demand |
| `session_recall` | Search every past session's transcript |
| `monitor` | Run a command on a schedule until its output matches (CI watch, log tails) |
| + MCP | `[[mcp]]` stdio servers bridge as `mcp__<server>__<tool>` |

### The turn loop

```
user input
  → zero-mem injection (identity + past-session evidence, zero LLM calls)
  → turn-context block (time · cwd · turn budget · context headroom)
  → compaction check: spill oversized results → summarize → handoff
  → stream (watchdog: 180s stall = loud failure, not hang)
  → stop_reason=length with tool calls? → fail them (truncated args never execute)
  → permission-check each call (hooks first; failing hook = fail-closed)
    → plan mode: read-only command allowlist parser
    → doom-loop gate: same call failing 3× in a row = refused
  → parallel fan-out of concurrency-safe tools; state-touching in order
  → results + output hints + JIT AGENTS.md + steering merge into the stream
  → verify_cmd / LSP diagnostics / script syntax gate (node --check) after edits
    → any failure → reflect loop sends the model back (≤3 rounds)
  → no tool calls? capture to memory, diffstat, footer — done
  → hard iteration cap (4× turn cap): nothing can spin forever
```

### Context management

- 80%-of-window compaction threshold; structured handoff brief via `model_fast`; tool_use/tool_result pair-safe boundaries; `compaction` event persisted for identical resume.
- **Spill pre-pass**: tool results over 24k chars are written to session artifacts and replaced with pointers *before* summarizing.
- **Refill guard**: two auto-compactions in a row that refill within 8 tool results pause auto-compaction and tell the model which result is too large.
- **Turn-context block** (past ~32k tokens): time, cwd, `turns_taken/max_turns`, context headroom.

### Permissions and sandboxing

- Modes `plan / ask / auto-edit / yolo`; deny→allow→prompt rule engine with glob patterns; compound-command decomposition; non-interactive fails closed.
- **Plan mode runs research commands**: quote-aware allowlist parser (`ls`, `cat`, `grep`, `find`, `git status/diff/log`, `gh pr list`, bare-GET `gh api /path`, …) — redirection, substitution, heredocs, and unknown interpreters deny. 13 sneaky bypass forms tested and blocked.
- **Workspace write-scoping** in unattended modes: `write_file`/`edit_file` traversal (`sub/../../escape.txt`, `../victim.txt`) blocked.
- Hooks (`PreToolUse`/`PostToolUse`/`Stop`): JSON on stdin, exit 2 blocks; failing hook fails closed.
- Sandboxing: Windows Job Object (default), Windows **AppContainer** (filesystem isolation + no network), Linux **Landlock**.
- SSRF guard: hex IP (`0x7f000001`), decimal (`2130706433`), octal (`0177.0.0.1`), v4-mapped v6, credentials in URL — all denied.

### Script syntax gate

Every written `.js`/`.mjs`/`.ts`/`.html` goes through `node --check` on a temp module copy (string/comment/template-aware lexical fallback when node is absent). Import maps (JSON, not JS) and external `src=` scripts skipped. A syntax error **cannot end a turn unfixed** — the reflect loop makes fixing it a precondition.

### Zero-token long-term memory (zero-mem)

A Rust port of [zero-mem-pi](https://github.com/woolcoxm/zero-mem-pi) — this repo author's own extension for the pi agent, implementing [Zero-Mem, arXiv:2607.29377](https://arxiv.org/abs/2607.29377). **Memory operations never call the LLM.** Passive capture; deterministic retrieval (BM25 + entity-graph Personalized PageRank, query-conditioned routing, pool-confidence gating, evidence closure); sticky identity slot (poison filter for default model names); atomic persistence with cross-process merge; retention bounds; sanitized snippets. Live-verified: a fresh session answers stored facts and the user's name entirely from injected memory. `/memory`, `/memory search <q>`, `/memory clear`.

### Frontends (one engine, six surfaces)

| Surface | What it is |
|---|---|
| REPL | Line-oriented ASCII; rustyline history; all slash commands |
| `myharness tui` | Custom ratatui TUI: streaming transcript, diff previews, denied≠failed states, thinking collapse, context bar, permission modals with two-stage "always", ctrl+r history search, ctrl+p command palette, **steering** (Enter mid-turn → injected after the tool batch), ESC interrupt |
| `myharness -p "task"` | Headless: final answer to stdout, usage line to stderr; `--output-schema` validates JSON with corrective rounds and exit-code discipline |
| `myharness -p "task" --autonomous` | **Overnight mode**: work until verified done — the model stopping triggers a self-check prompt (run the tests? read back every file? address everything?); two consecutive "GOAL COMPLETE" confirmations complete; any tool call during a self-check means found more work and continues. Bounded by `--budget-hours` (default 8) and `--budget-tokens` (default 2M). |
| `myharness serve` | JSON-RPC 2.0 over stdio (`initialize`, `session.attach/new`, `turn.run`, `turn.cancel`, `mh/*` notifications) |
| `myharness -p "task" --yolo` | No permission prompts (CI, scripting) |

### Persistence

Append-only JSONL event-sourced sessions; resume rebuilds messages, todos, read-guard set with staleness fingerprints, cwd, edit journal; every state change is an event. `/undo` restores journaled edits; per-file `+/-` diffstat per turn; sessions searchable via `session_recall`. API keys from environment only, never logged.

---

## Getting started

```bash
cargo build --release
export ZAI_API_KEY=sk-...
./target/release/myharness              # REPL
./target/release/myharness tui          # full-screen TUI
./target/release/myharness -p "fix it" --yolo
./target/release/myharness -p "build and test" --autonomous --budget-hours 4
./target/release/myharness -p "summarize deps" --output-schema schema.json
./target/release/myharness sessions
./target/release/myharness resume
```

Fully offline via the scripted mock provider: `--provider mock` with `MYHARNESS_MOCK_FILE=examples/smoke-script.json`.

### Configuration (`myharness.toml`)

```toml
[model]
provider = "anthropic"       # anthropic | openai | mock
name = "glm-5.3"
context_window = 200000
prompt_caching = true
# thinking_budget = 8192     # reasoning tokens per request
# reasoning_effort = "high"  # openai protocol
# model_fast = "glm-4.7-air" # cheaper model for compaction/summaries

[agent]
max_turns = 80
compact_ratio = 0.8
# verify_cmd = "cargo check" # after edit turns; failures trigger reflect loop
restrict_writes_to_workspace = true

[bash]
timeout_ms = 120000
sandbox = "job"              # windows: job | appcontainer; linux: strict | off

[web]
private_hosts = false        # allow web_fetch to reach localhost/private

[zero_mem]
enabled = true
top_k = 3
max_units = 5000
max_age_days = 180

# [[output_hint]] pattern = "API rate limit exceeded" hint = "check gh api rate_limit..."
# [[hooks]] event = "PreToolUse" tool = "bash" command = "my-guard.cmd"
# [[mcp]] name = "github" command = "npx" args = ["-y", "@modelcontextprotocol/server-github"]
# [[lsp]] name = "rust" languages = ["rs"] command = "rust-analyzer"
# [[permissions.allow]] tool = "bash" pattern = "cargo *"
```

`myharness config` prints the effective config plus the full annotated template.

### Skills and command files

SKILL.md packs discovered from `~/.agents/skills/<name>/SKILL.md` (user) and `.agents/skills/<name>/SKILL.md` (workspace, wins collisions) — the same locations pi-class harnesses read. Slash-command templates: `.agents/commands/*.md` with `$ARGUMENTS` expansion.

---

## Running on a token budget

Measured floor: ~3.8k tokens (system prompt + 17 tool schemas), pinned by a regression test. The levers, in order of magnitude:

1. **Model choice** (glm-4.7-air vs glm-5.3 — dwarfs everything else)
2. **`model_fast`** for compaction and web_fetch extraction
3. **Prompt caching** (byte-stable prefix; cache reads typically ~10% of fresh input)
4. **Output-token discipline** (v0.14's prompt round: grep files/count, offset/limit reads, repo_map)
5. **Thinking budget** (`[model] thinking_budget`) — the biggest per-request output lever
6. **Compaction** at 80% (drop to 0.6 to compact earlier)
7. **Subagents** keep exploration out of the cached thread; **session_recall** + **zero-mem** answer from memory instead of re-deriving

Economy preset in `myharness config`'s template. `-p` prints usage to stderr.

---

## Testing

**161 automated tests**, zero network (scripted mock provider):

| Suite | Tests | What it covers |
|---|---|---|
| Unit | 97 | Per-module: entity extraction, BM25 ranking, PPR, identity derivation, path resolution, config parsing, prompt budget, permissions, wrap, fingerprints, URI round-trips |
| Integration | 52 | End-to-end agent loop: tool execution, permission matrices, compaction + spill, steering, doom-loop, stale edits, session recall, memory injection, reflect loop, TUI frame render |
| Adversarial | 9 | Break-in attempts: sandbox traversal, permission smuggling, plan-mode bypass, SSRF encodings, hostile inputs, torn sessions, JS fuzz, output bounds |
| Concurrency | 1 | Two processes sharing one memory store don't lose units |
| Collision | 1 | Unit-id uniqueness across concurrent sessions |

Plus binary smoke tests with `--provider mock`, and a prompt-budget regression test that pins the model-facing payload floor.

`cargo test` and `cargo clippy --all-targets` are clean. CI (`.github/workflows/ci.yml`) runs the same on Windows and Ubuntu, including an offline smoke of the built binary.

---

## Development

Architecture and the reasoning behind every decision: [DESIGN.md](DESIGN.md) — including the v0.5 hostile self-review, the reverse-engineering rounds, and the honest "deliberately not built" sections. [AGENTS.md](AGENTS.md) is the contract for coding agents contributing to this repo.

## Known limitations

- Bash-driven file changes are not journaled — `/undo` covers write_file/edit_file only.
- Linux `strict` sandbox is compile-verified but not runtime-tested here.
- Images display on the Anthropic protocol; OpenAI degrades to a text note.
- Reasoning output is display-only.
- Background tasks live in memory — they don't survive a restart.
- MCP tools are called one-at-a-time per server.
- Resuming a session opens a fresh TUI transcript.
- Autonomous mode runs in-process — close the terminal and it stops.

## Roadmap

Native-scrollback inline TUI rendering, session-tree event format, SQLite session index, dense embeddings for zero-mem, cross-project memory federation.

## License

MIT — see [LICENSE](LICENSE).
