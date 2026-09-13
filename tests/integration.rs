//! End-to-end tests driving the real agent loop with the scripted mock
//! provider: tool execution, permission enforcement, compaction, subagents,
//! and session round-trips. No network involved.

use myharness::agent::state::AgentState;
use myharness::agent::Agent;
use myharness::config::{Config, ProviderKind, Rule, ShellChoice};
use myharness::llm::mock::MockProvider;
use myharness::llm::{ContentBlock, Provider, Role};
use myharness::perms::{PermissionEngine, PermissionMode, Decision};
use myharness::session::Session;
use myharness::tools::Registry;
use myharness::ui::Ui;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn test_config(dir: &Path) -> Arc<Config> {
    Arc::new(Config {
        provider: ProviderKind::Mock,
        model: "mock-model".to_string(),
        model_fast: None,
        base_url: "http://mock.invalid".to_string(),
        api_key: None,
        max_tokens: 4096,
        temperature: 0.3,
        context_window: 200_000,
        max_turns: 10,
        compact_ratio: 0.8,
        verify_cmd: None,
        restrict_writes_to_workspace: true,
        prompt_caching: true,
        bash_timeout_ms: 60_000,
        shell: ShellChoice::Auto,
        allow_rules: vec![],
        deny_rules: vec![],
        hooks: vec![],
        mcp_servers: vec![],
        lsp_servers: vec![],
        sandbox: myharness::config::SandboxMode::Off,
        data_dir: dir.join("data"),
        verbose: false,
        non_interactive: true,
    })
}

fn agent_with(
    script: Vec<serde_json::Value>,
    dir: &Path,
    mode: PermissionMode,
    session: Option<Session>,
) -> (Agent, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.to_path_buf());
    let agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        test_config(dir),
        state,
        Ui::quiet(),
        session,
        Registry::full(),
        PermissionEngine::new(mode, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    (agent, provider)
}

fn tool_result_texts(agent: &Agent) -> Vec<(String, bool)> {
    agent
        .state
        .messages
        .iter()
        .filter(|m| m.role == Role::User)
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolResult { content, is_error, .. } => {
                Some((content.clone(), *is_error))
            }
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn write_read_finish_flow() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "creating the file", "tool_calls": [
            {"name": "write_file", "input": {"path": "hello.txt", "content": "line1\nline2\nline3\n"}}
        ]}),
        json!({"text": "reading it back", "tool_calls": [
            {"name": "read_file", "input": {"path": "hello.txt"}}
        ]}),
        json!({"text": "all done: hello.txt verified"}),
    ];
    let session = Session::create(&dir.path().join("data").join("sessions"), "mock-model", dir.path()).unwrap();
    let session_path = session.path.clone();
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, Some(session));

    let outcome = agent.run_turn("create hello.txt with three lines").await.unwrap();
    assert!(!outcome.interrupted);
    assert!(outcome.final_text.contains("all done"));

    let content = std::fs::read_to_string(dir.path().join("hello.txt")).unwrap();
    assert_eq!(content, "line1\nline2\nline3\n");

    // The read result the model saw is cat -n formatted.
    let results = tool_result_texts(&agent);
    assert!(results.iter().any(|(c, err)| !err && c.contains("     2\tline2")), "read output not cat -n formatted: {results:?}");

    // Session file captured the whole exchange.
    let raw = std::fs::read_to_string(&session_path).unwrap();
    assert!(raw.contains("\"type\":\"message\""));
    assert!(raw.lines().count() >= 5);
}

#[tokio::test]
async fn edit_requires_read_and_uniqueness() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "foo\nfoo\n").unwrap();

    let script = vec![
        // Edit before read: must fail with the read-first guard.
        json!({"text": "editing", "tool_calls": [
            {"name": "edit_file", "input": {"path": "a.txt", "old_string": "foo", "new_string": "bar"}}
        ]}),
        // Read it.
        json!({"text": "reading first", "tool_calls": [
            {"name": "read_file", "input": {"path": "a.txt"}}
        ]}),
        // Ambiguous edit: two matches, must fail with count.
        json!({"text": "try again", "tool_calls": [
            {"name": "edit_file", "input": {"path": "a.txt", "old_string": "foo", "new_string": "baz"}}
        ]}),
        // replace_all works.
        json!({"text": "bulk replace", "tool_calls": [
            {"name": "edit_file", "input": {"path": "a.txt", "old_string": "foo", "new_string": "bar", "replace_all": true}}
        ]}),
        json!({"text": "edited with replace_all"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);

    agent.run_turn("change foo to bar").await.unwrap();
    let results = tool_result_texts(&agent);

    assert!(results[0].1, "edit before read should be an error");
    assert!(results[0].0.contains("not been read"));
    assert!(results[2].1, "ambiguous edit should be an error");
    assert!(results[2].0.contains("2 times"), "error should name the match count: {}", results[2].0);
    assert!(!results[3].1, "replace_all should succeed");
    assert_eq!(std::fs::read_to_string(dir.path().join("a.txt")).unwrap(), "bar\nbar\n");
}

#[tokio::test]
async fn write_refuses_blind_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("exists.txt"), "original\n").unwrap();
    let script = vec![
        json!({"text": "overwrite", "tool_calls": [
            {"name": "write_file", "input": {"path": "exists.txt", "content": "clobbered\n"}}
        ]}),
        json!({"text": "denied, done"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("overwrite exists.txt").await.unwrap();
    // File untouched: the tool refused.
    assert_eq!(std::fs::read_to_string(dir.path().join("exists.txt")).unwrap(), "original\n");
}

#[tokio::test]
async fn bash_runs_and_reports() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "running", "tool_calls": [
            {"name": "bash", "input": {"command": "echo mh_test_ok"}}
        ]}),
        json!({"text": "echo ran fine"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("echo something").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("mh_test_ok"), "output missing: {}", results[0].0);
    assert!(results[0].0.contains("Exit code: 0"));
}

#[tokio::test]
async fn plan_mode_blocks_mutations_but_allows_reads() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "planning", "tool_calls": [
            {"name": "glob", "input": {"pattern": "*.txt"}},
            {"name": "write_file", "input": {"path": "x.txt", "content": "nope\n"}}
        ]}),
        json!({"text": "here is my plan"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Plan, None);
    agent.run_turn("plan a thing").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(!results[0].1, "glob should be allowed in plan mode");
    assert!(results[1].1, "write should be denied in plan mode");
    assert!(results[1].0.to_lowercase().contains("plan mode"));
    assert!(!dir.path().join("x.txt").exists());
}

#[tokio::test]
async fn non_interactive_bash_is_denied_without_allow_rule() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "trying bash", "tool_calls": [
            {"name": "bash", "input": {"command": "echo hi"}}
        ]}),
        json!({"text": "bash denied, finishing"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::AutoEdit, None);
    agent.run_turn("echo hi").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].1, "bash should be denied in non-interactive auto-edit");
    assert!(results[0].0.contains("--yolo") || results[0].0.contains("allow rule"));
}

#[test]
fn permission_engine_rules() {
    let deny = Rule {
        tool: "bash".to_string(),
        pattern: glob::Pattern::new("rm *").unwrap(),
        raw_pattern: "rm *".to_string(),
    };
    let allow = Rule {
        tool: "bash".to_string(),
        pattern: glob::Pattern::new("git *").unwrap(),
        raw_pattern: "git *".to_string(),
    };
    let engine = PermissionEngine::new(PermissionMode::Yolo, vec![allow.clone()], vec![deny], false);

    // Deny wins even in yolo, including via compound splitting.
    assert!(matches!(engine.check("bash", "rm -rf /", false), Decision::Deny(_)));
    assert!(matches!(
        engine.check("bash", "git status && rm -rf /tmp/x", false),
        Decision::Deny(_)
    ));
    // Allow must cover every subcommand (checked in a gated mode, since
    // Yolo's default is Allow anyway).
    assert!(matches!(engine.check("bash", "git status", false), Decision::Allow));
    let gated = PermissionEngine::new(PermissionMode::Ask, vec![allow], vec![], false);
    assert!(matches!(
        gated.check("bash", "git status && cargo build", false),
        Decision::Prompt
    ));
    assert!(matches!(gated.check("bash", "git status", false), Decision::Allow));
    // Splitting behavior.
    let parts = PermissionEngine::split_compound("a && b ; c | d || e");
    assert_eq!(parts, vec!["a", "b", "c", "d", "e"]);
}

#[tokio::test]
async fn compaction_triggers_and_persists() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg_owner = test_config(dir.path());
    let mut cfg = (*cfg_owner).clone();
    cfg.context_window = 600; // threshold 480 < the 800-overhead estimate
    cfg_owner = Arc::new(cfg);

    let provider = Arc::new(MockProvider::new(vec![
        json!({"text": "step one", "tool_calls": [
            {"name": "write_file", "input": {"path": "t.txt", "content": "hi\n"}}
        ]}),
        json!({"text": "compacted and finished"}),
    ]));
    let state = AgentState::new(dir.path().to_path_buf());
    let session = Session::create(&dir.path().join("s"), "mock-model", dir.path()).unwrap();
    let session_path = session.path.clone();
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg_owner,
        state,
        Ui::quiet(),
        Some(session),
        Registry::full(),
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    let outcome = agent.run_turn("do the thing").await.unwrap();
    assert!(outcome.final_text.contains("finished"));
    assert!(agent.state.compacted, "compaction should have triggered");
    // Compaction event persisted and the summary leads the transcript.
    let raw = std::fs::read_to_string(&session_path).unwrap();
    assert!(raw.contains("\"type\":\"compaction\""));
}

#[tokio::test]
async fn subagent_returns_final_report_only() {
    let dir = tempfile::tempdir().unwrap();
    // Step 1: parent spawns subagent. The subagent consumes step 2 (its own
    // final report). Step 3: parent's final answer.
    let script = vec![
        json!({"text": "delegating", "tool_calls": [
            {"name": "task", "input": {
                "description": "explore the repo",
                "prompt": "count the files and report"
            }}
        ]}),
        json!({"text": "SUBAGENT REPORT: found 0 files, all quiet"}),
        json!({"text": "the subagent said everything is quiet"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    let outcome = agent.run_turn("check the repo").await.unwrap();
    // The parent's context contains the subagent's report as a tool result,
    // but NOT the subagent's internal messages.
    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("SUBAGENT REPORT"), "report missing: {:?}", results);
    assert_eq!(outcome.final_text.trim(), "the subagent said everything is quiet");
}

#[tokio::test]
async fn session_replay_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(&dir.path().join("s"), "mock-model", dir.path()).unwrap();
    let script = vec![
        json!({"text": "writing", "tool_calls": [
            {"name": "write_file", "input": {"path": "r.txt", "content": "round trip\n"}}
        ]}),
        json!({"text": "done for now"}),
    ];
    let session_path = session.path.clone();
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, Some(session));
    agent.run_turn("make r.txt").await.unwrap();
    let count = agent.state.messages.len();

    // Replay into a fresh state.
    let events = Session::read_events(&session_path).unwrap();
    let replayed = Session::replay(events, dir.path().to_path_buf());
    assert_eq!(replayed.messages.len(), count);
    assert_eq!(replayed.messages.first().unwrap().text(), "make r.txt");
    assert!(replayed.messages.iter().any(|m| m.text().contains("done for now")));
}

#[tokio::test]
async fn todo_write_updates_state() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "tracking", "tool_calls": [
            {"name": "todo_write", "input": {"todos": [
                {"content": "first", "status": "completed", "priority": "high"},
                {"content": "second", "status": "in_progress"}
            ]}}
        ]}),
        json!({"text": "todos set"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("track tasks").await.unwrap();
    assert_eq!(agent.state.todos.len(), 2);
    assert_eq!(agent.state.todos[0].status, "completed");
    // The rendered list is visible to the model in the tool result.
    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("[x] (high) first"));
    assert!(results[0].0.contains("[~] second"));
}

#[tokio::test]
async fn parallel_readonly_tools_all_execute() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("one.txt"), "1\n").unwrap();
    std::fs::write(dir.path().join("two.txt"), "2\n").unwrap();
    std::fs::write(dir.path().join("three.txt"), "3\n").unwrap();

    // Three read-only calls in ONE assistant message: they fan out in
    // parallel and all must execute, in the model's order in the results.
    let script = vec![
        json!({"text": "reading all three at once", "tool_calls": [
            {"name": "read_file", "input": {"path": "one.txt"}},
            {"name": "read_file", "input": {"path": "two.txt"}},
            {"name": "grep", "input": {"pattern": "3"}}
        ]}),
        json!({"text": "all three done"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("read everything").await.unwrap();
    let results = tool_result_texts(&agent);
    assert_eq!(results.len(), 3, "all three calls must produce results");
    assert!(results[0].0.contains("1"));
    assert!(results[1].0.contains("2"));
    assert!(results[2].0.contains("three.txt:1:3"), "grep found: {}", results[2].0);
    assert!(!results.iter().any(|(_, err)| *err));
}

#[tokio::test]
async fn background_bash_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "starting a long build in the background", "tool_calls": [
            {"name": "bash", "input": {"command": "echo bg_started", "run_in_background": true}}
        ]}),
        json!({"text": "polling now", "tool_calls": [
            {"name": "bash_output", "input": {"id": 1}}
        ]}),
        json!({"text": "background task handled"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("run the build").await.unwrap();

    // Task was registered.
    assert!(agent.state.background.contains_key(&1), "background task #1 should be registered");
    // Give the reader task a moment to capture the echo output.
    for _ in 0..50 {
        if agent.state.background[&1].done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("Background task #1 started"), "{}", results[0].0);
    assert!(
        results[1].0.contains("bg_started") || results[1].0.contains("still running"),
        "bash_output should show output or running state: {}",
        results[1].0
    );
    // Eventually the task finishes and reports an exit code.
    let task = agent.state.background.get(&1).unwrap();
    for _ in 0..100 {
        if task.done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(task.done.load(std::sync::atomic::Ordering::Relaxed), "background task should finish");
    let output = task.output.lock().unwrap().clone();
    assert!(output.contains("bg_started"), "captured output: {output}");
}

#[tokio::test]
async fn image_read_returns_image_block() {
    let dir = tempfile::tempdir().unwrap();
    // Smallest valid 1x1 transparent PNG.
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49,
        0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06,
        0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x44,
        0x41, 0x54, 0x78, 0x9C, 0x62, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D,
        0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42,
        0x60, 0x82,
    ];
    std::fs::write(dir.path().join("pixel.png"), png).unwrap();

    let script = vec![
        json!({"text": "looking at the image", "tool_calls": [
            {"name": "read_file", "input": {"path": "pixel.png"}}
        ]}),
        json!({"text": "I saw the image"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("look at pixel.png").await.unwrap();

    // The tool result carries the image block through the IR.
    let mut image_count = 0;
    for m in &agent.state.messages {
        for b in &m.content {
            if let myharness::llm::ContentBlock::ToolResult { images, .. } = b {
                for img in images {
                    assert_eq!(img.media_type, "image/png");
                    assert!(!img.data.is_empty(), "image data must be base64 text");
                    image_count += 1;
                }
            }
        }
    }
    assert_eq!(image_count, 1, "exactly one image should be attached");
}

#[tokio::test]
async fn files_read_survives_session_replay() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("keep.txt"), "data\n").unwrap();
    let session = Session::create(&dir.path().join("s"), "mock-model", dir.path()).unwrap();
    let session_path = session.path.clone();
    let script = vec![
        json!({"text": "reading", "tool_calls": [
            {"name": "read_file", "input": {"path": "keep.txt"}}
        ]}),
        json!({"text": "read it"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, Some(session));
    agent.run_turn("read keep.txt").await.unwrap();
    assert_eq!(agent.state.files_read.len(), 1);

    // Replay must restore the files_read guard set.
    let events = Session::read_events(&session_path).unwrap();
    let raw = std::fs::read_to_string(&session_path).unwrap();
    assert!(raw.contains("\"type\":\"files_read\""), "files_read event should be persisted");
    let replayed = Session::replay(events, dir.path().to_path_buf());
    assert_eq!(replayed.files_read.len(), 1, "files_read must survive replay");
}

#[tokio::test]
async fn writes_outside_workspace_denied_in_yolo() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap(); // different tree than workspace
    let script = vec![
        json!({"text": "writing outside", "tool_calls": [
            {"name": "write_file", "input": {"path": outside.path().join("escape.txt").display().to_string(), "content": "nope\n"}}
        ]}),
        json!({"text": "denied, finishing"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("write outside the workspace").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].1, "outside write must be denied even in yolo");
    assert!(results[0].0.contains("outside the workspace root"), "{}", results[0].0);
    assert!(!outside.path().join("escape.txt").exists());
}

#[tokio::test]
async fn verify_cmd_runs_after_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = (*test_config(dir.path())).clone();
    cfg.verify_cmd = Some("echo verify_marker_ok".to_string());
    let cfg = Arc::new(cfg);

    let script = vec![
        json!({"text": "editing", "tool_calls": [
            {"name": "write_file", "input": {"path": "src.txt", "content": "v1\n"}}
        ]}),
        json!({"text": "edited and verified"}),
    ];
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.path().to_path_buf());
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg,
        state,
        Ui::quiet(),
        None,
        Registry::full(),
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );
    agent.run_turn("edit src.txt").await.unwrap();

    // An auto-verification user message follows the tool results.
    let verify_msgs: Vec<String> = agent
        .state
        .messages
        .iter()
        .filter(|m| m.role == Role::User && m.text().starts_with("(auto-verification"))
        .map(|m| m.text())
        .collect();
    assert_eq!(verify_msgs.len(), 1, "exactly one verification message expected");
    assert!(verify_msgs[0].contains("verify_marker_ok"), "verify output: {}", verify_msgs[0]);
}

#[test]
fn project_context_loaded_into_system_prompt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "# Rules\nAlways use tabs here.\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}\n").unwrap();
    let state = AgentState::new(dir.path().to_path_buf());
    assert!(state.project_context.as_deref().unwrap().contains("Always use tabs here."));
    let system = myharness::agent::system_prompt::build_system(&state, false);
    assert!(system.contains("# Project instructions (AGENTS.md)"));
    assert!(system.contains("Always use tabs here."));
    // Repository layout digest is injected too.
    assert!(system.contains("# Repository layout"), "layout missing:\n{system}");
    assert!(system.contains("src/ — 1 files"), "layout content:\n{}", system);
}

#[tokio::test]
async fn pre_tool_use_hook_blocks() {
    let dir = tempfile::tempdir().unwrap();
    // A hook that denies every bash call with exit 2.
    let hook = if cfg!(windows) {
        let script = dir.path().join("deny.cmd");
        std::fs::write(&script, "@echo off\r\nexit /b 2\r\n").unwrap();
        format!("{}", script.display())
    } else {
        let script = dir.path().join("deny.sh");
        std::fs::write(&script, "#!/bin/sh\nexit 2\n").unwrap();
        format!("sh {}", script.display())
    };
    let mut cfg = (*test_config(dir.path())).clone();
    cfg.hooks = vec![myharness::config::HookDef {
        event: myharness::config::HookEvent::PreToolUse,
        tool: Some("bash".to_string()),
        command: hook,
        timeout_ms: Some(10_000),
    }];
    let cfg = Arc::new(cfg);

    let script = vec![
        json!({"text": "trying bash", "tool_calls": [
            {"name": "bash", "input": {"command": "echo hi"}}
        ]}),
        json!({"text": "bash was blocked by the hook, finishing"}),
    ];
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.path().to_path_buf());
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg,
        state,
        Ui::quiet(),
        None,
        Registry::full(),
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );
    agent.run_turn("run echo").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].1, "hook should have denied bash");
    assert!(results[0].0.contains("PreToolUse hook"), "{}", results[0].0);
}

/// Locate the built mock MCP server example relative to the test binary.
fn mock_mcp_path() -> std::path::PathBuf {
    let mut dir = std::env::current_exe().unwrap();
    // .../target/debug/deps/integration-xxx.exe → .../target
    for _ in 0..5 {
        dir.pop();
        if dir.file_name().and_then(|n| n.to_str()) == Some("target") {
            break;
        }
    }
    let candidate = dir
        .join("debug")
        .join("examples")
        .join(if cfg!(windows) { "mock-mcp-server.exe" } else { "mock-mcp-server" });
    assert!(candidate.exists(), "mock MCP server not built: {}", candidate.display());
    candidate
}

#[tokio::test]
async fn mcp_tool_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = (*test_config(dir.path())).clone();
    cfg.mcp_servers = vec![myharness::config::McpServerConfig {
        name: "mock".to_string(),
        command: mock_mcp_path().display().to_string(),
        args: vec![],
    }];
    let cfg = Arc::new(cfg);

    let (tools, warnings) = myharness::mcp::init_mcp_tools(&cfg).await;
    assert!(warnings.is_empty(), "warnings: {warnings:?}");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "mcp__mock__echo");

    // Run one agent turn that calls the bridged tool.
    let script = vec![
        json!({"text": "calling the MCP echo tool", "tool_calls": [
            {"name": "mcp__mock__echo", "input": {"text": "hello mcp"}}
        ]}),
        json!({"text": "mcp round trip done"}),
    ];
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.path().to_path_buf());
    let mut registry = Registry::full();
    for t in tools {
        registry.add_tool(t);
    }
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg,
        state,
        Ui::quiet(),
        None,
        registry,
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );
    agent.run_turn("echo via mcp").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(!results[0].1, "MCP call should succeed: {}", results[0].0);
    assert!(results[0].0.contains("ECHO: HELLO MCP"), "result: {}", results[0].0);
}

#[tokio::test]
async fn web_fetch_gated_but_plan_allowed() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "fetching docs", "tool_calls": [
            {"name": "web_fetch", "input": {"url": "http://127.0.0.1:9/nope"}}
        ]}),
        json!({"text": "done"}),
    ];
    // Plan mode: fetch runs (fails on network, NOT on permission).
    let (mut agent, _p) = agent_with(script.clone(), dir.path(), PermissionMode::Plan, None);
    agent.run_turn("fetch docs").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(!results[0].0.contains("permission denied"), "plan mode should allow the fetch itself: {}", results[0].0);
    assert!(results[0].0.contains("request failed") || results[0].0.contains("HTTP"), "expected a network error: {}", results[0].0);

    // Non-interactive auto-edit: gated, no prompt possible → denied.
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::AutoEdit, None);
    agent.run_turn("fetch docs").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].1, "web_fetch should be gated in -p mode");
    assert!(results[0].0.contains("permission denied"), "{}", results[0].0);
}

#[tokio::test]
async fn thinking_streamed_but_never_stored() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![json!({
        "thinking": "hmm, let me consider the approach carefully",
        "text": "the visible answer"
    })];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    let outcome = agent.run_turn("think then answer").await.unwrap();
    assert_eq!(outcome.final_text.trim(), "the visible answer");
    // Reasoning must NOT leak into any persisted block.
    for m in &agent.state.messages {
        let all = m.text();
        assert!(!all.contains("consider the approach"), "thinking leaked: {all}");
    }
    assert!(agent.state.messages.iter().any(|m| m.text().contains("the visible answer")));
}

#[tokio::test]
async fn stop_hook_forces_one_continuation() {
    let dir = tempfile::tempdir().unwrap();
    // One-shot Stop hook: the first attempt to end the turn is blocked
    // (marker created, exit 2); later attempts pass.
    let hook = if cfg!(windows) {
        let script = dir.path().join("deny_once.cmd");
        std::fs::write(
            &script,
            "@echo off\r\nif exist \"%~dp0once.marker\" exit /b 0\r\ntype nul > \"%~dp0once.marker\"\r\nexit /b 2\r\n",
        )
        .unwrap();
        format!("{}", script.display())
    } else {
        let script = dir.path().join("deny_once.sh");
        std::fs::write(&script, "#!/bin/sh\n[ -f \"$(dirname \"$0\")/once.marker\" ] && exit 0\ntouch \"$(dirname \"$0\")/once.marker\"\nexit 2\n").unwrap();
        format!("sh {}", script.display())
    };
    let mut cfg = (*test_config(dir.path())).clone();
    cfg.hooks = vec![myharness::config::HookDef {
        event: myharness::config::HookEvent::Stop,
        tool: None,
        command: hook,
        timeout_ms: Some(10_000),
    }];
    let cfg = Arc::new(cfg);

    let script = vec![
        json!({"text": "first attempt done"}),
        json!({"text": "second attempt done"}),
    ];
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.path().to_path_buf());
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg,
        state,
        Ui::quiet(),
        None,
        Registry::full(),
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
        false,
    );
    let outcome = agent.run_turn("do the thing").await.unwrap();
    assert_eq!(outcome.final_text.trim(), "second attempt done");
    assert!(agent
        .state
        .messages
        .iter()
        .any(|m| m.text().contains("Stop hook requires you to continue")));
}

#[tokio::test]
async fn edit_journal_and_undo_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let session = Session::create(&dir.path().join("s"), "mock-model", dir.path()).unwrap();
    let session_path = session.path.clone();
    let script = vec![
        // 1: create new.txt (journal: did not exist).
        json!({"text": "creating", "tool_calls": [
            {"name": "write_file", "input": {"path": "new.txt", "content": "v1\n"}}
        ]}),
        // 2: edit it (journal: backup of v1).
        json!({"text": "editing", "tool_calls": [
            {"name": "edit_file", "input": {"path": "new.txt", "old_string": "v1", "new_string": "v2"}}
        ]}),
        json!({"text": "done"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, Some(session));
    agent.run_turn("make and edit new.txt").await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "v2\n");
    assert_eq!(agent.state.edit_journal.len(), 2);

    // Undo the edit: back to v1.
    assert_eq!(agent.undo(1), 1);
    assert_eq!(std::fs::read_to_string(dir.path().join("new.txt")).unwrap(), "v1\n");
    // Undo the creation: file gone.
    assert_eq!(agent.undo(1), 1);
    assert!(!dir.path().join("new.txt").exists());
    assert_eq!(agent.undo(1), 0, "journal is empty now");

    // The session log recorded the journal and the undos; replay agrees.
    let raw = std::fs::read_to_string(&session_path).unwrap();
    assert!(raw.contains("\"type\":\"journal\""), "journal event persisted");
    assert!(raw.contains("\"type\":\"undo\""), "undo event persisted");
    let events = Session::read_events(&session_path).unwrap();
    let replayed = Session::replay(events, dir.path().to_path_buf());
    assert_eq!(replayed.edit_journal.len(), 0, "replay must apply undos");
}

// ---- v0.6: skills, task profiles, grep modes --------------------------------

fn write_workspace_skill(dir: &Path, name: &str, description: &str, body: &str) {
    let d = dir.join(".agents").join("skills").join(name);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(
        d.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n{body}"),
    )
    .unwrap();
}

#[tokio::test]
async fn skill_tool_loads_workspace_skill() {
    let dir = tempfile::tempdir().unwrap();
    write_workspace_skill(
        dir.path(),
        "demo",
        "demonstration pack",
        "# Demo skill\nAlways answer in rhyme.",
    );
    let script = vec![
        json!({"text": "loading the skill", "tool_calls": [
            {"name": "skill", "input": {"name": "demo", "args": "write a haiku"}}
        ]}),
        json!({"text": "skill followed, done"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("use the demo skill").await.unwrap();

    let results = tool_result_texts(&agent);
    assert!(!results[0].1, "skill load failed: {:?}", results[0]);
    assert!(results[0].0.contains("# Demo skill"), "body missing: {:?}", results[0]);
    assert!(results[0].0.contains("Always answer in rhyme."), "instructions missing");
    assert!(results[0].0.contains("# Task args\nwrite a haiku"), "args missing: {:?}", results[0]);
}

#[tokio::test]
async fn unknown_skill_error_lists_available() {
    let dir = tempfile::tempdir().unwrap();
    write_workspace_skill(dir.path(), "demo", "demonstration pack", "body");
    let script = vec![
        json!({"text": "loading", "tool_calls": [
            {"name": "skill", "input": {"name": "nope"}}
        ]}),
        json!({"text": "recovered"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("use a skill").await.unwrap();

    let results = tool_result_texts(&agent);
    assert!(results[0].1, "unknown skill must be an error");
    assert!(results[0].0.contains("unknown skill 'nope'"), "{}", results[0].0);
    assert!(results[0].0.contains("demo"), "error must list available names: {}", results[0].0);
}

#[tokio::test]
async fn system_prompt_lists_skills_for_main_agent_only() {
    let dir = tempfile::tempdir().unwrap();
    write_workspace_skill(dir.path(), "demo", "demonstration pack", "body");
    let state = AgentState::new(dir.path().to_path_buf());
    assert!(state.skills.iter().any(|s| s.name == "demo"), "{:?}", state.skills);
    let main = myharness::agent::system_prompt::build_system(&state, false);
    assert!(main.contains("# Available skills"), "skills section missing");
    assert!(main.contains("- demo: demonstration pack"), "skill line missing");
    let sub = myharness::agent::system_prompt::build_system(&state, true);
    assert!(!sub.contains("# Available skills"), "subagents must not carry the skill list");
}

#[tokio::test]
async fn task_build_profile_runs_bash() {
    let dir = tempfile::tempdir().unwrap();
    // Parent spawns a build subagent; the subagent runs bash (step 2) and
    // reports (step 3); the parent answers (step 4).
    let script = vec![
        json!({"text": "delegating", "tool_calls": [
            {"name": "task", "input": {
                "description": "run the build",
                "prompt": "run the command and report",
                "agent_type": "build"
            }}
        ]}),
        json!({"text": "running", "tool_calls": [
            {"name": "bash", "input": {"command": "echo built_ok > built.txt"}}
        ]}),
        json!({"text": "SUB REPORT: command executed"}),
        json!({"text": "delegated fine"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    let outcome = agent.run_turn("run the build in a subagent").await.unwrap();
    assert!(dir.path().join("built.txt").exists(), "build profile must allow bash");
    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("SUB REPORT"), "report missing: {:?}", results[0]);
    assert_eq!(outcome.final_text.trim(), "delegated fine");
}

#[tokio::test]
async fn task_explore_profile_rejects_bash() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "delegating", "tool_calls": [
            {"name": "task", "input": {
                "description": "explore only",
                "prompt": "try to run a command"
            }}
        ]}),
        json!({"text": "trying", "tool_calls": [
            {"name": "bash", "input": {"command": "echo nope > explored.txt"}}
        ]}),
        json!({"text": "SUB REPORT: bash unavailable"}),
        json!({"text": "ok"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("explore").await.unwrap();
    assert!(!dir.path().join("explored.txt").exists(), "explore profile must not run bash");
}

#[tokio::test]
async fn task_rejects_unknown_agent_type() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "delegating", "tool_calls": [
            {"name": "task", "input": {
                "description": "bad type",
                "prompt": "anything",
                "agent_type": "wizard"
            }}
        ]}),
        json!({"text": "recovered"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("spawn it").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(results[0].1, "invalid agent_type must be an error");
    assert!(results[0].0.contains("invalid agent_type 'wizard'"), "{}", results[0].0);
}

#[tokio::test]
async fn grep_output_modes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hit one\nmiss\nhit two\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "hit three\nmiss\n").unwrap();
    let script = vec![
        json!({"text": "files mode", "tool_calls": [
            {"name": "grep", "input": {"pattern": "hit", "output_mode": "files"}}
        ]}),
        json!({"text": "count mode", "tool_calls": [
            {"name": "grep", "input": {"pattern": "hit", "output_mode": "count"}}
        ]}),
        json!({"text": "content mode", "tool_calls": [
            {"name": "grep", "input": {"pattern": "hit", "output_mode": "content"}}
        ]}),
        json!({"text": "bad mode", "tool_calls": [
            {"name": "grep", "input": {"pattern": "hit", "output_mode": "bogus"}}
        ]}),
        json!({"text": "done"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("search").await.unwrap();

    let results = tool_result_texts(&agent);
    assert!(!results[0].1 && results[0].0.contains("a.txt (2 matches)"), "{}", results[0].0);
    assert!(!results[0].0.contains("a.txt:1:"), "files mode must not carry line text");
    assert!(results[1].0.contains("a.txt:2") && results[1].0.contains("b.txt:1"), "{}", results[1].0);
    assert!(results[2].0.contains("a.txt:3:hit two"), "{}", results[2].0);
    assert!(results[3].1 && results[3].0.contains("invalid output_mode 'bogus'"), "{}", results[3].0);
}

#[tokio::test]
async fn background_subagent_reports_via_task_entry() {
    let dir = tempfile::tempdir().unwrap();
    // The parent's final message and the subagent's report race for the
    // shared mock script queue, so both contending steps are identical.
    let script = vec![
        json!({"text": "spawning detached research", "tool_calls": [
            {"name": "task", "input": {
                "description": "explore in background",
                "prompt": "look around and report",
                "run_in_background": true
            }}
        ]}),
        json!({"text": "BG_DONE"}),
        json!({"text": "BG_DONE"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    let outcome = agent.run_turn("research in the background").await.unwrap();
    assert_eq!(outcome.final_text.trim(), "BG_DONE");

    let results = tool_result_texts(&agent);
    assert!(results[0].0.contains("Background task #1 started"), "{}", results[0].0);

    // The subagent finishes detached from the parent turn.
    let task = agent.state.background.get(&1).expect("bg task registered").clone();
    for _ in 0..250 {
        if task.done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(task.done.load(std::sync::atomic::Ordering::Relaxed), "subagent should finish");
    assert_eq!(*task.exit.lock().unwrap(), Some(0));
    assert!(task.output.lock().unwrap().contains("BG_DONE"), "report: {}", task.output.lock().unwrap());
    assert!(task.command.starts_with("subagent:"), "{}", task.command);
    assert!(task.pid.is_none(), "subagents have no pid to kill");

    // bash_output delivers the report exactly like it does for bash tasks.
    let mut effects = myharness::tools::ToolEffects::default();
    let files = std::collections::HashSet::new();
    let mut bg_map = std::collections::HashMap::new();
    bg_map.insert(1u32, task);
    let mut ctx = myharness::tools::ToolCtx {
        cwd: dir.path().to_path_buf(),
        workspace_root: dir.path().to_path_buf(),
        cfg: test_config(dir.path()),
        provider: Arc::new(MockProvider::new(vec![])) as Arc<dyn Provider>,
        files_read: &files,
        background: &bg_map,
        next_bg_id: 2,
        cancel: Arc::new(AtomicBool::new(false)),
        checkpoint_dir: dir.path().join("cp"),
        journal_next: 0,
        turns: 1,
        effects: &mut effects,
    };
    let out = myharness::tools::Registry::full()
        .get("bash_output")
        .unwrap()
        .execute(json!({"id": 1}), &mut ctx)
        .await;
    assert!(!out.is_error, "{}", out.content);
    assert!(out.content.contains("finished, exit code 0"), "{}", out.content);
    assert!(out.content.contains("BG_DONE"), "{}", out.content);
}

#[tokio::test]
async fn repo_map_lists_symbols() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "pub struct Config {}\npub fn load() {}\n").unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src").join("app.py"), "class App:\n    def run(self):\n        pass\n").unwrap();
    std::fs::write(dir.path().join("data.txt"), "not code\n").unwrap();
    let script = vec![
        json!({"text": "mapping the repo", "tool_calls": [
            {"name": "repo_map", "input": {}}
        ]}),
        json!({"text": "mapped"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("map it").await.unwrap();
    let results = tool_result_texts(&agent);
    assert!(!results[0].1, "{}", results[0].0);
    assert!(results[0].0.contains("struct Config"), "{}", results[0].0);
    assert!(results[0].0.contains("fn load"), "{}", results[0].0);
    assert!(results[0].0.contains("class App"), "{}", results[0].0);
    assert!(results[0].0.contains("def run"), "{}", results[0].0);
    assert!(results[0].0.contains("lib.rs") && results[0].0.contains("app.py"), "{}", results[0].0);
    assert!(!results[0].0.contains("data.txt"), "non-source files must be skipped: {}", results[0].0);
}

#[tokio::test]
async fn background_finish_notice_reaches_model() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "starting bg work", "tool_calls": [
            {"name": "bash", "input": {"command": "echo NOTICE_ME", "run_in_background": true}}
        ]}),
        json!({"text": "started, moving on"}),
        json!({"text": "acknowledged"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.run_turn("start it").await.unwrap();

    // Wait for the echo to finish, then run another turn: the completion
    // notice must be injected exactly once, whether it landed during turn 1
    // or ahead of turn 2's first request.
    let task = agent.state.background.get(&1).unwrap().clone();
    for _ in 0..250 {
        if task.done.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    assert!(task.done.load(std::sync::atomic::Ordering::Relaxed));
    agent.run_turn("next").await.unwrap();

    let everything: String = agent.state.messages.iter().map(|m| m.text()).collect();
    let count = everything.matches("Background task(s) finished").count();
    assert_eq!(count, 1, "notice delivered exactly once:\n{everything}");
    assert!(everything.contains("exit code 0"), "{everything}");
    assert!(everything.contains("NOTICE_ME"), "{everything}");
}

#[tokio::test]
async fn output_schema_gets_corrective_rounds() {
    let dir = tempfile::tempdir().unwrap();
    let schema = json!({
        "type": "object",
        "properties": {"answer": {"type": "string"}},
        "required": ["answer"]
    });
    let script = vec![
        json!({"text": "The answer is probably yes, I think."}),
        json!({"text": "{\"answer\": \"yes\"}"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.set_output_schema(schema);
    let outcome = agent.run_turn("answer in json").await.unwrap();

    // The corrective round produced schema-valid output.
    let parsed = myharness::schema_validate::parse_output(&outcome.final_text).unwrap();
    assert_eq!(parsed["answer"], "yes");
    // The model was told exactly why the first attempt failed.
    let everything: String = agent.state.messages.iter().map(|m| m.text()).collect();
    assert!(everything.contains("failed schema validation"), "{everything}");
    assert!(everything.contains("Output ONLY the corrected JSON"), "{everything}");
}

#[tokio::test]
async fn output_schema_exhausts_retries_and_returns_text() {
    let dir = tempfile::tempdir().unwrap();
    let script = vec![
        json!({"text": "no json here 1"}),
        json!({"text": "no json here 2"}),
        json!({"text": "no json here 3"}),
    ];
    let (mut agent, _p) = agent_with(script, dir.path(), PermissionMode::Yolo, None);
    agent.set_output_schema(json!({"type": "object"}));
    let outcome = agent.run_turn("answer in json").await.unwrap();
    // Two corrective rounds used, then the text is returned as-is (the
    // caller decides the failure exit code).
    assert_eq!(agent.schema_retries, 2);
    assert_eq!(outcome.final_text.trim(), "no json here 3");
}

fn mock_lsp_path() -> std::path::PathBuf {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target");
    let name = if cfg!(windows) { "mock-lsp-server.exe" } else { "mock-lsp-server" };
    // Smart App Control (this repo's development machine) sometimes flags a
    // freshly built debug binary by hash while older/release ones stay
    // runnable — fall back to the release build when the debug one is
    // permission-denied.
    let debug = base.join("debug").join("examples").join(name);
    let spawnable = std::process::Command::new(&debug)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|mut c| {
            let _ = c.kill();
        })
        .is_ok();
    if spawnable {
        return debug;
    }
    let release = base.join("release").join("examples").join(name);
    if release.exists() {
        return release;
    }
    debug
}

#[tokio::test]
async fn lsp_diagnostics_injected_after_edits() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = (*test_config(dir.path())).clone();
    cfg.lsp_servers = vec![myharness::config::LspServerConfig {
        name: "mock".to_string(),
        languages: vec!["rs".to_string()],
        command: mock_lsp_path().display().to_string(),
        args: vec![],
        timeout_ms: Some(4_000),
    }];
    let cfg = Arc::new(cfg);

    let script = vec![
        json!({"text": "writing a file", "tool_calls": [
            {"name": "write_file", "input": {"path": "src/thing.rs", "content": "fn broken(\n"}}
        ]}),
        json!({"text": "edited with diagnostics in view"}),
    ];
    let provider = Arc::new(MockProvider::new(script));
    let state = AgentState::new(dir.path().to_path_buf());
    let mut agent = Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg,
        state,
        Ui::quiet(),
        None,
        Registry::full(),
        PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
        "mock-model".to_string(),
        Arc::new(AtomicBool::new(false)),
        false,
    );
    agent.run_turn("write src/thing.rs").await.unwrap();

    // The mock server's deterministic diagnostic reached the model as a
    // message after the edit round.
    let everything: String = agent.state.messages.iter().map(|m| m.text()).collect();
    assert!(everything.contains("(lsp diagnostics after edits)"), "{everything}");
    assert!(everything.contains("MOCK_LSP_ERROR"), "{everything}");
    assert!(everything.contains("thing.rs"), "{everything}");
    // The client stays alive across turns for reuse.
    assert!(agent.lsp.contains_key("mock"));
}

#[test]
fn serve_mode_speaks_json_rpc() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("script.json"),
        r#"[{"text": "server says hello"}]"#,
    )
    .unwrap();
    let bin = env!("CARGO_BIN_EXE_myharness");
    let mut child = std::process::Command::new(bin)
        .arg("serve")
        .arg("--provider")
        .arg("mock")
        .env("MYHARNESS_MOCK_FILE", dir.path().join("script.json"))
        .env("MYHARNESS_DATA_DIR", dir.path().join("data"))
        .current_dir(dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn myharness serve");

    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().unwrap();
        // Build frames with serde_json, not hand-escaped format strings.
        let frames = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string(),
            json!({"jsonrpc": "2.0", "id": 2, "method": "turn.run", "params": {"input": "say hello"}}).to_string(),
            json!({"jsonrpc": "2.0", "id": 3, "method": "bogus.method", "params": {}}).to_string(),
        ];
        for f in frames {
            let _ = writeln!(stdin, "{f}");
        }
    } // dropping stdin ends the session loop

    let out = child.wait_with_output().unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();

    let init = lines.iter().find(|l| l.contains(r#""id":1"#)).expect("initialize response");
    assert!(init.contains("myharness"), "{init}");
    // Deltas streamed as notifications before the response.
    assert!(lines.iter().any(|l| l.contains("mh/turn.delta")), "{stdout}");
    let turn = lines.iter().find(|l| l.contains(r#""id":2"#)).expect("turn.run response");
    assert!(turn.contains("server says hello"), "{turn}");
    assert!(turn.contains(r#""interrupted":false"#), "{turn}");
    let bogus = lines.iter().find(|l| l.contains(r#""id":3"#)).expect("unknown method response");
    assert!(bogus.contains("-32601") && bogus.contains("unknown method"), "{bogus}");
}

/// Live AppContainer verification (Windows, opt-in via MYHARNESS_LIVE_AC=1):
/// a sandboxed process must write inside the workspace and be denied
/// everywhere else. Ignored by default — profile creation needs a real
/// interactive session, not a service/CI account.
#[cfg(windows)]
#[test]
#[ignore = "set MYHARNESS_LIVE_AC=1 on a real Windows session"]
fn appcontainer_blocks_writes_outside_workspace() {
    if std::env::var("MYHARNESS_LIVE_AC").ok().as_deref() != Some("1") {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let ws = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let script = format!(
            "echo inside_ok > \"{}\" 2> \"{}\" && echo tried_outside",
            ws.path().join("ok.txt").display(),
            outside.path().join("pwned.txt").display()
        );
        let ac = myharness::sandbox::appcontainer::launch(
            r"C:\Windows\System32\cmd.exe",
            &["/D".to_string(), "/C".to_string(), script],
            ws.path(),
            &[ws.path().to_path_buf()],
        )
        .expect("launch inside AppContainer");
        let ac = ac;
        let mut exit = None;
        for _ in 0..200 {
            if let Some(c) = ac.try_wait() {
                exit = Some(c);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(exit.is_some(), "sandboxed process should exit");
        assert!(ws.path().join("ok.txt").exists(), "write inside workspace must succeed");
        assert!(
            !outside.path().join("pwned.txt").exists(),
            "write outside the workspace must be DENIED by the container"
        );
    });
}

/// Live: the bash TOOL end-to-end under sandbox=appcontainer (opt-in like
/// the launcher test; exercises shell spawn inside the container, the ACL
/// grant, and output capture through the unified Spawned path).
#[cfg(windows)]
#[test]
#[ignore = "set MYHARNESS_LIVE_AC=1 on a real Windows session"]
fn appcontainer_bash_tool_round_trip() {
    if std::env::var("MYHARNESS_LIVE_AC").ok().as_deref() != Some("1") {
        return;
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = (*test_config(dir.path())).clone();
        cfg.sandbox = myharness::config::SandboxMode::AppContainer;
        cfg.shell = match std::env::var("MH_AC_SHELL").as_deref() {
            Ok("bash") => myharness::config::ShellChoice::Bash,
            _ => myharness::config::ShellChoice::Cmd, // system shell: AAP-readable
        };
        let cfg = Arc::new(cfg);
        let script = vec![
            json!({"text": "writing inside the sandbox", "tool_calls": [
                {"name": "bash", "input": {"command": "echo AC_TOOL_OK > ac_tool.txt"}}
            ]}),
            json!({"text": "bash round trip done"}),
        ];
        let provider = Arc::new(MockProvider::new(script));
        let state = AgentState::new(dir.path().to_path_buf());
        let mut agent = Agent::new(
            Arc::clone(&provider) as Arc<dyn Provider>,
            cfg,
            state,
            Ui::quiet(),
            None,
            Registry::full(),
            PermissionEngine::new(PermissionMode::Yolo, vec![], vec![], true),
            "mock-model".to_string(),
            Arc::new(AtomicBool::new(false)),
            false,
        );
        agent.run_turn("write the file").await.unwrap();
        let results = tool_result_texts(&agent);
        assert!(!results[0].1, "bash inside AppContainer failed: {}", results[0].0);
        assert!(results[0].0.contains("Exit code: 0"), "{}", results[0].0);
        assert!(dir.path().join("ac_tool.txt").exists(), "file must exist inside the workspace");
    });
}

/// Drive `myharness serve` with framed requests; returns stdout lines.
fn run_serve_frames(dir: &Path, script: &str, frames: &[serde_json::Value]) -> String {
    let script_path = dir.join(format!("script-{}.json", std::process::id() as u64 + frames.len() as u64));
    std::fs::write(&script_path, script).unwrap();
    // Unique-ish name per invocation to avoid overwriting between calls.
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_myharness"))
        .arg("serve")
        .arg("--provider")
        .arg("mock")
        .env("MYHARNESS_MOCK_FILE", &script_path)
        .env("MYHARNESS_DATA_DIR", dir.join("data"))
        .current_dir(dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn serve");
    {
        use std::io::Write as _;
        let mut stdin = child.stdin.take().unwrap();
        for f in frames {
            let _ = writeln!(stdin, "{f}");
        }
    }
    String::from_utf8_lossy(&child.wait_with_output().unwrap().stdout).to_string()
}

#[test]
fn serve_mode_attaches_and_resumes_sessions() {
    let dir = tempfile::tempdir().unwrap();

    // 1) Fresh serve: a turn creates a session; the response names it.
    let out = run_serve_frames(
        dir.path(),
        r#"[{"text": "first turn recorded"}]"#,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "turn.run", "params": {"input": "remember banana"}}),
        ],
    );
    let turn: serde_json::Value = out
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap_or(serde_json::Value::Null))
        .find(|v| v.get("id") == Some(&json!(1)))
        .expect("turn response");
    let session_id = turn["result"]["session_id"].as_str().expect("session_id present").to_string();
    assert!(!session_id.is_empty());

    // 2) New serve process: list finds it, attach resumes its messages,
    //    and the next turn continues under the SAME session id.
    let out = run_serve_frames(
        dir.path(),
        r#"[{"text": "second turn done"}]"#,
        &[
            json!({"jsonrpc": "2.0", "id": 1, "method": "session.list", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "session.attach", "params": {"id": session_id}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "turn.run", "params": {"input": "continue"}}),
        ],
    );
    let lines: Vec<serde_json::Value> = out
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap_or(serde_json::Value::Null))
        .collect();
    let list = lines.iter().find(|v| v.get("id") == Some(&json!(1))).unwrap();
    assert!(list["result"].to_string().contains(&session_id), "{list}");
    let attach = lines.iter().find(|v| v.get("id") == Some(&json!(2))).unwrap();
    assert_eq!(attach["result"]["resumed_messages"], 2, "{attach}"); // user prompt + assistant reply
    assert_eq!(attach["result"]["session_id"].as_str().unwrap(), session_id);
    let turn2 = lines.iter().find(|v| v.get("id") == Some(&json!(3))).unwrap();
    assert_eq!(turn2["result"]["session_id"].as_str().unwrap(), session_id, "{turn2}");
    assert!(turn2["result"]["final_text"].as_str().unwrap().contains("second turn done"));

    // 3) Unknown id fails loudly.
    let out = run_serve_frames(
        dir.path(),
        r#"[]"#,
        &[json!({"jsonrpc": "2.0", "id": 1, "method": "session.attach", "params": {"id": "zzznope"}})],
    );
    assert!(out.contains("-32002") && out.contains("no session id matching"), "{out}");
}
