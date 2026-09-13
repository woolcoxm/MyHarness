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
use crate::ui::Ui;
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
}

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
        }
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
        if !user_input.trim().is_empty() {
            let msg = Message::user_text(user_input);
            self.state.messages.push(msg.clone());
            self.persist(&Event::Message(msg));
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
            let (blocks, interrupted) = self.consume_stream(&mut rx).await?;
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

            if tool_uses.is_empty() || self.cancelled() {
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
                self.finish_turn(&cwd_before);
                return Ok(TurnOutcome { final_text, interrupted: false });
            }

            let results = self.execute_tool_uses(&tool_uses).await;
            let results_msg = Message::tool_results(results);
            self.state.messages.push(results_msg.clone());
            self.persist(&Event::Message(results_msg));

            // Opt-in verification loop: after edits, run the project's
            // verify command and hand the model the result before it goes on.
            let edits_made = tool_uses.iter().any(|(_, n, _)| n == "write_file" || n == "edit_file");
            if edits_made {
                if let Some(cmd) = self.cfg.verify_cmd.clone() {
                    let report = crate::tools::bash::run_verify(&self.cfg, &self.state.cwd, &cmd).await;
                    let msg = Message::user_text(format!(
                        "(auto-verification after edits) `{cmd}`\n{report}"
                    ));
                    self.state.messages.push(msg.clone());
                    self.persist(&Event::Message(msg));
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
                self.finish_turn(&cwd_before);
                return Ok(TurnOutcome { final_text, interrupted: false });
            }
        }
    }

    /// Consume one streaming response into content blocks; text deltas are
    /// printed live.
    async fn consume_stream(
        &mut self,
        rx: &mut tokio::sync::mpsc::Receiver<Result<StreamEvent>>,
    ) -> Result<(Vec<ContentBlock>, bool)> {
        let mut text = String::new();
        let mut tools: Vec<(String, String, String)> = Vec::new();
        let mut usage_acc = Usage::default();
        let mut interrupted = false;
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
                StreamEvent::MessageDelta { .. } => {}
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
        Ok((blocks, interrupted))
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
            background: self
                .state
                .background
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            next_bg_id: self.state.bg_counter + 1,
            cancel: Arc::clone(&self.cancel),
            checkpoint_dir: self.checkpoint_dir(),
            journal_next: self.state.edit_journal.len(),
            turns: self.state.turns,
        }
    }

    fn checkpoint_dir(&self) -> PathBuf {
        let session = self
            .session
            .as_ref()
            .map(|s| s.id.clone())
            .unwrap_or_else(|| format!("adhoc-{}", std::process::id()));
        self.cfg.data_dir.join("checkpoints").join(session)
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
        self.ui.tool_start(name, &self.input_summary(name, input));
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

    async fn maybe_compact(&mut self) {        let system = system_prompt::build_system(&self.state, false);
        if !compact::should_compact(&self.state, &system, self.cfg.compact_ratio, self.cfg.context_window) {
            return;
        }
        if let Err(e) = self.compact_now().await {
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

    pub async fn compact_now(&mut self) -> Result<()> {
        if self.state.messages.len() < 2 {
            self.ui.info("context is too small to be worth compacting");
            return Ok(());
        }
        let compaction_model = self
            .cfg
            .model_fast
            .clone()
            .unwrap_or_else(|| self.model.clone());
        let (summary, kept) = compact::compact(&self.provider, &compaction_model, &self.state).await?;
        let kept_count = kept.len();
        let mut messages = vec![Message::user_text(format!(
            "[Earlier conversation was compacted into this handoff summary. Continue from here.]\n\n{summary}"
        ))];
        messages.extend(kept);
        self.state.messages = messages;
        self.state.compacted = true;
        self.persist(&Event::Compaction { summary, kept: kept_count });
        self.ui.info(&format!(
            "context compacted: kept {kept_count} recent messages"
        ));
        Ok(())
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
            self.state.usage.input_tokens,
            self.state.usage.output_tokens,
        );
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
    background: std::collections::HashMap<u32, state::BgTask>,
    next_bg_id: u32,
    cancel: Arc<AtomicBool>,
    checkpoint_dir: PathBuf,
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
        background: &iso.background,
        next_bg_id: iso.next_bg_id,
        cancel: iso.cancel,
        checkpoint_dir: iso.checkpoint_dir,
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
fn always_pattern(tool: &str, arg: &str) -> String {
    if tool == "bash" {
        let first = arg.split_whitespace().next().unwrap_or(arg);
        format!("{first} *")
    } else {
        glob::Pattern::escape(arg)
    }
}
