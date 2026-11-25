//! V1 retrieval helpers (internal/base/retrieval bm25.go): tokenization,
//! BM25 scoring, relative-score pruning, query normalization and snippet
//! carving. The V2 shadow ranker (BM25F, TokensV2) has no production caller
//! and is deliberately not ported (tools.md §8.4).

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::collections::BTreeSet;

/// BM25 k1 (Go `bm25.go`).
const K1: f64 = 1.2;
/// BM25 b (Go `bm25.go`).
const B: f64 = 0.75;

fn is_cjk(r: char) -> bool {
    unicode_is_cjk(r)
}

/// Han / Hiragana / Katakana / Hangul membership, matching Go `unicode.In`.
fn unicode_is_cjk(r: char) -> bool {
    matches!(r,
        '\u{3040}'..='\u{30FF}'   // Hiragana, Katakana
        | '\u{3130}'..='\u{318F}' // Hangul Jamo
        | '\u{3400}'..='\u{4DBF}' // CJK Ext A
        | '\u{4E00}'..='\u{9FFF}' // CJK Unified
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility
    )
}

/// Tokens: lowercases Latin words and splits CJK runs into overlapping
/// bigrams (a lone CJK rune stays a single term) — the standard CJK indexing
/// unit (Lucene's CJKAnalyzer, SQLite FTS).
pub fn tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut word: String = String::new();
    let mut prev: char = '\0';
    let mut cjk_run: usize = 0;

    let flush = |out: &mut Vec<String>, word: &mut String| {
        if !word.is_empty() {
            out.push(std::mem::take(word));
        }
    };
    let end_cjk = |out: &mut Vec<String>, prev: char, cjk_run: &mut usize| {
        if *cjk_run == 1 {
            out.push(prev.to_string());
        }
        *cjk_run = 0;
    };

    for r in s.chars() {
        if is_cjk(r) {
            flush(&mut out, &mut word);
            if cjk_run > 0 {
                out.push(format!("{prev}{r}"));
            }
            prev = r;
            cjk_run += 1;
        } else if r.is_alphanumeric() || r == '_' {
            end_cjk(&mut out, prev, &mut cjk_run);
            word.push(r.to_ascii_lowercase());
        } else {
            flush(&mut out, &mut word);
            end_cjk(&mut out, prev, &mut cjk_run);
        }
    }
    flush(&mut out, &mut word);
    end_cjk(&mut out, prev, &mut cjk_run);
    out
}

/// Unique: terms in first-seen order.
pub fn unique(inp: &[String]) -> Vec<String> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut out: Vec<String> = Vec::new();
    for t in inp {
        if t.is_empty() || !seen.insert(t.as_str()) {
            continue;
        }
        out.push(t.clone());
    }
    out
}

/// Counts: term-frequency map.
pub fn counts(terms: &[String]) -> BTreeMap<String, i32> {
    let mut map: BTreeMap<String, i32> = BTreeMap::new();
    for t in terms {
        *map.entry(t.clone()).or_default() += 1;
    }
    map
}

/// DocumentFrequency: how many documents contain each term.
pub fn document_frequency(docs: &[BTreeMap<String, i32>]) -> BTreeMap<String, i32> {
    let mut df: BTreeMap<String, i32> = BTreeMap::new();
    for counts in docs {
        for term in counts.keys() {
            *df.entry(term.clone()).or_default() += 1;
        }
    }
    df
}

/// BM25Score scores one document against the query terms.
pub fn bm25_score(
    counts: &BTreeMap<String, i32>,
    length: usize,
    query_terms: &[String],
    df: &BTreeMap<String, i32>,
    total_docs: usize,
    avg_len: f64,
) -> f64 {
    if length == 0 || total_docs == 0 {
        return 0.0;
    }
    let avg_len = if avg_len <= 0.0 { 1.0 } else { avg_len };
    let doc_len = length as f64;
    let mut score = 0.0;
    for term in query_terms {
        let tf = counts.get(term).copied().unwrap_or(0);
        if tf == 0 {
            continue;
        }
        let term_df = df.get(term).copied().unwrap_or(0);
        if term_df == 0 {
            continue;
        }
        let idf = (1.0 + (total_docs as f64 - term_df as f64 + 0.5) / (term_df as f64 + 0.5)).ln();
        let freq = tf as f64;
        score += idf * (freq * (K1 + 1.0)) / (freq + K1 * (1.0 - B + B * doc_len / avg_len));
    }
    score
}

/// KeepTopRelativeScore keeps the best item and drops trailing items whose
/// score falls below `ratio * topScore`. Items must already be sorted
/// best-first.
pub fn keep_top_relative_score<T>(
    items: &mut Vec<T>,
    ratio: f64,
    score: impl Fn(&T) -> f64,
) -> Vec<T> {
    if items.is_empty() || ratio <= 0.0 {
        return std::mem::take(items);
    }
    let top = score(&items[0]);
    if top <= 0.0 {
        return std::mem::take(items);
    }
    let cutoff = top * ratio;
    let mut kept: Vec<T> = Vec::new();
    for item in items.drain(..) {
        if kept.is_empty() || score(&item) >= cutoff {
            kept.push(item);
        }
    }
    kept
}

/// QueryTerms normalizes a search string; errors when nothing searchable remains.
pub fn query_terms(query: &str) -> Result<Vec<String>, String> {
    let terms = unique(&tokens(query.trim()));
    if terms.is_empty() {
        return Err("query must contain at least one letter or number".to_owned());
    }
    Ok(terms)
}

/// CompactWhitespace collapses runs of whitespace into one ASCII space.
pub fn compact_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// MakeSnippet: a whitespace-compacted excerpt centered near the query.
pub fn make_snippet(text: &str, query: &str, terms: &[String], max_runes: usize) -> String {
    let text = compact_whitespace(text);
    if max_runes == 0 || text.chars().count() <= max_runes {
        return text;
    }
    let lower = text.to_lowercase();
    let query = query.trim().to_lowercase();
    let mut idx: Option<usize> = None;
    if !query.is_empty() {
        idx = lower.find(&query);
    }
    if idx.is_none() {
        for term in terms {
            let runes: Vec<char> = term.chars().collect();
            if runes.len() == 1 && !is_cjk(runes[0]) {
                continue;
            }
            if let Some(i) = lower.find(term.as_str()) {
                idx = Some(i);
                break;
            }
        }
    }
    let byte_idx = idx.unwrap_or(0);
    snippet_around(&text, byte_idx, max_runes)
}

fn snippet_around(text: &str, byte_idx: usize, max_runes: usize) -> String {
    let mut byte_idx = byte_idx.min(text.len());
    while byte_idx > 0 && !text.is_char_boundary(byte_idx) {
        byte_idx -= 1;
    }
    let runes: Vec<char> = text.chars().collect();
    let pos = text[..byte_idx].chars().count();
    let mut start = pos.saturating_sub(max_runes / 2);
    let mut end = start + max_runes;
    if end > runes.len() {
        end = runes.len();
        start = end.saturating_sub(max_runes);
    }
    let prefix = if start > 0 { "..." } else { "" };
    let suffix = if end < runes.len() { "..." } else { "" };
    format!(
        "{prefix}{}{suffix}",
        runes[start..end].iter().collect::<String>()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_split_cjk_into_bigrams_and_lowercase_latin() {
        let out = tokens("MCP 服务器配置");
        assert!(out.contains(&"mcp".to_owned()));
        assert!(out.contains(&"服务".to_owned()));
        assert!(out.contains(&"务器".to_owned()));
        assert!(out.contains(&"器配".to_owned()));
        assert!(out.contains(&"配置".to_owned()));
    }

    #[test]
    fn query_terms_rejects_a_blank_query() {
        assert!(query_terms("   ").is_err());
        assert!(query_terms("mcp servers").is_ok());
    }

    #[test]
    fn bm25_zero_for_empty_doc_or_corpus() {
        let empty = BTreeMap::new();
        assert_eq!(bm25_score(&empty, 0, &[], &BTreeMap::new(), 0, 1.0), 0.0);
    }

    #[test]
    fn snippet_carves_a_window_around_the_query() {
        let text = "alpha beta gamma delta epsilon zeta eta theta iota kappa";
        let snip = make_snippet(text, "delta", &["delta".to_owned()], 20);
        assert!(snip.contains("delta"));
        assert!(snip.starts_with("...") || snip.ends_with("..."));
    }
}
