//! web_search: query the web without a key, via DuckDuckGo's HTML endpoint
//! (html.duckduckgo.com/html/). Result titles/urls/snippets are parsed out of
//! the returned markup; the parser is best-effort with a loud, actionable
//! error if the layout ever changes. Egress is approval-gated exactly like
//! web_fetch — a query string can carry repository data away too.

use super::{schema_obj, truncate_middle, Tool, ToolCtx, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};

pub struct WebSearchTool;

const ENDPOINT: &str = "https://html.duckduckgo.com/html/?q=";
const MAX_RESULTS: usize = 8;
const MAX_SNIPPET: usize = 300;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &'static str {
        "web_search"
    }

    fn description(&self) -> &'static str {
        "Searches the web and returns ranked results (title, url, snippet) — use it when you do not yet know which URL to fetch; pair with web_fetch to read a promising result. Up to 8 results, no API key needed."
    }

    fn schema(&self) -> Value {
        schema_obj(
            json!({
                "query": {"type": "string", "description": "Search query (terms, error messages, API names)"},
                "max_results": {"type": "integer", "description": "Maximum results to return (default 8)"}
            }),
            &["query"],
        )
    }

    fn is_read_only(&self) -> bool {
        true
    }

    /// Network egress: same gating rationale as web_fetch.
    fn requires_approval(&self) -> bool {
        true
    }

    fn concurrency_safe(&self) -> bool {
        true
    }

    fn perm_summary(&self, input: &Value) -> String {
        input
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(80)
            .collect()
    }

    async fn execute(&self, input: Value, _ctx: &mut ToolCtx<'_>) -> ToolOutput {
        let query = match super::require_str(&input, "query") {
            Ok(q) => q,
            Err(e) => return ToolOutput::err(e.to_string()),
        };
        let max_results = super::opt_u64(&input, "max_results")
            .unwrap_or(None)
            .unwrap_or(MAX_RESULTS as u64)
            .clamp(1, MAX_RESULTS as u64) as usize;
        let url = format!("{ENDPOINT}{}", percent_encode(&query));
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent(concat!("myharness/", env!("CARGO_PKG_VERSION")))
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolOutput::err(format!("client error: {e}")),
        };
        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return ToolOutput::err(format!("search request failed: {e}")),
        };
        let status = resp.status();
        if !status.is_success() {
            return ToolOutput::err(format!(
                "search endpoint returned HTTP {} — retry once, then use web_fetch on a known URL",
                status.as_u16()
            ));
        }
        let body = match resp.text().await {
            Ok(t) => t,
            Err(e) => return ToolOutput::err(format!("search response unreadable: {e}")),
        };
        let results = parse_ddg_results(&body);
        if results.is_empty() {
            return ToolOutput::err(format!(
                "no results parsed for '{query}' — either the query has no hits or the \
                 DuckDuckGo HTML layout changed; if results should exist, use web_fetch on \
                 https://duckduckgo.com/html/?q={} directly and read the page",
                percent_encode(&query)
            ));
        }
        let mut out = String::new();
        for (i, hit) in results.iter().take(max_results).enumerate() {
            let snippet = truncate_middle(&hit.snippet, MAX_SNIPPET, MAX_SNIPPET, 0);
            out.push_str(&format!(
                "{}. {}\n   {}\n   {}\n",
                i + 1,
                hit.title,
                hit.url,
                snippet.trim()
            ));
        }
        ToolOutput::ok(out.trim_end().to_string())
    }
}

/// One search result: (title, url, snippet).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Parse DuckDuckGo's html endpoint result markup. Anchors look like
/// `<a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=<encoded>&amp;rut=...">Title</a>`
/// with an optional `class="result__snippet"` anchor shortly after.
pub(crate) fn parse_ddg_results(html: &str) -> Vec<SearchHit> {
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut rest = html;
    while let Some(pos) = rest.find("result__a") {
        let anchor = &rest[pos..];
        // Always advance past this occurrence before any continue, so a
        // skipped anchor can never spin the loop.
        rest = &rest[pos + "result__a".len()..];
        let Some(href) = extract_attr(anchor, "href") else { continue };
        // The slice starts inside the tag (at the class attribute): the
        // anchor text begins after the first '>'.
        let Some(tag_end) = anchor.find('>') else { continue };
        let title_zone = &anchor[tag_end + 1..];
        let title_end = title_zone.find("</a>").unwrap_or(title_zone.len().min(600));
        let title = strip_tags(&title_zone[..title_end]);
        let title = title.trim();
        if title.is_empty() {
            continue;
        }
        // DDG hrefs are redirects carrying the real url in uddg=; plain
        // absolute hrefs pass through unchanged.
        let url = match decode_uddg(&href) {
            Some(u) => u,
            None if href.starts_with("http") => href,
            None => continue,
        };
        if !url.starts_with("http") || !seen.insert(url.clone()) {
            continue;
        }
        // Snippet: the nearest result__snippet anchor after this result.
        let snippet = rest
            .find("result__snippet")
            .and_then(|sp| {
                let seg = &rest[sp..];
                let tag_end = seg.find('>')?;
                let body = &seg[tag_end + 1..];
                let end = body.find("</a>").unwrap_or(body.len().min(600));
                Some(strip_tags(&body[..end]))
            })
            .unwrap_or_default();
        hits.push(SearchHit {
            title: collapse_ws(title),
            url,
            snippet: collapse_ws(&snippet),
        });
        if hits.len() >= 30 {
            break;
        }
    }
    hits
}

/// Extract `name="value"` for the first attribute occurrence in `s`.
fn extract_attr(s: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=");
    let start = s.find(&needle)? + needle.len();
    let rest = &s[start..];
    let quote = rest.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let end = rest[1..].find(quote)? + 1;
    Some(crate::tools::web_fetch::decode_entities(&rest[1..end]))
}

/// Minimal tag stripper for one anchor's inner markup.
fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    crate::tools::web_fetch::decode_entities(&out)
}

/// DDG hrefs are redirects: `...?uddg=<percent-encoded real url>&...`.
fn decode_uddg(href: &str) -> Option<String> {
    let pos = href.find("uddg=")?;
    let raw = &href[pos + 5..];
    let raw = raw.split('&').next().unwrap_or(raw);
    Some(percent_decode(raw))
}

pub(crate) fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"
<div class="result results_links results_links_deep web-result">
  <h2 class="result__title">
    <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fstd%2Ffmt%2Fmacro.format.html&amp;rut=abc">format in std::fmt - Rust &amp; more</a>
  </h2>
  <a class="result__snippet" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fdoc.rust-lang.org%2Fstd%2Ffmt%2Fmacro.format.html&amp;rut=abc">The <b>format</b> macro creates a `String` using <code>Display</code> traits&#8230;</a>
</div>
<div class="result">
  <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdup">Duplicate domain hit</a>
</div>
<div class="result">
  <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdup">Duplicate URL dropped</a>
</div>
<div class="result">
  <a rel="nofollow" class="result__a" href="https://example.com/absolute">Absolute href kept</a>
</div>
"#;

    #[test]
    fn parses_titles_urls_and_snippets() {
        let hits = parse_ddg_results(FIXTURE);
        assert_eq!(hits.len(), 3, "{hits:?}");
        assert!(hits[0].title.contains("format in std::fmt"));
        assert!(hits[0].title.contains("Rust & more"), "entity decoded: {}", hits[0].title);
        assert_eq!(hits[0].url, "https://doc.rust-lang.org/std/fmt/macro.format.html");
        assert!(hits[0].snippet.contains("Display"), "tags stripped: {}", hits[0].snippet);
        assert_eq!(hits[2].url, "https://example.com/absolute");
    }

    #[test]
    fn percent_round_trip() {
        assert_eq!(percent_encode("a b&c/d"), "a%20b%26c%2Fd");
        assert_eq!(percent_decode("a%20b%26c%2Fd"), "a b&c/d");
        assert_eq!(percent_decode("plain"), "plain");
    }

    /// Opt-in live check against a saved DDG page: set MYHARNESS_LIVE_DDG to
    /// a path containing `curl -A myharness/<ver> "https://html.duckduckgo.com/html/?q=..."`
    /// output. Skipped (passes) when unset — the suite stays offline.
    #[test]
    #[ignore = "set MYHARNESS_LIVE_DDG=<saved html> to run"]
    fn live_ddg_page_parses() {
        let Ok(path) = std::env::var("MYHARNESS_LIVE_DDG") else { return };
        let html = std::fs::read_to_string(path).expect("readable MYHARNESS_LIVE_DDG file");
        let hits = parse_ddg_results(&html);
        assert!(!hits.is_empty(), "no results parsed from live page — layout changed?");
        for hit in &hits {
            assert!(hit.url.starts_with("http"), "bad url: {}", hit.url);
            assert!(!hit.title.is_empty(), "empty title for {}", hit.url);
        }
    }
}
