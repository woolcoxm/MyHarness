//! Configuration: defaults <- `myharness.toml` <- environment <- CLI flags.

use anyhow::Result;
use clap::ValueEnum;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    Openai,
    Mock,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ShellChoice {
    #[default]
    Auto,
    Bash,
    Powershell,
    Cmd,
}

/// Executable shell resolved for the bash tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shell {
    /// POSIX shell (git-bash on Windows, bash/sh on Unix).
    Posix(PathBuf),
    PowerShell,
    Cmd,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    model: Option<FileModel>,
    agent: Option<FileAgent>,
    bash: Option<FileBash>,
    web: Option<FileWeb>,
    permissions: Option<FilePermissions>,
    hooks: Option<Vec<HookDef>>,
    mcp: Option<Vec<McpServerConfig>>,
    lsp: Option<Vec<LspServerConfig>>,
    /// `[[output_hint]]` array-of-tables.
    output_hint: Option<Vec<OutputHintDef>>,
    zero_mem: Option<crate::zero_mem::ZeroMemCfg>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxMode {
    #[default]
    Off,
    /// Windows: Job Object kill-on-close containment.
    Job,
    /// Windows: AppContainer filesystem isolation (writes limited to the
    /// workspace + temp; no network inside the container).
    AppContainer,
    /// Linux: Landlock filesystem restriction (write: workspace + tmp).
    Strict,
}

impl SandboxMode {
    pub fn name(&self) -> &'static str {
        match self {
            SandboxMode::Off => "off",
            SandboxMode::Job => "job",
            SandboxMode::AppContainer => "appcontainer",
            SandboxMode::Strict => "strict",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    #[serde(alias = "pre_tool_use", alias = "pre")]
    PreToolUse,
    #[serde(alias = "post_tool_use", alias = "post")]
    PostToolUse,
    #[serde(alias = "stop")]
    Stop,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HookDef {
    pub event: HookEvent,
    /// Optional tool name matcher; omit to match all tools.
    pub tool: Option<String>,
    /// Shell command. JSON payload arrives on stdin; exit 2 blocks.
    pub command: String,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
}

/// One output-pattern hint: a distinctive substring to watch for in tool
/// results and the guidance to inject (throttled) when it appears.
#[derive(Debug, Clone, Deserialize)]
pub struct OutputHintDef {
    pub pattern: String,
    pub hint: String,
}

/// One language server: started lazily after the first relevant edit,
/// diagnostics collected post-edit (errors/warnings only).
#[derive(Debug, Clone, Deserialize)]
pub struct LspServerConfig {
    pub name: String,
    /// File extensions this server handles, e.g. ["rs"].
    pub languages: Vec<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// How long to wait for diagnostics per edit round (default 4000).
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileModel {
    provider: Option<ProviderKind>,
    name: Option<String>,
    base_url: Option<String>,
    api_key_env: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    context_window: Option<u64>,
    /// Cheaper model used for compaction summaries.
    model_fast: Option<String>,
    /// Anthropic-protocol prompt caching breakpoints (default true).
    prompt_caching: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileAgent {
    max_turns: Option<u32>,
    compact_ratio: Option<f32>,
    /// Command run automatically after write/edit turns (e.g. "cargo check").
    verify_cmd: Option<String>,
    /// Keep write_file/edit_file inside the workspace root in unattended modes.
    restrict_writes_to_workspace: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileBash {
    timeout_ms: Option<u64>,
    shell: Option<ShellChoice>,
    sandbox: Option<SandboxMode>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FileWeb {
    /// Allow web_fetch to reach localhost/private destinations (dev
    /// servers). Off by default: those ranges are the SSRF surface.
    private_hosts: Option<bool>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct FilePermissions {
    allow: Option<Vec<FileRule>>,
    deny: Option<Vec<FileRule>>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileRule {
    tool: String,
    pattern: String,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub tool: String,
    pub pattern: glob::Pattern,
    #[allow(dead_code)] // kept for diagnostics/logging
    pub raw_pattern: String,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub provider: ProviderKind,
    pub model: String,
    /// Cheaper model for compaction summaries (defaults to `model`).
    pub model_fast: Option<String>,
    pub base_url: String,
    pub api_key: Option<String>,
    pub max_tokens: u32,
    pub temperature: f32,
    pub context_window: u64,
    pub max_turns: u32,
    pub compact_ratio: f32,
    pub verify_cmd: Option<String>,
    pub restrict_writes_to_workspace: bool,
    pub prompt_caching: bool,
    pub bash_timeout_ms: u64,
    pub shell: ShellChoice,
    pub allow_rules: Vec<Rule>,
    pub deny_rules: Vec<Rule>,
    pub hooks: Vec<HookDef>,
    pub mcp_servers: Vec<McpServerConfig>,
    pub lsp_servers: Vec<LspServerConfig>,
    pub sandbox: SandboxMode,
    pub data_dir: PathBuf,
    /// web_fetch may reach localhost/private destinations (SSRF guard knob).
    pub web_fetch_private_hosts: bool,
    /// Substring → guidance table fired against tool results (throttled).
    pub output_hints: Vec<OutputHintDef>,
    /// Zero-token long-term memory (zero-mem port).
    pub zero_mem: crate::zero_mem::ZeroMemCfg,
    #[allow(dead_code)] // reserved for verbose diagnostics
    pub verbose: bool,
    /// Non-interactive (-p): no permission prompts can be asked.
    pub non_interactive: bool,
}

/// Values supplied on the command line; `None` means "not specified".
#[derive(Debug, Clone, Default)]
pub struct CliOverrides {
    pub provider: Option<ProviderKind>,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub verbose: bool,
}

impl Config {
    pub fn load(overrides: CliOverrides) -> Result<Self> {
        // Nearest .env (cwd upward) seeds the environment; real
        // environment variables always win over file values.
        Self::load_dotenv();
        let file = Self::load_file();

        let provider = overrides
            .provider
            .or(file.model.as_ref().and_then(|m| m.provider))
            .unwrap_or(ProviderKind::Anthropic);

        let default_base = match provider {
            ProviderKind::Anthropic => "https://api.z.ai/api/anthropic".to_string(),
            ProviderKind::Openai => "https://api.z.ai/api/paas/v4".to_string(),
            ProviderKind::Mock => "http://mock.invalid".to_string(),
        };
        let env_base = match provider {
            ProviderKind::Anthropic => std::env::var("ANTHROPIC_BASE_URL").ok(),
            _ => std::env::var("OPENAI_BASE_URL").ok(),
        };
        let base_url = overrides
            .base_url
            .or(env_base)
            .or(file.model.as_ref().and_then(|m| m.base_url.clone()))
            .unwrap_or(default_base);

        let model = overrides
            .model
            .or(file.model.as_ref().and_then(|m| m.name.clone()))
            .unwrap_or_else(|| "glm-5.3".to_string());

        let api_key = Self::resolve_api_key(provider, file.model.as_ref().and_then(|m| m.api_key_env.as_deref()));

        let allow_rules = file
            .permissions
            .as_ref()
            .and_then(|p| p.allow.as_ref())
            .map(|rs| rs.iter().filter_map(parse_rule).collect())
            .unwrap_or_default();
        let deny_rules = file
            .permissions
            .as_ref()
            .and_then(|p| p.deny.as_ref())
            .map(|rs| rs.iter().filter_map(parse_rule).collect())
            .unwrap_or_default();

        let data_dir = std::env::var("MYHARNESS_DATA_DIR")
            .map(PathBuf::from)
            .ok()
            .or_else(|| dirs::data_local_dir().map(|d| d.join("myharness")))
            .unwrap_or_else(|| PathBuf::from(".myharness"));

        Ok(Config {
            provider,
            model,
            model_fast: file.model.as_ref().and_then(|m| m.model_fast.clone()),
            base_url,
            api_key,
            max_tokens: file.model.as_ref().and_then(|m| m.max_tokens).unwrap_or(16384),
            temperature: file.model.as_ref().and_then(|m| m.temperature).unwrap_or(0.3),
            context_window: file.model.as_ref().and_then(|m| m.context_window).unwrap_or(200_000),
            max_turns: file.agent.as_ref().and_then(|a| a.max_turns).unwrap_or(80),
            compact_ratio: file.agent.as_ref().and_then(|a| a.compact_ratio).unwrap_or(0.8).clamp(0.3, 0.95),
            verify_cmd: file.agent.as_ref().and_then(|a| a.verify_cmd.clone()),
            restrict_writes_to_workspace: file
                .agent
                .as_ref()
                .and_then(|a| a.restrict_writes_to_workspace)
                .unwrap_or(true),
            prompt_caching: file
                .model
                .as_ref()
                .and_then(|m| m.prompt_caching)
                .unwrap_or(true),
            bash_timeout_ms: file.bash.as_ref().and_then(|b| b.timeout_ms).unwrap_or(120_000).clamp(1_000, 600_000),
            shell: file.bash.as_ref().and_then(|b| b.shell).unwrap_or(ShellChoice::Auto),
            allow_rules,
            deny_rules,
            hooks: file.hooks.clone().unwrap_or_default(),
            mcp_servers: file.mcp.clone().unwrap_or_default(),
            lsp_servers: file.lsp.clone().unwrap_or_default(),
            // Default: Job containment on Windows (harmless robustness),
            // off elsewhere until strict is proven in real workflows.
            sandbox: file.bash.as_ref().and_then(|b| b.sandbox).unwrap_or(if cfg!(windows) {
                SandboxMode::Job
            } else {
                SandboxMode::Off
            }),
            data_dir,
            web_fetch_private_hosts: file.web.as_ref().and_then(|w| w.private_hosts).unwrap_or(false),
            output_hints: {
                let mut hints = default_output_hints();
                hints.extend(file.output_hint.clone().unwrap_or_default());
                hints
            },
            zero_mem: file.zero_mem.unwrap_or_default(),
            verbose: overrides.verbose,
            non_interactive: false,
        })
    }

    /// Find the nearest `.env` from the cwd upward and export its values
    /// into the process environment (existing variables are never
    /// overridden — the real environment wins).
    fn load_dotenv() {
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            let candidate = d.join(".env");
            if candidate.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&candidate) {
                    for (k, v) in parse_dotenv(&raw) {
                        if std::env::var_os(&k).is_none() {
                            std::env::set_var(&k, &v);
                        }
                    }
                }
                return;
            }
            dir = d.parent().map(PathBuf::from);
        }
    }

    fn load_file() -> FileConfig {
        // cwd upward search, then the user-level config.
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            let candidate = d.join("myharness.toml");
            if candidate.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&candidate) {
                    match toml::from_str(&raw) {
                        Ok(cfg) => return cfg,
                        Err(e) => {
                            eprintln!("warning: invalid {} : {e}", candidate.display());
                        }
                    }
                }
            }
            dir = d.parent().map(PathBuf::from);
        }
        if let Some(home_cfg) = dirs::config_dir().map(|d| d.join("myharness").join("config.toml")) {
            if home_cfg.is_file() {
                if let Ok(raw) = std::fs::read_to_string(&home_cfg) {
                    if let Ok(cfg) = toml::from_str(&raw) {
                        return cfg;
                    }
                }
            }
        }
        FileConfig::default()
    }

    fn resolve_api_key(provider: ProviderKind, file_env: Option<&str>) -> Option<String> {
        if let Ok(k) = std::env::var("MYHARNESS_API_KEY") {
            return Some(k);
        }
        if let Some(name) = file_env {
            if let Ok(k) = std::env::var(name) {
                return Some(k);
            }
        }
        let candidates: &[&str] = match provider {
            ProviderKind::Anthropic => &["ZAI_API_KEY", "GLM_API_KEY", "ANTHROPIC_API_KEY"],
            ProviderKind::Openai => &["ZAI_API_KEY", "GLM_API_KEY", "OPENAI_API_KEY"],
            ProviderKind::Mock => &[],
        };
        candidates.iter().find_map(|n| std::env::var(n).ok().filter(|v| !v.is_empty()))
    }

    pub fn sessions_dir(&self) -> PathBuf {
        self.data_dir.join("sessions")
    }

    /// Resolve the shell used by the bash tool.
    pub fn resolve_shell(&self) -> Shell {
        match self.shell {
            ShellChoice::Bash => find_posix_shell().map(Shell::Posix).unwrap_or(Shell::Cmd),
            ShellChoice::Powershell => Shell::PowerShell,
            ShellChoice::Cmd => Shell::Cmd,
            ShellChoice::Auto => {
                if cfg!(windows) {
                    if let Some(bash) = find_posix_shell() {
                        Shell::Posix(bash)
                    } else {
                        Shell::PowerShell
                    }
                } else {
                    let sh = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
                    Shell::Posix(PathBuf::from(sh))
                }
            }
        }
    }
}

fn parse_rule(r: &FileRule) -> Option<Rule> {
    match glob::Pattern::new(&r.pattern) {
        Ok(pattern) => Some(Rule {
            tool: r.tool.clone(),
            pattern,
            raw_pattern: r.pattern.clone(),
        }),
        Err(_) => {
            eprintln!("warning: invalid permission pattern '{}' skipped", r.pattern);
            None
        }
    }
}

/// Built-in output-pattern hints (user [[output_hint]] entries are appended,
/// not replacing these).
fn default_output_hints() -> Vec<OutputHintDef> {
    vec![
        OutputHintDef {
            pattern: "API rate limit exceeded".to_string(),
            hint: "the GitHub API rate limit was hit (5,000/hr, shared across gh and any \
                   API calls). Run `gh api rate_limit` to see the reset time and switch to \
                   other work instead of retrying in a loop"
                .to_string(),
        },
        OutputHintDef {
            pattern: "command not found".to_string(),
            hint: "that command was not found. Check the spelling and the PATH before \
                   installing anything"
                .to_string(),
        },
    ]
}

/// Parse `.env` content: `KEY=VALUE` per line, optional `export ` prefix,
/// surrounding quotes stripped, `#` comments and blanks ignored. Values may
/// contain `=`.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some(eq) = line.find('=') else { continue };
        let key = line[..eq].trim().to_string();
        if key.is_empty() {
            continue;
        }
        let mut value = line[eq + 1..].trim().to_string();
        if value.len() >= 2
            && ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
        {
            value = value[1..value.len() - 1].to_string();
        }
        out.push((key, value));
    }
    out
}

/// Find a POSIX shell on Windows, deliberately ignoring WSL's
/// `C:\Windows\System32\bash.exe` (wrong filesystem semantics).
pub fn find_posix_shell() -> Option<PathBuf> {
    if !cfg!(windows) {
        return std::env::var("SHELL").ok().map(PathBuf::from).or(Some(PathBuf::from("/bin/bash")));
    }
    let candidates = [
        "C:\\Program Files\\Git\\bin\\bash.exe",
        "C:\\Program Files (x86)\\Git\\bin\\bash.exe",
    ];
    for c in candidates {
        let p = PathBuf::from(c);
        if p.is_file() {
            return Some(p);
        }
    }
    if let Ok(localappdata) = std::env::var("LOCALAPPDATA") {
        let p = PathBuf::from(localappdata).join("Programs").join("Git").join("bin").join("bash.exe");
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

pub fn default_config_toml() -> &'static str {
    r#"# myharness.toml — place in a project root or the user config dir.
[model]
provider = "anthropic"          # anthropic | openai | mock
name = "glm-5.3"
base_url = "https://api.z.ai/api/anthropic"
# api_key_env = "ZAI_API_KEY"   # env var to read the key from
max_tokens = 16384
temperature = 0.3
context_window = 200000
# model_fast = "glm-4.7-air"    # cheaper model for compaction summaries
prompt_caching = true           # anthropic-protocol cache breakpoints

[agent]
max_turns = 80
compact_ratio = 0.8             # compact when request exceeds 80% of window
# verify_cmd = "cargo check"    # run automatically after edit turns; the
                                # model sees the result before continuing
restrict_writes_to_workspace = true

[bash]
timeout_ms = 120000
shell = "auto"                  # auto | bash | powershell | cmd
sandbox = "job"                 # windows: job (kill-on-close containment)
                                # linux: strict (landlock) | off

[web]
private_hosts = false           # web_fetch may reach localhost/private
                                # destinations (dev servers); off = SSRF guard

# Zero-token long-term memory (zero-mem): every turn is captured to a
# per-project store; deterministic BM25 + entity-graph retrieval injects
# past-session evidence at turn start. No LLM calls for memory ops.
[zero_mem]
enabled = true
top_k = 3                    # snippets injected per turn
max_units = 5000             # retention bound
max_age_days = 180

# Output-pattern hints: a distinctive substring seen in a tool result fires
# the guidance as a throttled system note (built-ins always apply).
# [[output_hint]]
# pattern = "out of memory"
# hint = "the build ran out of memory; close other tasks or reduce parallelism"

# Lifecycle hooks: JSON payload on stdin; exit code 2 blocks
# (PreToolUse: deny / Stop: force one more round).
# [[hooks]]
# event = "PreToolUse"
# tool = "bash"                  # optional matcher
# command = "myharness-guard.cmd"

# MCP servers over stdio; tools appear as mcp__<name>__<tool>.
# [[mcp]]
# name = "github"
# command = "npx"
# args = ["-y", "@modelcontextprotocol/server-github"]

# Glob patterns matched against the tool's argument summary
# (bash: full command; write/edit: path).
[[permissions.allow]]
tool = "bash"
pattern = "git status"
[[permissions.allow]]
tool = "bash"
pattern = "cargo *"

[[permissions.deny]]
tool = "bash"
pattern = "rm -rf *"
[[permissions.deny]]
tool = "write_file"
pattern = "*.env"
"#
}

/// Format a compact summary of the effective config for `myharness config`.
pub fn summarize(cfg: &Config) -> Result<String> {
    let api = if cfg.api_key.is_some() { "set" } else { "NOT SET" };
    Ok(format!(
        "provider    : {} ({})\nmodel       : {}\nmodel_fast  : {}\nbase_url    : {}\napi_key     : {}\ntokens      : max {} @ temp {}\nwindow      : {} (compact at {}%)\nturns       : max {}\ncaching     : {}\nwrites      : scoped to workspace {}\nverify_cmd  : {}\nbash        : timeout {}ms, shell {:?}, sandbox {}\nweb         : private hosts {}\nzero-mem    : {} units max, memory {}\nhints       : {} output pattern(s)\nhooks       : {}\nmcp servers : {}\nsessions    : {}",
        cfg.provider.name_str(),
        cfg.provider.protocol(),
        cfg.model,
        cfg.model_fast.as_deref().unwrap_or("(same as model)"),
        cfg.base_url,
        api,
        cfg.max_tokens,
        cfg.temperature,
        cfg.context_window,
        (cfg.compact_ratio * 100.0) as u32,
        cfg.max_turns,
        if cfg.prompt_caching { "on (anthropic breakpoints)" } else { "off" },
        cfg.restrict_writes_to_workspace,
        cfg.verify_cmd.as_deref().unwrap_or("(none)"),
        cfg.bash_timeout_ms,
        cfg.shell,
        cfg.sandbox.name(),
        if cfg.web_fetch_private_hosts { "allowed" } else { "denied (SSRF guard)" },
        cfg.zero_mem.max_units,
        if cfg.zero_mem.enabled { "on" } else { "off" },
        cfg.output_hints.len(),
        cfg.hooks.len(),
        cfg.mcp_servers.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(", "),
        cfg.sessions_dir().display(),
    ))
}

impl ProviderKind {
    pub fn name_str(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
            ProviderKind::Mock => "mock",
        }
    }

    pub fn protocol(&self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "Anthropic Messages /v1/messages",
            ProviderKind::Openai => "OpenAI chat completions /chat/completions",
            ProviderKind::Mock => "scripted offline",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotenv_parses_comments_quotes_and_exports() {
        let text = "\n# comment line\nZAI_API_KEY=\"sk abc123\"\n\nexport GLM_API_KEY='single'\nPLAIN=no-quotes\nURL=https://x.example/?a=1&b=2\nEMPTY=\nnot-a-pair-line\n=badkey\n";
        let pairs = parse_dotenv(text);
        assert_eq!(
            pairs,
            vec![
                ("ZAI_API_KEY".to_string(), "sk abc123".to_string()),
                ("GLM_API_KEY".to_string(), "single".to_string()),
                ("PLAIN".to_string(), "no-quotes".to_string()),
                ("URL".to_string(), "https://x.example/?a=1&b=2".to_string()),
                ("EMPTY".to_string(), String::new()),
            ]
        );
    }

    #[test]
    fn output_hints_and_web_parse_from_toml() {
        let raw = r#"
[web]
private_hosts = true

[[output_hint]]
pattern = "out of memory"
hint = "reduce parallelism"

[[output_hint]]
pattern = ""
hint = "empty pattern is skipped at scan time"
"#;
        let cfg: FileConfig = toml::from_str(raw).unwrap();
        assert!(cfg.web.as_ref().and_then(|w| w.private_hosts) == Some(true));
        let hints = cfg.output_hint.unwrap();
        assert_eq!(hints.len(), 2);
        assert_eq!(hints[0].pattern, "out of memory");

        let mut merged = default_output_hints();
        merged.extend(hints);
        assert!(merged.len() >= 4);
        assert!(merged.iter().any(|h| h.pattern == "API rate limit exceeded"));
    }
}
