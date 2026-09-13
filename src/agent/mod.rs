//! The agent: one turn = one LLM request whose tool calls are executed and
//! fed back, repeated until the model ends its turn. Everything the model
//! experiences (system prompt, tool contracts, error strings) is decided
//! here or in the tools — this loop is the harness.

pub mod compact;
pub mod state;
pub mod system_prompt;
pub mod tokens;

use crate::llm::{ContentBlock, LlmRequest, Message, Provider, Role, StreamEvent, Usage};
use crate::perms::{Decision, PermissionEngine};
use crate::session::{Event, Session};
use crate::tools::{Registry, Tool, ToolCtx, ToolEffects, ToolOutput};
use crate::ui::{ApprovalAnswer, ApprovalRequest, Ui};
use anyhow::Result;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncBufReadExt;

pub struct Agent {
    pub provider: Arc<dyn Provider>,
    pub cfg: Arc<crate::config::Config>,
    pub state: state::AgentState,
    pub session: Option<Session>,
    pub ui: Ui,
    pub registry: Registry,
    pub perms: PermissionEngine,
    pub model: String,
    pub cancel: Arc<AtomicBool>,
    pub is_subagent: bool,
    /// When set, the final message must be JSON matching this schema
    /// (--output-schema); invalid output triggers corrective rounds.
    pub output_schema: Option<Value>,
    /// Hard turn cap; main agents use cfg.max_turns + a grace notice.
    turn_cap: Option<u32>,
    /// Stop-hook continuations used this turn (max 3).
    pub stop_blocks: u32,
    /// Schema-correction rounds used this turn (max 2; observable for tests).
    pub schema_retries: u32,
    /// Live LSP clients, keyed by config name (runtime-only, not resumed).
    pub lsp: HashMap<String, crate::lsp::LspClient>,
    /// Pattern → last-fired time for output hints (runtime-only throttle).
    hint_throttle: HashMap<String, std::time::Instant>,
    /// TUI mode: permission prompts are answered through this channel
    /// instead of stdin (the TUI owns the terminal).
    pub approval_tx: Option<tokio::sync::mpsc::UnboundedSender<ApprovalRequest>>,
    /// TUI steering: messages queued while a turn runs; injected after the
    /// current tool batch (pi's two-tier queue, steering tier).
    pub steer_rx: Option<tokio::sync::mpsc::UnboundedReceiver<String>>,
    /// verify_cmd reflect loop: failed rounds this user turn (cap 3, Aider).
    reflect_rounds: u32,
    verify_failed: bool,
    /// Consecutive identical failing tool calls (doom-loop gate, opencode).
    failed_streak: Vec<(String, String)>,
    /// Journal length when this user turn started (turn diffstat).
    journal_turn_start: usize,
    /// Zero-token long-term memory (None for subagents / when disabled).
    pub zero_mem: Option<crate::zero_mem::ZeroMem>,
}

/// Refill-rate guard: an auto-compaction that triggers again within this
/// many tool-result messages counts as a fast refill (a single result is
/// too large to carry).
const FAST_REFILL_RESULTS: u64 = 8;
/// Two fast refills in a row pause auto-compaction for the session.
const FAST_REFILL_STREAK: u32 = 2;
/// Doom-loop gate: the same call failing this many times in a row is a
/// loop; further identical calls are refused until something changes.
const DOOM_STREAK: usize = 3;
/// Turn-context block is skipped while the conversation is small (Goose).
const MOIM_MIN_TOKENS: u64 = 32_000;

#[derive(Debug, Clone)]
pub struct TurnOutcome {
    pub final_text: String,
    pub interrupted: bool,
}

/// One tool call after the permission phase.
enum ToolPlan {
    /// Already answered (denied / unknown tool): a finished result block.
    Done(ContentBlock),
    Run { tool: Arc<dyn Tool>, input: Value },
}

impl Agent {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: Arc<dyn Provider>,
        cfg: Arc<crate::config::Config>,
        state: state::AgentState,
        ui: Ui,
        session: Option<Session>,
        registry: Registry,
        perms: PermissionEngine,
        model: String,
        cancel: Arc<AtomicBool>,
        is_subagent: bool,
    ) -> Self {
        let zero_mem = if is_subagent || !cfg.zero_mem.enabled {
            None
        } else {
            let sid = session
                .as_ref()
                .map(|s| s.id.clone())
                .unwrap_or_else(|| format!("adhoc-{}", std::process::id()));
            let mut zm = crate::zero_mem::ZeroMem::open(
                &cfg.data_dir,
                &state.workspace_root,
                &sid,
                cfg.zero_mem.clone(),
            );
            zm.derive_identity();
            Some(zm)
        };
        Agent {
            provider,
            cfg,
            state,
            session,
            ui,
            registry,
            perms,
            model,
            cancel,
            is_subagent,
            output_schema: None,
            turn_cap: None,
            stop_blocks: 0,
            schema_retries: 0,
            lsp: HashMap::new(),
            hint_throttle: HashMap::new(),
            approval_tx: None,
            steer_rx: None,
            reflect_rounds: 0,
            verify_failed: false,
            failed_streak: Vec::new(),
            journal_turn_start: 0,
            zero_mem,
        }
    }

    /// Let a terminal-owning frontend (TUI) answer permission prompts.
    pub fn set_approval_tx(&mut self, tx: tokio::sync::mpsc::UnboundedSender<ApprovalRequest>) {
        self.approval_tx = Some(tx);
    }

    /// Let a frontend steer a running turn: queued messages are injected
    /// after the current tool batch, mid-turn.
    pub fn set_steering(&mut self, rx: tokio::sync::mpsc::UnboundedReceiver<String>) {
        self.steer_rx = Some(rx);
    }

    pub fn set_turn_cap(&mut self, cap: u32) {
        self.turn_cap = Some(cap);
    }

    pub fn set_output_schema(&mut self, schema: Value) {
        self.output_schema = Some(schema);
    }

    fn effective_turn_cap(&self) -> u32 {
        self.turn_cap.unwrap_or(self.cfg.max_turns)
    }

    fn persist(&mut self, event: &Event) {
        if let Some(s) = self.session.as_mut() {
            if let Err(e) = s.append(event) {
                self.ui.warn(&format!("session write failed: {e}"));
            }
        }
    }

    fn build_request(&self) -> LlmRequest {
        let system = system_prompt::build_system(&self.state, self.is_subagent);
        LlmRequest {
            model: self.model.clone(),
            system,
            messages: self.state.messages.clone(),
            tools: self.registry.schemas(),
            max_tokens: self.cfg.max_tokens,
            temperature: self.cfg.temperature,
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// Run one user input to completion (multiple LLM round-trips).
    pub async fn run_turn(&mut self, user_input: &str) -> Result<TurnOutcome> {
        self.stop_blocks = 0;
        self.schema_retries = 0;
        self.reflect_rounds = 0;
        self.verify_failed = false;
        self.failed_streak.clear();
        self.journal_turn_start = self.state.edit_journal.len();
        if !user_input.trim().is_empty() {
            let msg = Message::user_text(user_input);
            self.state.messages.push(msg.clone());
            self.persist(&Event::Message(msg));
        }
        // Zero-Mem: deterministic memory injection (zero LLM calls) — the
        // identity slot on each session's first turn and identity-class
        // queries, otherwise retrieval of past-session evidence.
        self.inject_memory(user_input);
        // Turn-context block (Goose MOIM): one agent-only message so the
        // model knows what time it is, where it is, and how long its leash
        // is — skipped while the conversation is still small.
        if !self.is_subagent {
            self.inject_turn_context();
        }
        let cwd_before = self.state.cwd.clone();

        loop {
            if self.cancelled() {
                return Ok(TurnOutcome { final_text: String::new(), interrupted: true });
            }
            if !self.is_subagent {
                self.maybe_compact().await;
                self.deliver_bg_notices().await;
            }

            let req = self.build_request();
            let mut rx = self.provider.stream(&req).await?;
            self.state.requests += 1;
            let (blocks, interrupted, stop_reason) = self.consume_stream(&mut rx).await?;
            if interrupted {
                self.ui.warn("turn interrupted; partial response discarded");
                return Ok(TurnOutcome { final_text: String::new(), interrupted: true });
            }
            let final_text = blocks
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let tool_uses: Vec<(String, String, Value)> = tool_uses_of(&blocks);
            let assistant = Message { role: Role::Assistant, content: blocks };
            self.state.messages.push(assistant.clone());
            self.persist(&Event::Message(assistant));

            // pi's correctness rule: tool calls carved from a length-
            // truncated message may carry truncated arguments; executing
            // them is how a transport hiccup becomes file damage.
            if !tool_uses.is_empty() && stop_reason.as_deref() == Some("length") {
                self.ui.warn(
                    "message hit the output length limit mid-tool-call; failing the calls instead of executing truncated arguments",
                );
                let results: Vec<ContentBlock> = tool_uses
                    .iter()
                    .map(|(id, name, _)| {
                        ContentBlock::ToolResult {
                            tool_use_id: id.clone(),
                            content: format!(
                                "tool call dropped: the model's message hit the output \
                                 length limit before this {name} call completed, so its \
                                 arguments may be truncated. Re-issue the call and keep any \
                                 accompanying text short so the whole call fits."
                            ),
                            images: Vec::new(),
                            is_error: true,
                        }
                    })
                    .collect();
                let results_msg = Message::tool_results(results);
                self.state.messages.push(results_msg.clone());
                self.persist(&Event::Message(results_msg));
                continue;
            }

            if tool_uses.is_empty() || self.cancelled() {
                // Follow-up tier (pi): steering queued while the turn ran
                // starts a fresh round instead of being stranded.
                if tool_uses.is_empty() && !self.cancelled() {
                    let mut queued_msgs: Vec<String> = Vec::new();
                    if let Some(rx) = &mut self.steer_rx {
                        while let Ok(queued) = rx.try_recv() {
                            if !queued.trim().is_empty() {
                                queued_msgs.push(queued);
                            }
                        }
                    }
                    if !queued_msgs.is_empty() {
                        for queued in queued_msgs {
                            let msg = Message::user_text(format!(
                                "(user, while you were working) {queued}"
                            ));
                            self.state.messages.push(msg.clone());
                            self.persist(&Event::Message(msg));
                        }
                        continue;
                    }
                }
                // Reflect loop (Aider): the verify command failed after the
                // last edits and the model is trying to stop anyway — send
                // it back for the failures (bounded).
                if self.verify_failed && !self.is_subagent && self.reflect_rounds < 3 {
                    self.reflect_rounds += 1;
                    self.verify_failed = false;
                    self.ui.warn(&format!(
                        "verify command still failing; reflecting round {}/3",
                        self.reflect_rounds
                    ));
                    let notice = Message::user_text(
                        "(system) The verify command that ran after your edits reported \
                         failures that were not resolved before you stopped. Fix them now: \
                         run the failing check yourself, address the errors, and make sure \
                         it passes before finishing.",
                    );
                    self.state.messages.push(notice.clone());
                    self.persist(&Event::Message(notice));
                    continue;
                }
                // Output schema: an invalid final message gets corrective
                // rounds (bounded) before the turn is allowed to end.
                if tool_uses.is_empty() && !self.cancelled() {
                    if let Some(schema) = self.output_schema.clone() {
                        let verdict = crate::schema_validate::parse_output(&final_text)
                            .and_then(|v| crate::schema_validate::validate(&v, &schema));
                        if let Err(e) = verdict {
                            if self.schema_retries < 2 {
                                self.schema_retries += 1;
                                self.ui.warn(&format!(
                                    "output failed schema validation ({}/2): {e}",
                                    self.schema_retries
                                ));
                                let notice = Message::user_text(format!(
                                    "(system) Your final output failed schema validation: {e}\n\
                                     Output ONLY the corrected JSON matching this schema, \
                                     with no other text:\n{schema}"
                                ));
                                self.state.messages.push(notice.clone());
                                self.persist(&Event::Message(notice));
                                continue;
                            }
                            self.ui.warn("output still fails the schema after retries");
                        }
                    }
                }
                // Stop hooks: exit 2 / deny pushes the reason back to the
                // model and forces continuation (bounded to 3 rounds).
                if tool_uses.is_empty() && !self.cancelled() && self.stop_blocks < 3 {
                    let ctx = self.hook_context();
                    if let crate::hooks::HookVerdict::Block(reason) =
                        crate::hooks::stop(&self.cfg, &ctx).await
                    {
                        self.stop_blocks += 1;
                        self.ui.warn(&format!(
                            "Stop hook blocked ending the turn ({}/3): {reason}",
                            self.stop_blocks
                        ));
                        let notice = Message::user_text(format!(
                            "(system) A Stop hook requires you to continue: {reason}"
                        ));
                        self.state.messages.push(notice.clone());
                        self.persist(&Event::Message(notice));
                        continue;
                    }
                }
                self.capture_memory(user_input, &final_text);
                self.finish_turn(&cwd_before);
                return Ok(TurnOutcome { final_text, interrupted: false });
            }

            let results = self.execute_tool_uses(&tool_uses).await;
            // Doom-loop gate (opencode): track consecutive identical calls
            // that keep failing; the plan phase refuses to run the fourth.
            for ((_, name, input), block) in tool_uses.iter().zip(results.iter()) {
                let key = (name.clone(), input.to_string());
                // Bash reports non-zero exits in-band ("Exit code: N", the
                // tool result itself is not an error), so read the code.
                let failed = match block {
                    ContentBlock::ToolResult { is_error: true, .. } => true,
                    ContentBlock::ToolResult { content, .. } if name == "bash" => content
                        .lines()
                        .next()
                        .and_then(|l| l.split_whitespace().nth(2))
                        .and_then(|c| c.parse::<i32>().ok())
                        .is_some_and(|code| code != 0),
                    _ => false,
                };
                if failed {
                    if self.failed_streak.last() == Some(&key) {
                        self.failed_streak.push(key);
                    } else {
                        self.failed_streak = vec![key];
                    }
                } else if self.failed_streak.last() == Some(&key) {
                    self.failed_streak.clear();
                }
            }
            // Output-pattern hints: a distinctive substring in a result
            // fires a throttled guidance note (persisted like any message,
            // so resume history stays complete).
            let fired = scan_output_hints(&results, &self.cfg.output_hints, &mut self.hint_throttle);
            let results_msg = Message::tool_results(results);
            self.state.messages.push(results_msg.clone());
            self.persist(&Event::Message(results_msg));
            self.state.tool_results_since_compact += 1;
            for hint in fired {
                self.ui.info(&format!("hint fired: {}", hint.lines().next().unwrap_or(&hint)));
                let msg = Message::user_text(format!("(system) {hint}"));
                self.state.messages.push(msg.clone());
                self.persist(&Event::Message(msg));
            }

            // JIT instructions (Gemini): when a tool touches a path, walk
            // that path's ancestors for AGENTS.md files not yet injected —
            // monorepo per-package instructions arrive exactly when the
            // model starts working there.
            self.jit_instructions(&tool_uses);

            // Steering: input queued by the frontend while this turn ran is
            // injected after the tool batch so the model sees it on the very
            // next request (pi's steering tier).
            let mut queued_msgs: Vec<String> = Vec::new();
            if let Some(rx) = &mut self.steer_rx {
                while let Ok(queued) = rx.try_recv() {
                    if !queued.trim().is_empty() {
                        queued_msgs.push(queued);
                    }
                }
            }
            for queued in queued_msgs {
                let msg = Message::user_text(format!("(user, while you were working) {queued}"));
                self.state.messages.push(msg.clone());
                self.persist(&Event::Message(msg));
            }

            // Opt-in verification loop: after edits, run the project's
            // verify command and hand the model the result before it goes on.
            let edits_made = tool_uses.iter().any(|(_, n, _)| n == "write_file" || n == "edit_file");
            if edits_made {
                if let Some(cmd) = self.cfg.verify_cmd.clone() {
                    let (report, ok) = crate::tools::bash::run_verify(&self.cfg, &self.state.cwd, &cmd).await;
                    let msg = Message::user_text(format!(
                        "(auto-verification after edits) `{cmd}`\n{report}"
                    ));
                    self.state.messages.push(msg.clone());
                    self.persist(&Event::Message(msg));
                    self.verify_failed = !ok;
                }
                // LSP diagnostics for the edited files (config-gated): the
                // model sees errors/warnings per file before continuing.
                if !self.cfg.lsp_servers.is_empty() {
                    let edited: Vec<PathBuf> = tool_uses
                        .iter()
                        .filter(|(_, n, _)| n == "write_file" || n == "edit_file")
                        .filter_map(|(_, _, input)| input.get("path").and_then(|v| v.as_str()))
                        .map(|p| crate::tools::resolve_path(&self.state.cwd, p))
                        .collect();
                    if let Some(report) = self.lsp_after_edits(&edited).await {
                        let msg = Message::user_text(format!(
                            "(lsp diagnostics after edits)\n{report}"
                        ));
                        self.state.messages.push(msg.clone());
                        self.persist(&Event::Message(msg));
                    }
                }
            }

            self.state.turns += 1;
            let cap = self.effective_turn_cap() as u64;
            if self.state.turns == cap.saturating_sub(2) && !self.state.limit_notice_sent {
                self.state.limit_notice_sent = true;
                let notice = Message::user_text(
                    "(system) The turn limit is approaching. Finish the remaining work and end with your summary now.",
                );
                self.state.messages.push(notice.clone());
                self.persist(&Event::Message(notice));
            }
            if self.state.turns >= cap {
                self.ui.warn("turn limit reached; stopping this turn");
                self.capture_memory(user_input, &final_text);
                self.finish_turn(&cwd_before);
                return Ok(TurnOutcome { final_text, interrupted: false });
            }
        }
    }

    /// Consume one streaming response into content blocks; text deltas are
    /// printed live. Returns (blocks, interrupted, stop_reason).
    async fn consume_stream(
        &mut self,
        rx: &mut tokio::sync::mpsc::Receiver<Result<StreamEvent>>,
    ) -> Result<(Vec<ContentBlock>, bool, Option<String>)> {
        let mut text = String::new();
        let mut tools: Vec<(String, String, String)> = Vec::new();
        let mut usage_acc = Usage::default();
        let mut interrupted = false;
        let mut stop_reason: Option<String> = None;
        while let Some(ev) = rx.recv().await {
            if self.cancelled() {
                interrupted = true;
                break;
            }
            match ev? {
                StreamEvent::MessageStart | StreamEvent::BlockStop => {}
                StreamEvent::TextDelta(s) => {
                    self.ui.assistant_delta(&s);
                    text.push_str(&s);
                }
                StreamEvent::ThinkingDelta(s) => {
                    // Reasoning is shown to the operator but never stored:
                    // providers don't accept it back as input.
                    self.ui.thinking_delta(&s);
                }
                StreamEvent::ToolUseStart { id, name } => {
                    tools.push((id, name, String::new()));
                }
                StreamEvent::ToolInputDelta(s) => {
                    if let Some(last) = tools.last_mut() {
                        last.2.push_str(&s);
                    }
                }
                StreamEvent::MessageDelta { stop_reason: sr } => stop_reason = sr,
                StreamEvent::Usage(u) => usage_acc.add(u),
                StreamEvent::MessageStop => break,
            }
        }
        self.state.usage.add(usage_acc);
        self.ui.end_stream();
        let mut blocks = Vec::new();
        if !text.trim().is_empty() {
            blocks.push(ContentBlock::Text { text });
        }
        for (id, name, raw) in tools {
            let input: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
            blocks.push(ContentBlock::ToolUse { id, name, input });
        }
        Ok((blocks, interrupted, stop_reason))
    }

    /// Permission-check every call (sequentially — prompts must come in the
    /// model's order), then execute: concurrency-safe tools fan out in
    /// parallel, state-touching tools run in order. Results are assembled
    /// in the model's original order regardless of completion order.
    async fn execute_tool_uses(&mut self, tool_uses: &[(String, String, Value)]) -> Vec<ContentBlock> {
        // Phase 1: resolve + hooks + permission.
        let mut plans: Vec<ToolPlan> = Vec::with_capacity(tool_uses.len());
        for (id, name, input) in tool_uses {
            plans.push(self.plan_tool_use(id, name, input).await);
        }

        // Phase 2a: spawn the concurrency-safe subset.
        let mut handles: Vec<(usize, tokio::task::JoinHandle<(ToolOutput, ToolEffects)>)> = Vec::new();
        for (idx, plan) in plans.iter().enumerate() {
            if let ToolPlan::Run { tool, input } = plan {
                if tool.concurrency_safe() {
                    handles.push((
                        idx,
                        tokio::spawn(run_isolated(
                            Arc::clone(tool),
                            input.clone(),
                            self.snapshot_ctx(),
                        )),
                    ));
                }
            }
        }

        // Phase 2b: run everything else in order while the batch flies.
        let mut results: Vec<Option<ContentBlock>> = tool_uses.iter().map(|_| None).collect();
        for (idx, plan) in plans.iter().enumerate() {
            match plan {
                ToolPlan::Done(block) => results[idx] = Some(block.clone()),
                ToolPlan::Run { tool, input } => {
                    if !tool.concurrency_safe() {
                        let (out, fx) = run_isolated(Arc::clone(tool), input.clone(), self.snapshot_ctx()).await;
                        fx.merge_into(&mut self.state);
                        self.ui.tool_end(&tool_uses[idx].1, &out);
                        results[idx] = Some(self.output_block(&tool_uses[idx].0, out));
                    }
                }
            }
        }

        // Phase 2c: collect the parallel batch.
        for (idx, handle) in handles {
            match handle.await {
                Ok((out, fx)) => {
                    fx.merge_into(&mut self.state);
                    self.ui.tool_end(&tool_uses[idx].1, &out);
                    results[idx] = Some(self.output_block(&tool_uses[idx].0, out));
                }
                Err(e) => {
                    let out = ToolOutput::err(format!("tool panicked: {e}"));
                    results[idx] = Some(self.output_block(&tool_uses[idx].0, out));
                }
            }
        }

        let final_results: Vec<ContentBlock> = results.into_iter().flatten().collect();
        // PostToolUse hooks observe executed results (verdicts logged only).
        self.run_post_hooks(tool_uses, &plans, &final_results).await;
        final_results
    }

    fn hook_context(&self) -> crate::hooks::HookContext {
        crate::hooks::HookContext {
            session_id: self.session.as_ref().map(|s| s.id.clone()).unwrap_or_default(),
            transcript_path: self
                .session
                .as_ref()
                .map(|s| s.path.display().to_string())
                .unwrap_or_default(),
            cwd: self.state.cwd.display().to_string(),
            permission_mode: self.perms.mode.name().to_string(),
        }
    }

    /// PostToolUse hooks for executed calls (observability; verdict logged).
    async fn run_post_hooks(&self, tool_uses: &[(String, String, Value)], plans: &[ToolPlan], results: &[ContentBlock]) {
        if self.cfg.hooks.is_empty() {
            return;
        }
        let ctx = self.hook_context();
        for (i, plan) in plans.iter().enumerate() {
            if matches!(plan, ToolPlan::Run { .. }) {
                if let Some(ContentBlock::ToolResult { content, is_error, .. }) = results.get(i) {
                    crate::hooks::post_tool_use(
                        &self.cfg,
                        &ctx,
                        &tool_uses[i].1,
                        &tool_uses[i].2,
                        content,
                        *is_error,
                    )
                    .await;
                }
            }
        }
    }

    fn snapshot_ctx(&self) -> IsolatedCtx {
        IsolatedCtx {
            cwd: self.state.cwd.clone(),
            workspace_root: self.state.workspace_root.clone(),
            cfg: Arc::clone(&self.cfg),
            provider: Arc::clone(&self.provider),
            files_read: self.state.files_read.clone(),
            file_stats: self.state.file_stats.clone(),
            background: self
                .state
                .background
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            next_bg_id: self.state.bg_counter + 1,
            cancel: Arc::clone(&self.cancel),
            checkpoint_dir: self.checkpoint_dir(),
            artifacts_dir: self.artifacts_dir(),
            journal_next: self.state.edit_journal.len(),
            turns: self.state.turns,
        }
    }

    fn session_label(&self) -> String {
        self.session
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| format!("adhoc-{}", std::process::id()))
    }

    fn checkpoint_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("checkpoints").join(self.session_label())
    }

    /// Oversized tool outputs spill here (data_dir/artifacts/<session>).
    fn artifacts_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("artifacts").join(self.session_label())
    }

    /// Undo the newest `n` journaled file mutations (restores backups,
    /// deletes files that were created). Returns how many were undone.
    pub fn undo(&mut self, n: usize) -> usize {
        let mut undone = 0;
        for _ in 0..n {
            let Some(entry) = self.state.edit_journal.pop() else { break };
            let path = std::path::PathBuf::from(&entry.path);
            if entry.existed {
                if entry.backup.is_empty() {
                    self.ui.warn(&format!("no backup for {} — cannot restore", entry.path));
                    continue;
                }
                if let Err(e) = std::fs::copy(&entry.backup, &path) {
                    self.ui.warn(&format!("restore {} failed: {e}", entry.path));
                    continue;
                }
            } else if path.exists() {
                let _ = std::fs::remove_file(&path);
            }
            self.ui.info(&format!("restored {}", entry.path));
            undone += 1;
        }
        if undone > 0 {
            self.persist(&Event::Undo { count: undone });
            self.state.persisted_journal_len = self.state.edit_journal.len();
        }
        undone
    }

    fn output_block(&self, tool_use_id: &str, out: ToolOutput) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: if out.content.is_empty() { "(no output)".to_string() } else { out.content },
            images: out.images,
            is_error: out.is_error,
        }
    }

    /// Resolve a tool call: validate the tool and its input, run PreToolUse
    /// hooks, check permissions (prompting when allowed), and either produce
    /// a finished error block or a runnable plan.
    async fn plan_tool_use(&mut self, id: &str, name: &str, input: &Value) -> ToolPlan {
        self.ui.tool_start(name, &self.input_summary(name, input), input);
        let Some(tool) = self.registry.get(name) else {
            let out = ToolOutput::err(format!(
                "unknown tool '{name}' (available: {:?})",
                self.registry.names()
            ));
            self.ui.tool_end(name, &out);
            return ToolPlan::Done(self.output_block(id, out));
        };
        if !input.is_object() {
            let out = ToolOutput::err("tool input must be a JSON object");
            self.ui.tool_end(name, &out);
            return ToolPlan::Done(self.output_block(id, out));
        }
        // Doom-loop gate: the identical call has failed enough times in a
        // row; refuse to run it again until the model changes something.
        let call_key = (name.to_string(), input.to_string());
        let streak = self.failed_streak.iter().rev().take_while(|k| **k == call_key).count();
        if streak >= DOOM_STREAK {
            let out = ToolOutput::err(format!(
                "refused: this exact {name} call has failed {streak} times in a row — \
                 repeating it unchanged will not work. Diagnose the failure from the last \
                 result, change the arguments or the approach, or explain in your final \
                 message why this cannot proceed."
            ));
            self.ui.tool_end(name, &out);
            return ToolPlan::Done(self.output_block(id, out));
        }
        // PreToolUse hooks run before permission checks; a hook deny wins.
        let hook_ctx = self.hook_context();
        if let crate::hooks::HookVerdict::Block(reason) =
            crate::hooks::pre_tool_use(&self.cfg, &hook_ctx, name, input).await
        {
            let out = ToolOutput::err(format!("blocked by PreToolUse hook: {reason}"));
            self.ui.tool_end(name, &out);
            return ToolPlan::Done(self.output_block(id, out));
        }
        let arg = tool.perm_summary(input);
        let read_only = tool.is_read_only();
        let gated = tool.requires_approval();
        let decision = self.perms.check_gated(name, &arg, read_only, gated);
        // Workspace write-scoping: in modes where edits run unattended, keep
        // file writes inside the workspace root. Ask mode still lets the
        // user approve an outside path explicitly.
        if let Decision::Allow = decision {
            if (name == "write_file" || name == "edit_file") && self.cfg.restrict_writes_to_workspace {
                if let Some(path) = input.get("path").and_then(|v| v.as_str()) {
                    let resolved = crate::tools::resolve_path(&self.state.cwd, path);
                    if !resolved.starts_with(&self.state.workspace_root) {
                        let out = ToolOutput::err(format!(
                            "{} resolves outside the workspace root ({}); writes are scoped to the workspace. \
                             Set restrict_writes_to_workspace = false in myharness.toml if this is intended.",
                            resolved.display(),
                            self.state.workspace_root.display()
                        ));
                        self.ui.tool_end(name, &out);
                        return ToolPlan::Done(self.output_block(id, out));
                    }
                }
            }
        }
        let verdict = match decision {
            Decision::Allow => None,
            Decision::Deny(reason) => Some(ToolOutput::err(format!("permission denied: {reason}"))),
            Decision::Prompt => Some(self.interactive_approval(name, &arg).await),
        };
        match verdict {
            Some(out) => {
                self.ui.tool_end(name, &out);
                ToolPlan::Done(self.output_block(id, out))
            }
            None => ToolPlan::Run { tool, input: input.clone() },
        }
    }

    async fn interactive_approval(&mut self, name: &str, arg: &str) -> ToolOutput {
        // TUI mode: the frontend owns the terminal; route the prompt there
        // and wait for its answer.
        if let Some(tx) = &self.approval_tx {
            let (rtx, rrx) = tokio::sync::oneshot::channel();
            let req = ApprovalRequest {
                tool: name.to_string(),
                arg: arg.to_string(),
                respond: rtx,
            };
            if tx.send(req).is_err() {
                return ToolOutput::err("permission denied: UI closed before answering");
            }
            return match rrx.await {
                Ok(ApprovalAnswer::Once) => ToolOutput::ok("(approved — proceeding)"),
                Ok(ApprovalAnswer::Always) => {
                    let pattern = always_pattern(name, arg);
                    self.perms.allow_session(name, &pattern);
                    self.ui.info(&format!("always allowing {name}: {pattern}"));
                    ToolOutput::ok("(approved — proceeding)")
                }
                Ok(ApprovalAnswer::Deny) | Err(_) => {
                    ToolOutput::err("permission denied by user")
                }
            };
        }
        self.ui.permission_ask(name, arg);
        let mut line = String::new();
        let mut stdin = tokio::io::BufReader::new(tokio::io::stdin());
        if stdin.read_line(&mut line).await.is_err() {
            return ToolOutput::err("permission denied: could not read answer");
        }
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => ToolOutput::ok("(approved — proceeding)"),
            "a" | "always" => {
                let pattern = always_pattern(name, arg);
                self.perms.allow_session(name, &pattern);
                self.ui.info(&format!("always allowing {name}: {pattern}"));
                ToolOutput::ok("(approved — proceeding)")
            }
            _ => ToolOutput::err("permission denied by user (answer 'y' or 'a' to approve)"),
        }
    }

    fn input_summary(&self, name: &str, input: &Value) -> String {
        match name {
            "bash" => input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(80)
                .collect(),
            "read_file" | "write_file" | "edit_file" => input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "task" => input
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "grep" | "glob" => input
                .get("pattern")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(60)
                .collect(),
            "web_search" => input
                .get("query")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(60)
                .collect(),
            "repo_map" => input
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "skill" => input
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            "web_fetch" => input
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .chars()
                .take(70)
                .collect(),
            _ => String::new(),
        }
    }

    /// Collect LSP diagnostics for the edited files: one client per matching
    /// configured server, started lazily, degrading to a warning on failure.
    /// Returns None when there is nothing to report.
    async fn lsp_after_edits(&mut self, edited: &[PathBuf]) -> Option<String> {
        // (server name, uri, language id, file text) per existing file.
        let mut jobs: Vec<(String, String, String, String)> = Vec::new();
        for path in edited {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
            let server = self
                .cfg
                .lsp_servers
                .iter()
                .find(|s| s.languages.iter().any(|l| l.eq_ignore_ascii_case(&ext)))?;
            let Ok(text) = std::fs::read_to_string(path) else { continue };
            jobs.push((
                server.name.clone(),
                crate::lsp::path_to_uri(path),
                ext.clone(),
                text,
            ));
        }
        if jobs.is_empty() {
            return None;
        }
        // Group by server, preserving the per-file order.
        let mut order: Vec<String> = Vec::new();
        for name in jobs.iter().map(|(n, ..)| n.clone()) {
            if !order.contains(&name) {
                order.push(name);
            }
        }
        let mut lines: Vec<String> = Vec::new();
        for name in order {
            let server_cfg = self.cfg.lsp_servers.iter().find(|s| s.name == name).cloned()?;
            // Start the client on first use; a failed start warns once and
            // skips that server for this round.
            if !self.lsp.contains_key(&name) {
                match crate::lsp::LspClient::start(&server_cfg, &self.state.workspace_root).await {
                    Ok(c) => {
                        self.lsp.insert(name.clone(), c);
                    }
                    Err(e) => {
                        self.ui.warn(&format!("LSP server '{name}' failed to start: {e}"));
                        continue;
                    }
                }
            }
            let Some(client) = self.lsp.get_mut(&name) else { continue };
            if client.is_dead() {
                self.ui.warn(&format!("LSP server '{name}' exited; diagnostics skipped"));
                self.lsp.remove(&name);
                continue;
            }
            let mut awaited: HashSet<String> = HashSet::new();
            for (_, uri, lang, text) in jobs.iter().filter(|(n, ..)| n == &name) {
                if client.open_or_change(uri, lang, text).await.is_err() {
                    self.ui.warn(&format!("LSP server '{name}' closed its input"));
                    break;
                }
                awaited.insert(uri.clone());
            }
            if awaited.is_empty() {
                continue;
            }
            let wait = server_cfg.timeout_ms.unwrap_or(4_000);
            let diags = client.collect_diagnostics(wait, &awaited).await;
            for d in diags {
                let sev = if d.severity == 1 { "error" } else { "warning" };
                lines.push(format!("{}:{}: {}: {}", d.path, d.line, sev, d.message));
            }
        }
        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    async fn maybe_compact(&mut self) {
        if self.state.compaction_stalled {
            // Refill guard tripped: compacting again would summarize the
            // summaries. The model was told how to shrink its results;
            // /compact still works by hand.
            return;
        }
        let system = system_prompt::build_system(&self.state, false);
        if !compact::should_compact(&self.state, &system, self.cfg.compact_ratio, self.cfg.context_window) {
            return;
        }
        if let Err(e) = self.compact_now(false).await {
            self.ui.warn(&format!("compaction failed: {e}"));
        }
    }

    /// Push completion notices for background tasks the model has not heard
    /// about yet (commands and subagents alike) — the polling contract gets a
    /// push-feel without a new mechanism: the notice rides ahead of the next
    /// request as a system-injected user message.
    async fn deliver_bg_notices(&mut self) {
        let finished: Vec<(u32, Option<i32>, String)> = self
            .state
            .background
            .iter()
            .filter(|(id, t)| t.done.load(Ordering::Relaxed) && !self.state.bg_noticed.contains(id))
            .map(|(id, t)| (*id, *t.exit.lock().unwrap(), t.output.lock().unwrap().clone()))
            .collect();
        if finished.is_empty() {
            return;
        }
        let mut notice = String::from("(system) Background task(s) finished since your last look:\n");
        for (id, exit, output) in &finished {
            self.state.bg_noticed.insert(*id);
            let head: String = output.lines().take(8).collect::<Vec<_>>().join("\n");
            let head = head.chars().take(600).collect::<String>();
            notice.push_str(&format!(
                "- task #{id}: exit code {} — {}\n",
                exit.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into()),
                if head.trim().is_empty() { "(no output)" } else { &head }
            ));
        }
        notice.push_str("Use bash_output with the id for the full output if relevant.");
        let msg = Message::user_text(notice);
        self.state.messages.push(msg.clone());
        self.persist(&Event::Message(msg));
    }

    /// Compact the context. `manual = true` is the user's `/compact` (always
    /// allowed, resets the refill guard); the automatic path classifies the
    /// refill rate and can stall itself when the context refills too fast.
    pub async fn compact_now(&mut self, manual: bool) -> Result<()> {
        if self.state.messages.len() < 2 {
            self.ui.info("context is too small to be worth compacting");
            return Ok(());
        }
        let compaction_model = self
            .cfg
            .model_fast
            .clone()
            .unwrap_or_else(|| self.model.clone());
        // Gemini-style pre-pass: spill oversized tool results to artifacts
        // and leave pointers — the summary request fits, and the context
        // that refilled shrinks instead of being re-carried (the real fix
        // for the refill pathology the v0.15 guard detects).
        let spills = self.spill_oversized_results();
        let (mut summary, kept) = compact::compact(&self.provider, &compaction_model, &self.state).await?;
        // The todo list no longer rides the system prompt (cache stability);
        // fold it into the handoff so it survives compaction.
        if !self.state.todos.is_empty() {
            summary.push_str("\n\n");
            summary.push_str(self.state.render_todos().trim_end());
        }
        let kept_count = kept.len();
        // Culprit scan before the message list is replaced: the current
        // messages are exactly everything since the last compaction.
        let culprit = self.largest_result_since_compact();

        // Refill guard: an auto-compaction that triggers again within a few
        // tool results means one result is too large to carry. Two fast
        // refills in a row pause auto-compaction (a manual /compact clears
        // the pause and gives the automatic path a fresh chance).
        if manual {
            self.state.consecutive_fast_refills = 0;
            self.state.compaction_stalled = false;
        } else if self.state.tool_results_since_compact < FAST_REFILL_RESULTS {
            self.state.consecutive_fast_refills += 1;
            if self.state.consecutive_fast_refills >= FAST_REFILL_STREAK {
                self.state.compaction_stalled = true;
            }
        } else {
            self.state.consecutive_fast_refills = 0;
        }
        self.state.tool_results_since_compact = 0;
        let stalled = self.state.compaction_stalled;
        let streak = self.state.consecutive_fast_refills;

        let mut messages = vec![Message::user_text(format!(
            "[Earlier conversation was compacted into this handoff summary. Continue from here.]\n\n{summary}"
        ))];
        messages.extend(kept);
        self.state.messages = messages;
        self.state.compacted = true;
        self.persist(&Event::Compaction { summary, kept: kept_count, fast_refill_streak: streak, stalled });
        if !spills.is_empty() {
            self.persist(&Event::Spill { spills });
        }
        self.ui.info(&format!(
            "context compacted: kept {kept_count} recent messages"
        ));
        if stalled {
            let (tool, chars) = culprit.unwrap_or_else(|| ("(unknown)".to_string(), 0));
            self.ui.warn(
                "auto-compaction paused: the context refills too fast (a tool result is too large to carry)",
            );
            let msg = Message::user_text(format!(
                "(system) Auto-compaction is paused: the context refilled within a few tool \
                 calls of each of the last {FAST_REFILL_STREAK} compactions, which means one \
                 result is too large to carry. The largest result since the last compaction \
                 was {tool} ({chars} chars). Shrink what comes back: grep with output_mode \
                 \"files\" or \"count\" instead of \"content\", read_file with offset/limit \
                 ranges instead of whole files, and run long builds in the background and \
                 poll them with bash_output. /compact still works by hand."
            ));
            self.state.messages.push(msg.clone());
            self.persist(&Event::Message(msg));
        }
        Ok(())
    }

    /// Replace oversized tool results with pointers to spilled artifacts.
    /// Returns (tool_use_id, path) pairs for the Spill event. Best-effort:
    /// a failed write leaves the original content in place.
    fn spill_oversized_results(&mut self) -> Vec<(String, String)> {
        const SPILL_THRESHOLD: usize = 24_000;
        let dir = self.artifacts_dir();
        let mut spills = Vec::new();
        for m in &mut self.state.messages {
            for b in &mut m.content {
                if let ContentBlock::ToolResult { tool_use_id, content, .. } = b {
                    if content.chars().count() > SPILL_THRESHOLD {
                        if let Some(path) = crate::tools::spill_artifact(&dir, "spill", content) {
                            let n = content.chars().count();
                            let p = path.display().to_string();
                            *content = format!(
                                "[oversized output ({n} chars) spilled to {p} — read_file \
                                 it with offset/limit to inspect]"
                            );
                            spills.push((tool_use_id.clone(), p));
                        }
                    }
                }
            }
        }
        if !spills.is_empty() {
            self.ui.info(&format!("{} oversized result(s) spilled to artifacts", spills.len()));
        }
        spills
    }

    /// The largest single tool result in the current messages, with the tool
    /// name that produced it. After a compaction the message list *is*
    /// everything since that compaction, so this is exactly the culprit
    /// scan for the refill guard.
    fn largest_result_since_compact(&self) -> Option<(String, usize)> {        let mut names: HashMap<String, String> = HashMap::new();
        let mut best: Option<(String, usize)> = None;
        for m in &self.state.messages {
            for b in &m.content {
                match b {
                    ContentBlock::ToolUse { id, name, .. } => {
                        names.insert(id.clone(), name.clone());
                    }
                    ContentBlock::ToolResult { tool_use_id, content, .. } => {
                        let n = content.chars().count();
                        if n > best.as_ref().map(|(_, c)| *c).unwrap_or(0) {
                            let who = names
                                .get(tool_use_id)
                                .cloned()
                                .unwrap_or_else(|| tool_use_id.clone());
                            best = Some((who, n));
                        }
                    }
                    _ => {}
                }
            }
        }
        best
    }

    fn finish_turn(&mut self, cwd_before: &std::path::Path) {
        if self.state.cwd != cwd_before {
            self.persist(&Event::Cwd { path: self.state.cwd.display().to_string() });
        }
        if !self.state.todos.is_empty() {
            self.persist(&Event::Todos { todos: self.state.todos.clone() });
        }
        if self.state.files_read != self.state.persisted_files_read {
            let files: Vec<String> = self
                .state
                .files_read
                .iter()
                .map(|p| p.display().to_string())
                .collect();
            self.persist(&Event::FilesRead { files });
            self.state.persisted_files_read = self.state.files_read.clone();
        }
        // Read-guard fingerprints, same dump-on-change pattern.
        if self.state.file_stats != self.state.persisted_file_stats {
            let stats: Vec<(String, u64, u64)> = self
                .state
                .file_stats
                .iter()
                .map(|(p, (m, l))| (p.display().to_string(), *m, *l))
                .collect();
            self.persist(&Event::FileStats { stats });
            self.state.persisted_file_stats = self.state.file_stats.clone();
        }
        // Turn diffstat (Codex): what this turn changed, from the journal.
        if self.state.edit_journal.len() > self.journal_turn_start {
            let entries = &self.state.edit_journal[self.journal_turn_start..];
            let parts: Vec<String> = entries
                .iter()
                .map(|e| {
                    if !e.existed {
                        let lines = std::fs::read_to_string(&e.path)
                            .map(|s| s.lines().count())
                            .unwrap_or(0);
                        format!("{} (+{lines}, new)", e.path)
                    } else if e.backup.is_empty() {
                        e.path.clone()
                    } else {
                        let old = std::fs::read_to_string(&e.backup).unwrap_or_default();
                        let new = std::fs::read_to_string(&e.path).unwrap_or_default();
                        let (a, r) = line_diffstat(&old, &new);
                        format!("{} +{a} -{r}", e.path)
                    }
                })
                .collect();
            self.ui.info(&format!("turn changes: {}", parts.join("; ")));
        }
        // Journal: persist only the entries added since the last write.
        if self.state.edit_journal.len() > self.state.persisted_journal_len {
            let new_entries = self.state.edit_journal[self.state.persisted_journal_len..].to_vec();
            self.persist(&Event::Journal { entries: new_entries });
            self.state.persisted_journal_len = self.state.edit_journal.len();
        }
        // Operator-facing hint: finished-but-unnoticed background tasks will
        // surface to the model at the start of the next turn.
        let unheard = self
            .state
            .background
            .iter()
            .filter(|(id, t)| t.done.load(Ordering::Relaxed) && !self.state.bg_noticed.contains(id))
            .count();
        if unheard > 0 {
            self.ui.info(&format!(
                "{unheard} background task(s) finished — the model is told on its next turn"
            ));
        }
        self.ui.turn_footer(
            self.state.turns,
            &self.state.usage,
            self.context_estimate(),
            self.cfg.context_window,
        );
    }

    /// Rough current context size (system + messages) for the TUI's
    /// context bar; same heuristic the compaction threshold uses.
    fn context_estimate(&self) -> u64 {
        let system = system_prompt::build_system(&self.state, self.is_subagent);
        compact::should_compact_size(&self.state, &system)
    }

    /// Passive capture: the turn's prompt + final answer become trace
    /// units; identity re-derived (newest naming wins); atomic persist.
    fn capture_memory(&mut self, user_input: &str, final_text: &str) {
        if let Some(zm) = self.zero_mem.as_mut() {
            zm.capture("user", user_input);
            zm.capture("assistant", final_text);
            zm.derive_identity();
            zm.persist();
        }
    }

    /// Zero-Mem injection: identity line (first turn of the session or an
    /// identity-class query) plus up to top-k past-session snippets from
    /// deterministic retrieval. All of it rides the message stream. The zm
    /// borrow is scoped so persistence (which needs all of self) runs after.
    fn inject_memory(&mut self, user_input: &str) {
        let lines: Vec<String> = {
            let Some(zm) = self.zero_mem.as_mut() else { return };
            let first_turn = self.state.messages.len() <= 1 && !self.state.compacted;
            let mut lines: Vec<String> = Vec::new();
            if first_turn || crate::zero_mem::identity_query(user_input) {
                if let Some(line) = crate::zero_mem::build_identity_line(zm.identity()) {
                    lines.push(line);
                }
            }
            if lines.is_empty() && !user_input.trim().is_empty() {
                let window: Vec<String> =
                    self.state.messages.iter().map(|m| m.text()).collect();
                let fps = crate::zero_mem::ZeroMem::window_fingerprints(&window);
                for h in zm.retrieve(user_input, &fps) {
                    lines.push(format!("- ({} days ago, {}) {}", h.when, h.role, h.snippet));
                }
                if !lines.is_empty() {
                    lines.insert(
                        0,
                        "(prior session memory — recollections from past sessions, not authoritative)"
                            .to_string(),
                    );
                }
            }
            lines
        };
        if !lines.is_empty() {
            let msg = Message::user_text(lines.join("
"));
            self.state.messages.push(msg.clone());
            self.persist(&Event::Message(msg));
        }
    }

    /// One agent-only turn-context message per user turn (Goose MOIM):
    /// time, cwd, turn budget, and — once meaningful — context headroom.
    fn inject_turn_context(&mut self) {
        let est = self.context_estimate();
        if est < MOIM_MIN_TOKENS {
            return;
        }
        let window = self.cfg.context_window;
        let pct = ((est as f64 / window.max(1) as f64) * 100.0).clamp(0.0, 100.0) as u64;
        let cap = self.effective_turn_cap() as u64;
        let mut parts = vec![
            chrono::Local::now().format("%Y-%m-%d %H:%M").to_string(),
            format!("cwd: {}", self.state.cwd.display()),
            format!("turn budget: {}/{}", self.state.turns.min(cap), cap),
        ];
        if pct >= 40 {
            parts.push(format!("context: {pct}% of window used"));
        }
        let msg = Message::user_text(format!("(turn context) {}", parts.join(" | ")));
        self.state.messages.push(msg.clone());
        self.persist(&Event::Message(msg));
    }

    /// JIT instruction loading (Gemini): when a file tool touches a path,
    /// inject any not-yet-seen AGENTS.md found walking that path's
    /// ancestors up through the workspace — capped per batch so a broad
    /// read can't flood the context.
    fn jit_instructions(&mut self, tool_uses: &[(String, String, Value)]) {
        const MAX_NEW: usize = 2;
        let mut found: Vec<(PathBuf, String)> = Vec::new();
        'outer: for (_, name, input) in tool_uses {
            if !matches!(name.as_str(), "read_file" | "write_file" | "edit_file") {
                continue;
            }
            let Some(p) = input.get("path").and_then(Value::as_str) else { continue };
            let resolved = crate::tools::resolve_path(&self.state.cwd, p);
            let mut dir = resolved.parent().map(PathBuf::from);
            while let Some(d) = dir {
                if d.starts_with(&self.state.workspace_root) {
                    let cand = d.join("AGENTS.md");
                    if let Ok(canon) = cand.canonicalize() {
                        let known = self
                            .state
                            .project_context_path
                            .as_ref()
                            .is_some_and(|pp| *pp == canon)
                            || self.state.instructions_loaded.contains(&canon)
                            || found.iter().any(|(p, _)| *p == canon);
                        if !known {
                            if let Ok(raw) = std::fs::read_to_string(&cand) {
                                let capped: String = raw.chars().take(8_000).collect();
                                found.push((canon, capped));
                                if found.len() >= MAX_NEW {
                                    break 'outer;
                                }
                            }
                        }
                    }
                }
                dir = d.parent().map(PathBuf::from);
            }
        }
        for (path, body) in found {
            self.state.instructions_loaded.push(path.clone());
            let msg = Message::user_text(format!(
                "(project instructions from {})\n{body}",
                path.display()
            ));
            self.state.messages.push(msg.clone());
            self.persist(&Event::Message(msg));
            self.persist(&Event::InstructionsLoaded {
                paths: vec![path.display().to_string()],
            });
        }
    }

    pub fn clear(&mut self) {
        self.state.messages.clear();
        self.state.todos.clear();
        self.state.usage = Usage::default();
        self.state.turns = 0;
        self.persist(&Event::Clear);
        self.ui.info("context cleared");
    }
}

/// Owned snapshot used to run one tool in isolation (parallel-safe).
struct IsolatedCtx {
    cwd: PathBuf,
    workspace_root: PathBuf,
    cfg: Arc<crate::config::Config>,
    provider: Arc<dyn Provider>,
    files_read: HashSet<PathBuf>,
    file_stats: HashMap<PathBuf, (u64, u64)>,
    background: std::collections::HashMap<u32, state::BgTask>,
    next_bg_id: u32,
    cancel: Arc<AtomicBool>,
    checkpoint_dir: PathBuf,
    artifacts_dir: PathBuf,
    journal_next: usize,
    turns: u64,
}

/// Execute one tool with its own effect collector. All inputs are owned, so
/// this can be awaited directly or spawned for parallelism.
async fn run_isolated(tool: Arc<dyn Tool>, input: Value, iso: IsolatedCtx) -> (ToolOutput, ToolEffects) {
    let mut effects = ToolEffects::default();
    let mut ctx = ToolCtx {
        cwd: iso.cwd,
        workspace_root: iso.workspace_root,
        cfg: iso.cfg,
        provider: iso.provider,
        files_read: &iso.files_read,
        file_stats: &iso.file_stats,
        background: &iso.background,
        next_bg_id: iso.next_bg_id,
        cancel: iso.cancel,
        checkpoint_dir: iso.checkpoint_dir,
        artifacts_dir: iso.artifacts_dir,
        journal_next: iso.journal_next,
        turns: iso.turns,
        effects: &mut effects,
    };
    let out = tool.execute(input, &mut ctx).await;
    (out, effects)
}

fn tool_uses_of(blocks: &[ContentBlock]) -> Vec<(String, String, Value)> {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, input } => {
                Some((id.clone(), name.clone(), input.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Pattern recorded for "always allow": first word + wildcard for commands
/// (so `cargo build` also allows `cargo test`), literal for paths.
pub(crate) fn always_pattern(tool: &str, arg: &str) -> String {
    if tool == "bash" {
        let first = arg.split_whitespace().next().unwrap_or(arg);
        format!("{first} *")
    } else {
        glob::Pattern::escape(arg)
    }
}

/// Line-level diff stat for the turn diffstat: exact LCS counts for
/// reasonable files, net-line approximation beyond the cap.
pub(crate) fn line_diffstat(old: &str, new: &str) -> (u64, u64) {
    const CAP: usize = 800;
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len() > CAP || b.len() > CAP {
        let net = b.len() as i64 - a.len() as i64;
        return (net.max(0) as u64, (-net).max(0) as u64);
    }
    let mut prev = vec![0u32; b.len() + 1];
    let mut cur = vec![0u32; b.len() + 1];
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            cur[j] = if a[i - 1] == b[j - 1] {
                prev[j - 1] + 1
            } else {
                prev[j].max(cur[j - 1])
            };
        }
        std::mem::swap(&mut prev, &mut cur);
        cur.iter_mut().for_each(|x| *x = 0);
    }
    let lcs = prev[b.len()] as usize;
    ((b.len() - lcs) as u64, (a.len() - lcs) as u64)
}

/// Fire output-pattern hints for a batch of tool results: every pattern
/// whose substring appears in any result fires once, throttled to one shot
/// per minute per pattern. Returns the hint texts to inject, in table order.
fn scan_output_hints(
    results: &[ContentBlock],
    hints: &[crate::config::OutputHintDef],
    throttle: &mut HashMap<String, std::time::Instant>,
) -> Vec<String> {
    const COOLDOWN: std::time::Duration = std::time::Duration::from_secs(60);
    let now = std::time::Instant::now();
    let mut fired = Vec::new();
    for h in hints {
        if h.pattern.is_empty() {
            continue;
        }
        let hit = results.iter().any(|b| match b {
            ContentBlock::ToolResult { content, .. } => content.contains(&h.pattern),
            _ => false,
        });
        if !hit {
            continue;
        }
        if let Some(last) = throttle.get(&h.pattern) {
            if now.duration_since(*last) < COOLDOWN {
                continue;
            }
        }
        throttle.insert(h.pattern.clone(), now);
        fired.push(h.hint.clone());
    }
    fired
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_diffstat_exact_and_capped() {
        let old = "a
b
c
d";
        let new = "b
c
e";
        assert_eq!(line_diffstat(old, new), (1, 2)); // add e; drop a and d
        assert_eq!(line_diffstat(old, old), (0, 0));
        assert_eq!(line_diffstat("", "x
y"), (2, 0));
        // Over the cap: net-line approximation.
        let big_old = vec!["l"; 900].join("
");
        let big_new = vec!["l"; 1000].join("
");
        assert_eq!(line_diffstat(&big_old, &big_new), (100, 0));
    }

    /// Cost guard: the model-facing payload (system prompt + tool
    /// schemas) is the per-request floor. This pins it so it can only
    /// shrink or grow deliberately. Print with --nocapture to see sizes.
    #[test]
    fn prompt_budget_stays_lean() {
        let dir = tempfile::tempdir().unwrap();
        let state = state::AgentState::new(dir.path().to_path_buf());
        let system = system_prompt::build_system(&state, false);
        let schemas = serde_json::to_string(&Registry::full().schemas()).unwrap();
        eprintln!(
            "-- prompt budget: system {} chars, tool schemas {} chars, floor ~{} tokens/request",
            system.len(),
            schemas.len(),
            (system.len() + schemas.len()) / 4
        );
        assert!(system.len() < 9_000, "system prompt crept up: {} chars", system.len());
        assert!(schemas.len() < 30_000, "tool schemas crept up: {} chars", schemas.len());
    }

    #[test]
    fn hints_fire_once_then_throttle() {
        let hints = vec![crate::config::OutputHintDef {
            pattern: "rate limit".to_string(),
            hint: "back off".to_string(),
        }];
        let result = ContentBlock::ToolResult {
            tool_use_id: "1".to_string(),
            content: "gh: API rate limit exceeded".to_string(),
            images: Vec::new(),
            is_error: false,
        };
        let mut throttle = HashMap::new();
        let first = scan_output_hints(std::slice::from_ref(&result), &hints, &mut throttle);
        assert_eq!(first, vec!["back off".to_string()]);
        // Same pattern within the cooldown window: silent.
        let second = scan_output_hints(std::slice::from_ref(&result), &hints, &mut throttle);
        assert!(second.is_empty());
        // Non-matching results never fire.
        let other = ContentBlock::ToolResult {
            tool_use_id: "2".to_string(),
            content: "all good".to_string(),
            images: Vec::new(),
            is_error: false,
        };
        let mut fresh = HashMap::new();
        assert!(scan_output_hints(std::slice::from_ref(&other), &hints, &mut fresh).is_empty());
    }

    #[test]
    fn largest_result_names_the_tool() {
        let mut state = state::AgentState::new(std::path::PathBuf::from("."));
        state.messages = vec![
            Message::user_text("go"),
            Message {
                role: Role::Assistant,
                content: vec![
                    ContentBlock::ToolUse {
                        id: "t1".into(),
                        name: "bash".into(),
                        input: serde_json::json!({}),
                    },
                    ContentBlock::ToolUse {
                        id: "t2".into(),
                        name: "read_file".into(),
                        input: serde_json::json!({}),
                    },
                ],
            },
            Message {
                role: Role::User,
                content: vec![
                    ContentBlock::ToolResult {
                        tool_use_id: "t1".into(),
                        content: "x".repeat(500),
                        images: Vec::new(),
                        is_error: false,
                    },
                    ContentBlock::ToolResult {
                        tool_use_id: "t2".into(),
                        content: "y".repeat(9_000),
                        images: Vec::new(),
                        is_error: false,
                    },
                ],
            },
        ];
        let agent = Agent {
            provider: Arc::new(crate::llm::mock::MockProvider::new(vec![])),
            cfg: Arc::new(test_cfg()),
            state,
            session: None,
            ui: Ui::quiet(),
            registry: Registry::full(),
            perms: crate::perms::PermissionEngine::new(crate::perms::PermissionMode::Yolo, vec![], vec![], true),
            model: "mock".to_string(),
            cancel: Arc::new(AtomicBool::new(false)),
            is_subagent: false,
            output_schema: None,
            turn_cap: None,
            stop_blocks: 0,
            schema_retries: 0,
            lsp: HashMap::new(),
            hint_throttle: HashMap::new(),
            approval_tx: None,
            steer_rx: None,
            reflect_rounds: 0,
            verify_failed: false,
            failed_streak: Vec::new(),
            journal_turn_start: 0,
            zero_mem: None,
        };
        assert_eq!(agent.largest_result_since_compact(), Some(("read_file".to_string(), 9_000)));
    }

    fn test_cfg() -> crate::config::Config {
        // Minimal offline config; only data_dir is consulted by these paths.
        crate::config::Config {
            provider: crate::config::ProviderKind::Mock,
            model: "mock".into(),
            model_fast: None,
            base_url: "http://mock.invalid".into(),
            api_key: None,
            max_tokens: 128,
            temperature: 0.1,
            context_window: 200_000,
            max_turns: 10,
            compact_ratio: 0.8,
            verify_cmd: None,
            restrict_writes_to_workspace: true,
            prompt_caching: false,
            bash_timeout_ms: 5_000,
            shell: crate::config::ShellChoice::Auto,
            allow_rules: Vec::new(),
            deny_rules: Vec::new(),
            hooks: Vec::new(),
            mcp_servers: Vec::new(),
            lsp_servers: Vec::new(),
            sandbox: crate::config::SandboxMode::Off,
            data_dir: std::env::temp_dir(),
            web_fetch_private_hosts: false,
            output_hints: Vec::new(),
            zero_mem: crate::zero_mem::ZeroMemCfg::default(),
            verbose: false,
            non_interactive: false,
        }
    }
}

