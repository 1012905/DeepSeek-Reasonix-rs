//! Retired-config-key stripping (docs/contracts/config.md §10.26;
//! retired_keys.go, load.go stripTOMLKeyLines).
//!
//! Boot runs these line-oriented text strips before `LoadForRoot` so a config
//! written by an older build cannot reintroduce a setting the runtime no
//! longer honors. This slice ports the pure text-strip functions; reading the
//! files, holding the edit lock, and the boot call site stay in the wiring
//! layer (CONFIG-015 wip).

use crate::toml_lex::{TomlLexState, scan_toml_line, toml_edit_section_header, toml_key_value};

/// One retired key set: the section its keys live under and the keys to drop
/// (retired_keys.go retiredStepLimits … retiredTaskBudget).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetiredKeySet {
    /// Human name used in migration notices and error wrapping.
    pub what: &'static str,
    /// TOML section header the keys live under; `""` means the document root.
    pub section: &'static str,
    /// Retired bare keys under that section.
    pub keys: &'static [&'static str],
}

/// Deprecated agent step limits (retiredStepLimits).
pub const RETIRED_STEP_LIMITS: RetiredKeySet = RetiredKeySet {
    what: "deprecated agent step limits",
    section: "agent",
    keys: &["max_steps", "planner_max_steps"],
};

/// Deprecated `secrets.redact_tool_output` (retiredRedactToolOutput).
pub const RETIRED_REDACT_TOOL_OUTPUT: RetiredKeySet = RetiredKeySet {
    what: "deprecated redact_tool_output",
    section: "secrets",
    keys: &["redact_tool_output"],
};

/// Deprecated `agent.memory_compiler` (retiredMemoryCompiler).
pub const RETIRED_MEMORY_COMPILER: RetiredKeySet = RetiredKeySet {
    what: "deprecated memory_compiler",
    section: "agent",
    keys: &["memory_compiler"],
};

/// Deprecated multi-threshold compaction keys (retiredMultiThreshold).
pub const RETIRED_MULTI_THRESHOLD: RetiredKeySet = RetiredKeySet {
    what: "deprecated multi-threshold compaction keys",
    section: "agent",
    keys: &[
        "soft_compact_ratio",
        "tool_result_snip_ratio",
        "compact_force_ratio",
        "cold_resume_prune",
        "context_editing",
        "context_soft_limit_tokens",
    ],
};

/// Deprecated task budget keys (retiredTaskBudget).
pub const RETIRED_TASK_BUDGET: RetiredKeySet = RetiredKeySet {
    what: "deprecated task budget keys",
    section: "agent",
    keys: &[
        "task_cost_budget",
        "task_time_budget_minutes",
        "task_token_budget",
    ],
};

/// The five retired key sets in the order boot applies them (retired_keys.go
/// migrateRetiredKeysFile).
pub const RETIRED_KEY_SETS: [RetiredKeySet; 5] = [
    RETIRED_STEP_LIMITS,
    RETIRED_REDACT_TOOL_OUTPUT,
    RETIRED_MEMORY_COMPILER,
    RETIRED_MULTI_THRESHOLD,
    RETIRED_TASK_BUDGET,
];

/// Result of one retired-key migration over a file's text (RetiredKeyResult,
/// minus the IO error the wiring layer owns).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RetiredMigration {
    /// Whether this set dropped at least one line.
    pub changed: bool,
}

fn section_header(line: &str) -> String {
    let trimmed = line.trim();
    if !trimmed.starts_with('[') {
        // A plain assignment or comment leaves the current section in force.
        return String::new();
    }
    let (name, _is_array, ok) = toml_edit_section_header(line);
    if !ok {
        // A malformed bracket line behaves like Go's "other" section: no real
        // section matches it, so its assignments are kept.
        return "other".to_owned();
    }
    name
}

fn matches_key(line: &str, key: &str) -> bool {
    let (got, _, ok) = toml_key_value(line);
    ok && got == key
}

/// Drops `key = ...` assignment lines under `section`, leaving every line
/// inside a TOML multiline string untouched (stripTOMLKeyLines). Returns the
/// rewritten text and whether anything was dropped.
pub fn strip_toml_key_lines(raw: &str, section: &str, keys: &[&str]) -> (String, bool) {
    let mut current = String::new();
    let mut state = TomlLexState::default();
    let mut changed = false;
    let mut out: Vec<&str> = Vec::new();

    for line in raw.split('\n') {
        if state.in_multiline_string() {
            out.push(line);
            scan_toml_line(line, &mut state, None);
            continue;
        }

        let header = section_header(line);
        if !header.is_empty() {
            current = header;
        }

        let mut next = TomlLexState::default();
        scan_toml_line(line, &mut next, None);

        let mut dropped = false;
        if current == section && !next.in_multiline_string() {
            dropped = keys.iter().any(|key| matches_key(line, key));
        }
        if dropped {
            changed = true;
            continue;
        }
        out.push(line);
        state = next;
    }

    (out.join("\n"), changed)
}

/// Applies every retired key set in order, mirroring migrateRetiredKeysFile:
/// each strip runs over the text the previous one produced. Returns the
/// rewritten text and one `changed` flag per set in `RETIRED_KEY_SETS` order.
pub fn strip_all_retired_keys(raw: &str) -> (String, [bool; 5]) {
    let mut text = raw.to_owned();
    let mut results = [false; 5];
    for (index, set) in RETIRED_KEY_SETS.iter().enumerate() {
        let (next, changed) = strip_toml_key_lines(&text, set.section, set.keys);
        text = next;
        results[index] = changed;
    }
    (text, results)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = r#"default_model = "deepseek-pro"

[agent]
max_steps = 9
planner_max_steps = 12
memory_compiler = "compact"
soft_compact_ratio = 0.5
tool_result_snip_ratio = 0.3
compact_force_ratio = 0.8
cold_resume_prune = true
context_editing = true
context_soft_limit_tokens = 4096
task_cost_budget = 3
task_time_budget_minutes = 20
task_token_budget = 80000
temperature = 0.7

[secrets]
redact_tool_output = true
filter_subprocess_env = true
"#;

    #[test]
    fn each_of_the_five_sets_drops_its_own_keys() {
        let (text, results) = strip_all_retired_keys(FIXTURE);
        assert!(results[0], "step limits changed");
        assert!(results[1], "redact_tool_output changed");
        assert!(results[2], "memory_compiler changed");
        assert!(results[3], "multi-threshold changed");
        assert!(results[4], "task budget changed");

        for retired in [
            "max_steps",
            "planner_max_steps",
            "memory_compiler",
            "soft_compact_ratio",
            "tool_result_snip_ratio",
            "compact_force_ratio",
            "cold_resume_prune",
            "context_editing",
            "context_soft_limit_tokens",
            "task_cost_budget",
            "task_time_budget_minutes",
            "task_token_budget",
            "redact_tool_output",
        ] {
            assert!(
                !text.lines().any(|line| matches_key(line, retired)),
                "{retired} kept"
            );
        }
        // Non-retired keys and the unrelated top-level value survive.
        assert!(text.lines().any(|line| line == "temperature = 0.7"));
        assert!(
            text.lines()
                .any(|line| line == "filter_subprocess_env = true")
        );
        assert!(
            text.lines()
                .any(|line| line == r#"default_model = "deepseek-pro""#)
        );
    }

    #[test]
    fn same_key_under_another_section_is_not_harmed() {
        // A retired name reused under a different section must stay: the strip
        // matches section first, key second.
        let raw = "[secrets]\nmax_steps = 3\n\n[agent]\nmax_steps = 9\n";
        let (text, changed) = strip_toml_key_lines(raw, "agent", &["max_steps"]);
        assert!(changed);
        assert!(text.lines().any(|line| line == "max_steps = 3"), "{text}");
        assert!(!text.lines().any(|line| line == "max_steps = 9"), "{text}");
    }

    #[test]
    fn retired_key_inside_a_multiline_string_is_kept() {
        // A system_prompt quoting a retired example must not be truncated.
        let raw = r#"[agent]
system_prompt = """
max_steps = 9
memory_compiler = "compact"
"""
max_steps = 9
"#;
        let (text, changed) = strip_toml_key_lines(raw, "agent", &["max_steps", "memory_compiler"]);
        assert!(changed);
        let inside = text
            .lines()
            .filter(|line| line.contains("max_steps") || line.contains("memory_compiler"))
            .collect::<Vec<_>>();
        assert_eq!(
            inside,
            vec!["max_steps = 9", "memory_compiler = \"compact\""],
            "{text}"
        );
    }

    #[test]
    fn dotted_and_commented_assignments_are_left_alone() {
        // Dotted keys and trailing-commented lines are not simple assignments;
        // stripping must not guess at them.
        let raw = "[agent]\nmax_steps.other = 9 # not the retired key\n# max_steps = 9\n";
        let (text, changed) = strip_toml_key_lines(raw, "agent", &["max_steps"]);
        assert!(!changed);
        assert_eq!(text, raw);
    }

    #[test]
    // Windows: the strip is line-oriented text and must behave identically on
    // CRLF checkouts as well; the logic itself is platform-independent.
    #[cfg(windows)]
    fn crlf_lines_keep_their_other_content() {
        let raw = "[agent]\r\nmax_steps = 9\r\ntemperature = 0.7\r\n";
        let (text, changed) = strip_toml_key_lines(raw, "agent", &["max_steps"]);
        assert!(changed);
        assert!(text.contains("temperature = 0.7"));
    }

    #[test]
    fn a_clean_config_is_untouched_byte_for_byte() {
        let clean = "[agent]\ntemperature = 0.7\n\n[secrets]\nfilter_subprocess_env = true\n";
        let (text, results) = strip_all_retired_keys(clean);
        assert_eq!(text, clean);
        assert_eq!(results, [false; 5]);
    }

    #[test]
    fn the_second_run_finds_nothing_left_to_drop() {
        let (once, _) = strip_all_retired_keys(FIXTURE);
        let (twice, results) = strip_all_retired_keys(&once);
        assert_eq!(twice, once);
        assert_eq!(results, [false; 5]);
    }
}
