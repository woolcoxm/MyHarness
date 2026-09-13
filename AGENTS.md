# AGENTS.md — guidance for coding agents working in this repo

## Project

`myharness` is a terminal coding-agent harness (bin + lib in one crate).
Read `DESIGN.md` first — it explains every architectural decision. Do not
change tool contracts or the session event schema casually: they are the
model-facing ABI.

## Commands

- `cargo test` — must pass; tests are offline (mock provider), no keys needed.
- `cargo clippy --all-targets` — must stay warning-free.
- `cargo build --release` — the shipped binary.

## Conventions

- Tools never prompt and never print: they take parsed JSON input + `ToolCtx`
  (owned snapshot + `&mut ToolEffects`) and return `ToolOutput`. State
  changes go through `ToolEffects` and are merged by the agent — that is
  what makes parallel execution of concurrency-safe tools sound. Mark new
  tools `concurrency_safe()` only if they touch nothing but effects.
  Permission enforcement happens in `agent/mod.rs` before dispatch; hooks
  (`PreToolUse`) run even before that, and a failing hook fails closed. UI
  lives only in `ui.rs` (and `agent` display calls).
- Error strings returned to the model are part of the contract: they must say
  exactly what to do next (see `edit_file` failures for the house style).
- Keep the session JSONL events append-only and replay-complete: anything
  that changes agent state across resume must be persisted as an event.
- Streaming: providers map wire events into the unified `StreamEvent`; the
  agent accumulates and prints live. Turn end is decided by "no tool calls",
  not by stop_reason — keep it provider-agnostic. Thinking/reasoning deltas
  are display-only.
- ASCII-only UI output (works in every console and CI log).
- Windows is a first-class target: no unix-only assumptions in tool code
  (`cfg!(windows)` branches where needed); tests run on Windows. Watch for
  two known traps: `cmd` requires `/D` before `/C`, and TOML double-quoted
  strings choke on `\U` in Windows paths (single-quote them).
- Sandbox additions must stay best-effort: a sandbox that fails must never
  take the command down with it, and Linux-only code is cfg-gated and
  cross-compile-checked (`landlock` API verified for
  `x86_64-unknown-linux-gnu`).
- Build both targets explicitly: `cargo build --release --bins --examples`
  (`--examples` alone does NOT build the binary).
