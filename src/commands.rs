//! Slash-command files: user-defined `/name [args]` prompts loaded from the
//! same `.agents` convention as skills — `~/.agents/commands/*.md` (user) and
//! `.agents/commands/*.md` (workspace, wins on name collisions). The format
//! is the one ZCode-class harnesses use: `---` frontmatter with `description`
//! and `argument-hint`, then a prompt body in which `$ARGUMENTS` is replaced
//! by the user's arguments (appended at the end when the placeholder is
//! absent).

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub description: String,
    pub argument_hint: Option<String>,
    pub body: String,
    pub path: PathBuf,
}

/// User-level commands directory (`MYHARNESS_COMMANDS_DIR` overrides the
/// default `~/.agents/commands`).
pub fn user_commands_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("MYHARNESS_COMMANDS_DIR") {
        if !d.trim().is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    dirs::home_dir().map(|h| h.join(".agents").join("commands"))
}

/// Discover commands for a session: user directory first, then the nearest
/// `.agents/commands` from `root` upward. Workspace files override user
/// files with the same name.
pub fn discover(root: &Path) -> Vec<Command> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(user) = user_commands_dir() {
        dirs.push(user);
    }
    let mut dir = Some(root.to_path_buf());
    while let Some(d) = dir {
        let candidate = d.join(".agents").join("commands");
        if candidate.is_dir() {
            dirs.push(candidate);
            break;
        }
        dir = d.parent().map(PathBuf::from);
    }
    discover_from(&dirs)
}

pub fn discover_from(dirs: &[PathBuf]) -> Vec<Command> {
    let mut out: Vec<Command> = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        let mut files: Vec<_> = entries.flatten().collect();
        files.sort_by_key(|e| e.file_name());
        for e in files {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.to_ascii_lowercase().ends_with(".md") {
                continue;
            }
            if !e.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(e.path()) else { continue };
            let stem = name.trim_end_matches(".md").to_string();
            let cmd = parse_command(&stem, &raw, e.path());
            match out.iter().position(|c| c.name == cmd.name) {
                Some(i) => out[i] = cmd,
                None => out.push(cmd),
            }
        }
    }
    out
}

/// Parse one command file. Frontmatter keys: `description`,
/// `argument-hint`. Missing frontmatter degrades to name = file stem and the
/// whole file as body.
pub fn parse_command(stem: &str, raw: &str, path: PathBuf) -> Command {
    let mut description = String::new();
    let mut argument_hint = None;
    let mut body = raw.trim().to_string();

    let trimmed = raw.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        let after_open = after_open.strip_prefix('\r').unwrap_or(after_open);
        let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
        if let Some(close) = after_open.find("\n---") {
            for line in after_open[..close].lines() {
                if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                } else if let Some(v) = line.strip_prefix("argument-hint:") {
                    argument_hint = Some(v.trim().trim_matches('"').to_string());
                }
            }
            let after_close = &after_open[close + 4..];
            let after_close = after_close.strip_prefix('\r').unwrap_or(after_close);
            let skip_fence_line = after_close.find('\n').unwrap_or(after_close.len());
            body = after_close[skip_fence_line..].trim().to_string();
        }
    }
    Command { name: stem.to_string(), description, argument_hint, body, path }
}

/// Fill the `$ARGUMENTS` placeholder; append the args at the end when the
/// body has no placeholder.
pub fn expand(body: &str, args: &str) -> String {
    let args = args.trim();
    if body.contains("$ARGUMENTS") {
        body.replace("$ARGUMENTS", if args.is_empty() { "(no arguments)" } else { args })
    } else if args.is_empty() {
        body.to_string()
    } else {
        format!("{body}\n\n{args}")
    }
}

/// One-line-per-command listing for `/commands` and HELP.
pub fn render_list(cmds: &[Command]) -> String {
    let mut s = String::from("Slash commands (.agents/commands/*.md):\n");
    for c in cmds {
        let hint = c
            .argument_hint
            .as_deref()
            .map(|h| format!(" {h}"))
            .unwrap_or_default();
        let desc = if c.description.is_empty() { "(no description)" } else { c.description.as_str() };
        s.push_str(&format!("/{name}{hint} — {desc}\n", name = c.name));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_frontmatter_and_hint() {
        let raw = "---\ndescription: Run the dev loop.\nargument-hint: \"[goal]\"\n---\nDo the thing: $ARGUMENTS\n";
        let c = parse_command("dev", raw, PathBuf::from("/x/dev.md"));
        assert_eq!(c.name, "dev");
        assert_eq!(c.description, "Run the dev loop.");
        assert_eq!(c.argument_hint.as_deref(), Some("[goal]"));
        assert_eq!(c.body, "Do the thing: $ARGUMENTS");
    }

    #[test]
    fn expansion_replaces_or_appends() {
        assert_eq!(expand("goal: $ARGUMENTS", "fix the build"), "goal: fix the build");
        assert_eq!(expand("goal: $ARGUMENTS", ""), "goal: (no arguments)");
        assert_eq!(expand("just a body", "extra"), "just a body\n\nextra");
        assert_eq!(expand("just a body", ""), "just a body");
    }

    #[test]
    fn discover_workspace_overrides_user() {
        let user = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        std::fs::write(user.path().join("ship.md"), "---\ndescription: user copy\n---\nuser body").unwrap();
        std::fs::write(user.path().join("notes.txt"), "not a command").unwrap();
        std::fs::write(work.path().join("ship.md"), "---\ndescription: work copy\n---\nwork body").unwrap();

        let cmds = discover_from(&[user.path().to_path_buf(), work.path().to_path_buf()]);
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        assert_eq!(cmds[0].body, "work body");
        assert_eq!(cmds[0].description, "work copy");
    }
}
