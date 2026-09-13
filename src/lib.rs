//! myharness — a terminal coding-agent harness for GLM.
//!
//! Library layout:
//! - [`llm`]: unified message model, `Provider` trait, streaming clients for
//!   Anthropic- and OpenAI-compatible endpoints (thinking included,
//!   display-only), and a scripted mock.
//! - [`tools`]: the tool contracts the model works against (including
//!   background bash and image results).
//! - [`agent`]: the turn loop, system prompt (AGENTS.md + repo layout),
//!   token estimation, compaction, and the permission/hook/parallel dispatch
//!   of tool calls.
//! - [`perms`]: permission modes and the deny→allow→prompt rule engine.
//! - [`hooks`]: PreToolUse/PostToolUse/Stop lifecycle commands
//!   (stdin-JSON / exit-code-2 contract).
//! - [`mcp`]: stdio MCP client; server tools bridge as `mcp__<s>__<t>`.
//! - [`lsp`]: minimal LSP client for post-edit diagnostics (errors and
//!   warnings published by configured language servers).
//! - [`sandbox`]: process containment (Windows Job Objects; Linux Landlock).
//! - [`schema_validate`]: the JSON-Schema subset behind `--output-schema`.
//! - [`session`]: JSONL event-sourced persistence, the edit journal, resume.
//! - [`skills`]: SKILL.md instruction packs discovered from the user and
//!   workspace `.agents/skills` directories (ZCode-compatible locations).
//! - [`commands`]: user-defined slash commands (`.agents/commands/*.md`)
//!   with `$ARGUMENTS` prompt templates.
//! - [`config`] / [`cli`] / [`ui`] / [`server`]: configuration, argument
//!   parsing, REPL, and the JSON-RPC serve mode for thin clients.

pub mod agent;
pub mod cli;
pub mod commands;
pub mod config;
pub mod hooks;
pub mod llm;
pub mod lsp;
pub mod mcp;
pub mod perms;
pub mod sandbox;
pub mod schema_validate;
pub mod server;
pub mod session;
pub mod skills;
pub mod tools;
pub mod tui;
pub mod ui;
pub mod zero_mem;
