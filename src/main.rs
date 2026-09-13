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
        let task = cli.task.clone().unwrap_or_default();
        if cli.autonomous {
            return autonomous_run(&mut agent, &task, &cli).await;
        }
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

/// Autonomous mode: work until verified done, bounded by time and token
/// budgets. The key insight (from ZCode's goal loop): the model ending its
/// turn without tool calls doesn't mean done — it often means it forgot,
/// assumed, or gave up. This loop injects a self-check prompt each time,
/// requiring two consecutive confirmed "done" turns to complete, and a
/// single "I found more work" to keep going.
async fn autonomous_run(agent: &mut Agent, task: &str, cli: &cli::Cli) -> Result<()> {
    let deadline = std::time::Instant::now()
        + std::time::Duration::from_secs_f64(cli.budget_hours.unwrap_or(8.0) * 3600.0);
    let token_budget = cli.budget_tokens.unwrap_or(2_000_000);

    eprintln!(
        "-- autonomous: budget {}h / {}M tokens | task: {}",
        cli.budget_hours.unwrap_or(8.0),
        token_budget / 1_000_000,
        task.chars().take(80).collect::<String>()
    );

    let mut done_confirmed = 0u32;
    let start = std::time::Instant::now();

    // The first turn runs the task.
    let outcome = agent.run_turn(task).await?;
    let mut last_final = outcome.final_text;

    loop {
        // Budget checks.
        let elapsed = start.elapsed();
        let total_tokens = agent.state.usage.input_tokens
            + agent.state.usage.output_tokens
            + agent.state.usage.cache_read_tokens;
        if std::time::Instant::now() >= deadline {
            eprintln!(
                "-- autonomous: time budget exhausted ({:.1}h, {} requests, {} turns)",
                elapsed.as_secs_f32() / 3600.0,
                agent.state.requests,
                agent.state.turns
            );
            break;
        }
        if total_tokens >= token_budget {
            eprintln!(
                "-- autonomous: token budget exhausted ({}/{}, {} requests, {} turns)",
                total_tokens,
                token_budget,
                agent.state.requests,
                agent.state.turns
            );
            break;
        }

        // Self-check: the model stopped — is it really done?
        // Two consecutive confirmations = goal complete. Any tool call in
        // the self-check response = found more work, keep going.
        eprintln!(
            "-- autonomous: self-check round {} (turns: {}, tokens: {:.0}k/{:.0}k, {:.1}h/{:.1}h)",
            done_confirmed + 1,
            agent.state.turns,
            total_tokens as f64 / 1000.0,
            token_budget as f64 / 1000.0,
            elapsed.as_secs_f32() / 3600.0,
            cli.budget_hours.unwrap_or(8.0),
        );

        let check_prompt = "(autonomous self-check) You just stopped working without any tool calls. \
             Before confirming the goal is complete, verify your work:\n\
             - Did you actually run the build/tests to confirm it works? (bash: cargo test, \
             npm test, or the project's check command)\n\
             - Did you read back every file you created/modified to check for syntax errors?\n\
             - Is there anything in the original task description you haven't addressed?\n\n\
             If everything is verified and complete, reply with exactly: GOAL COMPLETE\n\
             If there is any remaining work, any untested change, or any unaddressed \
             requirement, do that work now using the tools available."
        ;

        let check = agent.run_turn(check_prompt).await?;
        let check_text = check.final_text.trim().to_string();

        if check_text.contains("GOAL COMPLETE") {
            done_confirmed += 1;
            if done_confirmed >= 2 {
                eprintln!(
                    "-- autonomous: GOAL COMPLETE (verified twice, {} requests, {} turns, {:.1}k tokens, {:.1}h)",
                    agent.state.requests,
                    agent.state.turns,
                    (agent.state.usage.input_tokens + agent.state.usage.output_tokens) as f64 / 1000.0,
                    elapsed.as_secs_f32() / 3600.0,
                );
                break;
            }
            eprintln!("-- autonomous: first confirmation; running one more self-check");
        } else {
            done_confirmed = 0;
            // The model either found more work (tool calls happened) or
            // didn't say GOAL COMPLETE — treat both as "keep going".
            if check.final_text.trim().is_empty() && agent.state.turns > 0 {
                // The model ran tools during the self-check: it found work.
                // Continue the loop; the next iteration re-checks.
            }
        }
        last_final = check.final_text.clone();

        // Safety: if the turn cap fires inside run_turn, we'd loop here
        // forever — the hard iteration cap already bounds run_turn, and
        // the budget checks above bound this loop.
        if agent.state.turns >= agent.cfg.max_turns as u64 * 4 {
            eprintln!(
                "-- autonomous: turn ceiling reached ({}), stopping",
                agent.state.turns
            );
            break;
        }
    }

    // Final report.
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
    println!("{}", last_final.trim());
    Ok(())
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
