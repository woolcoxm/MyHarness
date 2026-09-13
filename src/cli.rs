//! CLI surface (clap derive).

use crate::config::ProviderKind;
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "myharness",
    version,
    about = "A terminal coding-agent harness for GLM — designed around the contract the model wants.",
    long_about = None
)]
pub struct Cli {
    /// Initial task. With -p, runs non-interactively and prints the final answer.
    #[arg(index = 1)]
    pub task: Option<String>,

    /// Non-interactive mode: run the task, print the final message, exit.
    #[arg(short = 'p', long = "print")]
    pub print: bool,

    /// With -p: validate the final message as JSON against this schema
    /// (inline JSON or a path to a .json file). Subset: type, properties,
    /// required, items, enum. Invalid output gets corrective retries; a
    /// still-invalid result exits non-zero.
    #[arg(long, requires = "print")]
    pub output_schema: Option<String>,

    /// Autonomous mode: work until verified done, not until the model
    /// stops. Each time the model ends its turn without tool calls, a
    /// self-check prompt asks it to verify — if it finds more work, it
    /// continues; if it confirms done twice in a row, the goal completes.
    /// Bounded by --budget-hours and --budget-tokens.
    #[arg(long, requires = "print")]
    pub autonomous: bool,

    /// Autonomous mode: wall-clock budget in hours (default 8).
    #[arg(long, requires = "autonomous")]
    pub budget_hours: Option<f64>,

    /// Autonomous mode: total token budget (input + output, default 1M).
    #[arg(long, requires = "autonomous")]
    pub budget_tokens: Option<u64>,

    /// Model name (default glm-5.3).
    #[arg(long, global = true)]
    pub model: Option<String>,

    /// Provider protocol: anthropic | openai | mock.
    #[arg(long, global = true)]
    pub provider: Option<ProviderKind>,

    /// API base URL override.
    #[arg(long, global = true)]
    pub base_url: Option<String>,

    /// Permission mode: plan | ask | auto-edit | yolo.
    #[arg(long, global = true)]
    pub mode: Option<String>,

    /// Allow everything without asking (same as --mode yolo).
    #[arg(long, alias = "dangerously-skip-permissions", global = true)]
    pub yolo: bool,

    /// Resume the most recent session.
    #[arg(short = 'c', long = "continue", global = true)]
    pub continue_last: bool,

    /// Resume a specific session id (prefix match).
    #[arg(long, global = true)]
    pub session: Option<String>,

    /// Verbose diagnostics.
    #[arg(long, global = true)]
    pub verbose: bool,

    #[command(subcommand)]
    pub cmd: Option<Command>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// List saved sessions.
    Sessions,
    /// Print the effective configuration.
    Config,
    /// Resume a session (id prefix; default: most recent).
    Resume { id: Option<String> },
    /// JSON-RPC 2.0 over stdio (one JSON object per line) so other tools
    /// can drive the harness: initialize, session.list, turn.run,
    /// turn.cancel; mh/* notifications stream during turns.
    Serve,
}
