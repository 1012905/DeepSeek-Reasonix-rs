//! Whole-output unified-diff recognition.
//!
//! Port of the Go package internal/base/diff (unified.go): the decision a sink
//! needs before tagging a shell result as an embedded diff. Depends on nothing
//! domain-specific, like every other module here.

/// Reports whether text is, in its entirety, a unified diff — the shape
/// `git diff` / `diff -u` writes, or `git show` / `git log -p`, whose `commit …`
/// header and indented message precede the file sections. Every line must
/// belong to that grammar and hunks must match their "@@" counts; one line
/// left over rejects it, so prose or a build log after the diff stays prose.
pub fn is_unified_diff(text: &str) -> bool {
    let mut p = DiffParser {
        lines: text.split('\n').collect(),
        i: 0,
    };
    while p.i < p.lines.len() && p.lines[p.i].trim().is_empty() {
        p.i += 1;
    }
    if p.i >= p.lines.len() {
        return false;
    }
    let mut changed = false;
    if p.at().starts_with("commit ") {
        loop {
            if !p.log_block(&mut changed) {
                return false;
            }
            p.skip_blank();
            if p.i >= p.lines.len() {
                return changed;
            }
            if !p.at().starts_with("commit ") {
                return false;
            }
        }
    }
    loop {
        if !p.file(&mut changed) {
            return false;
        }
        p.skip_blank();
        if p.i >= p.lines.len() {
            return changed;
        }
    }
}

struct DiffParser<'a> {
    lines: Vec<&'a str>,
    i: usize,
}

impl DiffParser<'_> {
    fn at(&self) -> &str {
        self.lines.get(self.i).copied().unwrap_or("")
    }

    fn skip_blank(&mut self) {
        while self.i < self.lines.len() && self.lines[self.i].trim().is_empty() {
            self.i += 1;
        }
    }

    // Parses one `git show` / `git log -p` entry: the "commit …" line, its
    // header fields, the indented message, and the file sections that follow. A
    // block with no file section (a merge or empty commit) is legal; the caller
    // still requires at least one hunk across the whole text.
    fn log_block(&mut self, changed: &mut bool) -> bool {
        if !self.at().starts_with("commit ") {
            return false;
        }
        self.i += 1;
        while self.i < self.lines.len() && !self.lines[self.i].trim().is_empty() {
            if !is_log_header_field(self.lines[self.i]) {
                return false;
            }
            self.i += 1;
        }
        // The message is indented four spaces; blank lines separate its
        // paragraphs and the file sections that follow.
        while self.i < self.lines.len()
            && (self.lines[self.i].trim().is_empty() || self.lines[self.i].starts_with("    "))
        {
            self.i += 1;
        }
        while self.at().starts_with("diff --git ") {
            if !self.file(changed) {
                return false;
            }
        }
        true
    }

    // Parses one file section: an optional "diff --git" line, its extended
    // headers, the "--- "/"+++ " pair, and one or more hunks.
    fn file(&mut self, changed: &mut bool) -> bool {
        if self.at().starts_with("diff --git ") {
            self.i += 1;
        }
        while self.i < self.lines.len() && is_git_extended_header(self.lines[self.i]) {
            self.i += 1;
        }
        if !self.at().starts_with("--- ") {
            return false;
        }
        self.i += 1;
        self.skip_no_newline();
        if !self.at().starts_with("+++ ") {
            return false;
        }
        self.i += 1;
        self.skip_no_newline();
        if !self.hunk(changed) {
            return false;
        }
        while self.at().starts_with("@@ ") {
            if !self.hunk(changed) {
                return false;
            }
        }
        true
    }

    // Parses one "@@" header and exactly the body it declares: oldCount lines
    // of " "/"-" and newCount lines of " "/"+". A "\" no-newline marker follows
    // the line it applies to and carries no count.
    fn hunk(&mut self, changed: &mut bool) -> bool {
        let Some(counts) = parse_hunk_header(self.at()) else {
            return false;
        };
        self.i += 1;
        let (mut old_left, mut new_left) = counts;
        while old_left > 0 || new_left > 0 {
            let ln = self.at();
            if ln.is_empty() || ln.starts_with('\\') {
                return false;
            }
            match ln.as_bytes()[0] {
                b' ' => {
                    if old_left <= 0 || new_left <= 0 {
                        return false;
                    }
                    old_left -= 1;
                    new_left -= 1;
                }
                b'-' => {
                    if old_left <= 0 {
                        return false;
                    }
                    old_left -= 1;
                    *changed = true;
                }
                b'+' => {
                    if new_left <= 0 {
                        return false;
                    }
                    new_left -= 1;
                    *changed = true;
                }
                _ => return false,
            }
            self.i += 1;
            self.skip_no_newline();
        }
        true
    }

    fn skip_no_newline(&mut self) {
        while self.at().starts_with("\\ ") {
            self.i += 1;
        }
    }
}

/// `@@ -old[,count] +new[,count] @@` — a missing count means 1, exactly as
/// `diff -u` writes it. Everything after the closing "@@" is ignored, like the
/// Go regexp anchor.
fn parse_hunk_header(line: &str) -> Option<(i64, i64)> {
    let rest = line.strip_prefix("@@ -")?;
    let (old, rest) = split_count(rest)?;
    let rest = rest.strip_prefix(' ')?;
    let rest = rest.strip_prefix('+')?;
    let (new, rest) = split_count(rest)?;
    if !rest.starts_with(" @@") {
        return None;
    }
    Some((old, new))
}

/// One number, optionally `,count`; a bare position implies count 1. A
/// non-numeric count rejects the header (Go's regexp would not match).
fn split_count(rest: &str) -> Option<(i64, &str)> {
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse::<i64>().ok()?;
    let rest = &rest[end..];
    if let Some(count_part) = rest.strip_prefix(',') {
        let cend = count_part
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(count_part.len());
        let count: i64 = count_part[..cend].parse().ok()?;
        Some((count, &count_part[cend..]))
    } else {
        Some((1, rest))
    }
}

/// Metadata lines git writes between its "diff --git" line and the
/// "--- "/"+++ " pair. Each is optional and carries no content.
fn is_git_extended_header(line: &str) -> bool {
    [
        "old mode ",
        "new mode ",
        "deleted file mode ",
        "new file mode ",
        "copy from ",
        "copy to ",
        "rename from ",
        "rename to ",
        "similarity index ",
        "dissimilarity index ",
        "index ",
    ]
    .iter()
    .any(|pre| line.starts_with(pre))
}

/// Metadata-line prefixes git writes between a "commit …" line and the blank
/// line before its indented message.
fn is_log_header_field(line: &str) -> bool {
    [
        "Author:",
        "AuthorDate:",
        "Commit:",
        "CommitDate:",
        "Date:",
        "Merge:",
    ]
    .iter()
    .any(|pre| line.starts_with(pre))
}

#[cfg(test)]
mod tests {
    use super::is_unified_diff;

    const SAMPLE_GIT_DIFF: &str = "diff --git a/x.go b/x.go\n\
         index 112294521..f9fdd2c37 100644\n\
         --- a/x.go\n\
         +++ b/x.go\n\
         @@ -1 +1 @@\n\
         -old\n\
         +new\n";

    // The default `git show` / `git log -p` shape: a commit header and its
    // indented message, then the file sections.
    const SAMPLE_SHOW: &str = "commit 0f2a1b9c\n\
         Author: A <a@b>\n\
         Date:   Mon Jan 1 00:00:00 2024 +0000\n\
         \n\
         \x20   subject line\n\
         \n\
         \x20   body paragraph\n\
         \n";

    // Ported: internal/base/diff/unified_test.go TestIsUnifiedDiff.
    #[test]
    fn recognizes_the_grammars_the_go_oracle_pins() {
        let cases: Vec<(&str, String, bool)> = vec![
            ("git-style whole diff", SAMPLE_GIT_DIFF.to_owned(), true),
            (
                "two files",
                format!(
                    "{SAMPLE_GIT_DIFF}diff --git a/y.go b/y.go\n--- a/y.go\n+++ b/y.go\n@@ -1 +1 @@\n-a\n+b\n"
                ),
                true,
            ),
            (
                "leading blank lines are skipped",
                format!("\n\n{SAMPLE_GIT_DIFF}"),
                true,
            ),
            ("trailing newline", format!("{SAMPLE_GIT_DIFF}\n"), true),
            (
                "--- style header",
                "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n".to_owned(),
                true,
            ),
            (
                "no-newline marker",
                "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n\\ No newline at end of file\n+new\n\\ No newline at end of file\n".to_owned(),
                true,
            ),
            (
                "zero-count hunk",
                "--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n-a\n-b\n+c\n".to_owned(),
                true,
            ),
            ("git show default format", format!("{SAMPLE_SHOW}{SAMPLE_GIT_DIFF}"), true),
            (
                "git show without a message",
                format!("commit 0f2a1b9c\nAuthor: A <a@b>\nDate:   now\n\n{SAMPLE_GIT_DIFF}"),
                true,
            ),
            (
                "git log -p two commits",
                format!(
                    "{SAMPLE_SHOW}{SAMPLE_GIT_DIFF}commit 1234567\nAuthor: B <b@b>\nDate:   later\n\n    second\n\n{SAMPLE_GIT_DIFF}"
                ),
                true,
            ),
            (
                "git log -p with an empty commit",
                format!("{SAMPLE_SHOW}{SAMPLE_GIT_DIFF}commit 89abcde\nAuthor: C <c@c>\nDate:   later\n\n    empty\n\n"),
                true,
            ),
            ("empty", String::new(), false),
            ("blank only", "\n \n".to_owned(), false),
            (
                "plain prose",
                "Replaced all 15 em dashes in README.md.\n".to_owned(),
                false,
            ),
            ("mixed output", format!("building...\n{SAMPLE_GIT_DIFF}"), false),
            (
                "diff then a command",
                format!("{SAMPLE_GIT_DIFF}ok  \tgithub.com/x/y\t0.02s\n"),
                false,
            ),
            (
                "diff then prose",
                format!("{SAMPLE_GIT_DIFF}That changed the helper.\n"),
                false,
            ),
            (
                "diff then a truncation note",
                format!("{SAMPLE_GIT_DIFF}…(512 more chars truncated)"),
                false,
            ),
            (
                "prose faking headers",
                "--- Summary ---\n@@ x @@\n- a\n+ b\n".to_owned(),
                false,
            ),
            (
                "git log header without a diff",
                "commit 0f2a1b9c\nAuthor: A <a@b>\nDate:   now\n\n    only a message\n".to_owned(),
                false,
            ),
            (
                "git show --stat, no hunk",
                "commit 0f2a1b9c\nAuthor: A <a@b>\nDate:   now\n\n    subject\n\n x.go | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n".to_owned(),
                false,
            ),
            (
                "commit line without header fields",
                format!("commit 0f2a1b9c\nnot a header field\n\n    subject\n\n{SAMPLE_GIT_DIFF}"),
                false,
            ),
            (
                "stat only, no hunk",
                "diff --git a/x b/x\n x | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n".to_owned(),
                false,
            ),
            (
                "hunk but no changed line",
                "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -1 +1 @@\n same\n".to_owned(),
                false,
            ),
            (
                "no hunk",
                "diff --git a/x b/x\n--- a/x\n+++ b/x\n-old\n+new\n".to_owned(),
                false,
            ),
            (
                "hunk count too short",
                "--- a/x\n+++ b/x\n@@ -1,3 +1,3 @@\n-a\n+b\n".to_owned(),
                false,
            ),
            (
                "hunk count too long",
                "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\n+b\n+c\n".to_owned(),
                false,
            ),
            (
                "hunk overshoots its removed count",
                "--- a/x\n+++ b/x\n@@ -1 +1,2 @@\n-a\n-a\n+b\n+b\n".to_owned(),
                false,
            ),
            (
                "hunk overshoots its added count",
                "--- a/x\n+++ b/x\n@@ -1,2 +1 @@\n-a\n+a\n+a\n".to_owned(),
                false,
            ),
            (
                "--- without +++",
                "--- a/x\n@@ -1 +1 @@\n-a\n+b\n".to_owned(),
                false,
            ),
            (
                "garbage in body",
                "--- a/x\n+++ b/x\n@@ -1 +1 @@\n-a\nnot a diff line\n".to_owned(),
                false,
            ),
        ];
        for (name, input, want) in cases {
            assert_eq!(is_unified_diff(&input), want, "case {name:?}");
        }
    }
}
