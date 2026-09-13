# myharness

A terminal coding-agent harness for GLM — built autonomously by the model that runs inside it.

One static Rust binary. TUI-first: `myharness` opens a full-screen terminal UI with streaming transcript, permission prompts, and steering. Headless mode (`-p`) for pipelines and CI. Autonomous mode for overnight builds.

---

## Quick Start

### 1. Build

Requires Rust (install from [rustup.rs](https://rustup.rs)). On Windows, also install the MSVC build tools (Visual Studio Build Tools with "Desktop development with C++" workload).

```bash
git clone https://github.com/woolcoxm/MyHarness.git
cd MyHarness
cargo build --release
```

The binary is at `target/release/myharness.exe` (or `myharness` on Linux/Mac). Copy it anywhere, or add it to your PATH:

```bash
# Linux/Mac
cp target/release/myharness ~/.local/bin/

# Windows (PowerShell)
copy target\release\myharness.exe C:\Users\you\.cargo\bin\
```

### 2. Log In

Run the login command from your terminal (NOT inside the TUI):

```bash
myharness login
```

You'll see:

```
myharness credential setup

Which provider are you using?
  1. Z.ai Coding Plan (subscription - recommended for GLM models)
  2. Standard Z.ai API (pay per token)
  3. Custom OpenAI-compatible endpoint

Enter your choice [1-3]: 1
Paste your API key: sk-xxx...

Credentials saved to C:\Users\you\.myharness\auth.json
```

**Option 1 — Coding Plan**: If you have a Z.ai coding plan subscription (the same one that powers ZCode CLI), choose this. myharness will use your subscription's token budget instead of pay-per-token API credits. It auto-detects the correct endpoint (`open.bigmodel.cn/api/coding/paas/v4`) and protocol (OpenAI-compatible).

**Option 2 — Standard API**: If you have a standard Z.ai API key (pay-per-token from [api.z.ai](https://api.z.ai)), choose this. Uses the Anthropic-compatible protocol.

**Option 3 — Custom**: For any other OpenAI-compatible endpoint (Ollama, LM Studio, OpenRouter, etc.). You'll be prompted for the base URL and model name.

Credentials are stored in `~/.myharness/auth.json` with the API key obfuscated at rest (XOR with a machine-derived key — copying the file to another machine won't work).

### 3. Run

```bash
myharness
```

The full-screen TUI opens. You'll see a streaming transcript, a prompt box at the bottom, and a status bar showing your model, mode, and context usage. Type a task and press Enter.

---

## Usage

### Interactive TUI (default)

```bash
myharness              # opens the TUI
myharness "fix the bug in main.py"   # TUI with an initial task
```

The TUI gives you:

- **Streaming transcript** — model output appears as it generates
- **Tool calls** — each tool execution shown inline with status and output
- **Permission prompts** — when a tool needs approval, a modal appears (y/n/a)
- **Steering** — type and press Enter while the agent works to inject guidance mid-turn
- **Context bar** — shows how full the context window is
- **Command palette** — `ctrl+p` to search all commands
- **History search** — `ctrl+r` to search past prompts
- **Interrupt** — `esc` to stop the current turn

#### Slash Commands (inside the TUI)

| Command | What it does |
|---|---|
| `/help` | Show all commands |
| `/login` | Tells you to exit and run `myharness login` (doesn't work inside TUI — raw mode captures stdin) |
| `/memory` | Show zero-mem status |
| `/memory search <q>` | Search past session memories |
| `/memory clear` | Wipe the zero-mem store |
| `/compact` | Force compaction now |
| `/mode <name>` | Switch permission mode (plan / ask / auto-edit / yolo) |
| `/model <name>` | Switch model |
| `/todos` | Show current task list |
| `/undo [n]` | Undo last n file edits |
| `/usage` | Show token usage for this session |
| `/info` | Show session details |
| `/skills` | List discovered skills |
| `/commands` | List custom slash commands |
| `/clear` | Clear the conversation |
| `/quit` | Exit |

#### Keyboard Shortcuts

| Key | Action |
|---|---|
| `Enter` | Send message / steer mid-turn |
| `Alt+Enter` | Insert newline (multi-line input) |
| `Escape` | Interrupt the current turn |
| `Ctrl+C` | Quit (press twice) |
| `Ctrl+R` | Search prompt history |
| `Ctrl+P` | Command palette |
| `Ctrl+W` | Delete word |
| `Ctrl+U` | Clear line |
| `Page Up/Down` | Scroll transcript |
| `Ctrl+G` | Re-follow transcript (after scrolling) |

### Non-Interactive Mode

```bash
myharness -p "explain the failing test in tests/"
```

Runs the task, prints the final answer to stdout, and exits. Usage metrics go to stderr. Use `--yolo` to skip permission prompts:

```bash
myharness -p "fix the bug" --yolo
```

Add structured output validation:

```bash
myharness -p "list all functions as JSON" --output-schema '[{"type":"array"}]'
```

Exit code is non-zero if the output doesn't match the schema.

### Autonomous Mode

```bash
myharness -p "build a REST API with tests" --autonomous --yolo
```

The agent works until it verifies the task is done — it doesn't stop when it feels like it. Each time the model stops, a self-check prompt demands verification:

1. **Syntax**: run the build/check command
2. **Runtime**: actually execute the code
3. **Read-back**: read every file created/modified
4. **Completeness**: re-read the original task
5. **Integration**: do all pieces work together

The model must confirm "GOAL COMPLETE" twice in a row. Any tool call during a self-check means it found more work and continues.

Budget controls:

```bash
myharness -p "task" --autonomous --budget-hours 4 --budget-tokens 500000 --yolo
```

Default: 8 hours, 1M tokens.

### Server Mode

```bash
myharness serve
```

JSON-RPC 2.0 over stdio. Send `initialize`, then `turn.run {"input": "..."}`. Receives `mh/turn.delta`, `mh/tool.start`, `mh/tool.end` notifications during turns. For thin clients and IDE integration.

### Session Management

```bash
myharness sessions        # list all saved sessions
myharness resume          # resume the most recent
myharness resume abc123   # resume by id prefix
```

---

## Skills

myharness ships with 27 built-in development skills covering debugging, testing, code review, system design, API design, frontend/backend patterns, TypeScript, React, Rust, Go, Python, Git, Docker, CI/CD, web security, performance optimization, and LLM integration.

### How skills work

Each skill is a directory containing a `SKILL.md` file with YAML frontmatter:

```markdown
---
name: debugging
description: Systematic debugging methodology...
---

# Debugging: A Systematic Method
...instructions...
```

The model sees the skill names in its system prompt. When it encounters a relevant task, it calls the `skill` tool to load the full instructions. This means skills cost zero tokens until they're actually needed.

### Where skills live

| Location | Scope |
|---|---|
| `~/.myharness/skills/` | User-level (available in all projects) |
| `.agents/skills/` | Project-level (available in this repo only) |

The 27 built-in skills ship in `.agents/skills/` in this repo, so cloning the repo gives you all of them automatically.

### Creating your own skill

```bash
mkdir -p ~/.myharness/skills/my-skill
cat > ~/.myharness/skills/my-skill/SKILL.md << 'EOF'
---
name: my-skill
description: Use whenever the user asks to deploy to Kubernetes. Covers kubectl commands, manifest writing, and troubleshooting.
---

# Kubernetes Deployment Guide

1. First, check the current state:
   kubectl get pods --all-namespaces
...
EOF
```

The model will discover it automatically on the next session.

---

## Permission Modes

| Mode | Reads | Edits | Bash |
|---|---|---|---|
| `plan` | allowed | blocked | read-only commands only |
| `ask` (default) | allowed | prompted | prompted |
| `auto-edit` | allowed | allowed | allow-rules only |
| `yolo` | allowed | allowed | allowed |

Switch modes: `/mode plan` in the TUI, or `--mode plan` on the command line.

**Plan mode** runs a quote-aware allowlist parser for read-only bash commands (`ls`, `cat`, `grep`, `git status/diff/log`, `gh pr list`, etc.). Redirection, substitution, and heredocs are denied.

**Deny rules** always win, in every mode. Compound commands are checked per-subcommand: `ls && rm -rf /` cannot ride an `ls` allow rule.

---

## Security

- **Workspace scoping**: `write_file`/`edit_file` cannot traverse outside the workspace (`sub/../../escape.txt` is blocked)
- **SSRF guard**: `web_fetch` denies private IPs (hex, decimal, octal, v4-mapped v6), credentials in URLs, and link-local addresses (including cloud metadata at 169.254.169.254)
- **Stale-context guard**: files edited externally after being read must be re-read before editing
- **Script gate**: written `.js`/`.html` files pass through `node --check` (syntax) and a Node VM runtime check (undefined variables, broken references)
- **Doom-loop gate**: identical failing tool calls refused after 5 consecutive attempts
- **Sandboxing**: Windows Job Objects (default), AppContainer (opt-in), Linux Landlock (opt-in)
- **API keys**: stored obfuscated in `~/.myharness/auth.json`, never logged, never sent anywhere except the configured API endpoint

---

## Token Efficiency

myharness's model-facing payload floor is **~1,993 tokens per request** (system prompt: 904 chars, tool schemas: 7,071 chars) — pinned by a regression test and 30% leaner than pi agent.

The levers, in order of impact:

1. **Model choice** — glm-4.7-air vs glm-5.3 dwarfs everything else
2. **Thinking budget** — `[model] thinking_budget = 4096` in config (default; the single biggest output-token lever)
3. **Prompt caching** — byte-stable system prompt and tool schemas; cache reads cost ~10% of fresh input
4. **Compaction** — auto-compacts at 80% of the context window; oversized results spill to artifact files
5. **Subagents** — exploration happens in fresh contexts; only the final report enters the parent's context
6. **Zero-mem** — ~100 tokens of relevant past context injected, instead of re-reading full transcripts

Economy preset in `myharness config`'s template. `-p` prints usage to stderr.

---

## Zero-Mem: Long-Term Memory

Zero-LLM-call persistent memory across sessions. Every turn's prompt and final answer are captured to a per-project store. At each new turn, deterministic retrieval (BM25 + entity-graph PageRank) injects up to 3 past-session snippets. A sticky identity slot means new sessions know your name.

```
mh> /memory
zero-mem: 42 unit(s), identity: user=Mark, agent=? | store ~/.myharness/zero-mem/proj.json
mh> /memory search deploy hook
(0 days ago, user) The VoltEdge staging deploy hook is deploy_hook_v2...
```

Configure via `[zero_mem]` in myharness.toml. Set `enabled = false` to disable.

---

## Configuration

`myharness.toml` in the project root (or `~/.myharness/config.toml` for global):

```toml
[model]
provider = "openai"              # auto-detected from coding plan
name = "glm-5.3"                 # or glm-5.3-flash, glm-4.7-air
max_tokens = 32768
context_window = 200000
prompt_caching = true
thinking_budget = 4096           # reasoning tokens per request
# model_fast = "glm-4.7-air"    # cheaper model for compaction summaries

[agent]
max_turns = 80
compact_ratio = 0.8
verify_cmd = "cargo check"       # runs after edit turns
restrict_writes_to_workspace = true
verbose_prompt = false           # true = richer system prompt (more tokens)

[bash]
timeout_ms = 120000
shell = "auto"
sandbox = "job"                  # windows: job | appcontainer; linux: strict | off

[web]
private_hosts = false            # allow web_fetch to reach localhost

[zero_mem]
enabled = true
top_k = 3
max_units = 5000
max_age_days = 180

# [[hooks]] ...
# [[mcp]] ...
# [[lsp]] ...
# [[output_hint]] ...
# [[permissions.allow]] ...
```

Run `myharness config` to see the full annotated template.

---

## Architecture

```
src/
  main.rs              binary: CLI dispatch, provider build, autonomous loop
  cli.rs               clap surface
  config.rs            layered config + coding plan discovery
  auth.rs              credential management (obfuscated at rest)
  agent/
    mod.rs             the turn loop, tool dispatch, steering, reflect
    state.rs           mutable session state
    system_prompt.rs   lean/full system prompt (byte-stable for caching)
    compact.rs         compaction + spill + refill guard
    tokens.rs          token estimation
  tools/
    mod.rs             tool trait, registry, budget_output, stale_check
    bash.rs            shell execution with sandboxing
    read_file.rs       cat -n reads + images
    edit_file.rs       exact-string edits with stale guard
    write_file.rs      file creation with read-gate
    net_guard.rs       SSRF destination guard
    js_check.rs        syntax + runtime verification for JS/HTML
    repo_map.rs        PageRank-ranked repo map
    session_recall.rs  cross-session memory search
    monitor.rs         periodic command execution
    web_fetch.rs       SSRF-guarded HTTP fetch
    web_search.rs      keyless DuckDuckGo search
    task.rs            subagent spawning
    todo.rs            task list management
    skill.rs           SKILL.md loading
  tui/
    mod.rs             the TUI app: event loop, worker actor, overlays
    render.rs          ratatui widget drawing
    input.rs           line editor with history
  ui.rs                UiEvent system + slash command handler
  auth.rs              credential storage (obfuscated)
  zero_mem.rs          zero-token long-term memory
  perms.rs             permission engine + plan-mode command guard
  session.rs           JSONL event-sourced sessions
  server.rs            JSON-RPC serve mode
  llm/
    mod.rs             IR, provider trait, SSE parser, retry with backoff
    anthropic.rs       Anthropic Messages streaming client
    openai.rs          OpenAI chat-completions streaming client
    mock.rs            scripted provider for testing
  skills.rs            SKILL.md discovery
  commands.rs          custom slash commands (.agents/commands/)
  hooks.rs             PreToolUse/PostToolUse/Stop hooks
  mcp.rs               MCP client (stdio)
  lsp.rs               LSP client for diagnostics
  schema_validate.rs   JSON schema subset validator
```

---

## Testing

**164 automated tests**, zero network required:

| Suite | Count | What it covers |
|---|---|---|
| Unit | 98 | Per-module logic |
| Integration | 52 | End-to-end agent loop scenarios |
| Adversarial | 9 | Sandbox escapes, permission smuggling, SSRF bypass attempts, hostile inputs |
| Auth | 4 | Credential round-trip, obfuscation |
| Concurrency | 1 | Multi-process store merge |

```bash
cargo test          # run all
cargo clippy        # lint (zero warnings enforced)
```

---

## How This Project Was Built

myharness was **written autonomously by a GLM-5.3 coding agent** — the model it serves. The human set direction; the model wrote the code. The method was research-driven:

1. **Literature survey** — studied Claude Code, Codex CLI, OpenCode, Aider, Goose, Gemini CLI
2. **Self-audit** — adversarial rounds re-reading the codebase for defects
3. **Reverse engineering** — mined other harnesses for ideas, reimplemented from scratch
4. **Live testing** — head-to-head benchmarks against pi agent and ZCode on identical prompts
5. **Exposure testing** — adversarial battery attacking every security surface

Full details: [DESIGN.md](DESIGN.md) (every decision with provenance) and [BENCHMARK.md](BENCHMARK.md) (three-way comparison data).

---

## License

MIT — see [LICENSE](LICENSE).
