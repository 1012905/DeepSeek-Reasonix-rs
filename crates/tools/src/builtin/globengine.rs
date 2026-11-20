//! Glob matching, two layers. `match_filepath` is the Go stdlib
//! `filepath.Match` (used for the non-`**` fast path and its walk); `match`
//! and `split_pattern` port bmatcuk/doublestar v4's `Match`/`SplitPattern`
//! with the `/` separator, which is what the recursive walk and
//! `MatchSlashGlob` consume. Ported rather than reimplemented so the
//! metacharacter semantics (`**`, `[]`, `{a,b}`, escapes) stay identical to
//! what produced the Go oracle results.

#![forbid(unsafe_code)]

use super::pathgo;

/// A malformed pattern (`filepath.ErrBadPattern` / doublestar's equivalent).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("syntax error in pattern")]
pub struct BadPattern;

/// Go `filepath.Match` with `runtime.GOOS` deciding escaping. `name` and
/// `pattern` use the OS separator.
pub fn match_filepath(pattern: &str, name: &str) -> Result<bool, BadPattern> {
    let sep = pathgo::separator() as u8;
    let windows = cfg!(windows);
    let mut pattern = pattern;
    let mut name = name;
    'pattern: while !pattern.is_empty() {
        let (star, chunk, rest) = scan_chunk(pattern, windows);
        pattern = rest;
        if star && chunk.is_empty() {
            return Ok(!name.bytes().any(|b| b == sep));
        }
        // Go: t, ok, err := matchChunk(chunk, name); a plain failure falls
        // through to the star backtrack, an error aborts.
        match match_chunk(chunk, name, windows) {
            ChunkMatch::Ok(t, _is_last_ok) => {
                // matched && (len(t) == 0 || len(pattern) > 0)
                if t == name.len() || !pattern.is_empty() {
                    name = &name[t..];
                    continue 'pattern;
                }
            }
            ChunkMatch::NoMatch(Some(_)) => return Err(BadPattern),
            ChunkMatch::NoMatch(None) => {}
        }
        if star {
            let b = name.as_bytes();
            let mut i = 0usize;
            while i < b.len() && b[i] != sep {
                match match_chunk(chunk, &name[i + 1..], windows) {
                    ChunkMatch::Ok(t, _ok) => {
                        // t is an offset into the sub-slice; the remainder
                        // Go hands back is name[i+1:][t:].
                        if pattern.is_empty() && (b.len() - i - 1) > t {
                            i += 1;
                            continue;
                        }
                        name = &name[i + 1 + t..];
                        continue 'pattern;
                    }
                    ChunkMatch::NoMatch(Some(_)) => return Err(BadPattern),
                    ChunkMatch::NoMatch(None) => {
                        i += 1;
                    }
                }
            }
        }
        return Ok(false);
    }
    Ok(name.is_empty())
}

enum ChunkMatch {
    /// Matched; carries the byte offset in `name` the chunk consumed up to,
    /// and whether the whole name was consumed by this chunk alone.
    Ok(usize, bool),
    NoMatch(Option<BadPattern>),
}

fn scan_chunk(pattern: &str, windows: bool) -> (bool, &str, &str) {
    let b = pattern.as_bytes();
    let mut star = false;
    let mut i = 0usize;
    while i < b.len() && b[i] == b'*' {
        i += 1;
        star = true;
    }
    let start = i;
    let mut in_range = false;
    while i < b.len() {
        match b[i] {
            b'\\' => {
                if !windows && i + 1 < b.len() {
                    i += 1;
                }
            }
            b'[' => in_range = true,
            b']' => in_range = false,
            b'*' if !in_range => {
                return (star, &pattern[start..i], &pattern[i..]);
            }
            _ => {}
        }
        i += 1;
    }
    (star, &pattern[start..], "")
}

fn match_chunk(chunk: &str, s: &str, windows: bool) -> ChunkMatch {
    let mut chunk = chunk;
    let mut failed = false;
    let b = s.as_bytes();
    let mut s_idx = 0usize;
    while !chunk.is_empty() {
        failed = failed || s_idx >= b.len();
        match chunk.as_bytes()[0] {
            b'[' => {
                let mut r = '\u{FFFD}';
                if !failed {
                    let rest = &s[s_idx..];
                    let n = match decode_rune(rest) {
                        Some((c, n)) => {
                            r = c;
                            n
                        }
                        None => 1,
                    };
                    s_idx += n;
                }
                chunk = &chunk[1..];
                let mut negated = false;
                if !chunk.is_empty() && chunk.as_bytes()[0] == b'^' {
                    negated = true;
                    chunk = &chunk[1..];
                }
                let mut matched = false;
                let mut nrange = 0usize;
                loop {
                    if !chunk.is_empty() && chunk.as_bytes()[0] == b']' && nrange > 0 {
                        chunk = &chunk[1..];
                        break;
                    }
                    match get_esc(chunk, windows) {
                        Ok((lo, rest)) => {
                            chunk = rest;
                            let mut hi = lo;
                            if !chunk.is_empty() && chunk.as_bytes()[0] == b'-' {
                                match get_esc(&chunk[1..], windows) {
                                    Ok((h, rest2)) => {
                                        hi = h;
                                        chunk = rest2;
                                    }
                                    Err(()) => return ChunkMatch::NoMatch(Some(BadPattern)),
                                }
                            }
                            matched = matched || (lo <= r && r <= hi);
                            nrange += 1;
                        }
                        Err(()) => return ChunkMatch::NoMatch(Some(BadPattern)),
                    }
                }
                failed = failed || matched == negated;
            }
            b'?' => {
                if !failed {
                    failed = s_idx < b.len() && b[s_idx] == pathgo::separator() as u8;
                    if let Some((_, n)) = decode_rune(&s[s_idx..]) {
                        s_idx += n;
                    } else {
                        s_idx += 1;
                    }
                }
                chunk = &chunk[1..];
            }
            b'\\' if !windows => {
                chunk = &chunk[1..];
                if chunk.is_empty() {
                    return ChunkMatch::NoMatch(Some(BadPattern));
                }
                if !failed {
                    failed = s_idx >= b.len() || chunk.as_bytes()[0] != b[s_idx];
                    s_idx += 1;
                }
                chunk = &chunk[1..];
            }
            c => {
                if !failed {
                    failed = s_idx >= b.len() || c != b[s_idx];
                    s_idx += 1;
                }
                chunk = &chunk[1..];
            }
        }
    }
    if failed {
        return ChunkMatch::NoMatch(None);
    }
    ChunkMatch::Ok(s_idx, s_idx >= b.len())
}

fn get_esc(chunk: &str, windows: bool) -> Result<(char, &str), ()> {
    if chunk.is_empty() || chunk.as_bytes()[0] == b'-' || chunk.as_bytes()[0] == b']' {
        return Err(());
    }
    let mut chunk = chunk;
    if chunk.as_bytes()[0] == b'\\' && !windows {
        chunk = &chunk[1..];
        if chunk.is_empty() {
            return Err(());
        }
    }
    let (r, n) = match decode_rune(chunk) {
        Some(x) => x,
        None => return Err(()),
    };
    let nchunk = &chunk[n..];
    if nchunk.is_empty() {
        return Err(());
    }
    Ok((r, nchunk))
}

fn decode_rune(s: &str) -> Option<(char, usize)> {
    let mut it = s.chars();
    let c = it.next()?;
    Some((c, c.len_utf8()))
}

/// doublestar v4 `Match` with the `/` separator. Both sides are slash paths.
pub fn match_doublestar(pattern: &str, name: &str) -> bool {
    match_doublestar_checked(pattern, name).is_ok_and(|m| m)
}

pub fn match_doublestar_checked(pattern: &str, name: &str) -> Result<bool, BadPattern> {
    do_match(pattern, name, -1isize, -1isize, -1isize, -1isize, 0, 0)
}

#[allow(clippy::too_many_arguments)]
fn do_match(
    pattern: &str,
    name: &str,
    ds_pat_bt: isize,
    ds_name_bt: isize,
    star_pat_bt: isize,
    star_name_bt: isize,
    pat_idx: usize,
    name_idx: usize,
) -> Result<bool, BadPattern> {
    let pat = pattern.as_bytes();
    let nam = name.as_bytes();
    let pat_len = pat.len();
    let name_len = nam.len();
    let mut ds_pat_bt = ds_pat_bt;
    let mut ds_name_bt = ds_name_bt;
    let mut star_pat_bt = star_pat_bt;
    let mut star_name_bt = star_name_bt;
    let mut pat_idx = pat_idx;
    let mut name_idx = name_idx;
    let mut start_of_segment = true;
    'match_loop: while name_idx < name_len {
        // Go's switch `break` exits the switch and falls into the backtrack
        // section below, still inside the loop; the labeled block mirrors it.
        'switch_blk: {
            if pat_idx < pat_len {
                match pat[pat_idx] {
                    b'*' => {
                        pat_idx += 1;
                        if pat_idx < pat_len && pat[pat_idx] == b'*' {
                            pat_idx += 1;
                            if start_of_segment {
                                if pat_idx >= pat_len {
                                    return Ok(true);
                                }
                                let (pr, prl) =
                                    decode_rune(&pattern[pat_idx..]).unwrap_or(('/', 1));
                                if pr == '/' {
                                    pat_idx += prl;
                                    ds_pat_bt = pat_idx as isize;
                                    ds_name_bt = name_idx as isize;
                                    star_pat_bt = -1;
                                    star_name_bt = -1;
                                    continue 'match_loop;
                                }
                            }
                        }
                        start_of_segment = false;
                        star_pat_bt = pat_idx as isize;
                        star_name_bt = name_idx as isize;
                        continue 'match_loop;
                    }
                    b'?' => {
                        start_of_segment = false;
                        let (nr, nrl) = decode_rune(&name[name_idx..]).unwrap_or(('\u{FFFD}', 1));
                        if nr == '/' {
                            break 'switch_blk;
                        }
                        pat_idx += 1;
                        name_idx += nrl;
                        continue 'match_loop;
                    }
                    b'[' => {
                        start_of_segment = false;
                        pat_idx += 1;
                        if pat_idx >= pat_len {
                            return Err(BadPattern);
                        }
                        let (nr, nrl) = decode_rune(&name[name_idx..]).unwrap_or(('\u{FFFD}', 1));
                        let mut matched = false;
                        let negate = pat[pat_idx] == b'!' || pat[pat_idx] == b'^';
                        if negate {
                            pat_idx += 1;
                        }
                        if pat_idx >= pat_len || pat[pat_idx] == b']' {
                            return Err(BadPattern);
                        }
                        let mut last = '\u{10FFFF}';
                        while pat_idx < pat_len && pat[pat_idx] != b']' {
                            let (mut pr, mut prl) =
                                decode_rune(&pattern[pat_idx..]).unwrap_or(('\u{FFFD}', 1));
                            pat_idx += prl;
                            if last < '\u{10FFFF}'
                                && pr == '-'
                                && pat_idx < pat_len
                                && pat[pat_idx] != b']'
                            {
                                if pat[pat_idx] == b'\\' {
                                    pat_idx += 1;
                                }
                                let (pr2, prl2) =
                                    decode_rune(&pattern[pat_idx..]).unwrap_or(('\u{FFFD}', 1));
                                pr = pr2;
                                prl = prl2;
                                pat_idx += prl;
                                if last <= nr && nr <= pr {
                                    matched = true;
                                    break;
                                }
                                continue 'match_loop;
                            }
                            if pr == '\\' {
                                let (pr2, prl2) =
                                    decode_rune(&pattern[pat_idx..]).unwrap_or(('\u{FFFD}', 1));
                                pr = pr2;
                                prl = prl2;
                                pat_idx += prl;
                            }
                            if pr == nr {
                                matched = true;
                                break;
                            }
                            last = pr;
                        }
                        if matched == negate {
                            if pat_idx >= pat_len {
                                return Err(BadPattern);
                            }
                            break 'switch_blk;
                        }
                        match index_unescaped_byte(&pattern[pat_idx..], b']', true) {
                            Some(ci) => pat_idx += ci + 1,
                            None => return Err(BadPattern),
                        }
                        name_idx += nrl;
                        continue 'match_loop;
                    }
                    b'{' => {
                        let before_idx = pat_idx;
                        pat_idx += 1;
                        let closing = index_matched_closing_alt(&pattern[pat_idx..], true)
                            .ok_or(BadPattern)?
                            + pat_idx;
                        let mut alt_pat_idx = pat_idx;
                        while let Some(cio) = index_next_alt(&pattern[alt_pat_idx..closing], true) {
                            let comma = cio + alt_pat_idx;
                            let rewritten = format!(
                                "{}{}{}",
                                &pattern[..before_idx],
                                &pattern[alt_pat_idx..comma],
                                &pattern[closing + 1..]
                            );
                            if do_match(
                                &rewritten,
                                name,
                                ds_pat_bt,
                                ds_name_bt,
                                star_pat_bt,
                                star_name_bt,
                                before_idx,
                                name_idx,
                            )? {
                                return Ok(true);
                            }
                            alt_pat_idx = comma + 1;
                        }
                        let rewritten = format!(
                            "{}{}{}",
                            &pattern[..before_idx],
                            &pattern[alt_pat_idx..closing],
                            &pattern[closing + 1..]
                        );
                        return do_match(
                            &rewritten,
                            name,
                            ds_pat_bt,
                            ds_name_bt,
                            star_pat_bt,
                            star_name_bt,
                            before_idx,
                            name_idx,
                        );
                    }
                    b'\\' => {
                        pat_idx += 1;
                        if pat_idx >= pat_len {
                            return Err(BadPattern);
                        }
                        let (pr, prl) = decode_rune(&pattern[pat_idx..]).unwrap_or(('\u{FFFD}', 1));
                        let (nr, nrl) = decode_rune(&name[name_idx..]).unwrap_or(('\u{FFFD}', 1));
                        if pr != nr {
                            break 'switch_blk;
                        }
                        pat_idx += prl;
                        name_idx += nrl;
                        start_of_segment = pr == '/';
                        continue 'match_loop;
                    }
                    c => {
                        let (pr, prl) = decode_rune(&pattern[pat_idx..]).unwrap_or(('\u{FFFD}', 1));
                        let (nr, nrl) = decode_rune(&name[name_idx..]).unwrap_or(('\u{FFFD}', 1));
                        let _ = c;
                        if pr != nr {
                            break 'switch_blk;
                        }
                        pat_idx += prl;
                        name_idx += nrl;
                        start_of_segment = pr == '/';
                        continue 'match_loop;
                    }
                }
            }
        }
        if star_pat_bt >= 0 {
            let (nr, nrl) = decode_rune(&name[star_name_bt as usize..]).unwrap_or(('\u{FFFD}', 1));
            if nr != '/' {
                star_name_bt += nrl as isize;
                pat_idx = star_pat_bt as usize;
                name_idx = star_name_bt as usize;
                start_of_segment = false;
                continue 'match_loop;
            }
        }
        if ds_pat_bt >= 0 {
            name_idx = ds_name_bt as usize;
            while name_idx < name_len {
                let (nr, nrl) = decode_rune(&name[name_idx..]).unwrap_or(('\u{FFFD}', 1));
                name_idx += nrl;
                if nr == '/' {
                    ds_name_bt = name_idx as isize;
                    pat_idx = ds_pat_bt as usize;
                    start_of_segment = true;
                    continue 'match_loop;
                }
            }
        }
        return Ok(false);
    }
    is_zero_length_pattern(&pattern[pat_idx..])
}

fn is_zero_length_pattern(pattern: &str) -> Result<bool, BadPattern> {
    match pattern.len() {
        0 => return Ok(true),
        1 => {
            if pattern == "*" {
                return Ok(true);
            }
        }
        2 => {
            if pattern == "**" {
                return Ok(true);
            }
        }
        3 => {
            let b = pattern.as_bytes();
            if &b[1..] == b"**" && b[0] == b'/' {
                return Ok(true);
            }
            if &b[..2] == b"**" && b[2] == b'/' {
                return Ok(true);
            }
        }
        4 => {
            let b = pattern.as_bytes();
            if &b[1..3] == b"**" && b[0] == b'/' && b[3] == b'/' {
                return Ok(true);
            }
        }
        _ => {}
    }
    if pattern.as_bytes().first() == Some(&b'{') {
        let closing = index_matched_closing_alt(&pattern[1..], true).ok_or(BadPattern)? + 1;
        let mut alt = 1usize;
        while let Some(cio) = index_next_alt(&pattern[alt..closing], true) {
            let comma = cio + alt;
            let rewritten = format!("{}{}", &pattern[alt..comma], &pattern[closing + 1..]);
            if is_zero_length_pattern(&rewritten)? {
                return Ok(true);
            }
            alt = comma + 1;
        }
        let rewritten = format!("{}{}", &pattern[alt..closing], &pattern[closing + 1..]);
        return is_zero_length_pattern(&rewritten);
    }
    Ok(false)
}

fn index_unescaped_byte(s: &str, c: u8, allow_escaping: bool) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if allow_escaping && b[i] == b'\\' {
            i += 1;
        } else if b[i] == c {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn index_matched_closing_alt(s: &str, allow_escaping: bool) -> Option<usize> {
    let b = s.as_bytes();
    let mut alts = 1isize;
    let mut i = 0usize;
    while i < b.len() {
        if allow_escaping && b[i] == b'\\' {
            i += 1;
        } else if b[i] == b'[' {
            i += 1;
            while i < b.len() {
                if allow_escaping && b[i] == b'\\' {
                    i += 1;
                } else if b[i] == b']' {
                    break;
                }
                i += 1;
            }
        } else if b[i] == b'{' {
            alts += 1;
        } else if b[i] == b'}' {
            alts -= 1;
            if alts == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn index_next_alt(s: &str, allow_escaping: bool) -> Option<usize> {
    let b = s.as_bytes();
    let mut alts = 1isize;
    let mut i = 0usize;
    while i < b.len() {
        if allow_escaping && b[i] == b'\\' {
            i += 1;
        } else if b[i] == b'[' {
            i += 1;
            while i < b.len() {
                if allow_escaping && b[i] == b'\\' {
                    i += 1;
                } else if b[i] == b']' {
                    break;
                }
                i += 1;
            }
        } else if b[i] == b'{' {
            alts += 1;
        } else if b[i] == b'}' {
            alts -= 1;
        } else if b[i] == b',' && alts == 1 {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// doublestar v4 `SplitPattern`: the longest literal prefix before the first
/// metacharacter, cut at the last `/` within it. Returns ("." , pattern)
/// when there is nowhere to split. Mirrors utils.go including the
/// splitIdx==0 "/" root case and meta unescaping of the base.
pub fn split_pattern(p: &str) -> (String, String) {
    let mut base = String::from(".");
    let mut pattern = String::from(p);

    let b = p.as_bytes();
    let mut split_idx: isize = -1;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'\\' {
            i += 1;
        } else if c == b'/' {
            split_idx = i as isize;
        } else if c == b'*' || c == b'?' || c == b'[' || c == b'{' {
            break;
        }
        i += 1;
    }

    if split_idx == 0 {
        base = String::from("/");
        pattern = String::from(&p[1..]);
    } else if split_idx > 0 {
        base = unescape_meta(&p[..split_idx as usize]);
        pattern = String::from(&p[split_idx as usize + 1..]);
    }
    (base, pattern)
}

fn unescape_meta(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// `fileutil.MatchSlashGlob`: slash-normalize both sides, doublestar Match,
/// with a `**/` prefix also matching at the root.
pub fn match_slash_glob(path: &str, pattern: &str) -> bool {
    // NormalizeSlashPath is ToSlash(Clean()): on Windows Clean switches the
    // separator back, so the slash form must be re-imposed after cleaning.
    let path = pathgo::to_slash(&pathgo::clean(&pathgo::to_slash(path)));
    let pattern = pathgo::to_slash(&pathgo::clean(&pathgo::to_slash(pattern)));
    if match_doublestar(&pattern, &path) {
        return true;
    }
    if let Some(after) = pattern.strip_prefix("**/") {
        return match_doublestar(after, &path);
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filepath_match_matches_go() {
        assert!(match_filepath("*.go", "main.go").unwrap());
        // On Windows the separator is backslash, so '*' matches through
        // '/' and this returns true (Go filepath.Match behavior).
        let want = cfg!(windows);
        assert_eq!(match_filepath("*.go", "src/main.go").unwrap(), want);
        assert!(match_filepath("a?c", "abc").unwrap());
        assert!(match_filepath("[a-c]x", "bx").unwrap());
        assert!(match_filepath("[^a]x", "bx").unwrap());
        assert!(!match_filepath("[^a]x", "ax").unwrap());
        assert!(match_filepath("*", "anything").unwrap());
        assert!(match_filepath("a/*", "a/b").unwrap());
        // On Windows the separator is backslash, so '*' crosses '/'.
        let want = cfg!(windows);
        assert_eq!(match_filepath("a/*", "a/b/c").unwrap(), want);
        assert!(match_filepath("x[y", "x[y").is_err());
    }

    #[test]
    fn doublestar_matches_recursive() {
        // v4's **/ matches zero directories, so a root-level file matches
        // directly; MatchSlashGlob's prefix rule is the wrapper-level
        // guarantee over Clean()ed inputs.
        assert!(match_doublestar("**/*.go", "main.go"));
        assert!(match_doublestar("**/*.go", "src/deep/main.go"));
        assert!(match_doublestar("src/**/*.ts", "src/a/b/c.ts"));
        assert!(match_doublestar("a/**/b", "a/b"));
        assert!(match_doublestar("a/**/b", "a/x/y/b"));
        assert!(!match_doublestar("a/**/b", "a/b/c"));
        assert!(match_doublestar("*.{go,ts}", "main.ts"));
        assert!(!match_doublestar("*.{go,ts}", "main.rs"));
        assert!(match_doublestar("src/*", "src/a"));
        assert!(!match_doublestar("src/*", "src/a/b"));
    }

    #[test]
    fn slash_glob_root_prefix_matches_root() {
        assert!(match_slash_glob("main.go", "**/*.go"));
        assert!(match_slash_glob("src/app.ts", "**/*.{go,ts}"));
        assert!(!match_slash_glob("README.md", "**/*.{go,ts}"));
    }

    #[test]
    fn split_pattern_finds_literal_base() {
        assert_eq!(
            split_pattern("src/**/*_test.go"),
            ("src".into(), "**/*_test.go".into())
        );
        assert_eq!(
            split_pattern("*.go"),
            (String::from("."), String::from("*.go"))
        );
        assert_eq!(
            split_pattern("/abs/**/*.go"),
            ("/abs".into(), "**/*.go".into())
        );
        assert_eq!(split_pattern("meta*/**"), (".".into(), "meta*/**".into()));
    }
}
