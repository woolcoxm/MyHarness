//! Zero-token long-term memory (a Rust port of woolcoxm/zero-mem-pi, itself
//! a reimplementation of "Zero-Mem: Zero-Token Memory Operations for LLM
//! Agents", arXiv:2607.29377).
//!
//! The defining property: **memory operations never call the LLM.** Capture
//! is passive (every turn's user prompt + final answer becomes a trace
//! unit), retrieval is deterministic math — BM25 lexical view + entity-
//! context graph scored by Personalized PageRank, query-conditioned
//! routing, min-max fusion, evidence closure, confidence gating — and the
//! top snippets ride the message stream at turn start. The port carries
//! over every hard-won lesson from the pi implementation's live bugs:
//! identity facts live in a derived sticky slot (never in the ranking),
//! weak pools inject nothing ("no memory beats confusing memory"),
//! persistence is atomic with cross-process merge, snippets are sanitized
//! (stored text is untrusted), and the current session's own units are
//! excluded from retrieval (they're already in context).
//!
//! Deliberately lexical-only: no embedding model ships in the binary. The
//! pi evals showed BM25 is competitive with small dense encoders on real
//! data; a dense view is the documented upgrade path.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// ── configuration ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ZeroMemCfg {
    pub enabled: bool,
    /// Snippets injected per turn (pi default: 3).
    pub top_k: usize,
    /// Snippet length in chars (pi default: 120).
    pub snippet_chars: usize,
    /// Minimum fused score to inject.
    pub min_score: f64,
    /// Retention bound on stored units.
    pub max_units: usize,
    /// Retention bound on unit age.
    pub max_age_days: u64,
    /// Fusion weight ρ for the primary view (paper default 0.6).
    pub rho: f64,
}

impl Default for ZeroMemCfg {
    fn default() -> Self {
        ZeroMemCfg {
            enabled: true,
            top_k: 3,
            snippet_chars: 120,
            min_score: 0.05,
            max_units: 5000,
            max_age_days: 180,
            rho: 0.6,
        }
    }
}

// ── store ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Unit {
    pub id: String,
    /// "user" | "assistant"
    pub role: String,
    pub text: String,
    pub session: String,
    pub ts: u64,
    #[serde(default)]
    pub fingerprint: String,
    #[serde(default)]
    pub entities: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    pub user_name: Option<String>,
    pub agent_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    #[serde(default)]
    identity: Identity,
    units: Vec<Unit>,
}

pub struct ZeroMem {
    path: PathBuf,
    session: String,
    units: Vec<Unit>,
    identity: Identity,
    /// mtime (unix ms) of the file as loaded — the cross-process merge
    /// check (two myharness processes share one store; last-writer-wins
    /// would erase the other's captures).
    loaded_mtime: u64,
    /// A failed load refuses to persist (never overwrite a broken store).
    load_ok: bool,
    cfg: ZeroMemCfg,
}

const TEXT_CAP: usize = 4_000;

impl ZeroMem {
    /// Open (or create) the per-project store. Best-effort: a broken store
    /// disables persistence but keeps in-memory operation.
    pub fn open(data_dir: &Path, workspace_root: &Path, session: &str, cfg: ZeroMemCfg) -> Self {
        let slug = project_slug(workspace_root);
        let dir = data_dir.join("zero-mem");
        let path = dir.join(format!("{slug}.json"));
        let mut zm = ZeroMem {
            path,
            session: session.to_string(),
            units: Vec::new(),
            identity: Identity::default(),
            loaded_mtime: 0,
            load_ok: true,
            cfg,
        };
        match std::fs::metadata(&zm.path)
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            Some(d) => zm.loaded_mtime = d.as_millis() as u64,
            None => zm.loaded_mtime = 0,
        }
        if let Ok(raw) = std::fs::read_to_string(&zm.path) {
            match serde_json::from_str::<StoreFile>(&raw) {
                Ok(f) if f.version == 1 => {
                    zm.units = f.units;
                    zm.identity = f.identity;
                }
                _ => zm.load_ok = false,
            }
        } else if zm.path.exists() {
            zm.load_ok = false; // exists but unreadable
        }
        zm.enforce_retention();
        zm
    }

    pub fn is_enabled(&self) -> bool {
        self.cfg.enabled
    }

    /// Passive capture: one unit per (role, text). Deduped by fingerprint
    /// (the same message re-captured after resume is a no-op).
    pub fn capture(&mut self, role: &str, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let text: String = text.chars().take(TEXT_CAP).collect();
        let fp = fingerprint(&text);
        if self.units.iter().any(|u| u.fingerprint == fp) {
            return;
        }
        let id = format!("u{}", self.units.len() + 1);
        let entities = extract_entities(&text);
        self.units.push(Unit {
            id,
            role: role.to_string(),
            text,
            session: self.session.clone(),
            ts: now_secs(),
            fingerprint: fp,
            entities,
        });
        self.enforce_retention();
    }

    fn enforce_retention(&mut self) {
        let cutoff = now_secs().saturating_sub(self.cfg.max_age_days * 86_400);
        let max = self.cfg.max_units;
        self.units.retain(|u| u.ts >= cutoff);
        if self.units.len() > max {
            let drop = self.units.len() - max;
            self.units.drain(..drop);
        }
        // Identity is a derived sticky slot — retention evicting the naming
        // units must never cause amnesia (pi v0.14h lesson).
    }

    /// Atomic persist with cross-process merge: if another process wrote
    /// since our load, merge its units by id before writing (temp+rename,
    /// so a crash can't tear the store).
    pub fn persist(&mut self) {
        if !self.load_ok {
            return;
        }
        // Cross-process merge check.
        if let Ok(raw) = std::fs::read_to_string(&self.path) {
            if let Ok(f) = serde_json::from_str::<StoreFile>(&raw) {
                if f.version == 1 {
                    let known: HashSet<&str> = self.units.iter().map(|u| u.id.as_str()).collect();
                    let mut foreign: Vec<Unit> = f
                        .units
                        .into_iter()
                        .filter(|u| !known.contains(u.id.as_str()))
                        .collect();
                    self.units.append(&mut foreign);
                    self.enforce_retention();
                }
            }
        }
        let file = StoreFile { version: 1, identity: self.identity.clone(), units: self.units.clone() };
        let Ok(json) = serde_json::to_string(&file) else { return };
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = self.path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() {
            let _ = std::fs::rename(&tmp, &self.path);
        }
    }

    /// Refresh derived identity from naming statements (newest confident
    /// naming wins; default-model names are poison).
    pub fn derive_identity(&mut self) {
        for u in self.units.clone() {
            if let Some(name) = naming_name(&u) {
                match name {
                    NamingName::User(n) => self.identity.user_name = Some(n),
                    NamingName::Agent(n) => self.identity.agent_name = Some(n),
                }
            }
        }
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// `/memory`: one-line status.
    pub fn status(&self) -> String {
        format!(
            "zero-mem: {} unit(s), identity: user={}, agent={} | store {} | /memory search <q> | /memory clear",
            self.units.len(),
            self.identity.user_name.as_deref().unwrap_or("?"),
            self.identity.agent_name.as_deref().unwrap_or("?"),
            self.path.display()
        )
    }

    /// `/memory clear`: wipe units + identity, persist the empty store.
    pub fn clear(&mut self) {
        self.units.clear();
        self.identity = Identity::default();
        self.persist();
    }

    /// The retrieval pipeline (zero LLM calls). `query` is the new user
    /// prompt; `active_fingerprints` prevents re-injecting text already in
    /// the model's window; the current session's units are excluded (they
    /// ARE the window).
    pub fn retrieve(&self, query: &str, active_fingerprints: &HashSet<String>) -> Vec<Hit> {
        if self.units.len() < 2 {
            return Vec::new();
        }
        let corpus: Vec<&Unit> = self
            .units
            .iter()
            .filter(|u| u.session != self.session && !active_fingerprints.contains(&u.fingerprint))
            .collect();
        if corpus.is_empty() {
            return Vec::new();
        }

        // ── view 1: BM25 lexical (temporal-hierarchy stand-in) ──
        let q_tokens = tokenize(query);
        if q_tokens.is_empty() {
            return Vec::new();
        }
        let bm = bm25(&corpus, &q_tokens);

        // ── view 2: entity graph + Personalized PageRank ──
        let q_ents: HashSet<String> = extract_entities(query).into_iter().collect();
        let graph = EntityGraph::build(&corpus);
        let g = if q_ents.is_empty() {
            HashMap::new()
        } else {
            graph.ppr(&q_ents, 0.6)
        };

        // ── routing (paper Eq 6-7): relational → graph-primary; temporal
        // cues → lexical-primary. ρ to the primary.
        let temporal = temporal_re().is_match(query);
        let relational = !q_ents.is_empty() && !temporal;
        let (w_graph, w_lex) = if relational {
            (self.cfg.rho, 1.0 - self.cfg.rho)
        } else {
            (1.0 - self.cfg.rho, self.cfg.rho)
        };

        // ── min-max normalize each view, fuse, scale once by pool
        // confidence (pi v0.14b: min-max stretches any non-constant pool
        // to [0,1]; weak pools must inject nothing).
        let lex_conf = {
            let bm_max = bm.values().cloned().fold(0.0f64, f64::max);
            let anchor = 1.2 + 2.8 * (corpus.len() as f64 / 600.0).min(1.0);
            (bm_max / anchor).min(1.0)
        };
        let g_conf = {
            let g_max = g.values().cloned().fold(0.0f64, f64::max);
            // Graph mass is corroborated only when it concentrates.
            if g_max > 0.0 { ((g_max - 0.15) / 0.5).clamp(0.0, 1.0) } else { 0.0 }
        };
        let pool_conf = lex_conf.max(g_conf * 0.9);
        if pool_conf < 0.2 {
            return Vec::new(); // no memory beats confusing memory
        }

        let lex_norm = minmax(&bm);
        let g_norm = minmax(&g);
        let mut fused: Vec<(String, f64)> = Vec::new();
        let mut ids: HashSet<&str> = HashSet::new();
        for id in lex_norm.keys().chain(g_norm.keys()) {
            ids.insert(id);
        }
        for id in ids {
            let l = lex_norm.get(id).copied().unwrap_or(0.0);
            let gr = g_norm.get(id).copied().unwrap_or(0.0);
            fused.push((id.to_string(), (w_lex * l + w_graph * gr) * pool_conf));
        }

        // ── evidence closure: 1-hop graph neighbors at a discount; the
        // candidates that seed closure may themselves be sub-min (pi
        // v0.13: closure is a rescue, not a reward).
        let mut closed: HashMap<String, f64> = fused.iter().cloned().collect();
        for (id, score) in &fused {
            if *score <= 0.0 {
                continue;
            }
            for nb in graph.neighbors(id) {
                let add = 0.35 * score;
                let e = closed.entry(nb).or_insert(0.0);
                if *e < add {
                    *e = add;
                }
            }
        }

        let by_id: HashMap<&str, &Unit> = corpus.iter().map(|u| (u.id.as_str(), *u)).collect();
        let mut hits: Vec<Hit> = closed
            .into_iter()
            .filter_map(|(id, score)| {
                let score = (score * 1000.0).round() / 1000.0;
                let u = *by_id.get(id.as_str())?;
                (score >= self.cfg.min_score).then(|| Hit {
                    role: u.role.clone(),
                    when: age_days(u.ts),
                    snippet: sanitize_snippet(&u.text, self.cfg.snippet_chars),
                    score,
                })
            })
            .collect();
        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        // Light MMR-style dedupe: drop near-identical snippets.
        let mut out: Vec<Hit> = Vec::new();
        for h in hits {
            if out
                .iter()
                .all(|o| !near_duplicate(&o.snippet, &h.snippet))
            {
                out.push(h);
                if out.len() >= self.cfg.top_k {
                    break;
                }
            }
        }
        out
    }

    /// Fingerprints of texts already visible in the model's window.
    pub fn window_fingerprints(texts: &[String]) -> HashSet<String> {
        texts.iter().map(|t| fingerprint(t)).collect()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub role: String,
    pub when: u64,
    pub snippet: String,
    pub score: f64,
}

// ── tokenization + BM25 ────────────────────────────────────────────────────

const STOPWORDS: &[&str] = &[
    "the", "a", "an", "and", "or", "but", "if", "then", "else", "when", "at", "by", "for",
    "with", "about", "into", "to", "from", "in", "on", "of", "is", "are", "was", "were", "be",
    "been", "it", "this", "that", "these", "those", "i", "you", "he", "she", "we", "they",
    "my", "your", "his", "her", "its", "our", "their", "me", "him", "them", "as", "do",
    "does", "did", "so", "not", "no", "can", "will", "just", "should", "would", "could",
    "what", "which", "who", "how", "why", "where", "there", "here", "all", "any", "some",
];

pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| w.len() >= 2 && !STOPWORDS.contains(w))
        .map(str::to_string)
        .collect()
}

/// Okapi BM25 (k1=1.5, b=0.75) over the unit corpus. Pure.
fn bm25(corpus: &[&Unit], q: &[String]) -> HashMap<String, f64> {
    const K1: f64 = 1.5;
    const B: f64 = 0.75;
    let docs: Vec<Vec<String>> = corpus.iter().map(|u| tokenize(&u.text)).collect();
    let n = docs.len() as f64;
    let avgdl = {
        let total: usize = docs.iter().map(Vec::len).sum();
        if docs.is_empty() { 1.0 } else { total as f64 / n }
    };
    // Document frequency per query term.
    let mut df: HashMap<&str, usize> = HashMap::new();
    for qt in q {
        let c = docs.iter().filter(|d| d.iter().any(|t| t == qt)).count();
        df.insert(qt.as_str(), c);
    }
    let mut out = HashMap::new();
    for (i, doc) in docs.iter().enumerate() {
        let dl = doc.len() as f64;
        let mut score = 0.0;
        for qt in q {
            let tf = doc.iter().filter(|t| *t == qt).count() as f64;
            if tf == 0.0 {
                continue;
            }
            let dfv = *df.get(qt.as_str()).unwrap_or(&0) as f64;
            let idf = ((n - dfv + 0.5) / (dfv + 0.5) + 1.0).ln();
            score += idf * (tf * (K1 + 1.0)) / (tf + K1 * (1.0 - B + B * dl / avgdl));
        }
        if score > 0.0 {
            out.insert(corpus[i].id.clone(), score);
        }
    }
    out
}

fn minmax(scores: &HashMap<String, f64>) -> HashMap<String, f64> {
    let mut out = HashMap::new();
    if scores.is_empty() {
        return out;
    }
    let max = scores.values().cloned().fold(0.0f64, f64::max);
    let min = scores.values().cloned().fold(f64::INFINITY, f64::min);
    let spread = max - min;
    for (k, v) in scores {
        // Eq 12's degenerate rule: a single-candidate (or uniform) pool
        // normalizes to 1 — pool confidence, not the normalizer, is what
        // gates weak pools (pi v0.14b).
        let norm = if spread < 1e-9 {
            if max > 0.0 { 1.0 } else { 0.0 }
        } else {
            (v - min) / spread
        };
        out.insert(k.clone(), norm);
    }
    out
}

// ── entity graph + PPR ─────────────────────────────────────────────────────

pub struct EntityGraph {
    entity_units: HashMap<String, Vec<String>>,
    unit_entities: HashMap<String, Vec<String>>,
    /// idf per entity (ubiquitous entities — >10% of units — are skipped,
    /// the paper's per-context normalization against clique wash-out).
    idf: HashMap<String, f64>,
    unit_adj: HashMap<String, HashMap<String, f64>>,
}

impl EntityGraph {
    fn build(corpus: &[&Unit]) -> Self {
        let n = corpus.len() as f64;
        let mut entity_units: HashMap<String, Vec<String>> = HashMap::new();
        let mut unit_entities: HashMap<String, Vec<String>> = HashMap::new();
        let mut df: HashMap<String, usize> = HashMap::new();
        for u in corpus {
            unit_entities.entry(u.id.clone()).or_default();
            let mut seen = HashSet::new();
            for e in &u.entities {
                if seen.insert(e) {
                    *df.entry(e.clone()).or_default() += 1;
                    entity_units.entry(e.clone()).or_default().push(u.id.clone());
                    unit_entities.get_mut(&u.id).unwrap().push(e.clone());
                }
            }
        }
        let idf: HashMap<String, f64> = df
            .iter()
            .filter(|(_, c)| (**c as f64) < n * 0.1)
            .map(|(e, c)| (e.clone(), (n / *c as f64).ln() + 1.0))
            .collect();
        // Unit adjacency weighted by shared-entity idf (paper Eq 4 spirit).
        let mut unit_adj: HashMap<String, HashMap<String, f64>> = HashMap::new();
        for u in corpus {
            for e in &u.entities {
                let Some(weight) = idf.get(e) else { continue };
                if let Some(others) = entity_units.get(e) {
                    for other in others {
                        if other != &u.id {
                            *unit_adj
                                .entry(u.id.clone())
                                .or_default()
                                .entry(other.clone())
                                .or_insert(0.0) += weight;
                        }
                    }
                }
            }
        }
        EntityGraph { entity_units, unit_entities, idf, unit_adj }
    }

    /// Personalized PageRank (paper Eq 8-10, pi v0.14): reset vector
    /// seeded by direct idf-weighted entity matches; π ← (1-γ)r + γPᵀπ;
    /// dangling units keep their mass; iterate to convergence.
    fn ppr(&self, q_ents: &HashSet<String>, gamma: f64) -> HashMap<String, f64> {
        let mut seeds: HashMap<String, f64> = HashMap::new();
        for (ent, units) in &self.entity_units {
            if !q_ents.contains(ent) {
                continue;
            }
            let w = self.idf.get(ent).copied().unwrap_or(1.0);
            for u in units {
                *seeds.entry(u.clone()).or_insert(0.0) += w;
            }
        }
        let mass: f64 = seeds.values().sum();
        if mass <= 0.0 || self.unit_adj.is_empty() {
            return HashMap::new();
        }
        let r: HashMap<String, f64> =
            seeds.iter().map(|(k, v)| (k.clone(), v / mass)).collect();
        let mut pi = r.clone();
        for _ in 0..24 {
            let mut next: HashMap<String, f64> =
                r.iter().map(|(k, v)| (k.clone(), (1.0 - gamma) * v)).collect();
            for (u, m) in &pi {
                if *m <= 0.0 {
                    continue;
                }
                match self.unit_adj.get(u) {
                    None => {
                        *next.entry(u.clone()).or_insert(0.0) += gamma * m; // dangling keeps mass
                    }
                    Some(row) => {
                        let row_sum: f64 = row.values().sum();
                        if row_sum <= 0.0 {
                            *next.entry(u.clone()).or_insert(0.0) += gamma * m;
                            continue;
                        }
                        for (v, w) in row {
                            *next.entry(v.clone()).or_insert(0.0) += gamma * m * (w / row_sum);
                        }
                    }
                }
            }
            let diff: f64 = next
                .keys()
                .map(|k| (next.get(k).copied().unwrap_or(0.0) - pi.get(k).copied().unwrap_or(0.0)).abs())
                .sum();
            pi = next;
            if diff < 1e-6 {
                break;
            }
        }
        pi
    }

    fn neighbors(&self, unit_id: &str) -> Vec<String> {
        let mut out = HashSet::new();
        if let Some(ents) = self.unit_entities.get(unit_id) {
            for e in ents {
                if let Some(units) = self.entity_units.get(e) {
                    for u in units {
                        if u != unit_id {
                            out.insert(u.clone());
                        }
                    }
                }
            }
        }
        out.into_iter().collect()
    }
}

// ── entity extraction (code-aware; the pi lesson: code regex matters more
//    than NLP NER for a coding agent) ───────────────────────────────────────

pub fn extract_entities(text: &str) -> Vec<String> {
    let mut found: HashSet<String> = HashSet::new();
    let add = |s: &str, found: &mut HashSet<String>| {
        let t = s.trim().to_lowercase();
        if t.len() < 2 || t.len() > 40 || STOPWORDS.contains(&t.as_str()) {
            return;
        }
        if t.chars().all(|c| c.is_ascii_digit()) {
            return;
        }
        found.insert(t);
    };
    // Paths (forward + backslash) with a dot/slash requirement so prose
    // like "and/or" stays out.
    let mut cur = String::new();
    let take_path = |cur: &mut String, found: &mut HashSet<String>| {
        let s = std::mem::take(cur);
        if s.contains('/') || s.contains('\\') || s.contains('.') {
            add(&s, found);
        }
    };
    for c in text.chars() {
        if c.is_ascii_alphanumeric() || c == '.' || c == '/' || c == '\\' || c == '_' || c == '-' {
            cur.push(c);
        } else {
            take_path(&mut cur, &mut found);
        }
    }
    take_path(&mut cur, &mut found);
    // Backticked identifiers.
    let bytes: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '`' {
            let mut j = i + 1;
            let mut ident = String::new();
            while j < bytes.len() && bytes[j] != '`' && bytes[j] != '\n' && ident.len() < 40 {
                ident.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() && bytes[j] == '`' && ident.len() >= 2 {
                add(&ident, &mut found);
            }
            i = j + 1;
        } else {
            i += 1;
        }
    }
    // snake_case and CamelCase identifiers, acronyms.
    let camel = regex::Regex::new(r"\b[A-Z][a-z]+(?:[A-Z][a-z]+)+\b").unwrap();
    let snake = regex::Regex::new(r"\b[a-z]+(?:_[a-z0-9]+)+\b").unwrap();
    let screaming = regex::Regex::new(r"\b[A-Z]+(?:_[A-Z0-9]+)+\b").unwrap();
    let acro = regex::Regex::new(r"\b[A-Z][A-Z0-9]{2,}\b").unwrap();
    for m in camel.find_iter(text) {
        add(m.as_str(), &mut found);
    }
    for m in snake.find_iter(text).chain(screaming.find_iter(text)) {
        add(m.as_str(), &mut found);
    }
    for m in acro.find_iter(text) {
        add(m.as_str(), &mut found);
    }
    // Multi-word capitalized names ("the Serializer Builder").
    let multi = regex::Regex::new(r"\b([A-Z][a-z]{2,})\s+([A-Z][a-z]{2,})\b").unwrap();
    for cap in multi.captures_iter(text) {
        let joined = format!("{} {}", &cap[1], &cap[2]);
        add(&joined, &mut found);
    }
    found.into_iter().take(16).collect()
}

// ── identity (pi v0.14b-h lessons, distilled) ──────────────────────────────

/// Self-statements naming a default model are poison, not evidence.
const POISON_NAMES: &[&str] = &[
    "pi", "glm", "gpt", "claude", "copilot", "gemini", "myharness", "assistant", "ai",
    "codex", "qwen", "llama",
];

const NON_NAME_WORDS: &[&str] = &[
    "a", "an", "the", "so", "not", "just", "sure", "back", "done", "here", "also", "now",
    "all", "your", "my", "me", "it", "this", "that", "going", "working", "using", "very",
    "really", "sorry", "glad", "happy", "curious", "plan", "idea", "default", "latest",
    "other", "another", "new", "old", "same", "one", "two", "change", "fix", "update",
    "whatever", "anything", "something", "first", "second", "version", "approach",
];

enum NamingName {
    User(String),
    Agent(String),
}

fn name_candidate(raw: &str) -> Option<String> {
    let t = raw.trim().trim_matches(|c| c == '\'' || c == '"' || c == '*' || c == '.');
    let lower = t.to_lowercase();
    if t.len() < 2 || t.len() > 24 {
        return None;
    }
    if NON_NAME_WORDS.contains(&lower.as_str()) || POISON_NAMES.contains(&lower.as_str()) {
        return None;
    }
    // Proper-noun gate: capitalized word, quoted word, or CamelCase.
    let first = t.chars().next().unwrap_or(' ');
    if first.is_uppercase() || first.is_ascii_digit() || t.contains('_') || t.contains('-') {
        Some(t.to_string())
    } else {
        None
    }
}

/// Newest-wins scan of naming statements across units.
fn naming_name(u: &Unit) -> Option<NamingName> {
    let text = u.text.to_lowercase();
    let rest_of = |pat: &str| -> Option<String> {
        let idx = text.find(pat)?;
        let tail = &u.text[idx + pat.len()..];
        let word: String = tail
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '\'' || *c == '"' || *c == '*')
            .collect();
        name_candidate(&word)
    };
    match u.role.as_str() {
        "user" => {
            if let Some(n) = rest_of("my name is ").or_else(|| rest_of("call me ")) {
                return Some(NamingName::User(n));
            }
            // User naming the agent: "your name is X" / "i'll call you X".
            if let Some(n) = rest_of("your name is ").or_else(|| rest_of("i'll call you ")) {
                return Some(NamingName::Agent(n));
            }
            None
        }
        "assistant" => {
            if let Some(n) = rest_of("call me ")
                .or_else(|| rest_of("my name is "))
                .or_else(|| rest_of("i'll go with "))
            {
                return Some(NamingName::Agent(n));
            }
            None
        }
        _ => None,
    }
}

pub fn identity_query(query: &str) -> bool {
    let q = query.to_lowercase();
    ["your name", "my name", "who am i", "do you remember me", "what are you called", "introduce yourself"]
        .iter()
        .any(|p| q.contains(p))
}

pub fn build_identity_line(id: &Identity) -> Option<String> {
    match (&id.user_name, &id.agent_name) {
        (None, None) => None,
        (Some(u), None) => Some(format!("(memory) The user goes by {u}.")),
        (None, Some(a)) => Some(format!("(memory) The user calls this agent {a}.")),
        (Some(u), Some(a)) => Some(format!("(memory) The user goes by {u}, and calls this agent {a}.")),
    }
}

// ── snippets, fingerprints, misc ───────────────────────────────────────────

// (regex static via OnceLock — compiled once)
static TEMPORAL_RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();

fn temporal_re() -> &'static regex::Regex {
    TEMPORAL_RE.get_or_init(|| {
        regex::Regex::new(
            r"(?i)\b(earlier|before|last|previous|yesterday|ago|used to|recently|back when)\b",
        )
        .unwrap()
    })
}

impl ZeroMem {
    /// Public accessor for tests: is this query temporal-routed?
    pub fn query_is_temporal(query: &str) -> bool {
        temporal_re().is_match(query)
    }
}

/// djb2-family fingerprint over normalized text (active-context check).
pub fn fingerprint(text: &str) -> String {
    let norm: String = text
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect();
    // Collapse separator runs so "hello, world!" == "hello world".
    let mut collapsed = String::with_capacity(norm.len());
    for c in norm.chars() {
        if c == ' ' && collapsed.ends_with(' ') {
            continue;
        }
        collapsed.push(c);
    }
    let norm = collapsed.trim();
    let norm: String = norm.chars().take(500).collect();
    let mut h: u64 = 5381;
    for c in norm.chars() {
        h = (h.wrapping_mul(33) ^ (c as u64)) & 0xffff_ffff;
    }
    format!("{h:032x}")
}

/// Stored text is untrusted: strip fences/headings/bullets so injected
/// snippets can't structure the prompt (pi v0.13 hardening).
pub fn sanitize_snippet(text: &str, max_chars: usize) -> String {
    let mut cleaned: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with("```") || t.starts_with('#') {
            continue;
        }
        if t.is_empty() && cleaned.last().map(|l: &String| l.is_empty()).unwrap_or(false) {
            continue;
        }
        let t = t.trim_start_matches(['-', '*', '>']).trim();
        cleaned.push(t.to_string());
    }
    let joined = cleaned.join(" ").trim().to_string();
    if joined.chars().count() <= max_chars {
        joined
    } else {
        let cut: String = joined.chars().take(max_chars).collect();
        format!("{cut}...")
    }
}

fn near_duplicate(a: &str, b: &str) -> bool {
    let at: HashSet<&str> = a.split_whitespace().collect();
    let bt: HashSet<&str> = b.split_whitespace().collect();
    if at.is_empty() || bt.is_empty() {
        return false;
    }
    let inter = at.intersection(&bt).count();
    let minlen = at.len().min(bt.len());
    inter as f64 / minlen as f64 > 0.8
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn age_days(ts: u64) -> u64 {
    now_secs().saturating_sub(ts) / 86_400
}

fn project_slug(root: &Path) -> String {
    let s = root.display().to_string().to_lowercase();
    let mut h: u64 = 1469598103934665603;
    for c in s.chars() {
        h = (h ^ (c as u64)).wrapping_mul(1099511628211);
    }
    let base = root
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| "project".into());
    let safe: String = base
        .chars()
        .take(24)
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' })
        .collect();
    format!("{safe}-{h:012x}")
}


#[cfg(test)]
mod tests {
    use super::*;

    fn zm_with(units: Vec<(&str, &str)>) -> ZeroMem {
        let dir = tempfile::tempdir().unwrap();
        let mut zm = ZeroMem::open(dir.path(), Path::new("/w/proj"), "s-cur", ZeroMemCfg::default());
        zm.path = dir.path().join("nonexistent-store.json"); // never persist in tests
        for (i, (role, text)) in units.iter().enumerate() {
            zm.units.push(Unit {
                id: format!("u{}", i + 1),
                role: role.to_string(),
                text: text.to_string(),
                session: if *role == "cur" { "s-cur".to_string() } else { "s-old".to_string() },
                ts: now_secs(),
                fingerprint: fingerprint(text),
                entities: extract_entities(text),
            });
        }
        zm
    }

    #[test]
    fn entities_are_code_aware() {
        let e = extract_entities("fixed the `session_recall` tool in src/tools/session_recall.rs, see ZERO_MEM config");
        let lower: Vec<String> = e.iter().map(|x| x.to_lowercase()).collect();
        assert!(lower.contains(&"session_recall".to_string()), "{e:?}");
        assert!(lower.iter().any(|x| x.contains(".rs")), "{e:?}");
        assert!(lower.contains(&"zero_mem".to_string()), "{e:?}");
        assert!(!lower.contains(&"the".to_string()));
    }

    #[test]
    fn bm25_ranks_relevant_unit_first() {
        let zm = zm_with(vec![
            ("user", "how do I bake sourdough bread at home"),
            ("assistant", "knead the dough, fold every 30 minutes"),
            ("user", "the flaky login test failed on the spinner await"),
            ("assistant", "we fixed the login test by awaiting the spinner before asserting"),
            ("user", "deploy notes: run ./deploy.sh from the repo root"),
        ]);
        let hits = zm.retrieve("fix the flaky login test", &HashSet::new());
        assert!(!hits.is_empty(), "must find the login-test memory");
        assert!(
            hits[0].snippet.contains("spinner"),
            "{hits:?}"
        );
    }

    #[test]
    fn weak_pool_injects_nothing() {
        // One unit, query shares only a stopword-ish token.
        let zm = zm_with(vec![
            ("user", "completely unrelated content about zebras"),
            ("assistant", "zebras have stripes"),
        ]);
        let hits = zm.retrieve("quantum entanglement", &HashSet::new());
        assert!(hits.is_empty(), "no memory beats confusing memory: {hits:?}");
    }

    #[test]
    fn current_session_units_excluded() {
        let zm = zm_with(vec![
            ("cur", "the password is swordfish"),
            ("cur", "reminder about swordfish"),
        ]);
        let hits = zm.retrieve("swordfish", &HashSet::new());
        assert!(hits.is_empty(), "current-session units are the live window");
    }

    #[test]
    fn active_context_not_reinjected() {
        let zm = zm_with(vec![
            ("user", "the api key rotates every 30 days"),
            ("assistant", "noted, the api key rotates monthly"),
        ]);
        let fp = fingerprint("the api key rotates every 30 days");
        let mut active = HashSet::new();
        active.insert(fp);
        let hits = zm.retrieve("api key rotation", &active);
        assert!(hits.iter().all(|h| !h.snippet.contains("rotates every 30")), "{hits:?}");
    }

    #[test]
    fn temporal_routing_detected() {
        assert!(ZeroMem::query_is_temporal("what did we do yesterday"));
        assert!(!ZeroMem::query_is_temporal("how does the config parser work"));
    }

    #[test]
    fn closure_rescues_adjacent_evidence() {
        // Plain prose carries no entities by design (the code-aware
        // extractor is deliberate); the units must share a backticked
        // entity for the graph edge closure follows.
        let zm = zm_with(vec![
            ("user", "we chose `postgres` for the `job_queue` storage"),
            ("assistant", "`postgres` also handles the `migrations` table"),
            ("user", "unrelated note about the ci pipeline"),
            ("assistant", "ci runs on github actions"),
        ]);
        let hits = zm.retrieve("why did we pick the job queue storage", &HashSet::new());
        assert!(
            hits.iter().any(|h| h.snippet.contains("migrations")),
            "closure should bring in the adjacent evidence: {hits:?}"
        );
    }

    #[test]
    fn identity_derivation_and_poison_filter() {
        let mut zm = zm_with(vec![
            ("assistant", "You can call me Pi if you like"), // poison: default name
            ("user", "my name is Mark and I run this repo"),
            ("assistant", "Got it — I'll go with Cipher."),
            ("user", "your name is Echo now"),
        ]);
        zm.derive_identity();
        let id = zm.identity();
        assert_eq!(id.user_name.as_deref(), Some("Mark"));
        // Newest confident naming wins: Echo (user-directed) last.
        assert_eq!(id.agent_name.as_deref(), Some("Echo"));
        let line = build_identity_line(id).unwrap();
        assert!(line.contains("Mark") && line.contains("Echo"), "{line}");
    }

    #[test]
    fn snippets_sanitized() {
        let out = sanitize_snippet("## heading\n```rust\nfn x() {}\n```\n- bullet point here\nnormal line", 120);
        assert!(!out.contains("```"), "{out}");
        assert!(!out.contains("##"), "{out}");
        assert!(!out.contains("- bullet"), "{out}");
        assert!(out.contains("normal line"), "{out}");
        let cut = sanitize_snippet(&"word ".repeat(100), 30);
        assert!(cut.chars().count() <= 33 && cut.ends_with("..."), "{cut}");
    }

    #[test]
    fn store_round_trip_and_retention() {
        let dir = tempfile::tempdir().unwrap();
        let mut zm = ZeroMem::open(dir.path(), Path::new("/w/proj"), "s1", ZeroMemCfg::default());
        zm.capture("user", "remember the deploy steps: ./deploy.sh then verify");
        zm.capture("assistant", "deploy confirmed");
        zm.persist();
        let zm2 = ZeroMem::open(dir.path(), Path::new("/w/proj"), "s2", ZeroMemCfg::default());
        assert_eq!(zm2.units.len(), 2);
        // Retention: aggressive bounds trim the store.
        let cfg = ZeroMemCfg { max_units: 1, ..ZeroMemCfg::default() };
        let zm3 = ZeroMem::open(dir.path(), Path::new("/w/proj"), "s3", cfg);
        assert_eq!(zm3.units.len(), 1);
    }

    #[test]
    fn fingerprint_stable_and_normalizing() {
        assert_eq!(fingerprint("Hello, World!"), fingerprint("hello world"));
        assert_ne!(fingerprint("alpha beta"), fingerprint("beta gamma"));
    }

    #[test]
    fn near_duplicate_dedup() {
        assert!(near_duplicate("fix the login test now", "fix the login test"));
        assert!(!near_duplicate("fix the login test", "deploy the api server"));
    }
}
