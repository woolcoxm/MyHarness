//! web_fetch: fetch a URL and return readable text. HTML is stripped to
//! text; JSON and plain text pass through. The model gets to check docs,
//! error messages, and package pages without leaving the harness.

use super::{budget_output, schema_obj, truncate_middle, Tool, ToolCtx, ToolOutput};
use crate::llm::{LlmRequest, Message};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::sync::Arc;

pub struct WebFetchTool;

const MAX_BODY: usize = 2_000_000;
const MAX_TEXT: usize = 20_000;
/// Page text fed to the extraction model when `prompt` is set.
const MAX_EXTRACT_INPUT: usize = 30_000;
const MAX_ANSWER: usize = 8_000;

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &'static str {
        "web_fetch"
    }

    fn description(&self) -> &'static str {
        "Fetches an HTTP(S) URL and returns readable text: HTML is converted to text (scripts/styles dropped), JSON and plain text pass through, everything is capped at ~20k chars (huge pages are head+tail truncated with the full text saved to a file you can read_file). Pass `prompt` to instead get a specific question answered against the page by the fast model — far cheaper on context than reading a long page. Private/localhost destinations are denied (SSRF guard). Redirects are NOT followed: a 3xx returns the Location for you to fetch directly. Use it to check documentation, error messages, and API references."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "url": {"type": "string", "description": "Absolute http(s) URL to fetch"},
                "prompt": {"type": "string", "description": "Optional question to answer against the page content (returns only the answer). Omit to get the raw readable text."}
            }),
            &["url"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Network egress is approval-gated: a fetched URL can carry repository
    /// data away (prompt-injection exfiltration vector), so ask/auto-edit
    /// modes prompt exactly like mutations. Plan mode still allows it —
    /// fetches cannot modify state.
    fn requires_approval(&self) -> bool {
        true
    }

    fn concurrency_safe(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("url")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(80)
            .collect()
    }

    async fn execute(&self, input: Value, ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let url = match super::require_str(&input, "url") {
            Ok(u) => u,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        // Destination guard: credentials always deny; private/local hosts
        // deny unless the config knob allows them (SSRF guard).
        let allow_private = ctx.cfg.web_fetch_private_hosts;
        if let Err(e) = super::net_guard::guard(&url, allow_private).await {
            return ToolOutput::err(format!("web_fetch blocked: {e}"));
        }
        // Redirects are returned, not followed — every hop re-guards.
        let client = match reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("myharness/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(format!("client error: {e}")),
        };
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("request failed: {e}")),
        };
        let status = resp.status();
        if status.is_redirection() {
            let location = resp
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .trim()
                .to_string();
            if location.is_empty() {
                return ToolOutput::err(format!(
                    "HTTP {} with no Location header; cannot follow",
                    status.as_u16()
                ));
            }
            // Pre-check the destination so a bad redirect is named as such.
            if location.contains("://") {
                if let Err(e) = super::net_guard::guard(&location, allow_private).await {
                    return ToolOutput::err(format!(
                        "HTTP {} redirects to {location}, and that destination is blocked: {e}",
                        status.as_u16()
                    ));
                }
            }
            return ToolOutput::ok(format!(
                "HTTP {} — redirects are not followed automatically. Location: {location}\n\
                 Fetch that URL directly if you need it.",
                status.as_u16()
            ));
        }
        if !status.is_success() {
            return ToolOutput::err(format!("HTTP {}", status.as_u16()));
        }
        let content_type = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        let textual = content_type.starts_with("text/")
            || content_type.contains("json")
            || content_type.contains("xml")
            || content_type.contains("javascript");
        if !textual {
            return ToolOutput::err(format!(
                "unsupported content-type '{content_type}' (only text/html/json/xml can be returned)"
            ));
        }
        // Cap the download while streaming.
        #[allow(clippy::while_let_on_iterator)] // stream.next() is StreamExt, not Iterator
        let mut stream = resp.bytes_stream();
        let mut body: Vec<u8> = Vec::with_capacity(16_384);
        let mut truncated = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(b) => {
                    if body.len() + b.len() > MAX_BODY {
                        truncated = true;
                        break;
                    }
                    body.extend_from_slice(&b);
                }
                Err(e) => return ToolOutput::err(format!("download failed: {e}")),
            }
        }
        let raw = String::from_utf8_lossy(&body);
        let text = if content_type.contains("html") {
            html_to_text(&raw)
        } else {
            raw.chars().map(|c| if c == '\r' { ' ' } else { c }).collect()
        };
        // Query-focused mode: answer a prompt against the page instead of
        // returning the raw text (saves the context window on long pages).
        if let Some(prompt) = super::opt_str(&input, "prompt").unwrap_or(None) {
            if !prompt.trim().is_empty() {
                let model = ctx
                    .cfg
                    .model_fast
                    .clone()
                    .unwrap_or_else(|| ctx.cfg.model.clone());
                let req = build_extract_request(&model, &url, text.trim(), &prompt);
                let provider = Arc::clone(&ctx.provider);
                return match crate::agent::compact::collect_text(&provider, &req).await {
                    Ok(ans) if !ans.trim().is_empty() => {
                        let capped = truncate_middle(ans.trim(), MAX_ANSWER, 7_000, 1_000);
                        ToolOutput::ok(format!("Answer for: {prompt}\n\n{capped}\n\n[source: {url}]"))
                    }
                    Ok(_) => ToolOutput::err(
                        "extraction returned an empty answer; retry with prompt omitted to get the raw page text",
                    ),
                    Err(e) => ToolOutput::err(format!(
                        "extraction failed: {e}; retry with prompt omitted to get the raw page text"
                    )),
                };
            }
        }
        let mut out = budget_output(ctx, "web_fetch", text.trim(), MAX_TEXT, 15_000, 3_000);
        if truncated {
            out.push_str("\n... [download capped at 2 MB] ...");
        }
        ToolOutput::ok(out)
    }
}

const EXTRACT_PROMPT: &str = "EXTRACT — answer the question at the end using ONLY the page text provided between the markers. Be precise and brief; quote exact values (versions, flags, names, code) verbatim. If the page does not contain the answer, say so plainly instead of guessing.";

pub(crate) fn build_extract_request(model: &str, url: &str, text: &str, prompt: &str) -> LlmRequest {
    let page = truncate_middle(text, MAX_EXTRACT_INPUT, 25_000, 3_000);
    LlmRequest {
        model: model.to_string(),
        system: EXTRACT_PROMPT.to_string(),
        messages: vec![Message::user_text(format!(
            "URL: {url}\n\n--- PAGE TEXT ---\n{page}\n--- END PAGE TEXT ---\n\nQUESTION: {prompt}"
        ))],
        tools: Vec::new(),
        max_tokens: 1024,
        temperature: 0.1,
    }
}

/// Minimal, dependency-free HTML → text: drop script/style content, strip
/// tags (block tags become newlines), decode common entities, collapse
/// blank runs.
pub(crate) fn html_to_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len() / 2);
    let mut i = 0usize;
    let bytes = html.as_bytes();
    let mut in_skip = None::<&'static str>; // inside <script>/<style>

    while i < bytes.len() {
        if bytes[i] == b'<' {
            // Find tag end.
            let Some(rel) = lower[i..].find('>') else { break };            let tag = &lower[i + 1..i + rel];
            let end = i + rel + 1;
            let name = tag.trim_start_matches('/').split_whitespace().next().unwrap_or("");
            if let Some(open) = in_skip {
                if name == open {
                    in_skip = None;
                }
            } else if name == "script" || name == "style" {
                in_skip = Some(name);
            } else if matches!(name, "p" | "div" | "br" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "pre" | "section" | "article" | "header" | "footer") {
                out.push('\n');
            } else if name == "td" || name == "th" {
                out.push(' ');
            }
            i = end;
            continue;
        }
        if in_skip.is_some() {
            // Skip text content of script/style.
            i += 1;
            continue;
        }
        if bytes[i] == b'&' {
            let rest = &html[i..];
            let decoded = decode_entity(rest);
            match decoded {
                Some((s, len)) => {
                    out.push_str(&s);
                    i += len;
                    continue;
                }
                None => {
                    out.push('&');
                    i += 1;
                    continue;
                }
            }
        }
        // Copy one UTF-8 char.
        let ch_len = utf8_len(bytes[i]);
        let end = (i + ch_len).min(bytes.len());
        out.push_str(&html[i..end]);
        i = end;
    }

    // Collapse: runs of spaces/tabs, 3+ newlines → 2.
    let mut collapsed = String::with_capacity(out.len());
    let mut last_nl = 0;
    for c in out.chars() {
        match c {
            '\n' => {
                last_nl += 1;
                if last_nl <= 2 {
                    collapsed.push('\n');
                }
            }
            ' ' | '\t' | '\r' => {
                if !collapsed.ends_with(' ') && !collapsed.ends_with('\n') && collapsed.chars().last().is_some() {
                    collapsed.push(' ');
                }
            }
            _ => {
                last_nl = 0;
                collapsed.push(c);
            }
        }
    }
    collapsed.trim().to_string()
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 { 1 } else if b >> 5 == 0b110 { 2 } else if b >> 4 == 0b1110 { 3 } else { 4 }
}

/// Decode every HTML entity in a string (best-effort; unknown entities pass
/// through). Shared with web_search's result parsing.
pub(crate) fn decode_entities(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'&' {
            if let Some((decoded, len)) = decode_entity(&s[i..]) {
                out.push_str(&decoded);
                i += len;
                continue;
            }
        }
        let ch_len = utf8_len(bytes[i]);
        let end = (i + ch_len).min(bytes.len());
        out.push_str(&s[i..end]);
        i = end;
    }
    out
}

fn decode_entity(rest: &str) -> Option<(String, usize)> {
    let semi = rest.find(';')?;
    if semi > 10 || semi == 1 {
        return None;
    }
    let ent = &rest[1..semi];
    let decoded = match ent {
        "amp" => "&".to_string(),
        "lt" => "<".to_string(),
        "gt" => ">".to_string(),
        "quot" => "\"".to_string(),
        "apos" | "#39" => "'".to_string(),
        "nbsp" => " ".to_string(),
        _ => {
            let num = ent.strip_prefix('#').and_then(|n| {
                u32::from_str_radix(n.trim_start_matches('x'), if n.starts_with('x') { 16 } else { 10 }).ok()
            })?;
            char::from_u32(num).map(|c| c.to_string()).unwrap_or_default()
        }
    };
    Some((decoded, semi + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_to_text_strips_and_decodes() {
        let html = r#"<html><head><style>body{color:red}</style><title>T</title></head>
<body><h1>Hello &amp; welcome</h1><script>evil()</script>
<p>Line &lt;one&gt;<br>Line&#32;two</p><!-- comment --></body></html>"#;
        let text = html_to_text(html);
        assert!(text.contains("Hello & welcome"), "text: {text}");
        assert!(text.contains("<one>"));
        assert!(!text.contains("evil()"), "script body must be dropped: {text}");
        assert!(!text.contains("color:red"), "style body must be dropped: {text}");
        assert!(text.contains("Line two"), "numeric entity: {text}");
        assert!(!text.contains("comment"), "raw comment text dropped: {text}");
    }

    #[test]
    fn html_to_text_collapses_whitespace() {
        let text = html_to_text("<p>a</p>\n\n\n\n<p>b</p>   <p>c</p>");
        assert_eq!(text, "a\n\nb\n\nc");
    }

    #[test]
    fn extract_request_carries_prompt_and_fast_model() {
        let req = build_extract_request(
            "glm-4.7-air",
            "https://docs.example/x",
            "page body text",
            "what is the default timeout?",
        );
        assert_eq!(req.model, "glm-4.7-air");
        assert!(req.system.starts_with("EXTRACT"));
        assert!(req.tools.is_empty());
        let user = match req.messages.last() {
            Some(crate::llm::Message { content, .. }) => match content.first() {
                Some(crate::llm::ContentBlock::Text { text }) => text.clone(),
                other => panic!("expected text block, got {other:?}"),
            },
            None => panic!("no message"),
        };
        assert!(user.contains("https://docs.example/x"));
        assert!(user.contains("page body text"));
        assert!(user.contains("QUESTION: what is the default timeout?"));
    }
}
