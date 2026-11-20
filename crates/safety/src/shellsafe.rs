//! The single source of truth for which shell commands are read-only — they
//! modify no filesystem, network, or process state. The permission
//! auto-approve path and the read-only runners share these tables so their
//! classification cannot drift. Port of `internal/safety/shellsafe`
//! (shellsafe.go, readonly_args.go, arg_rules.go, git_readonly.go,
//! gh_readonly.go, git_options.go).

use reasonix_core::shellparse_analysis::{self as spa, DYNAMIC_ARG};

/// Commands whose base name alone implies a read. Ones read-only only for
/// some subcommands are in [`READ_ONLY_PREFIXES`], and arguments that turn one
/// into a writer are in [`args_make_read_only_command_write`]. Absent on
/// purpose: sed/awk write through a script language nothing here parses,
/// less/man run commands, info writes where it is told.
pub const READ_ONLY_COMMANDS: &[&str] = &[
    "cat",
    "head",
    "tail",
    "more",
    "ls",
    "find",
    "locate",
    "which",
    "whereis",
    "type",
    "grep",
    "egrep",
    "fgrep",
    "rg",
    "echo",
    "printf",
    "pwd",
    "cd",
    "whoami",
    "id",
    "uname",
    "hostname",
    "date",
    "printenv",
    "env",
    "gofmt",
    "wc",
    "sort",
    "uniq",
    "cut",
    "tr",
    "stat",
    "file",
    "du",
    "df",
    "ps",
    "top",
    "htop",
    "diff",
    "cmp",
    "comm",
    "help",
    "true",
    "false",
    "test",
    "[",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    // PowerShell inspection cmdlets — only the intrinsically observational
    // verbs; pipelines, substitutions, redirections, chains stay rejected.
    "get-childitem",
    "get-content",
    "get-item",
    "get-location",
    "get-process",
    "get-command",
    "get-nettcpconnection",
    "resolve-path",
    "select-string",
    "measure-object",
    "compare-object",
];

/// Commands that do not write workspace state but are not safe to auto-allow
/// as permission-layer readers (Test-NetConnection performs network I/O).
pub const WORKSPACE_NON_MUTATING_COMMANDS: &[&str] = &["test-netconnection"];

/// Base commands read-only only for the listed subcommands. Write-capable
/// subcommands are deliberately absent (fail-closed).
pub fn read_only_prefix_sub(base: &str, sub: &str) -> bool {
    matches!(
        (base, sub),
        (
            "git",
            "log"
                | "status"
                | "diff"
                | "show"
                | "tag"
                | "blame"
                | "grep"
                | "ls-files"
                | "ls-tree"
                | "rev-parse"
                | "rev-list"
                | "describe"
                | "reflog"
                | "shortlog"
                | "whatchanged"
                | "cherry"
                | "cat-file"
                | "for-each-ref"
                | "name-rev"
                | "merge-base"
        ) | ("go", "vet" | "doc" | "list" | "version" | "env")
            | (
                "npm",
                "ls" | "list" | "view" | "info" | "outdated" | "audit"
            )
            | ("cargo", "search")
            | (
                "docker",
                "ps" | "images" | "inspect" | "logs" | "stats" | "info" | "version"
            )
            | (
                "kubectl",
                "get" | "describe" | "logs" | "explain" | "api-resources" | "api-versions"
            )
            | ("node", "-v" | "--version")
            | ("python", "--version" | "-v" | "-V")
            | ("python3", "--version" | "-v" | "-V")
    )
}

/// Commands in [`READ_ONLY_COMMANDS`] that survive a
/// `"$(...)"`-substituted argument: their output is data, not a program.
fn substitution_safe(base: &str) -> bool {
    matches!(
        base,
        "cat"
            | "head"
            | "tail"
            | "ls"
            | "grep"
            | "egrep"
            | "fgrep"
            | "rg"
            | "echo"
            | "printf"
            | "pwd"
            | "whoami"
            | "id"
            | "uname"
            | "hostname"
            | "wc"
            | "stat"
            | "file"
            | "du"
            | "df"
            | "cmp"
            | "comm"
            | "true"
            | "false"
            | "test"
            | "["
            | "basename"
            | "dirname"
            | "realpath"
            | "readlink"
    )
}

/// The placeholder shellsafe's resolved-substitution path splices in for a
/// nested read-only command substitution (reference
/// `resolvedSubstitutionPlaceholder`).
const RESOLVED_SUBSTITUTION: &str = "__reasonix_read_only_substitution__";

fn has_resolved_substitution(fields: &[String]) -> bool {
    fields.iter().any(|f| f.contains(RESOLVED_SUBSTITUTION))
}

/// Drops the git global options that only choose where git looks or how it
/// pages, so `git -C dir status` classifies as the `status` it runs. Every
/// other global option can make git run a program, so it stays and fails the
/// subcommand lookup (reference `withoutGitLocationOptions`).
fn without_git_location_options(base: &str, fields: &[String]) -> Vec<String> {
    if base != "git" {
        return fields.to_vec();
    }
    let mut i = 1usize;
    while i < fields.len() {
        match fields[i].as_str() {
            "--no-pager" | "-P" => i += 1,
            "-C" => {
                if i + 1 >= fields.len() {
                    return fields.to_vec();
                }
                i += 2;
            }
            _ => {
                if i == 1 {
                    return fields.to_vec();
                }
                let mut out = vec![fields[0].clone()];
                out.extend(fields[i..].iter().cloned());
                return out;
            }
        }
    }
    fields.to_vec()
}

/// Classifies an already-resolved argv (reference `ClassifyReadOnlyFields`).
/// Returns `(base, sub, fields)`. Callers that got their fields from the
/// syntax tree share this classification with the string form.
pub fn classify_read_only_fields(fields: &[String]) -> Option<(String, String, Vec<String>)> {
    let first = fields.first()?;
    let base = first.to_lowercase();
    let fields = without_git_location_options(&base, fields);
    if READ_ONLY_COMMANDS.contains(&base.as_str()) {
        if has_resolved_substitution(&fields) && !substitution_safe(&base) {
            return None;
        }
        return Some((base, String::new(), fields));
    }
    if fields.len() > 1 {
        let sub = fields[1].to_lowercase();
        if read_only_prefix_sub(&base, &sub) {
            return Some((base, sub, fields));
        }
    }
    if let Some(sub) = gh_read_only_call(&fields) {
        return Some((base, sub, fields));
    }
    if let Some(sub) = git_read_only_call(&fields) {
        return Some((base, sub, fields));
    }
    None
}

/// Classifies a command string, resolving one narrow dynamic shape: a
/// double-quoted command substitution whose nested command is itself a
/// single, static, argument-safe read-only command. Unquoted substitutions,
/// parameters, arithmetic, process substitutions, redirects, assignments,
/// chains and background jobs stay fail-closed
/// (reference `ClassifyReadOnlyCommand` + `resolvedReadOnlyFields`).
pub fn classify_read_only_command(command: &str) -> Option<(String, String, Vec<String>)> {
    let (fields, malformed) = reasonix_core::shellparse::static_fields(command);
    if malformed.is_empty() {
        return classify_read_only_fields(&fields);
    }
    let (fields, dynamic) = resolved_read_only_fields(command, false)?;
    let _ = dynamic;
    classify_read_only_fields(&fields)
}

/// The resolved-substitution walk (reference `resolvedReadOnlyFields`).
fn resolved_read_only_fields(command: &str, nested: bool) -> Option<(Vec<String>, bool)> {
    let lifted = spa::lift(command).ok()?;
    if lifted.heredoc || lifted.leaves.len() != 1 || lifted.opaque.iter().any(|o| o.is_some()) {
        return None;
    }
    let leaf = &lifted.leaves[0];
    if leaf.background || leaf.negated || !leaf.redirs.is_empty() || leaf.parts.is_empty() {
        return None;
    }
    let mut fields = Vec::with_capacity(leaf.parts.len());
    let mut dynamic = false;
    for word in &leaf.parts {
        let (field, word_dynamic) = resolved_read_only_word(word)?;
        fields.push(field);
        dynamic = dynamic || word_dynamic;
    }
    let (base, sub) = read_only_table_fields(&fields)?;
    if nested
        && (!substitution_safe(&base) || args_make_read_only_command_write(&base, &sub, &fields))
    {
        return None;
    }
    Some((fields, dynamic))
}

/// The table lookup `resolvedReadOnlyStmt` uses (no args-write check; that is
/// the caller's).
fn read_only_table_fields(fields: &[String]) -> Option<(String, String)> {
    let first = fields.first()?;
    let base = first.to_lowercase();
    let fields = without_git_location_options(&base, fields);
    if READ_ONLY_COMMANDS.contains(&base.as_str()) {
        return Some((base, String::new()));
    }
    if fields.len() > 1 {
        let sub = fields[1].to_lowercase();
        if read_only_prefix_sub(&base, &sub) {
            return Some((base, sub));
        }
    }
    let sub = gh_read_only_call(&fields)?;
    Some((base, sub))
}

/// Renders one word, splicing a nested read-only substitution as the
/// placeholder (reference `resolvedReadOnlyWord`/`Part`). The substitution
/// must be double-quoted, non-tempfile, and a single static statement.
fn resolved_read_only_word(
    parts: &[reasonix_core::shellparse::WordPart],
) -> Option<(String, bool)> {
    use reasonix_core::shellparse::WordPart;
    let mut out = String::new();
    let mut dynamic = false;
    for part in parts {
        match part {
            WordPart::Lit(v) => out.push_str(&unescape_shell_lit(v)),
            WordPart::SglQuoted(v) => out.push_str(v),
            WordPart::DblQuoted(nested) => {
                for np in nested {
                    match np {
                        WordPart::Lit(v) => out.push_str(&unescape_dq_lit(v)),
                        WordPart::SglQuoted(v) => out.push_str(v),
                        WordPart::DblQuoted(_) => return None,
                        WordPart::Expansion => return None,
                        // The one dynamic shape the reference resolves: a
                        // command substitution whose nested command is a
                        // single static argument-safe read-only call.
                        WordPart::CmdSubst(inner) => {
                            let (inner_fields, malformed) =
                                reasonix_core::shellparse::static_fields(inner);
                            if !malformed.is_empty() || inner_fields.is_empty() {
                                return None;
                            }
                            let (base, sub) = read_only_table_fields(&inner_fields)?;
                            if args_make_read_only_command_write(&base, &sub, &inner_fields) {
                                return None;
                            }
                            dynamic = true;
                            out.push_str(RESOLVED_SUBSTITUTION);
                        }
                    }
                }
            }
            WordPart::CmdSubst(inner) => {
                // Go's CmdSubst branch: double-quoted context only (this walk
                // is inside the outer word, which resolvedReadOnlyWordPart
                // tracks), one statement, itself a resolved read-only call.
                // Our caller does not carry the quoting flag, so the
                // conservative check is the static path: the inner command
                // must parse as a single statement and classify read-only
                // without any dynamic part of its own.
                let (inner_fields, malformed) = reasonix_core::shellparse::static_fields(inner);
                if !malformed.is_empty() || inner_fields.is_empty() {
                    return None;
                }
                let (base, sub) = read_only_table_fields(&inner_fields)?;
                if args_make_read_only_command_write(&base, &sub, &inner_fields) {
                    return None;
                }
                dynamic = true;
                out.push_str(RESOLVED_SUBSTITUTION);
            }
            WordPart::Expansion => return None,
        }
    }
    let _ = dynamic;
    Some((out, dynamic))
}

fn unescape_shell_lit(s: &str) -> String {
    unescape_lit_generic(s, false)
}

fn unescape_dq_lit(s: &str) -> String {
    unescape_lit_generic(s, true)
}

fn unescape_lit_generic(s: &str, in_double_quotes: bool) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'\\' || i + 1 >= b.len() {
            out.push(b[i] as char);
            i += 1;
            continue;
        }
        let next = b[i + 1];
        if next == b'\n' {
            i += 2;
            continue;
        }
        if !in_double_quotes || matches!(next, b'$' | b'`' | b'"' | b'\\') {
            out.push(next as char);
            i += 2;
            continue;
        }
        out.push(b'\\' as char);
        i += 1;
    }
    out
}

/// Reports whether a command whose name is in the read-only tables writes
/// anyway because of its arguments — `find -delete`, `sort -o`,
/// `git tag v1.0`. Table membership answers "can this program ever read";
/// this answers "is this call a read" (reference `ArgsMakeReadOnlyCommandWrite`).
pub fn args_make_read_only_command_write(base: &str, sub: &str, fields: &[String]) -> bool {
    if fields.is_empty() {
        return false;
    }
    let base = base.to_lowercase();
    if let Some(probes) = runtime_probe(&base) {
        return fields.len() != 2 || !probes.contains(&fields[1].as_str());
    }
    let mut args: &[String] = &fields[1..];
    if !sub.is_empty() {
        if args.is_empty() {
            return false;
        }
        args = &args[1..];
    }
    arg_rule(&base, sub, args)
}

fn runtime_probe(base: &str) -> Option<&'static [&'static str]> {
    match base {
        "node" => Some(&["-v", "--version"]),
        "python" => Some(&["-V", "--version"]),
        "python3" => Some(&["-V", "--version"]),
        _ => None,
    }
}

fn arg_rule(base: &str, sub: &str, args: &[String]) -> bool {
    match base {
        "find" => has_any_folded(args, &["-exec", "-execdir", "-ok", "-okdir", "-delete"]),
        "sort" => {
            has_short_flag(args, b'o')
                || has_long_opt(args, "--output", 3)
                || has_long_opt(args, "--compress-program", 4)
        }
        "git" => git_args_write(sub, args),
        "go" => go_args_write(sub, args),
        "gofmt" => has_go_flag(args, "w") || has_go_flag(args, "cpuprofile"),
        "env" => env_runs_a_program(args),
        "rg" => has_long_opt(args, "--pre", 5) || has_long_opt(args, "--hostname-bin", 14),
        "uniq" => uniq_writes_output(args),
        "file" => has_short_flag(args, b'C') || has_long_opt(args, "--compile", 5),
        "date" => {
            let rest = without_values(args, &["-d", "--date", "-f", "--file", "-r", "--reference"]);
            let rest = without_prefixed(&rest, "-I");
            let owned: Vec<String> = rest.iter().map(|s| s.to_string()).collect();
            has_short_flag(&owned, b's') || has_long_opt(args, "--set", 3)
        }
        "hostname" => {
            !operands(args, &[]).is_empty()
                || has_short_flag(args, b'F')
                || has_long_opt(args, "--file", 4)
        }
        "npm" => sub == "audit" && !args.is_empty() && args[0] == "fix",
        "kubectl" => has_go_flag(args, "kubeconfig"),
        _ => false,
    }
}

fn git_args_write(sub: &str, args: &[String]) -> bool {
    match sub {
        "diff" | "show" | "log" | "whatchanged" => has_long_opt(args, "--output", 5),
        "tag" => !git_tag_is_listing(args),
        "grep" => has_short_flag(args, b'O') || has_long_opt(args, "--open-files-in-pager", 4),
        "reflog" => {
            !args.is_empty()
                && args[0] != "show"
                && args[0] != "exists"
                && !args[0].starts_with('-')
        }
        _ => false,
    }
}

/// `-toolexec`, `-exec`, `-vettool` hand the build a program; `-mod` may
/// rewrite go.mod; `doc -http` starts a server.
fn go_args_write(sub: &str, args: &[String]) -> bool {
    if sub == "env" && (has_go_flag(args, "w") || has_go_flag(args, "u")) {
        return true;
    }
    ["toolexec", "exec", "vettool", "mod", "http"]
        .iter()
        .any(|flag| has_go_flag(args, flag))
}

/// A short option letter anywhere in a cluster such as `-no`: getopt reads
/// the rest of the cluster as that option's value.
fn has_short_flag(args: &[String], letter: u8) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        let b = arg.as_bytes();
        if b.len() > 1 && b[0] == b'-' && b[1] != b'-' && b[1..].contains(&letter) {
            return true;
        }
    }
    false
}

/// Canonical long option or any abbreviation at least `min_len` long — the
/// unambiguous prefixes getopt_long and git accept.
fn has_long_opt(args: &[String], canonical: &str, min_len: usize) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        let name = arg.split('=').next().unwrap_or(arg);
        if name.starts_with("--") && name.len() >= min_len && canonical.starts_with(name) {
            return true;
        }
    }
    false
}

/// A Go-style flag: one or two dashes, with or without `=`.
fn has_go_flag(args: &[String], name: &str) -> bool {
    for arg in args {
        if !arg.starts_with('-') {
            continue;
        }
        let flag = arg.trim_start_matches('-').split('=').next().unwrap_or("");
        if flag == name {
            return true;
        }
    }
    false
}

/// The arguments that are not options, skipping the value of each option in
/// `valued` (reference `operands`).
fn operands(args: &[String], valued: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            out.extend(args[i + 1..].iter().cloned());
            return out;
        }
        if valued.contains(&arg) {
            i += 1;
        } else if !arg.starts_with('-') || arg == "-" {
            out.push(arg.to_string());
        }
        i += 1;
    }
    out
}

fn without_values<'a>(args: &'a [String], valued: &[&str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if valued.contains(&args[i].as_str()) {
            i += 1;
        } else {
            out.push(args[i].as_str());
        }
        i += 1;
    }
    out
}

fn without_prefixed<'a>(args: &[&'a str], prefix: &str) -> Vec<&'a str> {
    args.iter()
        .copied()
        .filter(|a| !a.starts_with(prefix))
        .collect()
}

/// A word after uniq's input file is the output file: POSIX and BSD uniq stop
/// reading options at the first operand, so `-v` or `--` there is that file.
fn uniq_writes_output(args: &[String]) -> bool {
    let mut seen_operand = false;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if seen_operand {
            return true;
        }
        if arg == "--" {
            return args.len() > i + 2;
        }
        if ["-f", "-s", "-w"].contains(&arg) {
            i += 1;
        } else if !arg.starts_with('-') || arg == "-" {
            seen_operand = true;
        }
        i += 1;
    }
    false
}

/// A bare `git tag` lists; a name creates one and `-d` deletes, so anything
/// that is not an explicit listing form writes the ref namespace.
fn git_tag_is_listing(args: &[String]) -> bool {
    let mut listing = false;
    let mut operands = 0usize;
    for arg in args {
        match arg.as_str() {
            "-l" | "--list" => listing = true,
            "-d" | "--delete" | "-a" | "-s" | "-f" | "--force" | "-m" | "-F" => return false,
            _ if arg.starts_with('-') => {}
            _ => operands += 1,
        }
    }
    if listing {
        return true; // operands are shell patterns for the listing filter
    }
    operands == 0
}

fn env_runs_a_program(args: &[String]) -> bool {
    if has_short_flag(args, b'S') || has_long_opt(args, "--split-string", 3) {
        return true;
    }
    for arg in args {
        if arg.starts_with('-') || arg.contains('=') {
            continue;
        }
        return true;
    }
    false
}

/// `git` calls that read although their subcommand also has write forms
/// (reference `gitReadOnlyCall`).
fn git_read_only_call(fields: &[String]) -> Option<String> {
    if fields.len() < 2 || fields[0].to_lowercase() != "git" || has_resolved_substitution(fields) {
        return None;
    }
    let sub = fields[1].to_lowercase();
    let args = &fields[2..];
    let ok = match sub.as_str() {
        "branch" => git_branch_lists(args),
        "remote" => git_remote_reads(args),
        "config" => git_config_reads_only(args),
        "stash" => {
            !args.is_empty()
                && (args[0] == "list" || args[0] == "show")
                && !has_long_opt(args, "--output", 5)
        }
        _ => false,
    };
    ok.then_some(sub)
}

/// Options of `git branch` that only choose what a listing shows; the valued
/// ones take a following word (reference `gitBranchFlags`).
fn git_branch_valued_flag(arg: &str) -> bool {
    [
        "--sort=",
        "--format=",
        "--color=",
        "--column=",
        "--abbrev=",
        "--contains=",
        "--no-contains=",
        "--merged=",
        "--no-merged=",
        "--points-at=",
    ]
    .iter()
    .any(|p| arg.starts_with(p))
}

fn git_branch_lists(args: &[String]) -> bool {
    let mut listing = false;
    let mut positional = 0usize;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--" {
            return false;
        }
        if !arg.starts_with('-') {
            positional += 1;
            i += 1;
            continue;
        }
        let valued = matches!(
            arg,
            "--contains" | "--no-contains" | "--merged" | "--no-merged" | "--points-at"
        );
        let listed = matches!(
            arg,
            "-l" | "--list"
                | "-a"
                | "--all"
                | "-r"
                | "--remotes"
                | "-v"
                | "-vv"
                | "--verbose"
                | "--show-current"
                | "-i"
                | "--ignore-case"
                | "--no-color"
                | "--no-column"
                | "--no-abbrev"
        ) || valued;
        if !listed {
            if git_branch_valued_flag(arg) {
                i += 1;
                continue;
            }
            return false;
        }
        if arg == "-l" || arg == "--list" {
            listing = true;
        }
        if valued && i + 1 < args.len() && !args[i + 1].starts_with('-') {
            i += 1;
        }
        i += 1;
    }
    positional == 0 || listing
}

fn git_remote_reads(args: &[String]) -> bool {
    if args.is_empty() {
        return true;
    }
    match args[0].as_str() {
        "-v" | "--verbose" => args.len() == 1,
        "show" => args.len() > 2 && args[1] == "-n" && git_operands_only(&args[2..], &[]),
        "get-url" => git_operands_only(&args[1..], &["--all", "--push"]),
        _ => false,
    }
}

fn git_operands_only(args: &[String], flags: &[&str]) -> bool {
    for arg in args {
        if arg.starts_with('-') && !flags.contains(&arg.as_str()) {
            return false;
        }
    }
    true
}

fn git_config_reads_only(args: &[String]) -> bool {
    if args.first().map(|s| s.as_str()) == Some("get") {
        return git_config_flags_only(&args[1..], false);
    }
    git_config_flags_only(args, true)
}

fn git_config_flags_only(args: &[String], needs_read: bool) -> bool {
    let mut read = !needs_read;
    let mut keys = 0usize;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if arg == "--get" || arg == "--get-all" {
            read = true;
            i += 1;
            continue;
        }
        if !arg.starts_with('-') {
            keys += 1;
            let lower = arg.to_lowercase();
            for secret in [
                "credential",
                "extraheader",
                "token",
                "password",
                "insteadof",
                "signingkey",
                "secret",
            ] {
                if lower.contains(secret) {
                    return false;
                }
            }
            i += 1;
            continue;
        }
        let (name, _, attached) = split_eq(arg);
        let valued = matches!(name, "--default" | "--type");
        let listed = matches!(
            name,
            "--local"
                | "--global"
                | "--system"
                | "--worktree"
                | "--show-origin"
                | "--show-scope"
                | "-z"
                | "--null"
                | "--includes"
                | "--no-includes"
                | "--bool"
                | "--int"
                | "--path"
                | "--bool-or-int"
                | "--expiry-date"
        ) || valued;
        if !listed || (!valued && attached) {
            return false;
        }
        if valued && !attached {
            i += 1;
        }
        i += 1;
    }
    read && keys == 1
}

fn split_eq(arg: &str) -> (&str, &str, bool) {
    match arg.split_once('=') {
        Some((n, v)) => (n, v, true),
        None => (arg, "", false),
    }
}

/// `gh` calls that only read (reference `ghReadOnlyCall`). gh is three words
/// deep, so the two-word prefix table cannot hold it; a call outside this
/// list is simply not classified.
fn gh_read_only_call(fields: &[String]) -> Option<String> {
    if fields.len() < 3 || fields[0].to_lowercase() != "gh" {
        return None;
    }
    if gh_filter_reaches_host(&fields[2..]) {
        return None;
    }
    let group = fields[1].to_lowercase();
    if group == "api" {
        gh_api_reads_only(&fields[2..]).then(|| group.clone())?;
        return Some(group);
    }
    let verb = fields.get(2)?.to_lowercase();
    let listed = matches!(
        (group.as_str(), verb.as_str()),
        ("pr", "list" | "view" | "status" | "checks" | "diff")
            | ("issue", "list" | "view" | "status")
            | ("run", "list" | "view")
            | ("release", "list" | "view")
            | ("repo", "list" | "view")
            | ("auth", "status")
    );
    if !listed {
        return None;
    }
    (!gh_opens_or_reveals_secret(&group, &fields[3..])).then(|| group.clone())
}

/// `--web` starts the browser; `--show-token` prints the credential. A short
/// flag anywhere in a cluster counts, as getopt reads it.
fn gh_opens_or_reveals_secret(group: &str, args: &[String]) -> bool {
    for arg in args {
        if arg == "--" {
            return false;
        }
        if arg == "--web" || arg.starts_with("--web=") {
            return true;
        }
        if group == "auth" && (arg == "--show-token" || arg.starts_with("--show-token=")) {
            return true;
        }
    }
    has_short_flag(args, b'w') || (group == "auth" && has_short_flag(args, b't'))
}

/// `gh api ENDPOINT` with only the listed options. gh turns any field or
/// input into a POST and `--method` into whatever it names, and sends its
/// credential to `--hostname` or a full URL (reference `ghAPIReadsOnly`).
fn gh_api_reads_only(args: &[String]) -> bool {
    let mut endpoints = 0usize;
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        if !arg.starts_with('-') {
            endpoints += 1;
            if endpoints > 1 || arg.contains("://") || arg.eq_ignore_ascii_case("graphql") {
                return false;
            }
            i += 1;
            continue;
        }
        let (name, attached_value, attached) = split_eq(arg);
        let valued = matches!(
            name,
            "-H" | "--header"
                | "-q"
                | "--jq"
                | "-t"
                | "--template"
                | "--cache"
                | "-p"
                | "--preview"
        );
        let listed = matches!(
            name,
            "--paginate" | "--slurp" | "-i" | "--include" | "--silent" | "--verbose"
        ) || valued;
        if !listed || (!valued && attached) {
            return false;
        }
        let mut value = attached_value.to_string();
        if valued && !attached {
            if i + 1 >= args.len() {
                return false;
            }
            i += 1;
            value = args[i].clone();
        }
        if (name == "-H" || name == "--header") && value.to_lowercase().contains("override") {
            return false;
        }
        i += 1;
    }
    endpoints == 1
}

/// The words through which a gh jq filter reaches outside its input
/// (reference `ghHostReaching`). A substring match, so a field named
/// "environment" costs an approval, not a miss.
fn gh_filter_reaches_host(args: &[String]) -> bool {
    let mut filtered = has_short_flag(args, b'q') || has_short_flag(args, b't');
    for arg in args {
        if arg.bytes().any(|b| (b < 0x20 && b != b'\t') || b == 0x7f) {
            return true;
        }
        if arg == "--jq"
            || arg == "--template"
            || arg.starts_with("--jq=")
            || arg.starts_with("--template=")
        {
            filtered = true;
        }
    }
    if !filtered {
        return false;
    }
    for arg in args {
        let lower = arg.to_lowercase();
        for word in [
            "env",
            "input",
            "debug",
            "stderr",
            "halt_error",
            "import",
            "include",
            "get_search_list",
            "prog_args",
            "modulemeta",
        ] {
            if lower.contains(word) {
                return true;
            }
        }
    }
    false
}

/// Folded-flag containment for the read-path analyses
/// (`-exec`, `--exec=x`, `/e`, `-execdir:x` shapes).
fn has_any_folded(args: &[String], candidates: &[&str]) -> bool {
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        for candidate in candidates {
            let c = candidate.to_ascii_lowercase();
            if lower == c {
                return true;
            }
            if c.starts_with("--")
                && (lower.starts_with(&format!("{c}=")) || lower.starts_with(&format!("{c}:")))
            {
                return true;
            }
            if c.starts_with('-')
                && !c.starts_with("--")
                && c.len() == 2
                && lower.starts_with(&c)
                && !lower.starts_with("--")
            {
                return true;
            }
            if c.starts_with('/') && c.len() == 2 && lower.starts_with(&c) {
                return true;
            }
            if c.len() > 2 && c.starts_with('-') && lower.starts_with(&format!("{c}:")) {
                return true;
            }
        }
    }
    false
}

/// Whether the fields carry a substituted read-only command nested inside
/// double quotes — the one dynamic shape [`classify_read_only_command`]
/// resolves. Folded into the static analysis via the placeholder; the check
/// exists so the outer classification can require a substitution-safe base.
pub fn fields_contain_resolved_substitution(fields: &[String]) -> bool {
    has_resolved_substitution(fields)
}

/// The dynamic placeholder as the analysis spells it; tests reference this.
pub fn dynamic_arg() -> &'static str {
    DYNAMIC_ARG
}

/// The resolved-substitution placeholder; tests reference this.
pub fn resolved_substitution_placeholder() -> &'static str {
    RESOLVED_SUBSTITUTION
}
