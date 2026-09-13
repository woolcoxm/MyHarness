//! Permission engine: deny rules always win, then allow rules, then the
//! mode's default policy. In non-interactive mode anything that would need a
//! prompt is denied with guidance instead.

use crate::config::Rule;
use Decision::{Allow, Deny, Prompt};
use PermissionMode::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    /// Read-only tools allowed; mutations denied with "present a plan".
    Plan,
    /// Read-only allowed; edits allowed; bash prompted (or denied in -p).
    Ask,
    /// Read-only allowed; edits allowed; bash follows allow/deny rules only.
    AutoEdit,
    /// Everything allowed except deny rules.
    Yolo,
}

impl PermissionMode {
    pub fn parse(s: &str) -> anyhow::Result<Self> {
        match s.to_ascii_lowercase().as_str() {
            "plan" => Ok(Plan),
            "ask" => Ok(Ask),
            "auto-edit" | "autoedit" | "edit" => Ok(AutoEdit),
            "yolo" | "bypass" | "danger" => Ok(Yolo),
            other => anyhow::bail!("unknown mode '{other}' (plan | ask | auto-edit | yolo)"),
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Plan => "plan",
            Ask => "ask",
            AutoEdit => "auto-edit",
            Yolo => "yolo",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
    Prompt,
}

#[derive(Debug)]
pub struct PermissionEngine {
    pub mode: PermissionMode,
    pub allow_rules: Vec<Rule>,
    pub deny_rules: Vec<Rule>,
    /// (tool, pattern) pairs the user approved with "always" this session.
    pub session_allows: Vec<(String, glob::Pattern)>,
    pub non_interactive: bool,
}

impl PermissionEngine {
    pub fn new(mode: PermissionMode, allow_rules: Vec<Rule>, deny_rules: Vec<Rule>, non_interactive: bool) -> Self {
        PermissionEngine {
            mode,
            allow_rules,
            deny_rules,
            session_allows: Vec::new(),
            non_interactive,
        }
    }

    pub fn set_mode(&mut self, mode: PermissionMode) {
        self.mode = mode;
    }

    fn matches(rules: &[Rule], session: &[(String, glob::Pattern)], tool: &str, arg: &str) -> bool {
        rules.iter().any(|r| r.tool == tool && r.pattern.matches(arg))
            || session.iter().any(|(t, p)| t == tool && p.matches(arg))
    }

    /// Split a compound shell command into subcommands so a rule on `ls`
    /// cannot smuggle through `ls && rm -rf /`. Conservative: quoted
    /// separators over-split, which only causes an extra prompt, never a
    /// silent allow.
    pub fn split_compound(command: &str) -> Vec<String> {
        command
            .split(['&', '|', ';', '\n'])
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect()
    }

    pub fn check(&self, tool: &str, arg: &str, read_only: bool) -> Decision {
        self.check_gated(tool, arg, read_only, false)
    }

    /// `gated_read_only`: the tool cannot modify state but should still be
    /// approval-gated (network egress). Plan mode allows it; ask/auto-edit
    /// prompt; yolo allows.
    pub fn check_gated(&self, tool: &str, arg: &str, read_only: bool, gated_read_only: bool) -> Decision {
        // Bash rules apply to every subcommand of a compound command:
        // deny if any part matches a deny rule; allow only if ALL parts are
        // explicitly allowed.
        if tool == "bash" && !arg.is_empty() {
            let parts = Self::split_compound(arg);
            for part in &parts {
                if Self::matches(&self.deny_rules, &[], tool, part) {
                    return Deny(format!(
                        "denied by a deny rule (subcommand '{part}' of the command)"
                    ));
                }
            }
            let all_allowed = parts
                .iter()
                .all(|p| Self::matches(&self.allow_rules, &self.session_allows, tool, p));
            if all_allowed && !parts.is_empty() {
                return Allow;
            }
            return self.mode_default(tool, arg, read_only, gated_read_only);
        }
        if Self::matches(&self.deny_rules, &[], tool, arg) {
            return Deny(format!(
                "denied by a deny rule (tool {tool}, pattern matched '{arg}')"
            ));
        }
        if Self::matches(&self.allow_rules, &self.session_allows, tool, arg) {
            return Allow;
        }
        self.mode_default(tool, arg, read_only, gated_read_only)
    }

    fn mode_default(&self, tool: &str, arg: &str, read_only: bool, gated_read_only: bool) -> Decision {
        if read_only && !gated_read_only {
            return Allow;
        }
        match self.mode {
            Yolo => Allow,
            // Read-only-but-gated tools (web_fetch) stay available for
            // research in plan mode; mutations do not. Bash gets the
            // command guard: provably read-only commands may run.
            Plan if read_only => Allow,
            Plan => {
                if tool == "bash" && Self::plan_safe_command(arg) {
                    return Allow;
                }
                Deny(
                    "plan mode is active: read-only tools work and only provably read-only \
                     commands may run (ls, cat, grep, find, git status/diff/log, gh view/list); \
                     no redirection or substitution. Present your plan and ask the user to \
                     switch modes (/mode auto-edit) to make changes."
                        .to_string(),
                )
            }
            Ask | AutoEdit => {
                let is_edit = tool == "write_file" || tool == "edit_file";
                if is_edit && self.mode == AutoEdit {
                    return Allow;
                }
                if self.non_interactive {
                    Deny(format!(
                        "needs approval and no prompt is possible in -p mode \
                         (rerun with --yolo, or add an allow rule for {tool})"
                    ))
                } else {
                    Prompt
                }
            }
        }
    }

    /// Plan-mode bash guard (Cline's command guard, allowlist-flavored so
    /// unknown interpreters can't sneak through): the command must split,
    /// outside quotes, into subcommands whose programs and subcommand
    /// shapes are provably read-only. Redirection, command substitution,
    /// and heredocs deny outright.
    pub fn plan_safe_command(cmd: &str) -> bool {
        match split_and_screen(cmd) {
            None => false,
            Some(parts) => !parts.is_empty() && parts.iter().all(|p| subcommand_safe(p)),
        }
    }

    /// Record an "always allow" answer from an interactive prompt.
    pub fn allow_session(&mut self, tool: &str, pattern: &str) {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.session_allows.push((tool.to_string(), p));
        }
    }
}

/// Programs that cannot mutate anything when run without redirection or
/// substitution (both screened globally). Deliberately an allowlist:
/// unknown interpreters (python, node, awk, sed...) deny by default.
const SAFE_PROGRAMS: &[&str] = &[
    "ls", "pwd", "cat", "head", "tail", "wc", "file", "stat", "echo",
    "which", "where", "whoami", "date", "hostname", "uname", "printenv",
    "grep", "rg", "ag", "find", "sort", "uniq", "cut", "column", "diff",
    "tree", "du", "df", "ps", "id", "groups", "tty", "true", "false",
    "git", "gh",
    // PowerShell read-only cmdlets (session-shell fallback).
    "get-childitem", "get-content", "get-location", "get-item", "get-date",
];

const SAFE_GIT: &[&str] = &[
    "status", "diff", "log", "show", "blame", "rev-parse", "describe",
    "shortlog", "ls-files", "ls-remote", "reflog", "grep", "cat-file",
    "symbolic-ref", "name-rev", "worktree", "stash", "tag", "branch",
    "remote", "config",
];

/// Split a command on separators (; && || | & newline) that live outside
/// quotes. Returns None (auto-unsafe) for redirection (`>` `<`), command
/// substitution (`` ` `` `$( )`), or unterminated quotes.
fn split_and_screen(cmd: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = cmd.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Some(q) => {
                cur.push(c);
                if c == q {
                    quote = None;
                }
            }
            None => match c {
                '\'' | '"' => {
                    quote = Some(c);
                    cur.push(c);
                }
                '`' => return None,
                '$' if chars.get(i + 1) == Some(&'(') => return None,
                '>' | '<' => return None,
                ';' | '\n' | '&' | '|' => {
                    if (c == '&' || c == '|') && chars.get(i + 1) == Some(&c) {
                        i += 1;
                    }
                    parts.push(std::mem::take(&mut cur));
                }
                _ => cur.push(c),
            },
        }
        i += 1;
    }
    if quote.is_some() {
        return None;
    }
    parts.push(cur);
    Some(
        parts
            .into_iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    )
}

fn subcommand_safe(part: &str) -> bool {
    let words: Vec<&str> = part.split_whitespace().collect();
    let Some(first) = words.first() else { return false };
    // Strip any path prefix and a .exe suffix (Windows shell variants).
    let prog = first.replace('\\', "/");
    let prog = prog.rsplit('/').next().unwrap_or("");
    let prog = prog.strip_suffix(".exe").unwrap_or(prog).to_ascii_lowercase();
    if prog.contains('=') {
        return false; // env-assignment prefix: not worth parsing
    }
    if !SAFE_PROGRAMS.contains(&prog.as_str()) {
        return false;
    }
    let args = &words[1..];
    match prog.as_str() {
        "find" => !args.iter().any(|a| {
            let l = a.to_ascii_lowercase();
            l.starts_with("-delete") || l.starts_with("-exec") || l.starts_with("-fprint") || l.starts_with("-ok")
        }),
        "git" => git_subcommand_safe(args),
        "gh" => gh_subcommand_safe(args),
        _ => true,
    }
}

fn git_subcommand_safe(args: &[&str]) -> bool {
    // Global flags first, then the subcommand.
    let Some(idx) = args.iter().position(|a| !a.starts_with('-')) else {
        return false;
    };
    let sub = args[idx];
    if !SAFE_GIT.contains(&sub) {
        return false;
    }
    let rest = &args[idx + 1..];
    match sub {
        // `git branch name` / `git tag name` create refs: list-style only.
        "branch" | "tag" => rest.iter().all(|a| a.starts_with('-') || *a == "--list"),
        "remote" => rest.iter().all(|a| a.starts_with('-')),
        "stash" => rest.is_empty() || (rest.len() == 1 && rest[0] == "list"),
        "config" => rest.iter().any(|a| *a == "--get" || *a == "--list" || *a == "-l"),
        "worktree" => rest.first().is_some_and(|a| *a == "list"),
        _ => true,
    }
}

fn gh_subcommand_safe(args: &[&str]) -> bool {
    let Some(idx) = args.iter().position(|a| !a.starts_with('-')) else {
        return false;
    };
    let rest = &args[idx + 1..];
    match args[idx] {
        "pr" | "issue" => rest
            .first()
            .is_some_and(|a| matches!(*a, "list" | "view" | "status" | "diff" | "checks")),
        "repo" | "release" | "label" | "run" => {
            rest.first().is_some_and(|a| matches!(*a, "list" | "view"))
        }
        // `gh api`: only a bare GET path (`gh api /repos/...`); any flag
        // could change the method, so flags deny.
        "api" => !rest.is_empty() && rest.iter().all(|a| a.starts_with('/')),
        "auth" => rest.first().is_some_and(|a| *a == "status"),
        "search" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_guard_allows_readonly_commands() {
        for ok in [
            "ls -la",
            "pwd",
            "cat src/lib.rs",
            "git status",
            "git diff --stat",
            "git log --oneline -5",
            "git show HEAD:README.md",
            "cat a.txt | grep foo | sort",
            "grep -rn TODO src; ls",
            "find . -name '*.rs'",
            "rg 'fn main' .",
            "git branch",
            "git branch -a",
            "gh pr list",
            "gh api /repos/foo/bar",
            "echo hello",
            "wc -l src/lib.rs",
        ] {
            assert!(PermissionEngine::plan_safe_command(ok), "should allow: {ok}");
        }
    }

    #[test]
    fn plan_guard_denies_mutations_and_sneaky_forms() {
        for bad in [
            "git push",
            "rm x",
            "echo hi > file.txt",
            "cat $(which ls)",
            "cat `which ls`",
            "python -c 'x=1'",
            "sed -i s/a/b/ f",
            "awk '{print > \"f\"}'",
            "git branch new-branch",
            "git stash pop",
            "find . -exec rm {} ;",
            "ls; rm -rf /",
            "cargo build",
            "npm install",
            "gh pr create",
            "gh api -X POST /repos",
            "export X=1",
            "ls >> out",
            "cat <<EOF\nhi\nEOF",
            "",
            "   ",
            "unterminated 'quote",
        ] {
            assert!(!PermissionEngine::plan_safe_command(bad), "should deny: {bad}");
        }
    }

    #[test]
    fn plan_mode_allows_safe_bash_and_denies_rest() {
        let engine = PermissionEngine::new(Plan, vec![], vec![], false);
        assert_eq!(engine.check("bash", "git status", false), Decision::Allow);
        assert!(engine.check("bash", "git push", false).is_deny());
        // Deny rules still win over the guard.
        let rule = Rule {
            tool: "bash".to_string(),
            pattern: glob::Pattern::new("git status").unwrap(),
            raw_pattern: "git status".to_string(),
        };
        let engine = PermissionEngine::new(Plan, vec![], vec![rule], false);
        assert!(matches!(engine.check("bash", "git status", false), Decision::Deny(_)));
    }

    impl Decision {
        fn is_deny(&self) -> bool {
            matches!(self, Decision::Deny(_))
        }
    }
}
