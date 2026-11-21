//! The pipe-status probe (parity targets: `pipe_status.go` and
//! `shellparse/pipeline.go`). A pipeline hands the shell only its last
//! stage's status, so `go test | tail` exits zero on a failing suite and
//! the host cannot say whether the check passed. bash keeps every stage's
//! status in PIPESTATUS; asking for it costs one line and changes no
//! semantics, where `set -o pipefail` would change what `| head` means.

#![forbid(unsafe_code)]

use std::sync::OnceLock;

use reasonix_core::shellparse::Join;
use reasonix_core::shellparse_analysis::lift;

/// Every pipeline that could be the last thing the command runs, each as
/// its stage sources left to right (Go `TerminalPipelines`). bash's
/// PIPESTATUS describes whichever one actually ran, and a caller can tell
/// which when the candidates differ in stage count. A command that does not
/// decompose — negation, background, control syntax, a here-doc — answers
/// None.
pub fn terminal_pipelines(command: &str) -> Option<Vec<Vec<String>>> {
    if command.trim().is_empty() {
        return None;
    }
    let lifted = lift(command).ok()?;
    if lifted.heredoc || lifted.leaves.is_empty() {
        return None;
    }
    if lifted.opaque.iter().any(|o| o.is_some()) {
        return None;
    }
    // Only the last `;`-separated statement can be the last thing to run;
    // earlier ones always continue into it. The parser flattened each
    // left-associative chain into consecutive leaves, so the last statement
    // is the run of leaves joined back through And / Or / Pipe.
    let mut chain: Vec<usize> = Vec::new();
    let mut i = lifted.leaves.len();
    while i > 0 {
        i -= 1;
        let leaf = &lifted.leaves[i];
        if leaf.background || leaf.negated {
            return None;
        }
        chain.push(i);
        if i == 0 {
            break;
        }
        if !matches!(
            lifted.joins.get(i - 1),
            Some(Join::And | Join::Or | Join::Pipe | Join::PipeAll)
        ) {
            break;
        }
    }
    chain.reverse();
    // A pipeline is a maximal run of Pipe-joined leaves inside the chain;
    // `&&` and `||` both stop early, so every pipeline between them is a
    // candidate.
    let mut candidates: Vec<Vec<String>> = Vec::new();
    let mut stages: Vec<String> = Vec::new();
    for &idx in &chain {
        let leaf = &lifted.leaves[idx];
        stages.push(leaf.source(command).to_owned());
        if !matches!(
            lifted.joins.get(idx).copied().unwrap_or(Join::None),
            Join::Pipe | Join::PipeAll
        ) && !stages.is_empty()
        {
            candidates.push(std::mem::take(&mut stages));
        }
    }
    if candidates.is_empty() {
        return None;
    }
    Some(candidates)
}

/// Picks the candidate whose stage count matches a captured PIPESTATUS (Go
/// `PipelineForStatus`). A tie means two pipelines of the same width could
/// have produced it, and guessing between them would attribute a status to
/// a command that never ran.
pub fn pipeline_for_status(command: &str, status_len: usize) -> Option<Vec<String>> {
    if status_len == 0 {
        return None;
    }
    let candidates = terminal_pipelines(command)?;
    let mut matched: Option<Vec<String>> = None;
    for stages in candidates {
        if stages.len() != status_len {
            continue;
        }
        if matched.is_some() {
            return None;
        }
        matched = Some(stages);
    }
    matched
}

/// The verifier question the probe arms on (Go
/// `evidence.PipeStatusCanDecideVerification`): whether per-stage exit
/// statuses could answer the check inside command — a pipeline that could
/// end the command, whose stages have distinct widths, and whose check sits
/// before the stage that decides the status. The verifier name table is the
/// evidence slice's machine; until it installs through
/// [`set_verification_decider`] the probe stays disarmed, which reads as no
/// report rather than a wrong one.
pub fn pipe_status_can_decide_verification(command: &str) -> bool {
    match decider().get() {
        Some(d) => d(command),
        None => false,
    }
}

type Decider = fn(&str) -> bool;

fn decider() -> &'static OnceLock<Decider> {
    static DECIDER: OnceLock<Decider> = OnceLock::new();
    &DECIDER
}

/// Installs the verifier-table decider. Only the first call wins, the same
/// first-wins seam the permission engine uses.
pub fn set_verification_decider(d: Decider) -> bool {
    decider().set(d).is_ok()
}

/// The probe: a nonce marker appended to the command, harvested out of the
/// model-visible output onto the execution record.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PipeStatusProbe {
    nonce: String,
}

impl PipeStatusProbe {
    /// Arms on a foreground bash command whose check a per-stage report
    /// could decide. A command that exits or execs never reaches the
    /// report, which simply leaves it unreported.
    pub fn arm(is_bash: bool, command: &str, background: bool) -> PipeStatusProbe {
        if background || !is_bash || !pipe_status_can_decide_verification(command) {
            return PipeStatusProbe::default();
        }
        PipeStatusProbe {
            nonce: fresh_nonce(),
        }
    }

    pub fn is_active(&self) -> bool {
        !self.nonce.is_empty()
    }

    /// Appends the report and restores the status the command would have
    /// exited with, so nothing downstream sees a different result.
    pub fn wrap(&self, command: &str) -> String {
        if !self.is_active() {
            return command.to_owned();
        }
        format!(
            "{command}\n__rx_ps=(\"${{PIPESTATUS[@]}}\"); printf '%s %s\\n' '{}' \"${{__rx_ps[*]}}\" >&2; exit \"${{__rx_ps[${{#__rx_ps[@]}}-1]:-0}}\"\n",
            self.marker()
        )
    }

    pub fn marker(&self) -> String {
        if !self.is_active() {
            return String::new();
        }
        format!("__rx_pipestatus_{}", self.nonce)
    }

    /// Moves the report out of everything the model can see and returns the
    /// per-stage statuses. Output truncation or a killed process drops the
    /// line, which reads as no report rather than a wrong one.
    pub fn harvest(&self, out: &str, output_tail: &str) -> (String, String, Vec<i32>) {
        if !self.is_active() {
            return (out.to_owned(), output_tail.to_owned(), Vec::new());
        }
        let (out, status) = self.read(out);
        let (tail, _) = self.read(output_tail);
        (out, tail, status)
    }

    fn read(&self, out: &str) -> (String, Vec<i32>) {
        let marker = self.marker();
        if !out.contains(&marker) {
            return (out.to_owned(), Vec::new());
        }
        let mut kept: Vec<&str> = Vec::new();
        let mut status: Vec<i32> = Vec::new();
        for line in out.split('\n') {
            match line.trim().strip_prefix(&marker) {
                Some(rest) => status = parse_pipe_status(rest),
                None => kept.push(line),
            }
        }
        (kept.join("\n").trim_end_matches('\n').to_owned(), status)
    }
}

fn parse_pipe_status(fields: &str) -> Vec<i32> {
    let mut status = Vec::new();
    for field in fields.split_whitespace() {
        match field.parse::<i32>() {
            Ok(code) => status.push(code),
            Err(_) => return Vec::new(),
        }
    }
    status
}

/// Twelve bytes as hex would overshoot the Go shape; Go reads eight random
/// bytes as hex (Go `newPipeStatusProbe`). A CSPRNG failure leaves the zero
/// buffer, the way Go's ignored error leaves it — the nonce only has to be
/// unique within one command's output.
fn fresh_nonce() -> String {
    let mut raw = [0u8; 8];
    let _ = getrandom::fill(&mut raw);
    raw.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_pipelines_read_the_last_statement() {
        // Go pipeline_test.go oracle: a plain pipeline is one candidate.
        let got = terminal_pipelines("go test ./... | tail -20").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(
            got[0],
            vec![String::from("go test ./..."), String::from("tail -20")]
        );
        // A single command is one one-stage candidate.
        let got = terminal_pipelines("go test ./...").unwrap();
        assert_eq!(got, vec![vec![String::from("go test ./...")]]);
        // `&&` both stop early, so either side can be the last to run.
        let got = terminal_pipelines("go vet ./... && go test ./... | tail").unwrap();
        assert_eq!(
            got,
            vec![
                vec![String::from("go vet ./...")],
                vec![String::from("go test ./..."), String::from("tail")]
            ]
        );
        assert!(terminal_pipelines("").is_none());
        assert!(terminal_pipelines("   ").is_none());
    }

    #[test]
    fn terminal_pipelines_refuse_shapes_it_cannot_decide() {
        // Negation.
        assert!(terminal_pipelines("! go test ./...").is_none());
        // Background.
        assert!(terminal_pipelines("go test ./... &").is_none());
        // Here-doc.
        assert!(terminal_pipelines("cat <<EOF\nhello\nEOF").is_none());
    }

    #[test]
    fn pipeline_for_status_resolves_by_width() {
        let got = pipeline_for_status("go vet ./... && go test ./... | tail", 2).unwrap();
        assert_eq!(
            got,
            vec![String::from("go test ./..."), String::from("tail")]
        );
        assert!(pipeline_for_status("go test ./... | tail", 3).is_none());
        assert!(pipeline_for_status("go test ./...", 1).is_some());
        assert!(pipeline_for_status("go test ./...", 0).is_none());
    }

    #[test]
    fn pipeline_for_status_refuses_equal_widths() {
        // Two two-stage candidates: the status cannot be attributed.
        assert!(pipeline_for_status("a | b && c | d", 2).is_none());
    }

    // The probe's arm conditions short-circuit before the decider, so the
    // process-global seam never makes these order-dependent.
    #[test]
    fn probe_never_arms_off_bash_or_in_background() {
        assert!(!PipeStatusProbe::arm(true, "go test | tail", true).is_active());
        assert!(!PipeStatusProbe::arm(false, "go test | tail", false).is_active());
    }

    #[test]
    fn probe_wraps_and_harvests_when_armed() {
        assert!(
            set_verification_decider(|c| c == "armed-pipeline-cmd | tail"),
            "first decider wins; the harness installs its own"
        );
        let p = PipeStatusProbe::arm(true, "armed-pipeline-cmd | tail", false);
        assert!(p.is_active());
        let wrapped = p.wrap("armed-pipeline-cmd | tail");
        assert!(wrapped.starts_with("armed-pipeline-cmd | tail\n__rx_ps="));
        assert!(wrapped.contains(&p.marker()));
        assert!(wrapped.ends_with('\n'));
        let raw = format!("suite output\n{} 0 1\n", p.marker());
        let (out, tail, status) = p.harvest(&raw, &raw);
        assert_eq!(out, "suite output");
        assert_eq!(tail, "suite output");
        assert_eq!(status, vec![0, 1]);
        // A disarmed probe passes everything through untouched.
        let off = PipeStatusProbe::arm(true, "not the armed command", false);
        assert!(!off.is_active());
        assert_eq!(off.wrap("echo hi"), "echo hi");
        let (out, _, status) = off.harvest("x", "t");
        assert_eq!(out, "x");
        assert!(status.is_empty());
    }

    #[test]
    fn malformed_status_reads_as_no_report() {
        assert!(parse_pipe_status("0 x 1").is_empty());
        assert!(parse_pipe_status("").is_empty());
        assert_eq!(parse_pipe_status("0 1 2"), vec![0, 1, 2]);
    }
}
