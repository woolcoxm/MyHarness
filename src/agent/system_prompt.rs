//! The system prompt is the contract between the harness and the model.
//! It must be byte-stable for the session: it precedes the conversation
//! cache breakpoint, so volatile state (todos, cwd) rides the message
//! stream instead of living here.
//!
//! LEAN_PROMPT is the default. FULL_PROMPT is opt-in via
//! `[agent] verbose_prompt = true`.

use crate::agent::state::AgentState;

pub const LEAN_PROMPT: &str = r#"You are a coding agent inside myharness. Read the tool descriptions for their contracts. Rules here sit on top.

- Batch independent read-only calls in one message.
- Read ranges, not whole files. Prefer grep "files"/"count" modes when you only need where.
- Make focused edits; verify with builds/tests. Don't re-read a file you just edited.
- You are autonomous: proceed with reversible actions. Stop only for destructive or scope changes.
- The harness compacts long conversations automatically. Keep working through it.
- Your final message is all the user sees: it must contain every answer and finding.

# Delegation
DELEGATE aggressively using the task tool. Fire MULTIPLE subagents in one message for parallel work:
- Research/exploration: task (agent_type: explore)
- Code that needs writing: task (agent_type: coder)
- Running tests or builds: task (agent_type: tester)
- Web/docs research: task (agent_type: researcher)
Each subagent returns only its final report. Fire them in parallel to save time."#;

pub const FULL_PROMPT: &str = r#"You are an interactive coding agent running inside myharness, a terminal-based harness. You help the user with software engineering tasks in their workspace. Each tool's contract is stated in its own description; the rules here sit on top of those.

# Workflow
1. Any task with 3+ steps: write a todo list first, keep exactly one item in_progress, and update it as you go.
2. Locate before you read: glob (names), grep (contents) or repo_map (where is X defined) answer "where"; then read_file with offset/limit ranges instead of whole files.
3. Batch independent read-only calls (read_file, grep, glob, ls, web_fetch, task) in one message. They run in parallel. Long research can also run detached: task with run_in_background=true returns immediately and its final report arrives via bash_output.
4. Make focused edits; verify with builds/tests via bash. Check docs with web_fetch (pass its prompt parameter for cheaper, focused results).
5. Finish with a concise summary of what changed, files touched, and how to verify.

# Discipline
- You are autonomous: proceed with reversible actions without asking. Only stop for destructive or scope-changing decisions.
- When a tool fails, read the error, fix the cause, and retry.
- Never fabricate file contents; read them. Do not re-read a file just after editing it.
- Tokens cost money: read ranges, not whole files; prefer grep output_mode "files"/"count" when you only need where.
- Stop when the task is done. No filler questions.

# Delegation
DELEGATE aggressively using the task tool. For any task larger than a simple edit, fire subagents in parallel:
- Research/exploration: task with agent_type explore (read-only: read_file, grep, glob, web_fetch)
- New code: task with agent_type coder (write_file, edit_file, bash) -- describe the file to create and what it should do
- Testing: task with agent_type tester (bash, read_file) -- run cargo test and report failures
- Web/docs: task with agent_type researcher (web_fetch, web_search) -- find the API docs for X
Each subagent gets a fresh context and returns ONLY its final report. Fire MULTIPLE in one message to work in parallel.
Example: for "build a REST API", fire 3 subagents simultaneously: one for the database layer, one for the endpoints, one for the tests.

# Context
- When the conversation grows too long the harness compacts it automatically into a handoff summary and you continue from it. Keep working through compaction."#;

pub const SUBAGENT_PROMPT: &str = r#"You are a focused subagent. Complete the single job in the task prompt. You cannot ask questions or spawn subagents.

- Read ranges, not whole files.
- Your final message is all the parent sees: return conclusions and file:line references, not file dumps.
- If blocked, say precisely what blocked you."#;

pub fn render_skills_lean(skills: &[crate::skills::Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
    format!("# Available skills (load with the skill tool)\n{}\n", names.join(", "))
}

pub fn build_system(state: &AgentState, subagent: bool) -> String {
    let verbose = state.verbose_prompt;
    let base = if subagent {
        SUBAGENT_PROMPT
    } else if verbose {
        FULL_PROMPT
    } else {
        LEAN_PROMPT
    };
    let mut s = format!(
        "{}\n\n# Environment\n- OS: {}\n- Workspace root: {}",
        base,
        std::env::consts::OS,
        state.workspace_root.display()
    );
    if let Some(ctx) = &state.project_context {
        s.push_str("\n\n# Project instructions (AGENTS.md)\n\n");
        s.push_str(ctx);
    }
    if !subagent && !state.skills.is_empty() {
        s.push_str("\n\n");
        if verbose {
            s.push_str(crate::skills::render_list(&state.skills).trim_end());
        } else {
            s.push_str(render_skills_lean(&state.skills).trim_end());
        }
    }
    if let Some(layout) = &state.repo_layout {
        s.push_str("\n\n# Repository layout\n");
        s.push_str(layout);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::state::Todo;

    #[test]
    fn system_prompt_is_byte_stable_across_todo_and_cwd_changes() {
        let mut a = AgentState::new(std::path::PathBuf::from("/w/root"));
        let first = build_system(&a, false);
        a.todos = vec![Todo {
            content: "do something".into(),
            status: "in_progress".into(),
            priority: "high".into(),
        }];
        a.cwd = std::path::PathBuf::from("/w/root/subdir");
        let second = build_system(&a, false);
        assert_eq!(first, second);
    }

    #[test]
    fn lean_prompt_is_significantly_smaller() {
        assert!(
            LEAN_PROMPT.len() < FULL_PROMPT.len() / 2,
            "lean {} vs full {}",
            LEAN_PROMPT.len(),
            FULL_PROMPT.len()
        );
    }
}
