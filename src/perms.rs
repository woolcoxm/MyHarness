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
            return self.mode_default(tool, read_only, gated_read_only);
        }
        if Self::matches(&self.deny_rules, &[], tool, arg) {
            return Deny(format!(
                "denied by a deny rule (tool {tool}, pattern matched '{arg}')"
            ));
        }
        if Self::matches(&self.allow_rules, &self.session_allows, tool, arg) {
            return Allow;
        }
        self.mode_default(tool, read_only, gated_read_only)
    }

    fn mode_default(&self, tool: &str, read_only: bool, gated_read_only: bool) -> Decision {
        if read_only && !gated_read_only {
            return Allow;
        }
        match self.mode {
            Yolo => Allow,
            // Read-only-but-gated tools (web_fetch) stay available for
            // research in plan mode; mutations do not.
            Plan if read_only => Allow,
            Plan => Deny(
                "plan mode is active: read-only tools work, mutations do not. \
                 Present your plan and ask the user to switch modes (/mode auto-edit)."
                    .to_string(),
            ),
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

    /// Record an "always allow" answer from an interactive prompt.
    pub fn allow_session(&mut self, tool: &str, pattern: &str) {
        if let Ok(p) = glob::Pattern::new(pattern) {
            self.session_allows.push((tool.to_string(), p));
        }
    }
}
