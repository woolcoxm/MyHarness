# myharness

**A terminal coding-agent harness for GLM — designed and built autonomously by the same class of model that runs inside it, across 18 research-driven rounds.**

One static Rust binary, no runtime dependencies, Windows-first. Point it at a repo, give it a task, watch it work — through a line-oriented REPL, a custom full-screen TUI with mid-turn steering, a headless pipeline mode with structured output, or a JSON-RPC server mode.

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

The unusual part first. myharness was **written autonomously by a GLM-5.3 coding agent** — the same class of model the harness exists to serve. The human collaborator set direction, approved each round, and contributed one research project of his own ([zero-mem](#zero-token-long-term-memory-zero-mem), below); the model wrote every line of Rust, the design document, and the 141-test suite.

The method was research-driven. Every version round began from one of three sources:

1. **A literature survey.** The v0.1 core design came from studying what working harnesses had already converged on: Claude Code's tool contracts and permission semantics, OpenAI Codex CLI's Rust architecture and sandbox/approval orthogonality, OpenCode's client/server split, Aider's edit formats and tree-sitter repo map, Goose's compaction thresholds, Gemini CLI's documented failure modes. [DESIGN.md](DESIGN.md) records, for every feature, which harness taught it — and which anti-pattern each decision avoids.
2. **A self-audit.** Several rounds started by re-reading the whole codebase looking for defects (v0.5) or asking "what does each request *cost*?" (v0.14). These found real bugs: a Windows path-key mismatch that had broken create-then-edit since v0.1, and a system prompt that was invalidating the entire prompt cache on every `cd` or todo update.
3. **Reverse engineering.** The biggest rounds (v0.6, v0.15–v0.18) came from systematically mining other harnesses for ideas — the open-source ones (**Codex CLI, OpenCode, pi, Cline, Aider, Gemini CLI, Goose**) by reading their source, and commercial ones by studying their observable behavior and locally inspecting their distributed bundles. Every idea was **reimplemented from scratch** in Rust: no code copied, no prompts vendored. The research clones are deliberately outside this repository.

### What the research found

**Finding 1: serious harnesses independently converge on the same model-facing ABI.** `cat -n` file reads, exact-string edits with uniqueness-enforced loud errors, read-before-write gating, background bash, todo lists, subagents that return only a final report — every system that works at scale arrived at the same contracts, because these are the contracts coding models are trained on. That convergence *is* the design validation for this harness's v0.1.

**Finding 2: harness quality is contract quality.** The systems that feel best differ less in features than in the discipline of their model-facing surfaces — error strings that say exactly what to do next, outputs that can never eat the context window, deterministic tools, bounded everything. myharness treats its tool schemas and error strings as a versioned ABI for this reason (see [AGENTS.md](AGENTS.md)).

**Finding 3: the best individual ideas are scattered and mostly small.** What was adopted, and from where:

| Idea | Source | myharness |
|---|---|---|
| Exact-string edits, uniqueness errors, read-gating | Claude Code | v0.1 core contracts |
| Byte-stable system prompt (volatile state rides messages, never the prompt) | own audit, confirmed in every studied codebase | v0.14 |
| **Steering** — Enter mid-turn queues a message the agent injects after the current tool batch (or uses to revive a finished turn) | pi | v0.16 |
| **Zero-token long-term memory** — BM25 + entity-graph PageRank retrieval, identity slot, zero LLM calls for memory ops | [zero-mem-pi](https://github.com/woolcoxm/zero-mem-pi) (this repo's author), after [Zero-Mem, arXiv:2607.29377](https://arxiv.org/abs/2607.29377) | v0.18 |
| Compaction refill guard — context refilling too fast means one result is too large; stop summarizing the summaries | commercial harness (bundle study) | v0.15 |
| Oversized results spill to files the model can re-read — truncation becomes lossless | same | v0.15 |
| SSRF destination guard on web tools | same | v0.15 |
| Plan-mode read-only command execution (quote-aware allowlist parser) | Cline | v0.17 |
| Tool calls from a length-truncated message fail, never execute (truncated arguments) | pi | v0.17 |
| Turn-context block: time, cwd, turn budget, context headroom per turn | Goose | v0.17 |
| Verify-failure reflect loop (failing `verify_cmd` sends the model back, bounded) | Aider | v0.17 |
| Doom-loop gate (the identical failing call is refused on the 4th try) | OpenCode | v0.17 |
| Stale-context guard (externally edited files must be re-read first) | Cline | v0.17 |
| Cross-session recall (search every past transcript) | Goose | v0.17 |
| Monitor tool (command on a schedule until it matches) | Claude Code | v0.17 |
| JIT instruction loading (subdirectory AGENTS.md arrives when touched) | Gemini CLI | v0.17 |
| Repo-map ranking: files-in-context boosted, important files pinned | Aider | v0.17 |
| Denied ≠ failed rendering; diff previews; thinking collapse | OpenCode / pi | v0.16 TUI |
| Actor-shaped frontend (worker owns the agent, TUI owns the terminal) | Codex CLI | v0.16 TUI |
| Two-tier capture: prompt + final answer as memory trace units | zero-mem paper | v0.18 |

Ideas evaluated and deliberately **not** built — with reasons — are recorded in DESIGN.md's "Deliberately not built" sections (native-scrollback TUI rendering, session-tree event format, SQLite session index, LLM permission classifier, embedded-JS code-act tool, dense embeddings for memory).

---

## Install & run

Requires a Rust toolchain (rustup, MSVC on Windows). Then:

```
cargo build --release
# binary at target\release\myharness.exe (copy it anywhere)
```

Set an API key (picked up in this order):

| Variable | Used by |
|---|---|
| `MYHARNESS_API_KEY` | always wins, any provider |
| `ZAI_API_KEY` | both providers (recommended for GLM) |
| `GLM_API_KEY` | both providers |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` | respective providers |

**Easiest:** create a `.env` in the repo root (a template with comments is
included) — the harness loads the nearest `.env` automatically at startup.
Variables already set in your real environment always win over the file.
Keep `.env` out of git (already in `.gitignore`); keys are only ever read
from the environment and never logged or written to transcripts.

Defaults target Z.ai's Anthropic-compatible endpoint with model `glm-5.3`.
For the OpenAI-compatible endpoint: `--provider openai` (base URL
`https://api.z.ai/api/paas/v4`), or any provider via `--base-url`.

## Everyday use

```
myharness                          # interactive REPL
myharness tui                      # full-screen TUI (steering, palette, modals)
myharness "fix the build"          # start with a task, stay interactive
myharness -p "run the tests"       # non-interactive: print final answer, exit
myharness -p --output-schema s.json "summarize deps as JSON"
myharness -c                       # resume the most recent session
myharness resume <id-prefix>       # resume a specific session
myharness sessions                 # list sessions
myharness serve                    # JSON-RPC 2.0 over stdio for thin clients
myharness config                   # effective config + a sample myharness.toml
```

### Permission modes

| Mode | Reads | Edits | bash |
|---|---|---|---|
| `plan` | allowed | **blocked** (present a plan) | **read-only commands only** (see below) |
| `ask` (REPL default) | allowed | prompted | prompted |
| `auto-edit` (`-p` default) | allowed | allowed | allow-rules only, else denied |
| `yolo` (`--yolo`) | allowed | allowed | allowed |

Prompts answer `y` (once), `a` (always for this pattern), `n` (deny). Deny
rules always win, in every mode, and compound commands are checked per
subcommand (`ls && rm -rf /` cannot ride an `ls` allow rule).

**Plan mode runs research commands:** `ls`, `cat`, `grep`, `find`,
`git status/diff/log`, `gh pr list` and friends execute in plan mode via a
quote-aware allowlist parser — redirection, command substitution, heredocs,
and unknown interpreters deny. Research no longer needs a mode switch.

### The tools the model gets

`read_file` (cat -n format, **plus images** — png/jpg/gif/webp come back as
pictures the model can see), `write_file` (no blind overwrites), `edit_file`
(exact unique match or `replace_all`; refuses if the file changed on disk
since it was read — the stale-context guard), `bash` (**background mode**,
persistent cwd, timeouts, process-tree kills; oversized output spills whole
to a file the model can `read_file` — truncation is lossless),
`bash_output` (poll background tasks — commands, subagents, and monitors),
`glob`, `grep` (gitignore-aware; `output_mode` = content / files / count),
`ls`, `todo_write` (the list rides the message stream and survives
compaction), `skill` (load SKILL.md packs), `web_fetch` (HTML→text, or pass
`prompt` to get a question answered against the page by the cheap model),
`web_search` (keyless DuckDuckGo), `repo_map` (symbol digest ranked by
PageRank over the import graph; files-in-context boosted, Cargo.toml/
Makefile/README pinned), `task` (subagents — `explore` read-only / `build`
with bash — returning only their final report, synchronous or detached),
`session_recall` (search every past session's transcript), and `monitor`
(run a command on a schedule until its output matches — CI watch, log
tails). **Independent read-only calls in one message run in parallel.**

Plus MCP: `[[mcp]]` stdio servers bridge as `mcp__<name>__<tool>`, gated by
the normal permission engine.

### The loop, briefly

Each turn: zero-mem injection (past-session evidence + identity, zero LLM
calls) → turn-context block (time, cwd, turn budget, context headroom) →
compaction check (spill oversized results, then summarize into a handoff) →
streamed request with a cache-stable prompt → tool calls permission-checked
(hooks first; a failing hook fails closed) and fanned out → results, output
hints, JIT instructions, and steering messages merge into the stream →
`verify_cmd` / LSP diagnostics after edits (failures trigger a bounded
reflect loop) → capture the turn to memory, print a per-file `+/-` diffstat.
Tool calls whose message hit the output-length limit fail loudly instead of
executing truncated arguments; the same call failing three times in a row
is refused by the doom-loop gate.

### Zero-token long-term memory (zero-mem)

A Rust port of [zero-mem-pi](https://github.com/woolcoxm/zero-mem-pi) — this
repo author's own extension for the pi agent, implementing
[Zero-Mem (arXiv:2607.29377)](https://arxiv.org/abs/2607.29377). **Memory
operations never call the LLM.** Every turn's prompt and final answer are
captured passively to a per-project store; at each new turn, deterministic
retrieval — BM25 fused with an entity–context graph scored by Personalized
PageRank, query-conditioned routing, min-max normalization, pool-confidence
gating ("no memory beats confusing memory"), evidence closure — injects up
to three snippets under a not-authoritative header. A **sticky identity
slot** (derived from naming statements, with a poison filter for default
model names) means a new session never starts not knowing who you are.
Atomic persistence, cross-process merge, retention bounds.
`/memory`, `/memory search <q>`, `/memory clear`; configured via `[zero_mem]`.

### Full-screen TUI

`myharness tui` — one column, quiet chrome: streaming transcript with diff
previews at tool start, denied-vs-failed tool states, thinking that streams
then collapses to one line, a context bar (`ctx N%`, yellow at 75%, red at
90%), status verbs, permission modals with two-stage "always allow" (shows
the exact pattern before recording it), **ctrl+r history search**,
**ctrl+p command palette** over every slash command and skill, and
**steering**: press Enter while the agent works and your message is
injected after the current tool batch (or revives a finished turn) instead
of waiting. ESC interrupts; every REPL slash command works unchanged. The
plain REPL stays the default (pipes, CI, every console); `NO_COLOR` is
respected.

### Skills and command files

SKILL.md packs discovered from `~/.agents/skills/<name>/SKILL.md` (user) and
`.agents/skills/<name>/SKILL.md` (workspace, wins collisions) — the same
locations pi-class harnesses read, so existing skills work in both. Only
the name + description list rides the system prompt (budgeted); the model
loads the body through the `skill` tool when a task matches. Slash-command
templates live in `.agents/commands/*.md` with `$ARGUMENTS` expansion.

## Configuration

Drop a `myharness.toml` in the repo root (discovered upward) or the user
config dir. `myharness config` prints the effective config plus the full
annotated template. Highlights:

```toml
[model]
provider = "anthropic"   # anthropic | openai | mock
name = "glm-5.3"
context_window = 200000
prompt_caching = true    # anthropic-protocol cache breakpoints
# model_fast = "glm-4.7-air"  # cheaper model for compaction summaries

[agent]
compact_ratio = 0.8      # auto-compact at 80% of the window
max_turns = 80
# verify_cmd = "cargo check"   # after edit turns; failures trigger the reflect loop
restrict_writes_to_workspace = true

[bash]
timeout_ms = 120000
shell = "auto"           # auto | bash | powershell | cmd
sandbox = "job"          # windows: job | appcontainer | off; linux: strict | off

[web]
private_hosts = false    # allow web_fetch to reach localhost/private (dev servers)

[zero_mem]
enabled = true           # zero-LLM long-term memory
top_k = 3
max_units = 5000
max_age_days = 180

[[permissions.allow]]
tool = "bash"
pattern = "cargo *"

[[permissions.deny]]
tool = "write_file"
pattern = "*.env"
```

Also: `[[hooks]]` (`PreToolUse`/`PostToolUse`/`Stop`; exit 2 blocks; a
failing hook fails closed), `[[mcp]]` servers, `[[lsp]]` language servers
(edited files get real diagnostics injected after the edit round), and
`[[output_hint]]` pattern→guidance rules fired against tool results.

> Windows note: use single-quoted TOML strings for paths —
> `command = 'C:\tools\server.exe'` — because `\U` in double quotes is a
> TOML unicode escape.

**Project instructions:** an `AGENTS.md` at the repo root is injected into
the system prompt (nearest upward, 8k cap) along with a repository-layout
digest; subdirectory `AGENTS.md` files arrive just-in-time when the model
starts working there.

**Undo:** every `write_file`/`edit_file` journals the prior state; `/undo
[n]` restores it, surviving restarts. Each turn ends with a per-file
`+12 -8` diffstat computed from the journal.

**Sandboxing:** Windows Job Objects (default — command trees die with the
harness), Windows **AppContainer** (real filesystem isolation: writes only
to workspace + temp, no network inside the container), Linux **Landlock**
(write-scoped, opt-in).

**Network egress is gated and guarded:** `web_fetch` prompts in ask mode
(fails closed in `-p` without an allow rule); destinations are SSRF-guarded
(credentials denied; loopback/private/link-local hosts denied unless
`[web] private_hosts = true`); redirects are returned, not followed.

**Server mode:** `myharness serve` is line-framed JSON-RPC 2.0 over stdio —
`initialize`, `session.list`, `session.attach`/`session.new`, `turn.run`
(responses carry the `session_id`), `turn.cancel`, with `mh/*` notifications
streaming during turns.

**Structured output:** `myharness -p --output-schema '<json or path>' "task"`
requires the final message to be JSON matching the schema (subset: `type`,
`properties`, `required`, `items`, `enum`). Invalid output gets two
corrective rounds in-loop; still-failing output prints and exits non-zero —
stdout stays trustworthy for pipelines.

## Offline / CI testing

The `mock` provider replays scripted conversations
(`MYHARNESS_MOCK_FILE=examples/smoke-script.json`), so the whole harness is
testable with no network and no keys:

```
cargo test        # 141 tests: unit + end-to-end on the mock provider
cargo clippy      # clean, all targets
cargo build --release --bins --examples
```

CI (`.github/workflows/ci.yml`) runs the same on Windows and Ubuntu,
including an offline smoke of the built binary.

## Running on a token budget

Measured floor: the model-facing payload (system prompt + 16 tool schemas)
is **~3.8k tokens** — pinned by a regression test so it can only grow
deliberately. From there, cost is driven by conversation depth, and the
harness attacks that on every axis:

- **Prompt caching** is on by default: the system prompt, tools, and
  history prefix are cache-stable (volatile state rides messages), so
  repeat requests bill `cache_read_tokens` instead of full input — watch
  the ratio in the turn footer or `/usage`; cache reads typically cost a
  fraction of fresh input.
- **`model_fast`**: compaction summaries and web_fetch extraction go to a
  cheaper model (`glm-4.7-air` recommended).
- **Cheap reading patterns are trained into the prompt**: grep
  `files`/`count` modes, `read_file` offset/limit, `repo_map` instead of
  file crawls — the v0.14 round specifically attacked output tokens (the
  most expensive class).
- **Lossless spill**: oversized outputs aren't re-fetched — the model
  `read_file`s just the slice it needs.
- **Compaction** at 80% of the window (drop to 0.6 to compact earlier and
  keep average requests smaller), with oversized results spilled to
  artifacts first so summaries stay small.
- **Subagents** keep exploration noise out of the main (cached) thread;
  **`session_recall`** answers "how did we do X" from past sessions
  instead of re-deriving it; **zero-mem** injects ~100 tokens of relevant
  past context instead of whole transcripts.
- **Thinking is never fed back** into context (display-only), and the
  skills list is budgeted.

Economy preset (uncomment in `myharness config`'s template): main model
`glm-4.7-air`, `model_fast` the same, `compact_ratio = 0.6`. `-p` mode
prints a usage line (requests, in/out, cache read/write) to **stderr** so
pipelines stay clean while you watch spend.

## Development

Architecture and the reasoning behind every decision: [DESIGN.md](DESIGN.md)
— including the v0.5 hostile self-review (which found a real Windows
guard-key bug that had survived since v0.1), the reverse-engineering
rounds, and the honest "deliberately not built" sections.
[AGENTS.md](AGENTS.md) is the contract for coding agents contributing to
this repo (tool-ABI stability, ASCII UI, Windows-first, offline tests).

## Known limitations

- Bash-driven file changes are not journaled — `/undo` covers
  write_file/edit_file only.
- Linux `strict` sandbox is compile-verified but not runtime-tested here
  (needs a Linux host with kernel 5.13+); opt-in for that reason.
- Images in tool results display on the Anthropic-compatible protocol (the
  default); the OpenAI-compatible protocol degrades them to a text note.
- Reasoning output is display-only (never fed back — providers reject it on
  input).
- Background tasks live in memory — they don't survive a harness restart.
- MCP tools are called strictly one-at-a-time per server (no pipelining).
- Resuming a session opens a fresh TUI transcript (history lives in the
  session file; re-rendering it is roadmap).
- zero-mem is lexical (BM25 + entity graph) — no dense embeddings ship in
  the binary; that's the documented upgrade path.

## Roadmap

Native-scrollback inline TUI rendering, a session-tree event format
(in-place branching), a SQLite index over sessions, optional dense
embeddings for zero-mem, cross-project memory federation, tree-sitter
symbol precision as an optional repo-map upgrade.

## License

MIT — see [LICENSE](LICENSE).
