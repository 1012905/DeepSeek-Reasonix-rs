//! Parity ledger validation (AGENTS.md 5.2).
//!
//! "cargo xtask parity --check" parses docs/parity/LEDGER.md and fails on:
//! malformed rows, duplicate IDs, unknown statuses, and done/blocked/break
//! rows without evidence. Every in-scope behavior owns exactly one row; the
//! ledger is the single source of truth for rewrite progress.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use crate::manifest::repo_root;

const LEDGER_REL: &str = "docs/parity/LEDGER.md";

const HEADER: [&str; 6] = ["ID", "Area", "Reference", "Rust", "Status", "Evidence"];

pub const STATUSES: [&str; 5] = ["todo", "wip", "done", "blocked", "intentional-break"];

/// One ledger table row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub line_no: usize,
    pub cells: [String; 6],
}

impl Row {
    pub fn id(&self) -> &str {
        &self.cells[0]
    }

    pub fn status(&self) -> &str {
        &self.cells[4]
    }

    pub fn evidence(&self) -> &str {
        &self.cells[5]
    }
}

/// Validation failures: (markdown line number, message). Line 0 marks
/// file-level problems such as a missing header.
pub type Violation = (usize, String);

/// Splits a markdown table line into trimmed cells; None for non-table lines.
fn split_row(line: &str) -> Option<Vec<String>> {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return None;
    }
    let inner = trimmed.trim_start_matches('|').trim_end_matches('|');
    Some(
        inner
            .split('|')
            .map(|cell| cell.trim().to_owned())
            .collect(),
    )
}

/// A ledger ID: uppercase area, hyphen, alphanumeric index (CLI-001, ROUTE-012).
pub fn valid_id(id: &str) -> bool {
    let Some((area, index)) = id.split_once('-') else {
        return false;
    };
    !area.is_empty()
        && area.starts_with(|c: char| c.is_ascii_uppercase())
        && area
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        && !index.is_empty()
        && index.chars().all(|c| c.is_ascii_alphanumeric())
}

/// A markdown table separator row (--- cells).
fn is_separator(cells: &[String]) -> bool {
    !cells.is_empty()
        && cells
            .iter()
            .all(|cell| !cell.is_empty() && cell.chars().all(|ch| ch == '-'))
}

/// Validates ledger markdown; returns all violations (empty = valid).
pub fn validate(md: &str) -> Vec<Violation> {
    let mut violations = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut header_seen = false;

    for (idx, line) in md.lines().enumerate() {
        let line_no = idx + 1;
        let Some(cells) = split_row(line) else {
            continue;
        };
        if !header_seen {
            if is_separator(&cells) {
                continue; // separator above the header (tolerated)
            }
            if cells.len() == HEADER.len() && cells.iter().zip(HEADER).all(|(cell, h)| cell == h) {
                header_seen = true;
            } else {
                violations.push((
                    line_no,
                    "table row before the ID/Area/Reference/Rust/Status/Evidence header".to_owned(),
                ));
            }
            continue;
        }
        if is_separator(&cells) {
            continue;
        }
        if cells.len() != HEADER.len() {
            violations.push((
                line_no,
                format!("row has {} cells, expected {}", cells.len(), HEADER.len()),
            ));
            continue;
        }
        let row = Row {
            line_no,
            cells: std::array::from_fn(|i| cells[i].clone()),
        };
        if !valid_id(row.id()) {
            violations.push((
                line_no,
                format!("invalid ID {} (expected e.g. CLI-001)", row.id()),
            ));
        }
        if !seen.insert(row.id().to_owned()) {
            violations.push((line_no, format!("duplicate ID {}", row.id())));
        }
        for (i, cell) in row.cells.iter().enumerate() {
            if cell.is_empty() {
                violations.push((line_no, format!("empty {} cell", HEADER[i])));
            }
        }
        let evidence = row.evidence();
        let evidence_missing = evidence.is_empty()
            || evidence.eq_ignore_ascii_case("pending")
            || evidence == "-"
            || evidence == "—";
        match row.status() {
            "todo" | "wip" => {}
            "done" => {
                if evidence_missing {
                    violations.push((
                        line_no,
                        format!(
                            "done row {} needs evidence (test name or command)",
                            row.id()
                        ),
                    ));
                }
            }
            "blocked" => {
                if evidence_missing {
                    violations.push((
                        line_no,
                        format!("blocked row {} needs a reason in Evidence", row.id()),
                    ));
                }
            }
            "intentional-break" => {
                if !evidence.contains("docs/adr/") {
                    violations.push((
                        line_no,
                        format!(
                            "intentional-break row {} must link an ADR (docs/adr/...)",
                            row.id()
                        ),
                    ));
                }
            }
            other => violations.push((
                line_no,
                format!(
                    "unknown status {} (expected one of {})",
                    other,
                    STATUSES.join(", ")
                ),
            )),
        }
    }
    if !header_seen {
        violations.push((
            0,
            format!("ledger has no header row: | {} |", HEADER.join(" | ")),
        ));
    }
    violations
}

/// Runs the ledger gate against the repository ledger.
pub fn run(args: &[String]) -> ExitCode {
    match args {
        [] => {}
        [flag] if flag == "--check" => {}
        other => {
            eprintln!("xtask parity: unknown arguments: {}", other.join(" "));
            return ExitCode::from(2);
        }
    }
    let path: PathBuf = repo_root().join(LEDGER_REL);
    let md = match fs::read_to_string(&path) {
        Ok(md) => md,
        Err(err) => {
            eprintln!("xtask parity: cannot read {}: {err}", path.display());
            return ExitCode::FAILURE;
        }
    };
    let violations = validate(&md);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for line in md.lines() {
        if let Some(cells) = split_row(line)
            && cells.len() == HEADER.len()
            && STATUSES.contains(&cells[4].as_str())
        {
            *counts.entry(cells[4].clone()).or_default() += 1;
        }
    }
    let total: usize = counts.values().sum();
    let summary = STATUSES
        .iter()
        .filter_map(|status| {
            counts
                .get(*status)
                .map(|count| format!("{status}: {count}"))
        })
        .collect::<Vec<_>>()
        .join(", ");
    if violations.is_empty() {
        println!("xtask parity: OK ({total} rows; {summary})");
        ExitCode::SUCCESS
    } else {
        for (line_no, msg) in &violations {
            eprintln!("xtask parity: {}:{line_no}: {msg}", path.display());
        }
        eprintln!(
            "xtask parity: FAILED ({} violation(s) over {total} rows)",
            violations.len()
        );
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = concat!(
        "| ID | Area | Reference | Rust | Status | Evidence |\n",
        "|---|---|---|---|---|---|\n",
        "| CLI-001 | CLI | --version | reasonix-cli | done | tests/cli.rs |\n",
        "| CLI-002 | CLI | -v | reasonix-cli | todo | pending |\n",
    );

    #[test]
    fn valid_table_has_no_violations() {
        assert!(validate(VALID).is_empty());
    }

    #[test]
    fn header_must_come_first() {
        let md = concat!(
            "| CLI-001 | CLI | a | b | todo | pending |\n",
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
        );
        assert!(validate(md).iter().any(|(line_no, _)| *line_no == 1));
    }

    #[test]
    fn missing_header_is_rejected() {
        let violations = validate("no tables here");
        assert!(
            violations
                .iter()
                .any(|(line_no, msg)| *line_no == 0 && msg.contains("header"))
        );
    }

    #[test]
    fn done_requires_real_evidence() {
        let md = VALID.replace("tests/cli.rs", "pending");
        assert!(
            validate(&md)
                .iter()
                .any(|(_, msg)| msg.contains("needs evidence"))
        );
    }

    #[test]
    fn blocked_requires_a_reason() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
            "|---|---|---|---|---|---|\n",
            "| X-001 | X | a | b | blocked | pending |\n",
        );
        assert!(
            validate(md)
                .iter()
                .any(|(_, msg)| msg.contains("needs a reason"))
        );
    }

    #[test]
    fn break_rows_need_adr_links() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
            "|---|---|---|---|---|---|\n",
            "| X-001 | X | a | b | intentional-break | decided in chat |\n",
        );
        assert!(validate(md).iter().any(|(_, msg)| msg.contains("ADR")));
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
            "|---|---|---|---|---|---|\n",
            "| X-001 | X | a | b | todo | pending |\n",
            "| X-001 | X | a | b | todo | pending |\n",
        );
        assert!(
            validate(md)
                .iter()
                .any(|(line_no, msg)| *line_no == 4 && msg.contains("duplicate"))
        );
    }

    #[test]
    fn unknown_status_is_rejected() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
            "|---|---|---|---|---|---|\n",
            "| X-001 | X | a | b | finished | pending |\n",
        );
        assert!(
            validate(md)
                .iter()
                .any(|(_, msg)| msg.contains("unknown status"))
        );
    }

    #[test]
    fn malformed_rows_are_rejected() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status |\n",
            "|---|---|---|---|---|\n",
        );
        assert!(validate(md).iter().any(|(line_no, _)| *line_no == 1));
    }

    #[test]
    fn empty_cells_are_rejected() {
        let md = concat!(
            "| ID | Area | Reference | Rust | Status | Evidence |\n",
            "|---|---|---|---|---|---|\n",
            "| X-001 | | a | b | todo | pending |\n",
        );
        assert!(
            validate(md)
                .iter()
                .any(|(_, msg)| msg.contains("empty Area"))
        );
    }

    #[test]
    fn ids_must_match_the_pattern() {
        assert!(valid_id("CLI-001"));
        assert!(valid_id("ROUTE-012"));
        assert!(valid_id("INFRA-001"));
        assert!(valid_id("CLI-001A"));
        assert!(!valid_id("cli-001"));
        assert!(!valid_id("CLI-"));
        assert!(!valid_id("CLI"));
        assert!(!valid_id("-001"));
        assert!(!valid_id("CLI-001-x"));
    }
}
