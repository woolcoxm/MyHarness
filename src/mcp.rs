//! Minimal MCP (Model Context Protocol) client over stdio: spawns configured
//! servers, performs the initialize handshake, lists their tools, and bridges
//! each one into the registry as `mcp__<server>__<tool>`.
//!
//! Transport: newline-delimited JSON-RPC 2.0 (MCP stdio framing). Requests
//! are serialized per server (one connection, one in-flight call).

use crate::config::McpServerConfig;
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::Mutex;

pub struct McpConnection {
    pub name: String,
    #[allow(dead_code)] // keeps the server process alive; dropped = killed
    child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    reader: BufReader<tokio::process::ChildStdout>,
    next_id: u64,
}

pub struct McpToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

impl McpConnection {
    pub async fn start(def: &McpServerConfig, timeout_ms: u64) -> Result<Self> {
        let mut cmd = tokio::process::Command::new(&def.command);
        cmd.args(&def.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to start MCP server '{}'", def.name))?;
        let stdin = child
            .stdin
            .take()
            .context("MCP server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("MCP server stdout unavailable")?;
        let mut conn = McpConnection {
            name: def.name.clone(),
            child,
            stdin,
            reader: BufReader::new(stdout),
            next_id: 1,
        };
        // initialize handshake
        let _init = conn
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {"name": "myharness", "version": env!("CARGO_PKG_VERSION")},
                }),
                timeout_ms,
            )
            .await?;
        conn.notify("notifications/initialized").await?;
        Ok(conn)
    }

    async fn send(&mut self, method: &str, params: Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.stdin
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .context("MCP server closed stdin")?;
        self.stdin.flush().await?;
        Ok(id)
    }

    async fn notify(&mut self, method: &str) -> Result<()> {
        let msg = json!({"jsonrpc": "2.0", "method": method});
        self.stdin
            .write_all(format!("{msg}\n").as_bytes())
            .await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn request(&mut self, method: &str, params: Value, timeout_ms: u64) -> Result<Value> {
        let id = self.send(method, params).await?;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
        loop {
            let mut line = String::new();
            let n = tokio::time::timeout_at(deadline, self.reader.read_line(&mut line))
                .await
                .map_err(|_| anyhow::anyhow!("MCP request '{method}' timed out"))?
                .context("MCP server closed stdout")?;
            if n == 0 {
                bail!("MCP server closed stdout");
            }
            let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
            if v.get("id").and_then(|i| i.as_u64()) != Some(id) {
                continue; // notification or unrelated message
            }
            if let Some(err) = v.get("error") {
                bail!("MCP error: {err}");
            }
            return Ok(v["result"].clone());
        }
    }

    pub async fn list_tools(&mut self) -> Result<Vec<McpToolDef>> {
        let result = self
            .request("tools/list", json!({}), 30_000)
            .await?;
        let mut out = Vec::new();
        if let Some(tools) = result["tools"].as_array() {
            for t in tools {
                let name = t["name"].as_str().unwrap_or_default().to_string();
                if name.is_empty() {
                    continue;
                }
                out.push(McpToolDef {
                    description: t["description"].as_str().unwrap_or_default().to_string(),
                    input_schema: t["inputSchema"].clone(),
                    name,
                });
            }
        }
        Ok(out)
    }

    pub async fn call_tool(&mut self, name: &str, arguments: &Value, timeout_ms: u64) -> Result<String> {
        let result = self
            .request(
                "tools/call",
                json!({"name": name, "arguments": arguments}),
                timeout_ms,
            )
            .await?;
        let mut text = String::new();
        if let Some(content) = result["content"].as_array() {
            for part in content {
                if part["type"] == "text" {
                    if !text.is_empty() {
                        text.push('\n');
                    }
                    text.push_str(part["text"].as_str().unwrap_or_default());
                }
            }
        }
        if result["isError"].as_bool() == Some(true) {
            bail!("MCP tool error: {text}");
        }
        if text.is_empty() {
            text = "(no content)".to_string();
        }
        Ok(text)
    }
    // Process cleanup is handled by kill_on_drop when the connection drops.
}

/// A bridged MCP tool, named `mcp__<server>__<tool>` (the convention
/// Claude Code established and models recognize). The name/description are
/// leaked once at construction — Tool::name() returns &'static str and MCP
/// names are inherently dynamic; tool counts are tens, not millions.
pub struct McpTool {
    server: Arc<Mutex<McpConnection>>,
    tool_name: &'static str,
    tool_desc: &'static str,
    def_name: String,
    input_schema: Value,
}

impl McpTool {
    pub fn new(server: Arc<Mutex<McpConnection>>, server_name: &str, def: McpToolDef) -> Self {
        let tool_name: &'static str =
            Box::leak(format!("mcp__{server_name}__{}", def.name).into_boxed_str());
        let tool_desc: &'static str = if def.description.is_empty() {
            Box::leak(format!("Tool '{}' from MCP server '{server_name}'.", def.name).into_boxed_str())
        } else {
            Box::leak(def.description.clone().into_boxed_str())
        };
        McpTool {
            server,
            tool_name,
            tool_desc,
            def_name: def.name,
            input_schema: def.input_schema,
        }
    }
}

#[async_trait::async_trait]
impl crate::tools::Tool for McpTool {
    fn name(&self) -> &'static str {
        self.tool_name
    }

    fn description(&self) -> &'static str {
        self.tool_desc
    }

    fn schema(&self) -> Value {
        if self.input_schema.is_object() {
            self.input_schema.clone()
        } else {
            json!({"type": "object", "properties": {}})
        }
    }

    fn is_read_only(&self) -> bool {
        false // MCP tools can do anything; permission rules apply
    }

    fn perm_summary(&self, _input: &Value) -> String {
        self.tool_name.to_string()
    }

    async fn execute(&self, input: Value, _ctx: &mut crate::tools::ToolCtx<'_>) -> crate::tools::ToolOutput {
        let mut conn = self.server.lock().await;
        match conn.call_tool(&self.def_name, &input, 120_000).await {
            Ok(text) => crate::tools::ToolOutput::ok(text),
            Err(e) => crate::tools::ToolOutput::err(format!("MCP call failed: {e}")),
        }
    }
}

/// Start all configured MCP servers and bridge their tools.
/// Returns (tools, warnings); failures degrade gracefully.
pub async fn init_mcp_tools(cfg: &crate::config::Config) -> (Vec<Arc<dyn crate::tools::Tool>>, Vec<String>) {
    let mut tools: Vec<Arc<dyn crate::tools::Tool>> = Vec::new();
    let mut warnings = Vec::new();
    for def in &cfg.mcp_servers {
        match McpConnection::start(def, 30_000).await {
            Ok(conn) => {
                let name = def.name.clone();
                let shared = Arc::new(Mutex::new(conn));
                let listed = shared.lock().await.list_tools().await;
                match listed {
                    Ok(defs) => {
                        for d in defs {
                            tools.push(Arc::new(McpTool::new(Arc::clone(&shared), &name, d)));
                        }
                    }
                    Err(e) => warnings.push(format!("mcp '{}': tools/list failed: {e}", def.name)),
                }
            }
            Err(e) => warnings.push(format!("mcp '{}': {e}", def.name)),
        }
    }
    (tools, warnings)
}
