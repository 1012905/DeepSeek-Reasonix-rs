//! The AST-level bash analysis engine: the [`BashAnalysis`] implementation
//! that reads the full statement structure through
//! `reasonix_core::shellparse_analysis` and the shared read-only tables in
//! [`crate::shellsafe`]. It is what the reference's `internal/safety/
//! permission` builds on `internal/base/shellparse`'s mvdan AST. Installed
//! over the static fail-closed engine with [`install`].

use crate::permission::bash::{BashAnalysis, BashApprovalBlocker, BashApprovalClass};
use crate::shellsafe;

/// The installed engine (reference: the permission package's analysis over
/// the AST-capable shellparse).
pub(crate) static AST_ENGINE: AstBashAnalysis = AstBashAnalysis;

/// Installs the AST engine. `true` on the first call; once anything else
/// holds the seam ([`crate::permission::bash::set_bash_analysis`]) the
/// engine it installed keeps answering instead of being silently bypassed.
pub fn install() -> bool {
    crate::permission::bash::set_bash_analysis(Box::new(AstBashAnalysis))
}

#[derive(Debug, Default, Clone, Copy)]
pub struct AstBashAnalysis;

impl BashAnalysis for AstBashAnalysis {
    /// Reference `BashSubjectApprovalBlocker`: the first blocking segment
    /// wins. `SplitTopLevel` decides between per-segment and whole-string
    /// analysis; redirect normalization runs before every segment read.
    fn approval_blocker(&self, subject: &str) -> BashApprovalBlocker {
        if subject.trim().is_empty() {
            return BashApprovalBlocker::None;
        }
        let segments = match spa::split_top_level(subject) {
            Some((segments, _)) => segments,
            None => {
                return segment_blocker(subject).unwrap_or(BashApprovalBlocker::None);
            }
        };
        if segments.is_empty() {
            return BashApprovalBlocker::Unparsable;
        }
        for segment in &segments {
            let blocker = segment_blocker(segment).unwrap_or(BashApprovalBlocker::Unparsable);
            if blocker != BashApprovalBlocker::None {
                return blocker;
            }
        }
        BashApprovalBlocker::None
    }

    /// Reference `classifyBashApproval`: the strictest segment class.
    fn approval_class(&self, subject: &str) -> BashApprovalClass {
        if subject.trim().is_empty() {
            return BashApprovalClass::Reusable;
        }
        let segments = match spa::split_top_level(subject) {
            Some((segments, _)) => segments,
            None => return segment_class(subject),
        };
        if segments.is_empty() {
            return BashApprovalClass::RequireHuman;
        }
        let mut class = BashApprovalClass::Reusable;
        for segment in &segments {
            let segment_class = segment_class(segment);
            if segment_class > class {
                class = segment_class;
            }
            if class == BashApprovalClass::RequireHuman {
                break;
            }
        }
        class
    }

    /// Reference `DecomposeBashCommand`: the simple-command segments of a
    /// compound command, or nothing for a single command.
    fn decompose(&self, command: &str) -> Option<Vec<String>> {
        let (out, split) = spa::split_top_level(command)?;
        if !split || out.len() < 2 {
            return None;
        }
        Some(out)
    }

    /// Reference `isReadOnlyBashSubject` via shellsafe: network operands
    /// refuse first, then safe-redirect normalization, then the
    /// single-command / compound classification.
    fn is_read_only(&self, subject: &str) -> bool {
        bash_subject_proof(subject).0
    }
}

/// Why a command was not proven read-only (reference `planmode.Proof`'s Why
/// vocabulary). The `is_read_only` half of the engine answers through it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proof {
    pub why: ProofWhy,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ProofWhy {
    #[default]
    None,
    Background,
    NetworkPath,
    ShellConstruct,
    WriteArguments,
    UnknownProgram,
}

/// `BashReadOnlyProof` without the JSON wrapper: the structural reason a
/// bash subject was not proven read-only.
pub fn bash_subject_proof(subject: &str) -> (bool, Proof) {
    if crate::permission::network::operands_name_network_path(subject) {
        return (
            false,
            Proof {
                why: ProofWhy::NetworkPath,
                ..Proof::default()
            },
        );
    }
    let mut subject = subject.to_string();
    if let Some(normalized) =
        crate::permission::network::normalize_bash_safe_redirects_for_match(&subject)
    {
        subject = normalized;
    }
    if let Some((base, sub, fields)) = shellsafe::classify_read_only_command(&subject) {
        return args_proof(&base, &sub, &fields);
    }
    // A compound statement is not one classifiable command, but every
    // command it runs is; read-only leaves make the whole thing read-only.
    match spa::compound_leaves(&subject) {
        Ok(Some(leaves)) => return leaves_proof(&leaves),
        Ok(None) => {}
        Err(reason) => {
            return (
                false,
                Proof {
                    why: ProofWhy::ShellConstruct,
                    detail: reason.as_str().to_string(),
                },
            );
        }
    }
    let (argv, malformed) = reasonix_core::shellparse::static_fields(&subject);
    if malformed.is_empty() && !argv.is_empty() {
        return (
            false,
            Proof {
                why: ProofWhy::UnknownProgram,
                detail: program_label(&argv),
            },
        );
    }
    (false, Proof::default())
}

fn args_proof(base: &str, sub: &str, fields: &[String]) -> (bool, Proof) {
    if shellsafe::args_make_read_only_command_write(base, sub, fields) {
        return (
            false,
            Proof {
                why: ProofWhy::WriteArguments,
                detail: program_label(fields),
            },
        );
    }
    (true, Proof::default())
}

fn leaves_proof(leaves: &[Vec<String>]) -> (bool, Proof) {
    for argv in leaves {
        let Some((base, sub, fields)) = shellsafe::classify_read_only_fields(argv) else {
            if argv.len() > 1 && spa::is_dynamic_arg(&argv[1]) {
                return (
                    false,
                    Proof {
                        why: ProofWhy::ShellConstruct,
                        detail: reasonix_core::shellparse::StaticReason::Expansion
                            .as_str()
                            .to_string(),
                    },
                );
            }
            return (
                false,
                Proof {
                    why: ProofWhy::UnknownProgram,
                    detail: program_label(argv),
                },
            );
        };
        let (read_only, proof) = args_proof(&base, &sub, &fields);
        if !read_only {
            return (false, proof);
        }
    }
    (true, Proof::default())
}

/// Names the program a classifier judged: the command word, plus the
/// subcommand for the programs read-only only by subcommand.
fn program_label(argv: &[String]) -> String {
    let Some(first) = argv.first() else {
        return String::new();
    };
    let base = reasonix_core::shellparse_analysis::program_name(first);
    if read_only_by_subcommand(&base) && argv.len() > 1 {
        return format!("{base} {}", argv[1]);
    }
    base
}

fn read_only_by_subcommand(base: &str) -> bool {
    matches!(base, "git" | "go" | "npm" | "docker" | "kubectl")
}

/// Reference `segmentApprovalBlocker`: one simple command's blocker, with
/// the safe-redirect normalization the reference runs first.
fn segment_blocker(subject: &str) -> Option<BashApprovalBlocker> {
    let subject = normalized(subject);
    let Some(features) = spa::analyze_approval_features(&subject) else {
        if heredoc_fed_interpreter(&subject) {
            return Some(BashApprovalBlocker::InlineCode);
        }
        if compound_is_read_only(&subject) {
            return Some(BashApprovalBlocker::None);
        }
        if heredoc_present(&subject) {
            return Some(BashApprovalBlocker::HereDocBody);
        }
        return Some(BashApprovalBlocker::Unparsable);
    };
    if features.nested_execution {
        return Some(BashApprovalBlocker::NestedExecution);
    }
    if features.dynamic_command_name {
        return Some(BashApprovalBlocker::DynamicName);
    }
    if !features.command_prefix.is_empty() {
        let blocker = crate::permission::bash::indirect_execution_blocker(&features.command_prefix);
        if blocker != BashApprovalBlocker::None {
            return Some(blocker);
        }
        if spa::peel_wrappers(&subject).is_ok_and(|inv| inv.opaque) {
            return Some(BashApprovalBlocker::IndirectExecution);
        }
        if features.stdin_heredoc && is_interpreter(&features.command_prefix[0]) {
            return Some(BashApprovalBlocker::InlineCode);
        }
    }
    Some(BashApprovalBlocker::None)
}

/// Reference `classifyBashSegmentApproval`: whether a segment can carry a
/// reusable prefix rule.
fn segment_class(subject: &str) -> BashApprovalClass {
    if segment_blocker(subject) != Some(BashApprovalBlocker::None) {
        return BashApprovalClass::RequireHuman;
    }
    let subject = normalized(subject);
    let Some(features) = spa::analyze_approval_features(&subject) else {
        if compound_is_read_only(&subject) {
            // Approvable, but never as a reusable prefix: its globs and loop
            // variables make the next command with the same prefix a
            // different command.
            return BashApprovalClass::ExactOnly;
        }
        return BashApprovalClass::RequireHuman;
    };
    if features.expansion
        || features.assignment
        || features.redirection
        || spa::contains_unquoted_glob(&subject)
        || crate::permission::bash::has_env_wrapper_assignment(&features.command_prefix)
    {
        return BashApprovalClass::ExactOnly;
    }
    BashApprovalClass::Reusable
}

/// Reference `compoundIsReadOnly`: every command a compound would run is
/// read-only.
fn compound_is_read_only(subject: &str) -> bool {
    let Some(leaves) = (match spa::compound_leaves(subject) {
        Ok(Some(leaves)) => Some(leaves),
        _ => None,
    }) else {
        return false;
    };
    leaves_proof(&leaves).0
}

fn heredoc_fed_interpreter(subject: &str) -> bool {
    stdin_here_doc_programs(subject)
        .iter()
        .any(|p| is_interpreter(p))
}

/// Reference `StdinHereDocPrograms`: the program of each statement that
/// reads its source from a stdin here-doc.
fn stdin_here_doc_programs(subject: &str) -> Vec<String> {
    match spa::lift(subject) {
        Ok(lifted) => lifted
            .leaves
            .iter()
            .filter(|l| l.stdin_heredoc && !l.argv.is_empty())
            .map(|l| l.argv[0].clone())
            .collect(),
        Err(_) => Vec::new(),
    }
}

fn heredoc_present(subject: &str) -> bool {
    reasonix_core::shellparse::heredoc_present(subject).unwrap_or(false)
}

fn normalized(subject: &str) -> String {
    crate::permission::network::normalize_bash_safe_redirects_for_match(subject)
        .unwrap_or_else(|| subject.to_string())
}

fn is_interpreter(program: &str) -> bool {
    crate::permission::bash::is_interpreter(program)
}

use reasonix_core::shellparse_analysis as spa;
