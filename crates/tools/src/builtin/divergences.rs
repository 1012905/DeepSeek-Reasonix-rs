//! TOOL-057: Go↔Rust divergences review (tools.md §8). Each known divergence is
//! recorded as either an executable assertion the Rust side can check itself,
//! or a documented conclusion the integration slice carries into parity docs.
//! The reference wins on source; nothing here relaxes a frozen contract.

#![forbid(unsafe_code)]

/// How a divergence is held on the Rust side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceStatus {
    /// The Rust side checks the invariant in a test.
    Assertion,
    /// A documented parity conclusion for the integration/docs slice.
    Documented,
}

/// One reviewed divergence (tools.md §8 item).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Divergence {
    pub id: &'static str,
    pub title: &'static str,
    pub source: &'static str,
    pub conclusion: &'static str,
    pub status: DivergenceStatus,
}

/// Delegator names the reference never registers (§3.9, §8.1): guidance
/// prompts may name them, but a provider that follows the text fails through
/// the same unknown-tool path. The Rust port must not register them either.
pub const UNREGISTERED_DELEGATORS: [&str; 4] =
    ["task", "read_only_task", "parallel_tasks", "fleet"];

/// Whether `name` is one of the delegators deliberately kept unregistered.
pub fn is_unregistered_delegator(name: &str) -> bool {
    UNREGISTERED_DELEGATORS.contains(&name)
}

/// The reviewed divergence list, in §8 order.
pub fn divergences() -> &'static [Divergence] {
    &[
        Divergence {
            id: "D1",
            title: "Guidance prompts name the unregistered parallel_tasks tool",
            source: "session/control/turn_submit.go:535-536; session/control/controller.go:697",
            conclusion: "Reproduce the guidance strings byte-for-byte; do not register parallel_tasks (or task/read_only_task/fleet). A provider following the prompt resolves an unknown name the same way as the reference.",
            status: DivergenceStatus::Assertion,
        },
        Divergence {
            id: "D2",
            title: "Preview session-temp asymmetry (upstream inconsistency, match it)",
            source: "tools.md §7.6; writeFile.Preview vs editFile/multiEdit.Preview",
            conclusion: "write_file's preview expands session-temp spellings; edit_file and multi_edit previews resolve only against work_dir. Parity requires reproducing the asymmetry, not unifying it.",
            status: DivergenceStatus::Documented,
        },
        Divergence {
            id: "D3",
            title: "turn_exit_tools comment drift",
            source: "tools.md §8.3; turn_exit_tools.go",
            conclusion: "The comment describes three turn-exit ways but only ask and await_user are registered. The registration set is authoritative; the comment-only drift is non-blocking.",
            status: DivergenceStatus::Documented,
        },
        Divergence {
            id: "D4",
            title: "Retrieval V2 is shadow-only",
            source: "internal/base/retrieval/v2.go; tools.md §6.5, §8.4",
            conclusion: "BM25F field weighting, code-symbol splitting and mixed CJK uni+bigrams have no production caller. The docs tool ranks V1 only; do not port V2 unless a production caller appears.",
            status: DivergenceStatus::Assertion,
        },
        Divergence {
            id: "D5",
            title: "Catalog completeness: view_image is the 34th workspace tool",
            source: "tools.md §2.1, §3.7",
            conclusion: "The §2.1 table lists 33 registrations; view_image surfaces through the workspace assembly and must be included in the Rust catalog surface.",
            status: DivergenceStatus::Documented,
        },
        Divergence {
            id: "D6",
            title: "Embedded docs corpus parity",
            source: "tools.md §6.3, §8.6; productdocs embed.FS",
            conclusion: "The corpus must embed the same docs/*.md tree byte-identically or the sha256 manifest digest diverges; release-stamped version/revision (and vcs.revision/+dirty fallbacks) needs the Rust build-script equivalent.",
            status: DivergenceStatus::Documented,
        },
        Divergence {
            id: "D7",
            title: "bash_output prose promises task support",
            source: "tools.md §5.1, §8.7; internal/tools/jobs",
            conclusion: "A task-kind job can exist for legacy/compat reasons and surfaces its result once in Output; keep the prose and code path, but do not register task.",
            status: DivergenceStatus::Documented,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_unregistered_delegator_set_matches_section_eight() {
        assert_eq!(
            UNREGISTERED_DELEGATORS,
            ["task", "read_only_task", "parallel_tasks", "fleet"]
        );
        assert!(is_unregistered_delegator("parallel_tasks"));
        assert!(!is_unregistered_delegator("read_file"));
    }

    #[test]
    fn every_divergence_carries_an_id_title_source_and_conclusion() {
        for d in divergences() {
            assert!(!d.id.is_empty());
            assert!(!d.title.is_empty());
            assert!(!d.source.is_empty());
            assert!(!d.conclusion.is_empty());
        }
        assert_eq!(divergences().len(), 7);
    }

    #[test]
    fn the_v2_shadow_ranker_is_recorded_as_not_ported() {
        // D4: the V2 BM25F/CamelCase ranker has no production caller; the
        // productdocs surface ranks V1 only and exposes no v2 entry point.
        let d4 = divergences()
            .iter()
            .find(|d| d.id == "D4")
            .expect("D4 recorded");
        assert_eq!(d4.status, DivergenceStatus::Assertion);
        assert!(d4.conclusion.contains("V1"));
    }
}
