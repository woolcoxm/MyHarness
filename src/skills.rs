//! Skills: reusable instruction packs (SKILL.md files) discovered from the
//! same locations ZCode-class harnesses read — the user's `~/.agents/skills`
//! and the workspace's `.agents/skills`. A skill is a directory containing a
//! `SKILL.md` with `---`-delimited frontmatter (`name`, `description`) and a
//! markdown body of instructions.
//!
//! The system prompt carries only the name + description list (cheap, always
//! visible); the `skill` tool loads the full body on demand so a skill costs
//! context only when it is actually used.

use std::path::{Path, PathBuf};

/// How many skills the system-prompt list may carry.
const MAX_LISTED: usize = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub body: String,
    pub path: PathBuf,
}

/// User-level skills directory (`MYHARNESS_SKILLS_DIR` overrides the default
/// `~/.agents/skills`, which ZCode-class harnesses share).
pub fn user_skills_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("MYHARNESS_SKILLS_DIR") {
        if !d.trim().is_empty() {
            return Some(PathBuf::from(d));
        }
    }
    dirs::home_dir().map(|h| h.join(".agents").join("skills"))
}

/// Discover skills for a session: user directory first, then the nearest
/// `.agents/skills` from `root` upward (mirroring the AGENTS.md lookup).
/// Workspace skills override user skills with the same name.
pub fn discover(root: &Path) -> Vec<Skill> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(user) = user_skills_dir() {
        dirs.push(user);
    }
    let mut dir = Some(root.to_path_buf());
    while let Some(d) = dir {
        let candidate = d.join(".agents").join("skills");
        if candidate.is_dir() {
            dirs.push(candidate);
            break;
        }
        dir = d.parent().map(PathBuf::from);
    }
    discover_from(&dirs)
}

/// Load skills from explicit directories (user first, workspace later wins).
pub fn discover_from(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    for d in dirs {
        let Ok(entries) = std::fs::read_dir(d) else { continue };
        let mut subdirs: Vec<_> = entries.flatten().collect();
        subdirs.sort_by_key(|e| e.file_name());
        for e in subdirs {
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let path = e.path().join("SKILL.md");
            let Ok(raw) = std::fs::read_to_string(&path) else { continue };
            let dir_name = e.file_name().to_string_lossy().to_string();
            let skill = parse_skill(&dir_name, &raw, path);
            match out.iter().position(|s| s.name == skill.name) {
                Some(i) => out[i] = skill,
                None => out.push(skill),
            }
        }
    }
    out
}

/// Parse one SKILL.md. Frontmatter is `---` on the first line, `name:` /
/// `description:` key-value lines, closed by a second `---`; everything
/// after is the instruction body. Missing frontmatter degrades to
/// name = directory name, empty description, whole file as body.
pub fn parse_skill(dir_name: &str, raw: &str, path: PathBuf) -> Skill {
    let mut name = dir_name.to_string();
    let mut description = String::new();
    let mut body = raw.trim().to_string();

    let trimmed = raw.trim_start();
    if let Some(after_open) = trimmed.strip_prefix("---") {
        let after_open = after_open.strip_prefix('\r').unwrap_or(after_open);
        let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
        if let Some(close) = after_open.find("\n---") {
            for line in after_open[..close].lines() {
                if let Some(v) = line.strip_prefix("name:") {
                    if !v.trim().is_empty() {
                        name = v.trim().to_string();
                    }
                } else if let Some(v) = line.strip_prefix("description:") {
                    description = v.trim().to_string();
                }
            }
            let after_close = &after_open[close + 4..];
            let after_close = after_close.strip_prefix('\r').unwrap_or(after_close);
            let skip_fence_line = after_close.find('\n').unwrap_or(after_close.len());
            body = after_close[skip_fence_line..].trim().to_string();
        }
    }
    Skill { name, description, body, path }
}

/// Per-skill description display cap in the prompt list.
const MAX_DESC_CHARS: usize = 120;
/// Total char budget for the list body (a hundred installed skills must
/// never eat the system prompt).
const MAX_SECTION_CHARS: usize = 4_000;

/// The always-visible system-prompt section, budgeted: full name +
/// description lines while they fit, name-only lines when the descriptions
/// push past the budget, and a "+N more" footer past that. Workspace skills
/// override user skills in place, so the winner of a collision always keeps
/// its slot.
pub fn render_list(skills: &[Skill]) -> String {
    let mut s = String::from(
        "# Available skills\n\
         Reusable instruction packs. When a request matches a description below, call the\n\
         `skill` tool with that name BEFORE doing the work and follow the returned\n\
         instructions. The user can also invoke one directly with /<name>.\n\n",
    );
    let listed: Vec<&Skill> = skills.iter().take(MAX_LISTED).collect();
    let mut used = 0usize;
    let mut names_only = false;
    let mut shown = 0usize;
    while shown < listed.len() {
        let sk = listed[shown];
        let line = if names_only {
            format!("- {}\n", sk.name)
        } else {
            let mut desc: String = sk.description.chars().take(MAX_DESC_CHARS).collect();
            if sk.description.chars().count() > MAX_DESC_CHARS {
                desc.push_str("...");
            }
            if desc.trim().is_empty() {
                desc = "(no description)".to_string();
            }
            format!("- {}: {}\n", sk.name, desc)
        };
        if used + line.chars().count() > MAX_SECTION_CHARS {
            if names_only {
                break; // over budget even name-only: stop and footer
            }
            names_only = true; // re-render this skill as name-only
            continue;
        }
        s.push_str(&line);
        used += line.chars().count();
        shown += 1;
    }
    if skills.len() > shown {
        s.push_str(&format!(
            "... and {} more (the skill tool can load them by name)\n",
            skills.len() - shown
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_skill(dir: &Path, name: &str, frontmatter: &str, body: &str) {
        let d = dir.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("SKILL.md"), format!("---\n{frontmatter}---\n{body}")).unwrap();
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let raw = "---\nname: override\ndescription: does a thing\n---\n\n# Steps\n1. do it\n";
        let sk = parse_skill("dirname", raw, PathBuf::from("/x/SKILL.md"));
        assert_eq!(sk.name, "override");
        assert_eq!(sk.description, "does a thing");
        assert_eq!(sk.body, "# Steps\n1. do it");
    }

    #[test]
    fn missing_frontmatter_degrades_to_dir_name() {
        let sk = parse_skill("plain", "just instructions\n", PathBuf::from("/x/SKILL.md"));
        assert_eq!(sk.name, "plain");
        assert_eq!(sk.description, "");
        assert_eq!(sk.body, "just instructions");
    }

    #[test]
    fn discover_workspace_overrides_user_by_name() {
        let user = tempfile::tempdir().unwrap();
        let work = tempfile::tempdir().unwrap();
        write_skill(user.path(), "alpha", "name: alpha\ndescription: user copy\n", "user body");
        write_skill(user.path(), "beta", "description: only in user dir\n", "beta body");
        write_skill(work.path(), "alpha", "name: alpha\ndescription: workspace copy\n", "work body");

        let skills = discover_from(&[user.path().to_path_buf(), work.path().to_path_buf()]);
        assert_eq!(skills.len(), 2, "{skills:?}");
        let alpha = skills.iter().find(|s| s.name == "alpha").unwrap();
        assert_eq!(alpha.body, "work body");
        assert!(skills.iter().any(|s| s.name == "beta"));
    }

    #[test]
    fn render_list_includes_names_and_marker() {
        let skills = vec![
            Skill { name: "a".into(), description: "desc a".into(), body: String::new(), path: PathBuf::new() },
            Skill { name: "b".into(), description: String::new(), body: String::new(), path: PathBuf::new() },
        ];
        let list = render_list(&skills);
        assert!(list.contains("- a: desc a"));
        assert!(list.contains("- b: (no description)"));
        assert!(list.contains("skill"));
    }

    #[test]
    fn render_list_degrades_to_names_over_budget() {
        let skills: Vec<Skill> = (0..30)
            .map(|i| Skill {
                name: format!("skill-{i}"),
                description: "d".repeat(300),
                body: String::new(),
                path: PathBuf::new(),
            })
            .collect();
        let list = render_list(&skills);
        // Some full lines, then name-only lines once descriptions blow the
        // budget — the list must never exceed its section budget by much.
        assert!(list.contains("- skill-0: "));
        assert!(list.contains("\n- skill-"), "name-only lines expected: {list}");
        let body = list.split("\n\n").nth(1).unwrap_or("");
        assert!(body.chars().count() < MAX_SECTION_CHARS + 200, "body {} chars", body.chars().count());
    }

    #[test]
    fn render_list_footers_when_even_names_do_not_fit() {
        // Names long enough that even name-only lines blow the budget.
        let skills: Vec<Skill> = (0..30)
            .map(|i| Skill {
                name: format!("very-long-skill-name-number-{i}-").to_string() + &"p".repeat(200),
                description: String::new(),
                body: String::new(),
                path: PathBuf::new(),
            })
            .collect();
        let list = render_list(&skills);
        assert!(list.contains("more (the skill tool can load them by name)"), "{list}");
    }

    #[test]
    fn render_list_caps_long_descriptions() {
        let skills = vec![Skill {
            name: "a".into(),
            description: "x".repeat(500),
            body: String::new(),
            path: PathBuf::new(),
        }];
        let list = render_list(&skills);
        assert!(list.contains("..."), "capped description needs an ellipsis: {list}");
        let line = list.lines().find(|l| l.starts_with("- a:")).unwrap();
        assert!(line.chars().count() < 150, "line not capped: {line}");
    }
}
