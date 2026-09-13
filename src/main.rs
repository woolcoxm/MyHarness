//! myharness — a terminal coding-agent harness for GLM (binary entry point).

use myharness::{agent, cli, config, llm, mcp, perms, server, session, tools, tui, ui};

use agent::state::AgentState;
use agent::Agent;
use anyhow::{bail, Context, Result};
use clap::Parser;
use config::{CliOverrides, Config, ProviderKind};
use llm::Provider;
use perms::{PermissionEngine, PermissionMode};
use session::Session;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use ui::Ui;

fn main() -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build async runtime")?;
    runtime.block_on(async_main())
}

async fn async_main() -> Result<()> {
    let cli = cli::Cli::parse();

    let mut cfg = Config::load(CliOverrides {
        provider: cli.provider,
        model: cli.model.clone(),
        base_url: cli.base_url.clone(),
        verbose: cli.verbose,
    })?;

    // Subcommands that don't need a provider.
    match &cli.cmd {
        Some(cli::Command::Sessions) => {
            list_sessions(&cfg);
            return Ok(());
        }
        Some(cli::Command::Config) => {
            println!("{}", config::summarize(&cfg)?);
            println!("\n--- default myharness.toml ---\n{}", config::default_config_toml());
            return Ok(());
        }
        Some(cli::Command::Serve) => {
            let provider = build_provider(&cfg)?;
            return server::serve(cfg, provider).await;
        }
        _ => {}
    }

    // Resume selection: explicit --session > --continue > subcommand Resume > new.
    let resume_id = cli
        .session
        .clone()
        .or_else(|| match &cli.cmd {
            Some(cli::Command::Resume { id }) => id.clone(),
            None if cli.continue_last => Some(String::new()),
            _ => None,
        });

    let non_interactive = cli.print;
    if non_interactive && cli.task.is_none() {
        bail!("-p/--print requires a task argument (nothing to run non-interactively)");
    }
    let non_interactive = non_interactive && cli.task.is_some();
    cfg.non_interactive = non_interactive;

    let provider: Arc<dyn Provider> = build_provider(&cfg)?;
    let (state, session) = match resume_id {
        Some(id) => {
            let path = find_session(&cfg, if id.is_empty() { None } else { Some(&id) })?;
            let events = Session::read_events(&path)?;
            let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let mut state = Session::replay(events, root.clone());
            let mut session = Session::open(&path)?;
            crate::session::cwd_note_if_diverged(&mut state, &mut session, &root);
            (state, Some(session))
        }
        None => {
            let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let state = AgentState::new(root);
            // -p keeps a session too: the transcript is the debug record and
            // usage history for pipeline runs, not just interactive ones.
            let session = Session::create(&cfg.sessions_dir(), &cfg.model, &state.cwd)?;
            (state, Some(session))
        }
    };

    // Default modes: -p runs auto-edit (bash follows allow rules); the REPL
    // asks. --yolo/--mode always win.
    let mode = if cli.yolo {
        PermissionMode::Yolo
    } else if let Some(m) = &cli.mode {
        PermissionMode::parse(m).with_context(|| format!("invalid --mode '{m}'"))?
    } else if non_interactive {
        PermissionMode::AutoEdit
    } else {
        PermissionMode::Ask
    };

    let perms = PermissionEngine::new(
        mode,
        cfg.allow_rules.clone(),
        cfg.deny_rules.clone(),
        non_interactive,
    );

    let mut ui = if non_interactive { Ui::quiet() } else { Ui::new() };
    let model = cfg.model.clone();
    let mut registry = tools::Registry::full();
    // MCP servers: bridge their tools; failures degrade to warnings.
    if !cfg.mcp_servers.is_empty() {
        let (mcp_tools, warnings) = mcp::init_mcp_tools(&cfg).await;
        for w in warnings {
            if non_interactive {
                eprintln!("!! {w}");
            } else {
                ui.warn(&w);
            }
        }
        let count = mcp_tools.len();
        for t in mcp_tools {
            registry.add_tool(t);
        }
        if !non_interactive && count > 0 {
            ui.info(&format!("{count} MCP tool(s) available"));
        }
    }
    let mut agent = Agent::new(
        provider,
        Arc::new(cfg),
        state,
        ui,
        session,
        registry,
        perms,
        model,
        Arc::new(AtomicBool::new(false)),
        false,
    );

    if let Some(spec) = &cli.output_schema {
        let schema = load_output_schema(spec)?;
        agent.set_output_schema(schema);
    }

    if non_interactive {
        let task = cli.task.unwrap();
        let outcome = agent.run_turn(&task).await?;
        println!("{}", outcome.final_text.trim());
        // Usage metrics go to stderr so stdout stays clean for pipes.
        let u = &agent.state.usage;
        eprintln!(
            "-- usage: {} request(s) | in {} (cache: {} read, {} write) | out {} | turns {}",
            agent.state.requests,
            u.input_tokens,
            u.cache_read_tokens,
            u.cache_creation_tokens,
            u.output_tokens,
            agent.state.turns,
        );
        if outcome.interrupted {
            std::process::exit(130);
        }
        // Pipeline contract: with --output-schema the printed output must
        // validate, or the run fails loudly even though the text was printed.
        if let Some(schema) = &agent.output_schema {
            let verdict = myharness::schema_validate::parse_output(&outcome.final_text)
                .and_then(|v| myharness::schema_validate::validate(&v, schema));
            if let Err(e) = verdict {
                eprintln!("!! output failed schema validation: {e}");
                std::process::exit(2);
            }
        }
        return Ok(());
    }

    if let Some(task) = cli.task.filter(|t| !t.trim().is_empty()) {
        if let Err(e) = agent.run_turn(&task).await {
            agent.ui.warn(&format!("turn failed: {e}"));
        }
    }
    if matches!(cli.cmd, Some(cli::Command::Tui)) {
        return tui::run(agent).await;
    }
    ui::repl(agent).await
}

/// --output-schema accepts inline JSON or a path to a .json file.
fn load_output_schema(spec: &str) -> Result<serde_json::Value> {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(spec) {
        if v.is_object() {
            return Ok(v);
        }
    }
    let path = PathBuf::from(spec);
    let raw = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "--output-schema is neither inline JSON nor a readable file (tried '{spec}')"
        )
    })?;
    serde_json::from_str(&raw).with_context(|| format!("invalid JSON in schema file '{spec}'"))
}

fn build_provider(cfg: &Config) -> Result<Arc<dyn Provider>> {    match cfg.provider {
        ProviderKind::Anthropic => {
            let key = cfg.api_key.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "no API key: set ZAI_API_KEY (or GLM_API_KEY / ANTHROPIC_API_KEY), \
                     or use --provider mock with MYHARNESS_MOCK_FILE=<script.json>"
                )
            })?;
            let mut p = llm::anthropic::AnthropicProvider::new(
                cfg.base_url.clone(),
                key,
                cfg.prompt_caching,
            );
            if let Some(b) = cfg.thinking_budget {
                p = p.with_thinking_budget(b);
            }
            Ok(Arc::new(p))
        }
        ProviderKind::Openai => {
            let key = cfg.api_key.clone().ok_or_else(|| {
                anyhow::anyhow!(
                    "no API key: set ZAI_API_KEY (or GLM_API_KEY / OPENAI_API_KEY), \
                     or use --provider mock with MYHARNESS_MOCK_FILE=<script.json>"
                )
            })?;
            let mut p = llm::openai::OpenAiProvider::new(cfg.base_url.clone(), key);
            if let Some(e) = &cfg.reasoning_effort {
                p = p.with_reasoning_effort(e);
            }
            Ok(Arc::new(p))
        }
        ProviderKind::Mock => {
            let path = std::env::var("MYHARNESS_MOCK_FILE")
                .map(PathBuf::from)
                .with_context(|| "mock provider needs MYHARNESS_MOCK_FILE=<script.json>")?;
            Ok(Arc::new(llm::mock::MockProvider::from_file(&path)?))
        }
    }
}

fn find_session(cfg: &Config, id: Option<&str>) -> Result<PathBuf> {
    let dir = cfg.sessions_dir();
    let entries = Session::list(&dir);
    if entries.is_empty() {
        bail!("no sessions found in {}", dir.display());
    }
    match id {
        None => {
            let newest = entries.first().unwrap();
            Ok(dir.join(format!("{}.jsonl", newest.0)))
        }
        Some(prefix) => entries
            .iter()
            .find(|(sid, ..)| sid.starts_with(prefix))
            .map(|(sid, ..)| dir.join(format!("{sid}.jsonl")))
            .ok_or_else(|| anyhow::anyhow!("no session id matching '{prefix}'")),
    }
}

fn list_sessions(cfg: &Config) {
    let entries = Session::list(&cfg.sessions_dir());
    if entries.is_empty() {
        println!("no sessions in {}", cfg.sessions_dir().display());
        return;
    }
    println!("{:<22} {:<16} {:<10} {:<6} FIRST MESSAGE", "ID", "WHEN", "MODEL", "MSGS");
    for (id, mtime, model, first, count) in entries.iter().take(30) {
        let id: String = id.chars().take(22).collect();
        let when = chrono::DateTime::<chrono::Local>::from(*mtime)
            .format("%Y-%m-%d %H:%M")
            .to_string();
        println!("{id:<22} {when:<16} {model:<10.10} {count:<6} {first}");
    }
}
