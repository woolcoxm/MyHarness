//! bash: runs commands in the session shell with a persistent working
//! directory (a marker line reports the post-command cwd back to the
//! harness), timeouts with process-tree kills, head+tail truncation, and
//! optional background execution polled via the `bash_output` tool.

use super::{budget_output, schema_obj, truncate_middle, Tool, ToolCtx, ToolOutput};
use crate::agent::state::BgTask;
use crate::config::Shell;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct BashTool;

const MAX_OUTPUT: usize = 30_000;
const MAX_BG_OUTPUT: usize = 200_000;
const CWD_MARKER: &str = "__MH_CWD__";

fn wrap_posix(cmd: &str) -> String {
    format!("{cmd}\n__mh_rc=$?\nprintf '\\n{CWD_MARKER}%s\\n' \"$PWD\"\nexit $__mh_rc")
}

fn wrap_powershell(cmd: &str) -> String {
    format!("{cmd}\nWrite-Output \"{CWD_MARKER}$($PWD.Path)\"\nexit $LASTEXITCODE")
}

fn wrap_cmd(cmd: &str) -> String {
    // cmd has no reliable late-bound exit code echo; exit code fidelity is
    // best-effort on this last-resort shell.
    format!("{cmd} & echo {CWD_MARKER}%CD%")
}

fn extract_marker(stdout: &str) -> (String, Option<String>) {
    let Some(pos) = stdout.rfind(CWD_MARKER) else {
        return (stdout.to_string(), None);
    };
    let line_start = stdout[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = stdout[pos..].find('\n').map(|i| pos + i + 1).unwrap_or(stdout.len());
    let path = stdout[pos + CWD_MARKER.len()..line_end].trim().to_string();
    let mut cleaned = String::with_capacity(stdout.len());
    cleaned.push_str(&stdout[..line_start]);
    cleaned.push_str(&stdout[line_end..]);
    if cleaned.ends_with('\n') {
        cleaned.pop();
    }
    (cleaned, if path.is_empty() { None } else { Some(path) })
}

async fn kill_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    if cfg!(windows) {
        let _ = tokio::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await;
    }
}

fn build_command(shell: &Shell, script: &str, cwd: &std::path::Path, sandbox: crate::config::SandboxMode) -> tokio::process::Command {
    let mut cmd = match shell {
        Shell::Posix(bash) => {
            let mut c = tokio::process::Command::new(bash);
            c.arg("-c").arg(script);
            c
        }
        Shell::PowerShell => {
            let mut c = tokio::process::Command::new("powershell");
            c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command"])
                .arg(script);
            c
        }
        Shell::Cmd => {
            let mut c = tokio::process::Command::new("cmd");
            c.args(["/D", "/C"]).arg(script);
            c
        }
    };
    cmd.current_dir(cwd)
        .env("MYHARNESS", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    // Linux strict mode: Landlock rules applied in the child before exec.
    #[cfg(target_os = "linux")]
    if sandbox == crate::config::SandboxMode::Strict {
        let ws = cwd.to_path_buf();
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.as_std_mut().pre_exec(move || {
                crate::sandbox::linux_restrict(&ws);
                Ok(())
            });
        }
    }
    let _ = sandbox;
    cmd
}

fn shell_desc(s: &Shell) -> String {
    match s {
        Shell::Posix(p) => format!("posix {}", p.display()),
        Shell::PowerShell => "powershell".to_string(),
        Shell::Cmd => "cmd".to_string(),
    }
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &'static str {
        "bash"
    }

    fn description(&self) -> &'static str {
        "Runs a shell command. Persistent cwd, timeout with process-tree kill. run_in_background=true for long tasks (poll with bash_output). Huge output spills to a file you can read_file."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "command": {"type": "string", "description": "The command to run"},
                "timeout_ms": {"type": "integer", "description": "Foreground timeout in milliseconds (default 120000, max 600000)"},
                "description": {"type": "string", "description": "Short human description of what this command does"},
                "run_in_background": {"type": "boolean", "description": "Start without waiting; poll later with bash_output (default false)"}
            }),
            &["command"],
        )
    }

    fn is_read_only(&self) -> bool {
        false
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .chars()
            .take(100)
            .collect()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let command = match super::require_str(&input, "command") {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let run_in_background = super::opt_bool(&input, "run_in_background")
            .unwrap_or(None)
            .unwrap_or(false);
        let timeout_ms = super::opt_u64(&input, "timeout_ms")
            .unwrap_or(None)
            .unwrap_or(ctx.cfg.bash_timeout_ms)
            .clamp(1_000, 600_000);
        let cwd = ctx.cwd.clone();
        let shell = ctx.cfg.resolve_shell();

        let script = match &shell {
            Shell::Posix(_) => wrap_posix(&command),
            Shell::PowerShell => wrap_powershell(&command),
            Shell::Cmd => wrap_cmd(&command),
        };
        let started = std::time::Instant::now();
        let mut spawned = match spawn_shell(&shell, &script, &cwd, ctx) {
            Ok(s) => s,
            Err(e) => {
                return ToolOutput::err(format!(
                    "failed to start shell ({:?}): {e}",
                    shell_desc(&shell)
                ))
            }
        };
        let pid = spawned.id();

        if run_in_background {
            let id = ctx.next_bg_id;
            let task = spawn_background(spawned, command.clone());
            ctx.effects.background = Some((id, task));
            ctx.effects.bg_counter = Some(id);
            return ToolOutput::ok(format!(
                "Background task #{id} started (pid {}) — {}. Continue working and poll it with bash_output ({{\"id\": {id}}}).",
                pid.map(|p| p.to_string()).unwrap_or_else(|| "?".to_string()),
                command.chars().take(120).collect::<String>()
            ));
        }

        // Foreground: bounded by timeout, abortable on user interrupt.
        let stdout_buf = Arc::new(Mutex::new(String::new()));
        let stderr_buf = Arc::new(Mutex::new(String::new()));
        let out_handle = spawned.take_stdout().map(|s| tokio::spawn(drain_into(s, Arc::clone(&stdout_buf))));
        let err_handle = spawned.take_stderr().map(|s| tokio::spawn(drain_into(s, Arc::clone(&stderr_buf))));
        let deadline = tokio::time::Instant::now() + Duration::from_millis(timeout_ms);
        let exit = loop {
            // Poll at a short cadence: try_wait is non-blocking for both
            // tokio children and AppContainer processes.
            tokio::time::sleep(Duration::from_millis(50)).await;
            if ctx.cancel.load(Ordering::Relaxed) {
                spawned.kill();
                kill_tree(pid).await;
                return ToolOutput::err("command interrupted by user and killed");
            }
            if tokio::time::Instant::now() >= deadline {
                spawned.kill();
                kill_tree(pid).await;
                return ToolOutput::err(format!(
                    "command timed out after {}s and was killed: {}",
                    timeout_ms as f64 / 1000.0,
                    command.chars().take(200).collect::<String>()
                ));
            }
            if let Ok(Some(code)) = spawned.try_wait() {
                break code;
            }
        };
        // Pipes close at exit; give the drain tasks a moment to finish so
        // the tail of the output is not lost.
        if let Some(h) = out_handle {
            let _ = tokio::time::timeout(Duration::from_millis(250), h).await;
        }
        if let Some(h) = err_handle {
            let _ = tokio::time::timeout(Duration::from_millis(250), h).await;
        }
        let stdout = stdout_buf.lock().unwrap().clone();
        let stderr = stderr_buf.lock().unwrap().clone();
        report_foreground(&stdout, &stderr, exit, started, ctx)
    }
}

/// A spawned shell: a plain tokio child, or (AppContainer sandbox) a raw
/// CreateProcessW child carrying the container attribute. The shared
/// surface is deliberately tiny: pid, pipes, wait, kill.
enum Spawned {
    Tokio(Box<tokio::process::Child>),
    #[cfg(windows)]
    Ac(crate::sandbox::appcontainer::AcProcess),
}

impl Spawned {
    fn id(&self) -> Option<u32> {
        match self {
            Spawned::Tokio(c) => c.id(),
            #[cfg(windows)]
            Spawned::Ac(ac) => Some(ac.pid),
        }
    }

    fn take_stdout(&mut self) -> Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        match self {
            Spawned::Tokio(c) => c
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            #[cfg(windows)]
            Spawned::Ac(ac) => ac
                .stdout
                .take()
                .map(|f| Box::new(tokio::fs::File::from_std(f)) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        }
    }

    fn take_stderr(&mut self) -> Option<Box<dyn tokio::io::AsyncRead + Send + Unpin>> {
        match self {
            Spawned::Tokio(c) => c
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
            #[cfg(windows)]
            Spawned::Ac(ac) => ac
                .stderr
                .take()
                .map(|f| Box::new(tokio::fs::File::from_std(f)) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        }
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        match self {
            Spawned::Tokio(c) => Ok(c.try_wait()?.map(|s| s.code().unwrap_or(-1))),
            #[cfg(windows)]
            Spawned::Ac(ac) => Ok(ac.try_wait()),
        }
    }

    fn kill(&self) {
        match self {
            Spawned::Tokio(_) => {}
            #[cfg(windows)]
            Spawned::Ac(ac) => ac.terminate(),
        }
    }
}

/// Spawn the session shell, honoring the sandbox mode (AppContainer
/// launches through raw CreateProcessW; everything else through tokio).
fn spawn_shell(
    shell: &Shell,
    script: &str,
    cwd: &std::path::Path,
    ctx: &ToolCtx<'_>,
) -> std::io::Result<Spawned> {
    if ctx.cfg.sandbox == crate::config::SandboxMode::AppContainer {
        #[cfg(windows)]
        {
            let program: String = match shell {
                Shell::Posix(p) => p.display().to_string(),
                Shell::PowerShell => "powershell".to_string(),
                Shell::Cmd => "cmd".to_string(),
            };
            let args: Vec<String> = match shell {
                Shell::Posix(_) => vec!["-c".to_string(), script.to_string()],
                Shell::PowerShell => vec![
                    "-NoProfile".to_string(),
                    "-NonInteractive".to_string(),
                    "-ExecutionPolicy".to_string(),
                    "Bypass".to_string(),
                    "-Command".to_string(),
                    script.to_string(),
                ],
                Shell::Cmd => vec!["/D".to_string(), "/C".to_string(), script.to_string()],
            };
            let write_dirs = vec![ctx.workspace_root.clone(), std::env::temp_dir()];
            return crate::sandbox::appcontainer::launch(&program, &args, cwd, &write_dirs)
                .map(Spawned::Ac)
                .map_err(std::io::Error::other);
        }
        #[cfg(not(windows))]
        {
            return Err(std::io::Error::other(
                "sandbox 'appcontainer' is Windows-only; use 'strict' (Landlock) on Linux",
            ));
        }
    }
    let child = build_command(shell, script, cwd, ctx.cfg.sandbox).spawn()?;
    // Windows job containment: assign immediately after spawn.
    #[cfg(windows)]
    if ctx.cfg.sandbox == crate::config::SandboxMode::Job && !crate::sandbox::contain_child(&child) {
        eprintln!("(sandbox: job assignment failed; continuing unsandboxed)");
    }
    Ok(Spawned::Tokio(Box::new(child)))
}

fn report_foreground(
    stdout_raw: &str,
    stderr: &str,
    exit: i32,
    started: std::time::Instant,
    ctx: &mut ToolCtx<'_>,
) -> ToolOutput {
    let (stdout, marker_path) = extract_marker(stdout_raw);
    let mut cwd_note: Option<String> = None;
    if let Some(p) = marker_path {
        let pb = std::path::PathBuf::from(&p);
        if pb.is_dir() {
            // cwd moves in-band (the system prompt stays byte-stable for the
            // prompt cache), so tell the model when a cd actually took.
            if !same_dir(&pb, &ctx.cwd) {
                cwd_note = Some(pb.display().to_string());
            }
            ctx.effects.new_cwd = Some(pb);
        }
    }

    let mut report = format!("Exit code: {exit}\n\n--- stdout ---\n{}", stdout.trim_end());
    if !stderr.trim().is_empty() {
        report.push_str("\n\n--- stderr ---\n");
        report.push_str(stderr.trim_end());
    }
    if stdout.trim().is_empty() && stderr.trim().is_empty() {
        report = format!("Exit code: {exit}\n(no output)");
    }
    report.push_str(&format!("\n\n[ran in {:.1}s]", started.elapsed().as_secs_f32()));
    if let Some(cwd) = cwd_note {
        report.push_str(&format!("\n(cwd: {cwd})"));
    }
    ToolOutput::ok(budget_output(ctx, "bash", &report, MAX_OUTPUT, 20_000, 8_000))
}

/// Best-effort directory equality: canonicalized when both sides resolve,
/// else case-insensitive slash-normalized display strings (Windows).
fn same_dir(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => {
            let norm = |p: &std::path::Path| p.display().to_string().replace('\\', "/").to_ascii_lowercase();
            norm(a) == norm(b)
        }
    }
}

/// Append everything readable from `s` into the shared output buffer,
/// capped at MAX_BG_OUTPUT chars — symmetric head+tail (Codex's
/// head_tail_buffer): the first HEAD_KEEP and a rolling tail, with an
/// omission marker, so long-running tasks keep both their opening lines
/// and their latest output.
async fn drain_into<R: tokio::io::AsyncRead + Unpin>(mut s: R, out: Arc<Mutex<String>>) {
    use tokio::io::AsyncReadExt;
    const HEAD_KEEP: usize = 20_000;
    let mut buf = vec![0u8; 8192];
    let mut head = String::new();
    let mut tail = std::collections::VecDeque::new();
    let mut tail_len = 0usize;
    let mut total = 0usize;
    let mut omitted = false;
    loop {
        match s.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                total += chunk.chars().count();
                if head.chars().count() < HEAD_KEEP {
                    let room = HEAD_KEEP - head.chars().count();
                    let mut it = chunk.chars();
                    for c in it.by_ref().take(room) {
                        head.push(c);
                    }
                    let rest: String = it.collect();
                    if !rest.is_empty() {
                        tail_len += rest.chars().count();
                        tail.push_back(rest);
                    }
                } else {
                    tail_len += chunk.chars().count();
                    tail.push_back(chunk);
                }
                // Trim the tail from the front to the rolling budget.
                while tail_len > MAX_BG_OUTPUT.saturating_sub(HEAD_KEEP) {
                    if let Some(front) = tail.pop_front() {
                        tail_len -= front.chars().count();
                        omitted = true;
                    } else {
                        break;
                    }
                }
            }
        }
    }
    let mut guard = out.lock().unwrap();
    if omitted {
        guard.push_str(&head);
        guard.push_str("\n... [middle output omitted] ...\n");
        guard.push_str(&tail.into_iter().collect::<String>());
        let tail_budget = MAX_BG_OUTPUT - HEAD_KEEP;
        guard.push_str(&format!(
            "\n... [total output {total} chars, kept first {HEAD_KEEP} + last {tail_budget}] ..."
        ));
    } else {
        guard.push_str(&head);
        guard.push_str(&tail.into_iter().collect::<String>());
    }
    if guard.chars().count() > MAX_BG_OUTPUT {
        let keep: String = guard.chars().take(MAX_BG_OUTPUT).collect();
        *guard = keep;
    }
}

/// Spawn the reader/waiter task for a background command and return the
/// shared handle the bash_output tool polls.
fn spawn_background(mut spawned: Spawned, command: String) -> BgTask {
    let pid = spawned.id();
    let output = Arc::new(Mutex::new(String::new()));
    let done = Arc::new(AtomicBool::new(false));
    let exit = Arc::new(Mutex::new(None::<i32>));

    let out_buf = Arc::clone(&output);
    let exit_ref = Arc::clone(&exit);
    let done_ref = Arc::clone(&done);
    tokio::spawn(async move {
        let stdout = spawned.take_stdout();
        let stderr = spawned.take_stderr();
        let read_out = async {
            if let Some(s) = stdout {
                drain_into(s, Arc::clone(&out_buf)).await;
            }
        };
        let read_err = async {
            if let Some(s) = stderr {
                drain_into(s, Arc::clone(&out_buf)).await;
            }
        };
        let (_, _) = tokio::join!(read_out, read_err);
        // Pipes closed (process exit or stdio close); poll for the code.
        let mut code = None;
        for _ in 0..200 {
            match spawned.try_wait() {
                Ok(Some(c)) => {
                    code = Some(c);
                    break;
                }
                _ => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
        *exit_ref.lock().unwrap() = code;
        done_ref.store(true, Ordering::Relaxed);
    });

    BgTask {
        pid,
        command,
        started: chrono::Local::now(),
        output,
        done,
        exit,
    }
}

/// Run the opt-in verification command after edits and format a compact
/// report for the model. Synchronous by design; bounded by the bash
/// timeout. Returns (report, exit_ok) — the bool drives the reflect loop.
pub(crate) async fn run_verify(
    cfg: &crate::config::Config,
    cwd: &std::path::Path,
    cmd: &str,
) -> (String, bool) {
    let shell = cfg.resolve_shell();
    let mut command = build_command(&shell, cmd, cwd, cfg.sandbox);
    let timeout = Duration::from_millis(cfg.bash_timeout_ms.min(180_000));
    let started = std::time::Instant::now();
    let spawned = command.spawn();
    let output = match spawned {
        Ok(child) => match tokio::time::timeout(timeout, child.wait_with_output()).await {
            Ok(res) => match res {
                Ok(o) => o,
                Err(e) => return (format!("failed to run verify command: {e}"), false),
            },
            Err(_) => {
                return (
                    format!("verify command timed out after {:.0}s and was killed", timeout.as_secs_f32()),
                    false,
                );
            }
        },
        Err(e) => return (format!("failed to start verify command: {e}"), false),
    };
    let exit = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Clean runs get one line — the empty header scaffold is pure token noise
    // on the (common) green path.
    if exit == 0 && stdout.trim().is_empty() && stderr.trim().is_empty() {
        return (
            format!("Exit code: 0 (in {:.1}s) — no output (clean)", started.elapsed().as_secs_f32()),
            true,
        );
    }
    let trunc = |s: &str, max: usize| truncate_middle(s.trim(), max, max * 2 / 3, max / 3);
    (
        format!(
            "Exit code: {exit} (in {:.1}s)\n--- stdout ---\n{}\n--- stderr ---\n{}",
            started.elapsed().as_secs_f32(),
            trunc(&stdout, 3_000),
            trunc(&stderr, 2_000)
        ),
        exit == 0,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_cwd_marker_line() {
        let stdout = "build ok\n\n__MH_CWD__C:/Users/mark/proj\n";
        let (clean, path) = extract_marker(stdout);
        assert_eq!(clean.trim_end(), "build ok");
        assert_eq!(path.as_deref(), Some("C:/Users/mark/proj"));
    }

    #[test]
    fn no_marker_leaves_output_intact() {
        let (clean, path) = extract_marker("plain output\n");
        assert_eq!(clean, "plain output\n");
        assert!(path.is_none());
    }

    #[test]
    fn truncate_marker_style() {
        let s = "x".repeat(100_000);
        let out = truncate_middle(&s, 30_000, 20_000, 8_000);
        assert!(out.contains("chars truncated]"));
        assert!(out.len() < 32_000);
    }
}
