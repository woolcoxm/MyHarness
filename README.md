# myharness

A terminal coding-agent harness for GLM, written in Rust. One static binary,
no runtime dependencies. Point it at a repo, give it a task, watch it work.

```
$ set ZAI_API_KEY=sk-...
$ myharness
myharness 0.1.0 | model glm-5.3 | mode ask
session 20260907-212234-494afa | /help for commands

mh> add a --verbose flag to the CLI
Parsing the CLI first...
  * read_file(src/cli.rs)
    ok        1	use clap::{Parser, Subcommand};
  * edit_file(src/cli.rs)
    ok  Replaced 1 occurrence(s) in src/cli.rs (now 64 lines)
  * bash(cargo check)
    ok  Exit code: 0
Done. `--verbose` is now accepted and threaded through the config.
-- turn 3 | 7.2k in / 1.1k out (cumulative)
```

## Install & run

Requires a Rust toolchain (rustup, MSVC on Windows). Then:

```
cargo build --release
# binary at target\release\myharness.exe (copy it anywhere)
```

Set an API key (picked up in this order):

| Variable | Used by |
|---|---|
| `ZAI_API_KEY` | both providers (recommended for GLM) |
| `GLM_API_KEY` | both providers |
| `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` | respective providers |
| `MYHARNESS_API_KEY` | always wins, any provider |

**Easiest:** create a `.env` in the repo root (a template with comments is
included) — the harness loads the nearest `.env` automatically at startup.
Variables already set in your real environment always win over the file.
Keep `.env` out of git (already in `.gitignore`).

Defaults target Z.ai's Anthropic-compatible endpoint with model `glm-5.3`.
For the OpenAI-compatible endpoint: `--provider openai` (base URL
`https://api.z.ai/api/paas/v4`), or any other provider via `--base-url`.

## Everyday use

```
myharness                          # interactive REPL
myharness "fix the build"          # start with a task, stay interactive
myharness -p "run the tests"       # non-interactive: print final answer, exit
myharness -c                       # resume the most recent session
myharness resume <id-prefix>       # resume a specific session
myharness sessions                 # list sessions
myharness config                   # effective config + a sample myharness.toml
```

### Permission modes

| Mode | Reads | Edits | bash |
|---|---|---|---|
| `plan` | allowed | **blocked** (present a plan instead) | blocked |
| `ask` (REPL default) | allowed | prompted | prompted |
| `auto-edit` (`-p` default) | allowed | allowed | allow-rules only, else denied |
| `yolo` (`--yolo`) | allowed | allowed | allowed |

Prompts answer `y` (once), `a` (always for this pattern), `n` (deny). Deny
rules in config always win, in every mode, and compound commands are checked
per subcommand (`ls && rm -rf /` cannot ride an `ls` allow rule).

### In the REPL

`/help /quit /clear /compact /mode [plan|ask|auto-edit|yolo] /model [name]
/todos /usage /info /session` — plus Ctrl-C once to interrupt the running
turn, twice to quit.

### Tools the model gets

`read_file` (cat -n format, **plus images** — png/jpg/gif/webp come back as
pictures the model can see), `write_file` (no blind overwrites), `edit_file`
(exact unique match or replace_all), `bash` (**background mode** with
`run_in_background`, persistent cwd, timeouts, head+tail truncation),
`bash_output` (poll background tasks — commands **and** subagents), `glob`,
`grep` (gitignore-aware; `output_mode` = content / files / count), `ls`,
`web_fetch` (docs/references from http(s) URLs; pass `prompt` to get a
question answered against the page by the cheap model instead of the raw
text), `todo_write` (task list injected into every prompt), `skill` (load a
SKILL.md instruction pack), `web_search` (keyless DuckDuckGo-backed ranked
results), `repo_map` (symbol-level "where is X defined" digest,
ranked by import references then recency), and `task` (subagents with `agent_type` presets
— `explore` for read-only research, `build` to also run bash — that return
only their final report, synchronously or detached via
`run_in_background`). Finished background tasks announce themselves to the
model at the next opportunity.

**Independent read-only calls in one message run in parallel** — batch your
reads, searches, fetches, and subagents; edits and bash still run in order.

### Skills (SKILL.md packs)

Reusable instruction packs discovered from `~/.agents/skills/<name>/SKILL.md`
(user) and `.agents/skills/<name>/SKILL.md` (workspace, wins on name
collisions) — the same locations ZCode-class harnesses read, so your existing
skills work in both. A `SKILL.md` is `---` frontmatter (`name`,
`description`) plus a markdown body. Only the name + description list rides
the system prompt; the model loads the full body through the `skill` tool when
a task matches, and `/name [args]` in the REPL invokes one directly.
`MYHARNESS_SKILLS_DIR` overrides the user directory.

### Slash-command files

Drop markdown files into `.agents/commands/` (workspace) or
`~/.agents/commands/` (user) to define `/name [args]` commands — the format
ZCode-class harnesses use:

```markdown
---
description: Run the full verification loop.
argument-hint: "[what to verify]"
---

Verify the project end to end: $ARGUMENTS
Run the build, then the tests; report failures with file:line references.
```

`$ARGUMENTS` is replaced with whatever follows the command (appended at the
end when the template omits it); `/commands` lists what's discovered.

## Configuration

Drop a `myharness.toml` in the repo root (discovered upward) or the user
config dir. `myharness config` prints the full annotated sample. Highlights:

```toml
[model]
provider = "anthropic"   # anthropic | openai | mock
name = "glm-5.3"
context_window = 200000
prompt_caching = true    # anthropic-protocol cache breakpoints (cheaper long sessions)
# model_fast = "glm-4.7-air"  # cheaper model for compaction summaries

[agent]
compact_ratio = 0.8      # auto-compact when the request exceeds 80% of window
max_turns = 80
# verify_cmd = "cargo check"   # runs after edit turns; the model sees the result
restrict_writes_to_workspace = true  # deny file writes outside the repo in unattended modes

[[permissions.allow]]
tool = "bash"
pattern = "cargo *"

[[permissions.deny]]
tool = "write_file"
pattern = "*.env"
```

**Project instructions:** drop an `AGENTS.md` in the repo root — it's
injected into the system prompt (nearest file upward, capped at 8k chars),
along with a repository-layout digest, so per-project conventions and
orientation reach the model automatically.

**Reasoning:** GLM thinking output (`reasoning_content` / `thinking_delta`)
is streamed to your terminal with a `~` prefix while the model works.

**Undo:** every `write_file`/`edit_file` backs up the prior file state
before changing it. `/undo [n]` (default 1) restores the newest edits —
backups are copied back, agent-created files removed — and the journal
survives session restarts (`myharness -c` then `/undo`). Changes made
directly through bash are not journaled.

**Sandboxing:** on Windows every bash command runs inside a Job Object
(`sandbox = "job"`, default) — command trees die with the harness, nothing
orphaned. `"appcontainer"` (Windows) is the real filesystem isolation:
commands run inside a dedicated AppContainer profile — writes limited to
the workspace + temp dir, and no network at all inside the container.
On Linux set `sandbox = "strict"` for Landlock filesystem rules
(read everywhere; writes only in the workspace and temp). `"off"` disables.

**Hooks** (`[[hooks]]`): `event = "PreToolUse" | "PostToolUse" | "Stop"`,
optional `tool` matcher, `command` receives JSON on stdin; exit code 2
blocks (denies the call / forces one more round at Stop), or print
`{"decision":"deny","reason":"..."}` to stdout. PreToolUse hooks run
before permission checks and a failing hook fails closed.

**Network egress is gated:** `web_fetch` prompts in ask mode (and fails
closed in `-p` without an allow rule) — fetched URLs can carry data away.
Plan mode still allows it for research. Deny rules work as usual:
`[[permissions.deny]] tool = "web_fetch" pattern = "https://internal*"`.

**MCP servers** (`[[mcp]]`): stdio servers are started at launch and their
tools appear as `mcp__<name>__<tool>`, gated by the normal permission
engine:

```toml
[[mcp]]
name = "github"
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
```

> Windows note: use single-quoted TOML strings for paths —
> `command = 'C:\tools\server.exe'` — because `\U` in double quotes is a
> TOML unicode escape.

**LSP diagnostics after edits** (`[[lsp]]`): configured language servers
are started lazily, edited files are opened via LSP, and the errors and
warnings they publish are handed to the model immediately after the edit
round — per-file feedback without a full rebuild:

```toml
[[lsp]]
name = "rust"
languages = ["rs"]
command = "rust-analyzer"
```

**Server mode:** `myharness serve` is a JSON-RPC 2.0 endpoint over stdio
(one JSON object per line) — `initialize`, `session.list`,
`session.attach {id}` / `session.new`, `turn.run` (responses carry the
`session_id`), `turn.cancel`, with `mh/*` notifications streaming during
turns. Attach replays an existing session into the server (same resume
path as the CLI), so thin clients survive restarts. For TUIs, IDEs, and
shell scripts driving the harness as a child process.

**Structured output for pipelines:** `myharness -p --output-schema '<json or
path>' "task"` requires the final message to be JSON matching the schema
(subset: `type`, `properties`, `required`, `items`, `enum`). Invalid output
gets two corrective rounds in-loop; if it still fails, the raw text prints
and the process exits non-zero — stdout stays trustworthy for piping.

Environment overrides: `ANTHROPIC_BASE_URL` / `OPENAI_BASE_URL`,
`MYHARNESS_DATA_DIR` (sessions + history location).

## Offline / CI testing

The `mock` provider replays a scripted conversation (see
`examples/smoke-script.json`):

```
set MYHARNESS_MOCK_FILE=examples\smoke-script.json
myharness -p "run the smoke test" --provider mock
```

## Development

```
cargo test        # 43 tests: unit + end-to-end on the mock provider, no network
cargo clippy      # clean
cargo build --release --bins --examples
```

CI (`.github/workflows/ci.yml`) runs the same on Windows and Ubuntu,
including an offline smoke of the built binary.

Architecture and the reasoning behind every decision: see [DESIGN.md](DESIGN.md)
— including the v0.5 hostile self-review, which found and fixed a real
Windows guard-key bug that had survived since v0.1.

## Known limitations

- Bash-driven file changes are not journaled — `/undo` covers
  write_file/edit_file only.
- Windows sandboxing is process-tree containment (Job Objects), not
  filesystem isolation — AppContainer is roadmap work. File tools are
  workspace-scoped in unattended modes; deny rules cover the rest.
- Linux `strict` sandbox is compile-verified but not runtime-tested here
  (needs a Linux host with kernel 5.13+); opt-in for that reason.
- Images in tool results display on the Anthropic-compatible protocol (the
  default); the OpenAI-compatible protocol degrades them to a text note.
- Reasoning output is display-only (never fed back — providers reject it on
  input).
- Background tasks live in memory — they don't survive a harness restart.
- MCP tools are called strictly one-at-a-time per server (no pipelining).
