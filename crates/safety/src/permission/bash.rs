//! Bash command analysis behind the permission gate.
//!
//! The reference reads full bash ASTs (`mvdan.cc/sh/v3/syntax`) through
//! `internal/base/shellparse` to name every command a statement would run.
//! `reasonix-core::shellparse` ports the static-command subset only, so the
//! machine here is expressed as the [`BashAnalysis`] trait — the frozen seam —
//! with a conservative static implementation ([`StaticBashAnalysis`]) that
//! answers every question fail-closed from the static port. A later slice
//! porting the mvdan-level analysis replaces the engine without touching the
//! policy below it.

use crate::permission::inline_code::{argv_carries_inline_code, executable_base};

/// Names the programs that execute source handed to them rather than a fixed
/// operation. Kept beside [`indirect_execution_blocker`], which enumerates
/// the same programs for their inline-code flags.
pub fn is_interpreter(program: &str) -> bool {
    matches!(
        executable_base(program).as_str(),
        "python"
            | "python3"
            | "py"
            | "pypy"
            | "pypy3"
            | "node"
            | "bun"
            | "deno"
            | "perl"
            | "ruby"
            | "lua"
            | "luajit"
            | "r"
            | "rscript"
            | "osascript"
            | "php"
            | "bash"
            | "dash"
            | "fish"
            | "ksh"
            | "sh"
            | "zsh"
            | "powershell"
            | "pwsh"
    )
}

/// Which shape forced a command to a human. Hosts use it to say what they
/// actually stopped: a caller told "inline interpreter code is blocked" about
/// `env | grep` will rewrite toward the wrong thing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BashApprovalBlocker {
    None,
    /// A command static analysis could not read at all, so nothing about it
    /// can be trusted.
    Unparsable,
    /// Command substitution or process substitution: $(...), `...`, <(...).
    NestedExecution,
    DynamicName,
    /// An interpreter running code from an argument: python -c, node -e,
    /// bash -c.
    InlineCode,
    /// A wrapper that runs some other command the argv does not name: eval,
    /// source, xargs, find -exec.
    IndirectExecution,
    /// A here-document the parser reads but an approval cannot: its body is
    /// file content, not arguments.
    HereDocBody,
}

/// How a command can be approved: a reusable prefix rule, only an exact
/// match, or never without a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BashApprovalClass {
    Reusable,
    ExactOnly,
    RequireHuman,
}

/// The questions the permission layer puts to a bash command. Each method
/// fails closed: an engine that cannot answer reports "needs a human" /
/// "not read-only" rather than guessing.
pub trait BashAnalysis: Send + Sync {
    /// Why `subject` needs a human, or [`BashApprovalBlocker::None`].
    fn approval_blocker(&self, subject: &str) -> BashApprovalBlocker;
    /// How `subject` can be approved.
    fn approval_class(&self, subject: &str) -> BashApprovalClass;
    /// The simple-command segments of a compound command, when it can be
    /// decomposed safely. `None` falls back to whole-string matching.
    fn decompose(&self, command: &str) -> Option<Vec<String>>;
    /// Whether a bash command is a known foreground read-only operation.
    fn is_read_only(&self, subject: &str) -> bool;
}

/// The engine over `reasonix-core::shellparse`'s static port. The static
/// parser rejects everything beyond one simple command, so compound
/// statements, redirects and any dynamic shape classify as
/// require-human / not-read-only — never broader than the reference.
#[derive(Debug, Default, Clone, Copy)]
pub struct StaticBashAnalysis;

impl BashAnalysis for StaticBashAnalysis {
    fn approval_blocker(&self, subject: &str) -> BashApprovalBlocker {
        if subject.trim().is_empty() {
            return BashApprovalBlocker::None;
        }
        match super::shellparse::static_fields(subject) {
            (_, malformed) if !malformed.is_empty() => BashApprovalBlocker::Unparsable,
            (fields, _) => indirect_execution_blocker(&fields),
        }
    }

    fn approval_class(&self, subject: &str) -> BashApprovalClass {
        if subject.trim().is_empty() {
            return BashApprovalClass::Reusable;
        }
        if self.approval_blocker(subject) != BashApprovalBlocker::None {
            return BashApprovalClass::RequireHuman;
        }
        // The static parser rejects redirects, expansions and compound
        // statements outright, so anything it accepted with no indirect
        // execution carries no dynamic features — the same facts the
        // reference reads off the AST.
        BashApprovalClass::Reusable
    }

    fn decompose(&self, _command: &str) -> Option<Vec<String>> {
        // SplitTopLevel needs the statement-level AST, which the static port
        // does not carry; whole-string matching is the conservative fallback.
        None
    }

    fn is_read_only(&self, _subject: &str) -> bool {
        // No read-only table has been ported in this slice; unproven reads
        // are not reads.
        false
    }
}

/// The process-wide engine: whatever installed first through
/// [`set_bash_analysis`] or [`super::bash_engine::install`], else the AST
/// engine that production runs on.
static ENGINE: std::sync::OnceLock<Box<dyn BashAnalysis>> = std::sync::OnceLock::new();

pub fn bash_analysis() -> &'static dyn BashAnalysis {
    ENGINE
        .get()
        .map(|b| b.as_ref())
        .unwrap_or(&super::bash_engine::AST_ENGINE)
}

/// Installs the analysis engine. Only the first call wins; later calls are
/// ignored, so a test installing an engine never leaks into another.
pub fn set_bash_analysis(engine: Box<dyn BashAnalysis>) -> bool {
    ENGINE.set(engine).is_ok()
}

/// Separates code carried in an argument from wrappers that run a command the
/// argv does not name. Both need a human; only the first is fixed by writing
/// the code to a file. Pure argv analysis, ported whole from
/// `bash_approval.go`'s `indirectExecutionBlocker`.
pub fn indirect_execution_blocker(fields: &[String]) -> BashApprovalBlocker {
    if fields.is_empty() {
        return BashApprovalBlocker::IndirectExecution;
    }
    let base = executable_base(&fields[0]);
    let mut args: &[String] = &fields[1..];
    match base.as_str() {
        "eval" | "source" | "." | "xargs" => BashApprovalBlocker::IndirectExecution,
        "env" => {
            while let Some(first) = args.first() {
                if is_environment_assignment(first) {
                    args = &args[1..];
                } else {
                    break;
                }
            }
            match args.first() {
                None => BashApprovalBlocker::IndirectExecution,
                Some(a) if a.starts_with('-') => BashApprovalBlocker::IndirectExecution,
                Some(_) => indirect_execution_blocker(args),
            }
        }
        "builtin" | "exec" | "nohup" | "sudo" => match args.first() {
            None => BashApprovalBlocker::IndirectExecution,
            Some(a) if a.starts_with('-') => BashApprovalBlocker::IndirectExecution,
            Some(_) => indirect_execution_blocker(args),
        },
        "command" => command_builtin_blocker(args),
        "find" => {
            if has_any_folded_arg(args, &["-exec", "-execdir", "-ok", "-okdir"]) {
                BashApprovalBlocker::IndirectExecution
            } else {
                BashApprovalBlocker::None
            }
        }
        _ if argv_carries_inline_code(fields) => BashApprovalBlocker::InlineCode,
        _ => BashApprovalBlocker::None,
    }
}

/// `command -v`/`-V` report how a name would resolve and run nothing, which
/// makes them readers like `which`. Every other form hands the call to the
/// program it names.
fn command_builtin_blocker(args: &[String]) -> BashApprovalBlocker {
    let mut rest = args;
    while let Some(first) = rest.first() {
        if !first.starts_with('-') {
            break;
        }
        let flag = first.as_str();
        rest = &rest[1..];
        if flag == "--" {
            break;
        }
        if flag.contains(['v', 'V']) {
            return BashApprovalBlocker::None;
        }
    }
    if rest.is_empty() {
        return BashApprovalBlocker::IndirectExecution;
    }
    indirect_execution_blocker(rest)
}

/// Reports whether a wrapper's argv carries an environment assignment
/// (`env FOO=1 cmd`). Ported from `hasEnvWrapperAssignment`.
pub fn has_env_wrapper_assignment(fields: &[String]) -> bool {
    if fields.len() < 2 || executable_base(&fields[0]) != "env" {
        return false;
    }
    for arg in &fields[1..] {
        if is_environment_assignment(arg) {
            return true;
        }
        if !arg.starts_with('-') {
            return false;
        }
    }
    false
}

/// Folded-flag containment: `-exec`, `--exec=x`, `/e`, `-execdir:x` shapes.
/// Ported from `hasAnyFoldedArg`.
pub(crate) fn has_any_folded_arg(args: &[String], candidates: &[&str]) -> bool {
    for arg in args {
        let lower = arg.to_ascii_lowercase();
        for candidate in candidates {
            let candidate = candidate.to_ascii_lowercase();
            if lower == candidate {
                return true;
            }
            if candidate.starts_with("--")
                && (lower.starts_with(&format!("{candidate}="))
                    || lower.starts_with(&format!("{candidate}:")))
            {
                return true;
            }
            if candidate.starts_with('-')
                && !candidate.starts_with("--")
                && candidate.len() == 2
                && lower.starts_with(&candidate)
                && !lower.starts_with("--")
            {
                return true;
            }
            if candidate.starts_with('/') && candidate.len() == 2 && lower.starts_with(&candidate) {
                return true;
            }
            if candidate.len() > 2
                && candidate.starts_with('-')
                && lower.starts_with(&format!("{candidate}:"))
            {
                return true;
            }
        }
    }
    false
}

/// Reports whether arg is a `NAME=value` environment assignment.
pub fn is_environment_assignment(arg: &str) -> bool {
    let Some((name, _)) = arg.split_once('=') else {
        return false;
    };
    if name.is_empty() {
        return false;
    }
    name.chars().enumerate().all(|(i, r)| {
        let letter = r.is_ascii_alphabetic();
        let digit = i > 0 && r.is_ascii_digit();
        letter || digit || r == '_'
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn indirect_execution_table() {
        assert_eq!(
            indirect_execution_blocker(&argv(&["awk", "BEGIN{system(\"id\")}"])),
            BashApprovalBlocker::InlineCode
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["awk", "-f", "report.awk", "data.txt"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["eval", "touch", "x"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["source", "./s.sh"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&[".", "./s.sh"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["xargs", "rm"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["find", ".", "-exec", "rm", "{}", ";"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["find", ".", "-name", "x"])),
            BashApprovalBlocker::None
        );
        // command -v reports; command runs
        assert_eq!(
            indirect_execution_blocker(&argv(&["command", "-v", "uv"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["command", "-V", "pytest"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["command", "git", "status"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["command", "--", "git", "status"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["command"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["command", "eval", "x"])),
            BashApprovalBlocker::IndirectExecution
        );
        // env / sudo / wrappers recurse
        assert_eq!(
            indirect_execution_blocker(&argv(&["env", "rm", "b"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["env", "X=1", "rm", "b"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["env"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["sudo", "rm", "b"])),
            BashApprovalBlocker::None
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["sudo", "-u", "root", "rm", "b"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["sudo", "-s"])),
            BashApprovalBlocker::IndirectExecution
        );
        assert_eq!(
            indirect_execution_blocker(&argv(&["python3", "-c", "x"])),
            BashApprovalBlocker::InlineCode
        );
    }

    #[test]
    fn environment_assignments() {
        assert!(is_environment_assignment("X=1"));
        assert!(is_environment_assignment("_x=1"));
        assert!(!is_environment_assignment("a-b=1"));
        assert!(!is_environment_assignment("1X=1"));
        assert!(!is_environment_assignment("=1"));
        assert!(!is_environment_assignment("-u"));
    }

    #[test]
    fn interpreter_set() {
        assert!(is_interpreter("python3"));
        assert!(is_interpreter("/usr/bin/bash"));
        assert!(is_interpreter("pwsh.exe"));
        assert!(!is_interpreter("cat"));
    }
}
