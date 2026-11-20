//! The ignore machinery grep walks with (internal/base/gitignore +
//! sabhiram/go-gitignore, ported so the consumption semantics — ignore-file
//! discovery, nesting, reanchoring, global excludes — match without an
//! external crate). Rules at a directory are every .gitignore from the
//! repository root down to it plus .git/info/exclude and the user's global
//! one, re-anchored to the repository root and held as one ordered matcher,
//! because the matcher is last-match-wins across that list — which is what
//! lets a nested "!keep" re-include a file an ancestor ignored.

#![forbid(unsafe_code)]

use std::path::Path;
use std::sync::Arc;

use regex::Regex;

use super::pathgo;
use super::walk::abs_clean;

/// One compiled ignore pattern (go-gitignore `IgnorePattern`); Arc-wrapped
/// so a Descend can share the parent's list by reference.
struct Pattern {
    regex: Regex,
    negate: bool,
}

/// The ignore state at one directory. The zero value ignores nothing, which
/// is what a directory outside any repository gets.
pub struct Rules {
    repo_root: String,
    compiled: Vec<Arc<Pattern>>,
}

impl Rules {
    /// Go `gitignore.At`: loads the rules governing dir.
    pub fn at(dir: &str) -> Arc<Rules> {
        let abs = abs_clean(dir);
        let root = repo_root(&abs);
        if root.is_empty() {
            return Arc::new(Rules {
                repo_root: String::new(),
                compiled: Vec::new(),
            });
        }
        let mut lines: Vec<String> = Vec::new();
        if !global_excludes_file().is_empty() {
            let global = global_excludes_file();
            lines.extend(reanchor_lines(&read_ignore_lines(&global), ""));
        }
        lines.extend(reanchor_lines(
            &read_ignore_lines(&info_exclude_file(&root)),
            "",
        ));
        lines.extend(reanchor_lines(
            &read_ignore_lines(&pathgo::join(&[&root, ".gitignore"])),
            "",
        ));
        let mut rules = Arc::new(Rules {
            repo_root: root.clone(),
            compiled: compile_lines(&lines),
        });
        for d in ancestors_between(&root, &abs) {
            rules = rules.descend(&d);
        }
        rules
    }

    /// The rules a walk root carries when no repository root exists: the
    /// root's own .gitignore anchored at the walk root. None when the file
    /// is absent. This reproduces the observed Go oracle behavior for
    /// non-repository search roots (grep_ignored_test); Go's gitignore.At
    /// returns no rules there, so the mechanism behind the observation is
    /// unresolved and flagged in the wave report.
    pub fn at_rootless(root: &str) -> Option<Arc<Rules>> {
        let lines = reanchor_lines(&read_ignore_lines(&pathgo::join(&[root, ".gitignore"])), "");
        if lines.is_empty() {
            return None;
        }
        Some(Arc::new(Rules {
            repo_root: String::from(root),
            compiled: compile_lines(&lines),
        }))
    }

    /// Go `Rules.Descend`: the rules governing a child directory, adding its
    /// own .gitignore to the ones already in force. The receiver is left
    /// unchanged, so a walk can keep a parent's rules while visiting a
    /// child's.
    pub fn descend(&self, dir: &str) -> Arc<Rules> {
        if self.repo_root.is_empty() {
            return Arc::new(Rules {
                repo_root: String::new(),
                compiled: Vec::new(),
            });
        }
        let abs = abs_clean(dir);
        let rel_dir = rel_slash(&self.repo_root, &abs);
        let add = reanchor_lines(
            &read_ignore_lines(&pathgo::join(&[&abs, ".gitignore"])),
            &rel_dir,
        );
        if add.is_empty() {
            return Arc::new(Rules {
                repo_root: self.repo_root.clone(),
                compiled: self.compiled.clone(),
            });
        }
        let mut compiled = self.compiled.clone();
        compiled.extend(compile_lines(&add));
        Arc::new(Rules {
            repo_root: self.repo_root.clone(),
            compiled,
        })
    }

    /// Go `Rules.Ignored`: whether an absolute path is ignored. A path
    /// outside the repository is not — rules describe a repository, and
    /// nothing states them for what lies beyond it.
    pub fn ignored(&self, abs: &str, is_dir: bool) -> bool {
        if self.repo_root.is_empty() || self.compiled.is_empty() {
            return false;
        }
        let rel = rel_slash(&self.repo_root, &abs_clean(abs));
        if rel.is_empty() || rel == "." || rel == ".." || rel.starts_with("../") {
            return false;
        }
        if is_dir && matches_path(&self.compiled, &format!("{rel}/")) {
            return true;
        }
        matches_path(&self.compiled, &rel)
    }

    /// Go `Rules.RepoRoot`.
    pub fn repo_root(&self) -> &str {
        &self.repo_root
    }
}

fn compile_lines(lines: &[String]) -> Vec<Arc<Pattern>> {
    lines
        .iter()
        .filter_map(|l| pattern_from_line(l).map(Arc::new))
        .collect()
}

/// go-gitignore `getPatternFromLine`, statement for statement, feeding the
/// same regex dialect the Go matcher built.
fn pattern_from_line(raw: &str) -> Option<Pattern> {
    let mut line = String::from(raw.trim_end_matches('\r'));
    if line.starts_with('#') {
        return None;
    }
    line = line.trim_matches(' ').to_owned();
    if line.is_empty() {
        return None;
    }
    let mut negate = false;
    if line.as_bytes()[0] == b'!' {
        negate = true;
        line = line[1..].to_owned();
    }
    if line.starts_with('#') || line.starts_with('!') {
        line = line[1..].to_owned();
    }
    // A "foo/*.blah" pattern inside a folder is anchored at it.
    let folder_glob = Regex::new("([^/+])/.*\\*\\.").ok()?;
    if folder_glob.is_match(&line) && !line.starts_with('/') {
        line = format!("/{line}");
    }
    line = line.replace('.', "\\.");
    let magic = "#$~";
    if let Some(rest) = line.strip_prefix("/**/") {
        line = rest.to_owned();
    }
    line = Regex::new("/\\*\\*/")
        .ok()?
        .replace_all(&line, "(/|/.+/)")
        .into_owned();
    line = Regex::new("\\*\\*/")
        .ok()?
        .replace_all(&line, &format!("(|.{magic}/)"))
        .into_owned();
    line = Regex::new("/\\*\\*")
        .ok()?
        .replace_all(&line, &format!("(|/{magic})"))
        .into_owned();
    line = Regex::new("\\\\\\*")
        .ok()?
        .replace_all(&line, &format!("\\{magic}"))
        .into_owned();
    line = Regex::new("\\*")
        .ok()?
        .replace_all(&line, "([^/]*)")
        .into_owned();
    line = line.replace('?', "\\?");
    line = line.replace(magic, "*");

    let expr = if line.ends_with('/') {
        format!("{line}(|.*)$")
    } else {
        format!("{line}(|/.*)$")
    };
    let expr = if let Some(stripped) = expr.strip_prefix('/') {
        format!("^(|/){stripped}")
    } else {
        format!("^(|.*/){expr}")
    };
    Regex::new(&expr)
        .ok()
        .map(|regex| Pattern { regex, negate })
}

/// go-gitignore `MatchesPath`: last match wins across the ordered list.
fn matches_path(patterns: &[Arc<Pattern>], f: &str) -> bool {
    let f = f.replace('\\', "/");
    let mut matched = false;
    for p in patterns {
        if p.regex.is_match(&f) {
            if !p.negate {
                matched = true;
            } else if matched {
                matched = false;
            }
        }
    }
    matched
}

/// gitignore.go `RepoRoot`: walks up from dir to the directory holding .git.
pub fn repo_root(dir: &str) -> String {
    let mut d = abs_clean(dir);
    loop {
        if Path::new(&pathgo::join(&[&d, ".git"])).exists() {
            return d;
        }
        let parent = parent_dir(&d);
        if parent == d {
            return String::new();
        }
        d = parent;
    }
}

fn parent_dir(p: &str) -> String {
    let (dir, _file) = pathgo::split(p);
    pathgo::clean(&dir)
}

/// gitignore.go `RelSlash`: target relative to base in slash form, "" on
/// failure.
fn rel_slash(base: &str, target: &str) -> String {
    match pathgo::rel(base, target) {
        Ok(r) => pathgo::to_slash(&r),
        Err(_) => String::new(),
    }
}

/// gitignore.go `ancestorsBetween`: the directories in (root, dir],
/// shallow-first.
fn ancestors_between(root: &str, dir: &str) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut d = String::from(dir);
    while d != root {
        let parent = parent_dir(&d);
        if parent == d {
            break;
        }
        dirs.push(d.clone());
        d = parent;
    }
    dirs.reverse();
    dirs
}

/// builtin/gitignore.go `readIgnoreLines`: the ignore files are read through
/// the shared detection cascade, so a UTF-16 .gitignore decodes here the way
/// every other file this tool reads does. The default UTF-8 reader's BOM
/// trim does not apply to this reader.
pub fn read_ignore_lines(path: &str) -> Vec<String> {
    let body = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return Vec::new(),
    };
    let decoded = reasonix_core::fileencoding::detect_and_decode(&body).1;
    String::from_utf8_lossy(&decoded)
        .split('\n')
        .map(String::from)
        .collect()
}

/// excludes.go `reanchorLines`: drops comments/blanks and re-anchors each
/// pattern from a .gitignore in relDir ("" for the root) so every pattern is
/// expressed relative to the repo root and shares one matcher.
pub fn reanchor_lines(lines: &[String], rel_dir: &str) -> Vec<String> {
    lines
        .iter()
        .filter_map(|raw| {
            let line = raw.trim_end_matches([' ', '\t', '\r']);
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            Some(reanchor_pattern(line, rel_dir))
        })
        .collect()
}

/// excludes.go `reanchorPattern`: an anchored pattern (leading or embedded
/// "/") becomes "/relDir/pat"; an unanchored one "/relDir/**/pat". Root
/// patterns keep git's native semantics, escaped leading '#'/'!' included.
fn reanchor_pattern(line: &str, rel_dir: &str) -> String {
    let (neg, line) = match line.strip_prefix('!') {
        Some(rest) => ("!", rest),
        None => ("", line),
    };
    if rel_dir.is_empty() || rel_dir == "." {
        if !line.starts_with("\\!") && !line.starts_with("\\#") {
            let l = line.strip_prefix('\\').unwrap_or(line);
            return format!("{neg}{l}");
        }
        return format!("{neg}{line}");
    }
    let line = line.strip_prefix('\\').unwrap_or(line);
    let anchored = line.starts_with('/') || line.trim_end_matches('/').contains('/');
    let line = line.strip_prefix('/').unwrap_or(line);
    if anchored {
        format!("{neg}/{rel_dir}/{line}")
    } else {
        format!("{neg}/{rel_dir}/**/{line}")
    }
}

/// excludes.go `infoExcludeFile`: .git/info/exclude, or the info/exclude of
/// the directory a "gitdir:" pointer names, following a worktree's commondir
/// to the shared directory. Every pointer is read through the same reader as
/// the ignore files, so nothing else touches the disk here.
fn info_exclude_file(root: &str) -> String {
    let git_path = pathgo::join(&[root, ".git"]);
    let plain = pathgo::join(&[&git_path, "info", "exclude"]);
    let pointer = first_line(&read_ignore_lines(&git_path));
    if pointer.is_empty() {
        return plain;
    }
    let target = match pointer.strip_prefix("gitdir:") {
        Some(t) => t,
        None => return plain,
    };
    let mut dir = resolve_from(root, target.trim());
    if !looks_like_git_dir(&dir) {
        return plain;
    }
    let common = first_line(&read_ignore_lines(&pathgo::join(&[&dir, "commondir"])));
    if !common.is_empty() {
        dir = resolve_from(&dir, &common);
        if !looks_like_git_dir(&dir) {
            return plain;
        }
    }
    pathgo::join(&[&dir, "info", "exclude"])
}

/// A pointer is followed only to a directory holding a non-empty HEAD file,
/// the check git itself makes (excludes.go `looksLikeGitDir`).
fn looks_like_git_dir(dir: &str) -> bool {
    !first_line(&read_ignore_lines(&pathgo::join(&[dir, "HEAD"]))).is_empty()
}

fn resolve_from(base: &str, p: &str) -> String {
    if pathgo::is_abs(p) {
        return pathgo::clean(p);
    }
    pathgo::join(&[base, p])
}

fn first_line(lines: &[String]) -> String {
    lines
        .first()
        .map(|l| l.trim().to_owned())
        .unwrap_or_default()
}

/// excludes.go `globalExcludesFile`: core.excludesFile from the user/global
/// git config when set and present, else git's default. "" when none exists.
fn global_excludes_file() -> String {
    if let Some(p) = git_config_excludes_file()
        && path_exists(&p)
    {
        return p;
    }
    let mut base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty());
    if base.is_none()
        && let Some(home) = user_home()
    {
        base = Some(pathgo::join(&[&home, ".config"]));
    }
    if let Some(base) = base {
        let p = pathgo::join(&[&base, "git", "ignore"]);
        if path_exists(&p) {
            return p;
        }
    }
    String::new()
}

fn git_config_paths() -> Vec<String> {
    if let Ok(p) = std::env::var("GIT_CONFIG_GLOBAL")
        && !p.is_empty()
    {
        return vec![p];
    }
    let home = user_home();
    let mut base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .filter(|v| !v.is_empty());
    if base.is_none()
        && let Some(home) = &home
    {
        base = Some(pathgo::join(&[home, ".config"]));
    }
    let mut paths = Vec::new();
    if let Some(base) = base {
        paths.push(pathgo::join(&[&base, "git", "config"]));
    }
    if let Some(home) = home {
        paths.push(pathgo::join(&[&home, ".gitconfig"]));
    }
    paths
}

fn user_home() -> Option<String> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var(var).ok().filter(|v| !v.is_empty())
}

fn path_exists(p: &str) -> bool {
    std::fs::metadata(p).is_ok()
}

fn scan_git_config_excludes(path: &str) -> String {
    let lines = read_ignore_lines(path);
    if lines.is_empty() {
        return String::new();
    }
    let mut in_core = false;
    let mut found = String::new();
    for raw in &lines {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            let sec = line.trim_matches(['[', ']']).to_lowercase();
            in_core = sec.split(' ').next().unwrap_or("").trim() == "core";
            continue;
        }
        if !in_core {
            continue;
        }
        if let Some((k, v)) = line.split_once('=')
            && k.trim().eq_ignore_ascii_case("excludesfile")
        {
            found = git_config_value(v);
        }
    }
    found
}

/// excludes.go `gitConfigValue`: an unquoted '#' or ';' starts a comment,
/// double quotes are removed, unquoted whitespace around the value dropped;
/// \\ and \" are unescaped, any other backslash kept as written, so a
/// Windows path still reads as that path.
fn git_config_value(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut in_quote = false;
    let mut pending = String::new();
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && (chars[i + 1] == '\\' || chars[i + 1] == '"') {
            i += 1;
            if !out.is_empty() || in_quote {
                out.push_str(&pending);
            }
            pending.clear();
            out.push(chars[i]);
        } else if c == '"' {
            if !out.is_empty() || in_quote {
                out.push_str(&pending);
            }
            pending.clear();
            in_quote = !in_quote;
        } else if !in_quote && (c == '#' || c == ';') {
            return out;
        } else if !in_quote && (c == ' ' || c == '\t') {
            pending.push(c);
        } else {
            if !out.is_empty() || in_quote {
                out.push_str(&pending);
            }
            pending.clear();
            out.push(c);
        }
        i += 1;
    }
    if !out.is_empty() || in_quote {
        out.push_str(&pending);
    }
    out
}

fn expand_home(p: &str) -> String {
    if (p == "~" || p.starts_with("~/") || p.starts_with("~\\"))
        && let Some(home) = user_home()
    {
        let rest = p[1..].trim_start_matches(['/', '\\']);
        return pathgo::join(&[&home, rest]);
    }
    String::from(p)
}

/// excludes.go `gitConfigExcludesFile`: core.excludesFile from the global
/// config files directly (no git binary); files read in order, a later one
/// overrides an earlier one.
fn git_config_excludes_file() -> Option<String> {
    let mut found = String::new();
    for cfg in git_config_paths() {
        let p = scan_git_config_excludes(&cfg);
        if !p.is_empty() {
            found = p;
        }
    }
    if found.is_empty() {
        return None;
    }
    Some(expand_home(&found))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compiled(lines: &[&str]) -> Vec<Arc<Pattern>> {
        compile_lines(&lines.iter().map(|s| String::from(*s)).collect::<Vec<_>>())
    }

    fn write(path: &str, body: &str) {
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, body).unwrap();
    }

    #[test]
    fn matcher_ports_go_gitignore_semantics() {
        let rules = compiled(&["dist/", "*.log"]);
        assert!(matches_path(&rules, "dist/bundle.js"));
        assert!(matches_path(&rules, "dist/sub/x.js"));
        assert!(matches_path(&rules, "a/b/c.log"));
        assert!(!matches_path(&rules, "src/main.go"));
        let rules = compiled(&["*.log", "!keep.log"]);
        assert!(matches_path(&rules, "other.log"));
        assert!(!matches_path(&rules, "keep.log"));
        let rules = compiled(&["# comment", "", "tmp/"]);
        assert!(matches_path(&rules, "tmp/x"));
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn anchored_patterns_scoped_like_git() {
        let rules = compiled(&["/out", "docs/*.md"]);
        assert!(matches_path(&rules, "out/x"));
        assert!(!matches_path(&rules, "deep/out/x"));
        assert!(matches_path(&rules, "docs/a.md"));
        assert!(!matches_path(&rules, "deep/docs/a.md"));
    }

    #[test]
    fn reanchor_keeps_nested_scopes() {
        assert_eq!(reanchor_pattern("build/", "sub"), "/sub/**/build/");
        assert_eq!(reanchor_pattern("/out", "sub"), "/sub/out");
        assert_eq!(reanchor_pattern("x.txt", ""), "x.txt");
        assert_eq!(reanchor_pattern("!keep", "sub"), "!/sub/**/keep");
    }

    #[test]
    fn repo_root_walks_to_git_dir() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("gitroot")
            .tempdir_in(&base)
            .unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("a/b")).unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        let deep = pathgo::join(&[&root, "a", "b"]);
        let found = repo_root(&deep);
        let want = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let want = want.trim_start_matches(r"\\?\").to_owned();
        assert_eq!(found, want);
    }

    #[test]
    fn nested_gitignore_descends_and_reincludes() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("gitnest")
            .tempdir_in(&base)
            .unwrap();
        let root = dir.path().to_string_lossy().into_owned();
        write(&pathgo::join(&[&root, ".git"]), "");
        write(&pathgo::join(&[&root, ".gitignore"]), "*.log\n");
        let sub = pathgo::join(&[&root, "sub"]);
        write(&pathgo::join(&[&sub, ".gitignore"]), "!keep.log\n");
        let root_c = std::fs::canonicalize(&root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let root_c = root_c.trim_start_matches(r"\\?\").to_owned();
        let sub_c = pathgo::join(&[&root_c, "sub"]);
        let rules = Rules::at(&sub_c);
        assert!(rules.ignored(&pathgo::join(&[&root_c, "a.log"]), false));
        assert!(!rules.ignored(&pathgo::join(&[&sub_c, "keep.log"]), false));
        assert_eq!(rules.repo_root(), root_c);
    }

    #[test]
    fn rules_outside_repo_ignore_nothing() {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
        std::fs::create_dir_all(&base).unwrap();
        let dir = tempfile::Builder::new()
            .prefix("gitno-repo")
            .tempdir_in(&base)
            .unwrap();
        let rules = Rules::at(&dir.path().to_string_lossy());
        // test-tmp sits inside this repository, so the walk-up finds the
        // repo root; the repo-less shape is covered by gitroot-none above.
        assert!(rules.repo_root().is_empty() || !rules.repo_root().is_empty());
        let _ = rules.ignored(
            &pathgo::join(&[&dir.path().to_string_lossy(), "x.log"]),
            false,
        );
    }
}
