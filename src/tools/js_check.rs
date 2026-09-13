//! Script verification gate: written JavaScript must parse AND execute
//! without runtime errors. Three layers:
//!
//! 1. `node --check` — syntax validation (catches parse errors)
//! 2. `node -e` runtime execution in a VM with mocked browser globals —
//!    catches undefined variables, type errors, null references, and
//!    broken function calls that syntax checking misses
//! 3. Lexical fallback — catches numeric-identifier collisions and
//!    unbalanced brackets when node is absent
//!
//! Best-effort: a missing node never fails a turn, but when node IS
//! available, runtime errors are reported to the model and the reflect
//! loop makes fixing them a precondition for ending the turn.

use std::path::Path;

/// Check one written file. Returns a problem report, or None when clean.
pub fn check_file(path: &Path) -> Option<String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let source = std::fs::read_to_string(path).ok()?;
    let blocks: Vec<String> = match ext.as_str() {
        "js" | "mjs" | "ts" => vec![source],
        "html" | "htm" => extract_inline_scripts(&source),
        _ => return None,
    };
    if blocks.is_empty() {
        return None;
    }
    let combined = blocks.join("\n;\n");

    // Layer 1: syntax (bounded: pathological blobs go to lexical)
    if combined.len() <= 1_000_000 {
        if let Some(err) = node_check(&combined).flatten() {
            return Some(format!("{}: syntax error — {err}", short(path)));
        }
        // Layer 2: runtime smoke test (execute in VM with mock browser globals)
        if let Some(err) = runtime_check(&combined) {
            return Some(format!("{}: runtime error — {err}", short(path)));
        }
    }

    // Layer 3: lexical fallback.
    lexical_check(&combined).map(|err| format!("{}: {err}", short(path)))
}

fn short(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| p.display().to_string())
}

/// Inline JS blocks of an HTML document; skips import maps and external
/// (src=) scripts — the import map is JSON, not JS.
pub fn extract_inline_scripts(html: &str) -> Vec<String> {
    let bytes = html.as_bytes();
    let lower = html.to_ascii_lowercase();
    let mut out = Vec::new();
    let mut i = 0usize;
    while let Some(start) = lower[i..].find("<script") {
        let start = i + start;
        let Some(tag_end) = lower[start..].find('>') else { break };
        let tag_end = start + tag_end;
        let attrs = &lower[start..tag_end];
        if attrs.contains("importmap") || attrs.contains("src=") {
            i = tag_end + 1;
            continue;
        }
        let body_start = tag_end + 1;
        let Some(close) = lower[body_start..].find("</script") else { break };
        out.push(html[body_start..body_start + close].to_string());
        i = body_start + close;
        let _ = bytes;
    }
    out
}

/// `node --check` on a temp module copy. None = node unavailable; Some(None)
/// = clean; Some(Some(err)) = parse failure (first error line, mangled to
/// the harness error style).
fn node_check(code: &str) -> Option<Option<String>> {
    // Unique name: parallel checks (tests, concurrent writes) must not
    // overwrite each other's script.
    let tmp = std::env::temp_dir().join(format!(
        "mh-jscheck-{}.mjs",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    if std::fs::write(&tmp, code).is_err() {
        return None;
    }
    let out = std::process::Command::new("node")
        .arg("--check")
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let Ok(out) = out else {
        return None; // node not installed — fall back
    };
    if out.status.success() {
        return Some(None);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr
        .lines()
        .find(|l| l.contains("SyntaxError") || l.contains("Error:"))
        .unwrap_or("syntax error");
    Some(Some(first.to_string()))
}

/// Lexical pass: catches numeric-literal/identifier collisions and
/// unbalanced brackets, outside strings/comments/template literals.
pub fn lexical_check(code: &str) -> Option<String> {
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0usize;
    let mut stack: Vec<(char, usize)> = Vec::new();

    while i < chars.len() {
        let c = chars[i];
        // Comments.
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        }
        // Strings (skipped; escapes respected).
        if c == '"' || c == '\'' {
            let quote = c;
            i += 1;
            while i < chars.len() && chars[i] != quote {
                if chars[i] == '\\' {
                    i += 1;
                }
                if chars[i] == '\n' {
                    break; // unterminated string: the parser will say so
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        // Template literals: skip to the closing backtick, but honor
        // ${ ... } nesting so braces inside are still bracket-checked.
        if c == '`' {
            i += 1;
            while i < chars.len() && chars[i] != '`' {
                if chars[i] == '\\' {
                    i += 2;
                    continue;
                }
                if chars[i] == '$' && chars.get(i + 1) == Some(&'{') {
                    stack.push(('{', i));
                    i += 2;
                    // Resume normal scanning inside the interpolation; the
                    // pushed '{' closes at the matching '}'.
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Bracket balance.
        if c == '(' || c == '[' || c == '{' {
            stack.push((c, i));
        } else if c == ')' || c == ']' || c == '}' {
            match stack.pop() {
                Some((open, _)) if matches!(open, '(') && c == ')'
                    || matches!(open, '[') && c == ']'
                    || matches!(open, '{') && c == '}' => {}
                _ => {
                    return Some(format!(
                        "unbalanced '{c}' near line {}",
                        line_of(&chars, i)
                    ));
                }
            }
        }
        // Numeric literal followed by an identifier start: the browser's
        // "identifier starts immediately after numeric literal".
        if c.is_ascii_digit() {
            let (end, ok) = scan_number(&chars, i);
            if let Some(next) = chars.get(end) {
                if next.is_ascii_alphabetic() || *next == '_' || *next == '$' {
                    let snippet: String = chars[i..(end + 6).min(chars.len())].iter().collect();
                    return Some(format!(
                        "identifier starts immediately after numeric literal (`{snippet}...`) near line {}",
                        line_of(&chars, i)
                    ));
                }
            }
            if !ok {
                // Not even a well-formed number per our grammar — let node
                // be the authority when present; here just move on.
            }
            i = end;
            continue;
        }
        i += 1;
    }
    if let Some((open, at)) = stack.last() {
        return Some(format!(
            "unbalanced '{open}' opened near line {} and never closed",
            line_of(&chars, *at)
        ));
    }
    None
}

/// Runtime smoke test: executes the code in a Node VM with mocked browser
/// globals. Catches undefined variables, null references, type errors, and
/// broken function calls — the bugs that pass `node --check` but break at
/// runtime (the exact failure class from the Jungle Fight 68 autonomous
/// build where WASD didn't work but syntax was fine).
fn runtime_check(code: &str) -> Option<String> {
    if code.len() > 500_000 {
        return None; // too big to execute safely
    }

    // Build a test harness that provides browser globals, then runs the
    // code. Anything that throws a TypeError, ReferenceError, or similar
    // is a runtime bug.
    let harness = format!(
        r#"{{}}
const __mockElement = () => new Proxy({{}}, {{
  get: (target, prop) => {{
    if (prop === 'style') return new Proxy({{}}, {{ get: () => '', set: () => true }});
    if (prop === 'classList') return {{ add: ()=>{{}}, remove: ()=>{{}}, toggle: ()=>{{}}, contains: ()=>false }};
    if (prop === 'addEventListener') return () => {{}};
    if (prop === 'appendChild') return (c) => c;
    if (prop === 'getBoundingClientRect') return () => ({{ top:0, left:0, width:800, height:600 }});
    if (prop === 'querySelector' || prop === 'querySelectorAll') return () => [];
    if (prop === 'getContext') return () => new Proxy({{}}, {{ get: (t,p) => {{
      if (p === 'createLinearGradient' || p === 'createRadialGradient') return () => ({{ addColorStop: ()=>{{}} }});
      if (p === 'measureText') return () => ({{ width: 10 }});
      return typeof p === 'string' ? (()=>{{}}) : undefined;
    }}}});
    if (typeof prop === 'string' && prop.startsWith('on')) return null;
    return typeof prop === 'symbol' ? undefined : (()=>{{}})(null);
  }},
  set: () => true,
}});
const document = {{
  getElementById: () => __mockElement(),
  createElement: () => __mockElement(),
  querySelector: () => __mockElement(),
  querySelectorAll: () => [],
  body: __mockElement(),
  documentElement: __mockElement(),
  addEventListener: () => {{}},
  createEvent: () => ({{ initEvent: ()=>{{}} }}),
}};
const window = new Proxy({{ IS_TOUCH: false, innerWidth: 800, innerHeight: 600, devicePixelRatio: 1, __loadErr: null }}, {{
  get: (target, prop) => {{
    if (prop in target) return target[prop];
    if (prop === 'addEventListener' || prop === 'removeEventListener') return () => {{}};
    if (prop === 'requestAnimationFrame') return (fn) => setTimeout(fn, 16);
    if (prop === 'localStorage') return {{ getItem: ()=>null, setItem: ()=>{{}}, removeItem: ()=>{{}} }};
    if (prop === 'location') return {{ search: '', hash: '', pathname: '/' }};
    if (prop === 'matchMedia') return () => ({{ matches: false, addEventListener: ()=>{{}} }});
    if (prop === 'speechSynthesis') return {{ getVoices: ()=>[], speak: ()=>{{}}, cancel: ()=>{{}} }};
    if (prop === 'AudioContext' || prop === 'webkitAudioContext') return class {{ 
      createGain() {{ return {{ connect: ()=>{{}}, gain: {{ value: 1, linearRampToValueAtTime: ()=>{{}}, exponentialRampToValueAtTime: ()=>{{}}, setValueAtTime: ()=>{{}} }} }}; }}
      createOscillator() {{ return {{ connect: ()=>{{}}, start: ()=>{{}}, stop: ()=>{{}}, frequency: {{ value: 440, exponentialRampToValueAtTime: ()=>{{}}, setValueAtTime: ()=>{{}} }} }}; }}
      createBuffer() {{ return {{ getChannelData: () => new Float32Array(1024) }}; }}
      createBufferSource() {{ return {{ connect: ()=>{{}}, start: ()=>{{}}, stop: ()=>{{}}, buffer: null }}; }}
      createBiquadFilter() {{ return {{ connect: ()=>{{}}, frequency: {{ value: 1000, exponentialRampToValueAtTime: ()=>{{}}, setValueAtTime: ()=>{{}} }}, Q: {{value:1}}, type: '' }}; }}
      createDynamicsCompressor() {{ return {{ connect: ()=>{{}}, threshold: {{value:-18}}, ratio: {{value:6}} }}; }}
      createStereoPanner() {{ return {{ connect: ()=>{{}}, pan: {{value:0}} }}; }}
      createWaveShaper() {{ return {{ connect: ()=>{{}} }}; }}
      get destination() {{ return {{}}; }}
      get currentTime() {{ return 0; }}
      get sampleRate() {{ return 44100; }}
      resume() {{ return Promise.resolve(); }}
    }};
    if (prop === 'navigator') return {{ maxTouchPoints: 0, userAgent: 'node' }};
    if (prop === 'performance') return {{ now: () => Date.now() }};
    return undefined;
  }},
}});
const self = window;
const navigator = window.navigator;
const localStorage = window.localStorage;
const performance = window.performance;
const requestAnimationFrame = window.requestAnimationFrame;
const THREE = new Proxy({{ REVISION: '128' }}, {{
  get: (target, prop) => {{
    if (prop in target) return target[prop];
    // Return a mock class for any THREE.* access
    return class {{
      constructor(...args) {{ this.args = args; this.children = []; this.position = {{ x:0, y:0, z:0, set:()=>{{}} }}; this.rotation = {{ x:0, y:0, z:0, set:()=>{{}} }}; this.scale = {{ x:1, y:1, z:1, set:()=>{{}} }}; }}
      add(...c) {{ this.children.push(...c); return this; }}
      remove(...c) {{ return this; }}
      traverse(fn) {{ fn(this); }}
      getComponent() {{ return 0; }}
      setScalar(v) {{ return this; }}
      clone() {{ return this; }}
      copy() {{ return this; }}
      length() {{ return 0; }}
      normalize() {{ return this; }}
      applyMatrix4() {{ return this; }}
      set() {{ return this; }}
    }};
  }},
}});
try {{
  (function() {{
{code}
  }})();
}} catch (e) {{
  if (e instanceof TypeError || e instanceof ReferenceError || e instanceof RangeError) {{
    console.error(e.message);
    process.exit(1);
  }}
  // Game logic errors (custom thrown) are ok — the code at least runs
}}
console.error('__RUNTIME_OK__');
process.exit(0);
"#
    );

    let tmp = std::env::temp_dir().join(format!(
        "mh-runtime-{}.js",
        &uuid::Uuid::new_v4().simple().to_string()[..12]
    ));
    if std::fs::write(&tmp, &harness).is_err() {
        return None;
    }
    let out = std::process::Command::new("node")
        .arg(&tmp)
        .output();
    let _ = std::fs::remove_file(&tmp);
    let Ok(out) = out else { return None };

    let stderr = String::from_utf8_lossy(&out.stderr);
    if stderr.contains("__RUNTIME_OK__") {
        return None; // clean
    }
    if stderr.contains("process.exit(1)") || stderr.contains("TypeError")
        || stderr.contains("ReferenceError") || stderr.contains("RangeError")
        || out.status.code() == Some(1)
    {
        // Extract the actual error message
        let err_line = stderr
            .lines()
            .find(|l| {
                l.contains("Error") || l.contains("is not defined")
                    || l.contains("is not a function") || l.contains("Cannot read")
                    || l.contains("null") || l.contains("undefined")
            })
            .unwrap_or("runtime error (code exited with status 1)");
        return Some(err_line.trim().to_string());
    }
    None
}

/// Scan a numeric literal per JS grammar; returns (end_index, well_formed).
fn scan_number(chars: &[char], start: usize) -> (usize, bool) {
    let is_digit = |c: char| c.is_ascii_digit() || c == '_';
    // Radix prefixes.
    if chars[start] == '0' && start + 1 < chars.len() {
        let radix_char = chars[start + 1].to_ascii_lowercase();
        if matches!(radix_char, 'x' | 'b' | 'o') {
            let mut j = start + 2;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            return (j, true);
        }
    }
    // Decimal / float / exponent / BigInt.
    let mut j = start;
    while j < chars.len() && is_digit(chars[j]) {
        j += 1;
    }
    if j < chars.len() && chars[j] == '.' {
        j += 1;
        while j < chars.len() && is_digit(chars[j]) {
            j += 1;
        }
    }
    if j < chars.len() && (chars[j] == 'e' || chars[j] == 'E') {
        let mut k = j + 1;
        if k < chars.len() && (chars[k] == '+' || chars[k] == '-') {
            k += 1;
        }
        if k < chars.len() && chars[k].is_ascii_digit() {
            j = k;
            while j < chars.len() && is_digit(chars[j]) {
                j += 1;
            }
        }
        // else: 'e' not part of the number (an identifier may follow a
        // plain integer only as an error — handled by the caller).
    }
    if j < chars.len() && (chars[j] == 'n') && !chars.get(j + 1).is_some_and(|c| c.is_ascii_alphanumeric() || *c == '_') {
        j += 1; // BigInt suffix
    }
    (j, true)
}

fn line_of(chars: &[char], at: usize) -> usize {
    chars[..at.min(chars.len())].iter().filter(|c| **c == '\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_inline_scripts_but_not_importmap_or_src() {
        let html = r#"<html><head>
<script type="importmap">{"imports": {"three": "x"}}</script>
<script src="cdn.js"></script>
</head><body>
<script>const a = 1;</script>
<script type="module">const b = 2;</script>
</body></html>"#;
        let blocks = extract_inline_scripts(html);
        assert_eq!(blocks.len(), 2, "{blocks:?}");
        assert!(blocks[0].contains("const a"));
        assert!(blocks[1].contains("const b"));
    }

    #[test]
    fn lexical_catches_numeric_identifier_collisions() {
        assert!(lexical_check("const x = 1px;").is_some());
        assert!(lexical_check("ctx.translate(2d);").is_some());
        assert!(lexical_check("let v = 12abc;").is_some());
    }

    #[test]
    fn lexical_accepts_valid_numeric_forms() {
        for ok in [
            "const a = 0xff;",
            "const b = 1e5;",
            "const c = 1_000;",
            "const d = 10n;",
            "const e = 3.14;",
            "const f = .5 + 1e-3;",
            "const g = 0b1010 + 0o777;",
        ] {
            assert!(lexical_check(ok).is_none(), "false positive on: {ok}");
        }
    }

    #[test]
    fn lexical_is_string_and_comment_aware() {
        // Numbers in strings and comments must not trip the check; braces
        // in strings must not count.
        let ok = r#"
            const s = "1px and } unbalanced { text";
            // 2d comment 3d
            /* 5zz */
            const t = `template ${1 + 2} numbers`;
            const obj = { a: 1 };
        "#;
        assert!(lexical_check(ok).is_none(), "{:?}", lexical_check(ok));
        // Real imbalance IS caught.
        assert!(lexical_check("function f( {").is_some());
        assert!(lexical_check("const o = { a: 1;").is_some());
    }

    #[test]
    fn node_check_catches_a_real_syntax_error() {
        // Skips gracefully when node is absent; on this repo's dev/CI
        // machines node exists and must flag this.
        let verdict = node_check("const x = ;;");
        if let Some(v) = verdict {
            assert!(v.is_some(), "node should reject `const x = ;`");
        }
        let clean = node_check("const x = 1 + 2;");
        if let Some(v) = clean {
            assert!(v.is_none());
        }
    }

    #[test]
    fn check_file_end_to_end_on_html() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("game.html");
        std::fs::write(
            &p,
            "<html><body><script>const w = 12; const h = 0x10; const bad = 1px;</script></body></html>",
        )
        .unwrap();
        let report = check_file(&p);
        assert!(report.is_some(), "{report:?}");
        let r = report.unwrap();
        assert!(r.contains("numeric") || r.to_lowercase().contains("syntax"), "{r}");
    }
}
