//! repo_map: a symbol-level digest of the repository — "what is defined
//! where" instead of the layout digest's "what lives where". Definitions are
//! extracted with per-language line regexes (best-effort, zero dependencies —
//! tree-sitter precision stays on the roadmap) and files are ranked by a
//! simplified PageRank over the import graph (the most-referenced files
//! first — Aider's insight, git-less), with modification time as tiebreaker.

use super::{schema_obj, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct RepoMapTool;

const DEFAULT_ENTRIES: usize = 120;
const MAX_PER_FILE: usize = 6;
const MAX_OUTPUT: usize = 10_000;
const MAX_FILE: u64 = 1_000_000;
const SKIP_DIRS: &[&str] = &[
    "target", "node_modules", "dist", "build", ".venv", "__pycache__", ".next", "vendor", ".git",
];

#[async_trait]
impl Tool for RepoMapTool {
    fn name(&self) -> &'static str {
        "repo_map"
    }

    fn description(&self) -> &'static str {
        "Returns a symbol map of the repository: per file, the definitions it contains (functions, structs, classes, traits...), most recently modified files first. Cheaper than grep when the question is 'where is X defined' — then read_file/grep for details. Best-effort regex extraction; generated/minified files are skipped."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "path": {"type": "string", "description": "Subdirectory to map (default: working directory)"},
                "max_entries": {"type": "integer", "description": "Maximum files to list (default 120)"}
            }),
            &[],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let base = match super::opt_str(&input, "path") {
            Ok(Some(p)) => super::resolve_path(&ctx.cwd, &p),
            _ => ctx.cwd.clone(),
        };
        if !base.is_dir() {
            return ToolOutput::err(format!(
                "no such directory: {} (repo_map maps a directory tree)",
                base.display()
            ));
        }
        let max_entries = super::opt_u64(&input, "max_entries")
            .unwrap_or(None)
            .unwrap_or(DEFAULT_ENTRIES as u64)
            .clamp(1, 500) as usize;

        let cwd_disp = ctx.cwd.clone();
        let boost_paths: Vec<std::path::PathBuf> = ctx.files_read.iter().cloned().collect();
        let base = base.clone();
        let handle = tokio::task::spawn_blocking(move || {
            // (mtime, display path, symbols, imports)
            let mut files: Vec<(std::time::SystemTime, String, Vec<String>, Vec<String>)> = Vec::new();
            let mut scanned = 0usize;
            let mut builder = ignore::WalkBuilder::new(&base);
            builder.hidden(true).git_ignore(true).git_global(true).git_exclude(true).parents(true);
            builder.filter_entry(|e| {
                e.file_type()
                    .map(|ft| !ft.is_dir() || !SKIP_DIRS.contains(&e.file_name().to_string_lossy().as_ref()))
                    .unwrap_or(true)
            });
            for entry in builder.build().flatten() {
                let Some(ft) = entry.file_type() else { continue };
                if !ft.is_file() {
                    continue;
                }
                let path = entry.path();
                let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                if name.contains(".min.") || name.ends_with(".map") || name.starts_with('.') {
                    continue;
                }
                let Some(ext) = path.extension().and_then(|e| e.to_str()) else { continue };
                let ext = ext.to_ascii_lowercase();
                let Some(lang) = language_of(&ext) else { continue };
                let Ok(meta) = std::fs::metadata(path) else { continue };
                if meta.len() > MAX_FILE {
                    continue;
                }
                scanned += 1;
                let Ok(text) = std::fs::read_to_string(path) else { continue };
                let symbols = extract_symbols(lang, &text);
                let imports = parse_imports(lang, &text);
                if symbols.is_empty() {
                    continue;
                }
                let mtime = meta.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((mtime, super::display_path(&cwd_disp, path), symbols, imports));
            }
            // Rank: PageRank over resolved imports (most-referenced first),
            // mtime as the tiebreaker. Aider's fit math: files already read
            // this session score as if referenced 50x (they are "in the
            // chat"), and important project files pin at the top regardless.
            let boost: Vec<String> = boost_paths
                .iter()
                .map(|p| super::display_path(&cwd_disp, p))
                .collect();
            let ranked = rank_files(&files, &boost);
            (ranked, scanned)
        })
        .await;

        let (files, scanned) = handle.unwrap_or((Vec::new(), 0));
        if files.is_empty() {
            return ToolOutput::ok(format!(
                "No definitions found in {} files scanned (supported: Rust, Python, JS/TS, Go, Java/C#).",
                scanned
            ));
        }
        let mut out = String::new();
        let mut listed = 0usize;
        // Pinned project files (Aider): the handful of files that orient
        // any reader, even though they carry no extractable symbols.
        const PINNED: &[&str] = &[
            "Cargo.toml", "package.json", "go.mod", "pyproject.toml",
            "Makefile", "README.md", "build.gradle", "pom.xml",
        ];
        let mut pinned_lines = Vec::new();
        for name in PINNED {
            let p = super::resolve_path(&ctx.cwd, name);
            if p.is_file() && pinned_lines.len() < 4 {
                pinned_lines.push(format!("{} (pinned)", super::display_path(&ctx.cwd, &p)));
            }
        }
        for line in &pinned_lines {
            out.push_str(line);
            out.push('\n');
            listed += 1;
        }
        for (_, path, symbols, _) in files.iter().take(max_entries) {
            let syms: Vec<&str> = symbols.iter().map(|s| s.as_str()).collect();
            let line = format!("{path} — {}", syms.iter().take(MAX_PER_FILE).copied().collect::<Vec<_>>().join(", "));
            if out.chars().count() + line.chars().count() > MAX_OUTPUT {
                out.push_str(&format!(
                    "\n... [map truncated at {} chars; {} files had symbols from {} scanned — narrow with the path parameter]",
                    MAX_OUTPUT, files.len(), scanned
                ));
                break;
            }
            out.push_str(&line);
            out.push('\n');
            listed += 1;
        }
        out.push_str(&format!(
            "\n[{} of {} symbol-bearing files, {} scanned — ranked by import references, then recency]",
            listed, files.len(), scanned
        ));
        ToolOutput::ok(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Lang {
    Rust,
    Python,
    JsTs,
    Go,
    JavaLike,
}

fn language_of(ext: &str) -> Option<Lang> {
    match ext {
        "rs" => Some(Lang::Rust),
        "py" | "pyi" => Some(Lang::Python),
        "js" | "mjs" | "cjs" | "jsx" | "ts" | "tsx" => Some(Lang::JsTs),
        "go" => Some(Lang::Go),
        "java" | "cs" | "kt" | "scala" => Some(Lang::JavaLike),
        _ => None,
    }
}

/// Extract `kind name` definition strings from source text. Line-regex,
/// best-effort — the point is navigation, not completeness.
pub(crate) fn extract_symbols(lang: Lang, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let hit = match lang {
            Lang::Rust => rust_symbol(t),
            Lang::Python => python_symbol(t),
            Lang::JsTs => jsts_symbol(t),
            Lang::Go => go_symbol(t),
            Lang::JavaLike => java_symbol(t),
        };
        if let Some(sym) = hit {
            if !out.iter().any(|s: &String| s == &sym) {
                out.push(sym);
            }
        }
    }
    out
}

fn word_after(t: &str, prefix: &str) -> Option<String> {
    t.strip_prefix(prefix)?
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .find(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn rust_symbol(t: &str) -> Option<String> {
    let vis = t.strip_prefix("pub ").unwrap_or(t);
    let vis = vis.strip_prefix("pub(crate) ").unwrap_or(vis);
    let vis = vis.strip_prefix("async ").unwrap_or(vis);
    let vis = vis.strip_prefix("unsafe ").unwrap_or(vis);
    let vis = vis.strip_prefix("extern \"C\" ").unwrap_or(vis);
    for kind in ["fn", "struct", "enum", "trait", "mod", "type", "union"] {
        if let Some(name) = word_after(vis, &format!("{kind} ")) {
            return Some(format!("{kind} {name}"));
        }
    }
    if let Some(rest) = vis.strip_prefix("impl") {
        let name = rest
            .trim_start_matches(|c: char| c == '<' || c.is_alphanumeric() || c == '_')
            .trim_start();
        let name: String = name.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == ':').collect();
        if !name.is_empty() {
            return Some(format!("impl {name}"));
        }
    }
    None
}

fn python_symbol(t: &str) -> Option<String> {
    let t = t.strip_prefix("async ").unwrap_or(t);
    if let Some(name) = word_after(t, "def ") {
        return Some(format!("def {name}"));
    }
    if let Some(name) = word_after(t, "class ") {
        return Some(format!("class {name}"));
    }
    None
}

fn jsts_symbol(t: &str) -> Option<String> {
    let t = t.strip_prefix("export ").unwrap_or(t);
    let t = t.strip_prefix("default ").unwrap_or(t);
    let t = t.strip_prefix("declare ").unwrap_or(t);
    let t = t.strip_prefix("abstract ").unwrap_or(t);
    let t = t.strip_prefix("async ").unwrap_or(t);
    if let Some(name) = word_after(t, "function ") {
        return Some(format!("fn {name}"));
    }
    if t.starts_with("function*") {
        return word_after(t, "function* ").map(|name| format!("fn {name}"));
    }
    if let Some(name) = word_after(t, "class ") {
        return Some(format!("class {name}"));
    }
    if let Some(name) = word_after(t, "interface ") {
        return Some(format!("interface {name}"));
    }
    // const/let bindings that look like functions or components.
    for kw in ["const ", "let ", "var "] {
        if let Some(rest) = t.strip_prefix(kw) {
            let name: String = rest.chars().take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$').collect();
            let tail = &rest[name.len()..];
            if !name.is_empty() && (tail.trim_start().starts_with('=') || tail.trim_start().starts_with(':')) {
                let body = tail.split_once('=').map(|(_, b)| b).unwrap_or(tail);
                let body = body.trim_start_matches(':').trim();
                if body.starts_with('(') || body.starts_with("async") || body.starts_with("function") {
                    return Some(format!("fn {name}"));
                }
            }
        }
    }
    None
}

fn go_symbol(t: &str) -> Option<String> {
    if let Some(rest) = t.strip_prefix("func ") {
        let recv: String = rest.chars().take_while(|c| *c != ')').collect();
        let after = if rest.starts_with('(') {
            rest.get(recv.len()..).and_then(|r| r.split_once(')')).map(|(_, a)| a.trim_start()).unwrap_or(rest)
        } else {
            rest
        };
        let name: String = after.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
        if !name.is_empty() {
            return Some(format!("fn {name}"));
        }
        return None;
    }
    if let Some(rest) = t.strip_prefix("type ") {
        if let Some((name, tail)) = rest.split_once(' ') {
            if tail.starts_with("struct") || tail.starts_with("interface") {
                return Some(format!("type {name}"));
            }
        }
    }
    None
}

fn java_symbol(t: &str) -> Option<String> {
    for kw in ["class ", "interface ", "enum ", "record "] {
        if let Some(name) = word_after(t, kw) {
            return Some(format!("{} {name}", kw.trim()));
        }
    }
    None
}

/// Extract module-ish import paths from source text (for the ranking graph
/// only — fuzzy is fine, misses just soften the ranking).
pub(crate) fn parse_imports(lang: Lang, text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        match lang {
            Lang::Rust => {
                if let Some(rest) = t.strip_prefix("use ") {
                    let path = rest.trim_end_matches(';').split(" as ").next().unwrap_or(rest);
                    let path = path.split('{').next().unwrap_or(path).trim();
                    let cleaned = path
                        .trim_start_matches("crate::")
                        .trim_start_matches("self::")
                        .trim_start_matches("super::")
                        .trim_start_matches("crate::");
                    if !cleaned.is_empty() {
                        out.push(cleaned.replace("::", "/"));
                    }
                }
            }
            Lang::Python => {
                if let Some(rest) = t.strip_prefix("import ") {
                    let path = rest.split(" as ").next().unwrap_or(rest);
                    out.push(path.trim().replace('.', "/"));
                } else if let Some(rest) = t.strip_prefix("from ") {
                    if let Some((path, _)) = rest.split_once(" import ") {
                        out.push(path.trim().replace('.', "/"));
                    }
                }
            }
            Lang::JsTs => {
                for pat in ["from \"", "from '", "import \"", "import '", "require(\"", "require('"] {
                    if let Some(pos) = t.find(pat) {
                        let rest = &t[pos + pat.len()..];
                        if let Some(end) = rest.find(['"', '\'']) {
                            let p = &rest[..end];
                            if p.starts_with("./") || p.starts_with("../") {
                                out.push(p.trim_start_matches("./").trim_start_matches("../").to_string());
                            }
                            break;
                        }
                    }
                }
            }
            Lang::Go => {
                if t.starts_with('"') && t.ends_with('"') && t.len() > 2 {
                    // Import block line: rank by the last path segment.
                    let seg = t.trim_matches('"').rsplit('/').next().unwrap_or("").to_string();
                    if !seg.is_empty() {
                        out.push(seg);
                    }
                }
            }
            Lang::JavaLike => {
                if let Some(rest) = t.strip_prefix("import ") {
                    out.push(rest.trim_end_matches(';').replace('.', "/"));
                }
            }
        }
    }
    out
}

/// Resolve one import path against the file list; a file matches when its
/// path (extension-stripped) ends with the import path (module-path or
/// relative). Returns file indices.
fn resolve_import(import: &str, paths: &[String]) -> Vec<usize> {
    let import = import.trim_matches('/');
    paths
        .iter()
        .enumerate()
        .filter(|(_, p)| {
            let stem = p.trim_end_matches(".rs").trim_end_matches(".py");
            let stem = stem.trim_end_matches("/mod").trim_end_matches("/index");
            let norm = stem.replace('\\', "/");
            norm == import || norm.ends_with(&format!("/{import}"))
        })
        .map(|(i, _)| i)
        .collect()
}

/// Order files by a simplified PageRank (damping 0.85, 20 iterations) over
/// the resolved import graph, mtime desc as tiebreaker. `files` tuples are
/// (mtime, path, symbols, imports); the returned order is (mtime, path,
/// symbols, imports) sorted most-important-first.
pub(crate) fn rank_files(
    files: &[(std::time::SystemTime, String, Vec<String>, Vec<String>)],
    boost: &[String],
) -> Vec<(std::time::SystemTime, String, Vec<String>, Vec<String>)> {
    let n = files.len();
    let paths: Vec<String> = files.iter().map(|(_, p, _, _)| p.clone()).collect();
    // Adjacency: edges[j] = indices j imports (j → i means i is referenced).
    let mut edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (j, (_, _, _, imports)) in files.iter().enumerate() {
        for imp in imports {
            for target in resolve_import(imp, &paths) {
                if target != j && !edges[j].contains(&target) {
                    edges[j].push(target);
                }
            }
        }
    }
    let out_degree: Vec<f64> = edges.iter().map(|e| e.len().max(1) as f64).collect();
    let mut score = vec![1.0f64; n];
    for _ in 0..20 {
        let mut next = vec![0.15f64; n];
        for (j, targets) in edges.iter().enumerate() {
            if targets.is_empty() {
                continue;
            }
            let share = 0.85 * score[j] / out_degree[j];
            for i in targets {
                next[*i] += share;
            }
        }
        score = next;
    }
    // Aider's in-chat boost: files already read this session are what the
    // conversation is about — rank them as if referenced 50x.
    for (j, (_, path, _, _)) in files.iter().enumerate() {
        if boost.iter().any(|b| b == path) {
            score[j] *= 50.0;
        }
    }
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| {
        score[b]
            .partial_cmp(&score[a])
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(files[b].0.cmp(&files[a].0))
    });
    order.into_iter().map(|i| files[i].clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rust_symbols_include_kinds() {
        let src = "pub struct Config { }\nimpl Tool for X { }\nasync fn run(&self) {}\nfn main() {}\nlet x = 1;\n";
        let syms = extract_symbols(Lang::Rust, src);
        assert_eq!(syms, vec!["struct Config", "impl Tool", "fn run", "fn main"]);
    }

    #[test]
    fn python_and_go_and_jsts() {
        let py = "class Foo:\n    def bar(self):\n        async def baz(self):\n";
        assert_eq!(extract_symbols(Lang::Python, py), vec!["class Foo", "def bar", "def baz"]);
        let go = "func (s *Server) Start() {}\nfunc main() {}\ntype Reader struct {}\n";
        assert_eq!(extract_symbols(Lang::Go, go), vec!["fn Start", "fn main", "type Reader"]);
        let ts = "export function load(): void {}\nconst add = (a, b) => a + b;\nclass Widget {}\nconst total = 3;\n";
        assert_eq!(extract_symbols(Lang::JsTs, ts), vec!["fn load", "fn add", "class Widget"]);
    }

    #[test]
    fn imports_parse_per_language() {
        assert_eq!(
            parse_imports(Lang::Rust, "use crate::agent::state;\nuse std::collections::HashMap;\n"),
            vec!["agent/state", "std/collections/HashMap"]
        );
        assert_eq!(
            parse_imports(Lang::Python, "import os.path\nfrom myapp.models import User\n"),
            vec!["os/path", "myapp/models"]
        );
        assert_eq!(
            parse_imports(Lang::JsTs, "import { x } from \"./util\";\nconst y = require('./helper');\n"),
            vec!["util", "helper"]
        );
        assert_eq!(parse_imports(Lang::Go, "\t\"github.com/foo/bar/server\"\n"), vec!["server"]);
    }

    #[test]
    fn referenced_files_outrank_leaves() {
        use std::time::Duration;
        let epoch = std::time::SystemTime::UNIX_EPOCH;
        // core.rs is imported by two files; leaf.rs imports nothing and is
        // newer (recency must not beat two references).
        let files = vec![
            (epoch, "src/core.rs".to_string(), vec!["fn core".to_string()], vec![]),
            (epoch, "src/a.rs".to_string(), vec!["fn a".to_string()], vec!["src/core".to_string()]),
            (epoch, "src/b.rs".to_string(), vec!["fn b".to_string()], vec!["src/core".to_string()]),
            (epoch + Duration::from_secs(9999), "src/leaf.rs".to_string(), vec!["fn leaf".to_string()], vec![]),
        ];
        let ranked = rank_files(&files, &[]);
        assert_eq!(ranked[0].1, "src/core.rs", "most-referenced file first: {:?}", ranked.iter().map(|f| &f.1).collect::<Vec<_>>());
        // Ties (a, b, leaf all score the base rank) break by recency: the
        // newest of them comes next — but never above the referenced core.
        assert_eq!(ranked[1].1, "src/leaf.rs", "mtime breaks ties: {:?}", ranked.iter().map(|f| &f.1).collect::<Vec<_>>());
        assert!(ranked[2].1 == "src/a.rs" || ranked[2].1 == "src/b.rs");
    }
}
