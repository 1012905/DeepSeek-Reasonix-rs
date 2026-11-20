//! Tags a finished shell tool result whose whole output is a unified diff
//! (docs/contracts/sse.md §5; Go `markdiff.go`). The frontend then renders it
//! as a diff. `enabled false` is a pass-through; the tag is computed once
//! here, at the one sink every emitter shares, so the tool loop, synthetic
//! calls and sub-agents are tagged alike.

use std::sync::Arc;

use reasonix_contract::eventwire::event::Event;
use reasonix_core::diff::is_unified_diff;

use super::Sink;

/// Wraps `sink` so a shell result that is, in its entirety, a unified diff
/// carries `outputDiff`. Go `MarkEmbeddedDiffs`; `enabled false` returns the
/// sink unchanged.
pub fn mark_embedded_diffs(sink: Arc<dyn Sink>, enabled: bool) -> Arc<dyn Sink> {
    if !enabled {
        return sink;
    }
    Arc::new(MarkDiffSink { inner: sink })
}

struct MarkDiffSink {
    inner: Arc<dyn Sink>,
}

impl Sink for MarkDiffSink {
    fn emit(&self, mut e: Event) {
        // A result the host trimmed to fit context is not a whole diff any
        // more, so its lossy bound keeps it out of the tag rather than showing
        // a partial diff.
        if e.kind == "tool_result" {
            let taggable = e.tool.as_ref().is_some_and(|t| {
                t.execution.is_some()
                    && t.err.is_empty()
                    && !bound_lossy(t.bound.as_ref())
                    && !t.output_diff
            });
            if taggable && let Some(tool) = e.tool.as_mut() {
                tool.output_diff = is_unified_diff(&tool.output);
            }
        }
        self.inner.emit(e);
    }
}

/// Content the model never saw was discarded outright (Go `OutputBound.Lossy`).
fn bound_lossy(bound: Option<&reasonix_contract::eventwire::tool::Bound>) -> bool {
    bound.is_some_and(|b| b.kind == "truncated")
}

#[cfg(test)]
mod tests {
    use super::*;
    use reasonix_contract::eventwire::tool::{Bound, ShellExecution, Tool};
    use std::sync::Mutex;

    struct RecordSink(Mutex<Vec<Event>>);

    impl Sink for RecordSink {
        fn emit(&self, e: Event) {
            self.0.lock().unwrap_or_else(|e| e.into_inner()).push(e);
        }
    }

    fn recorder() -> Arc<RecordSink> {
        Arc::new(RecordSink(Mutex::new(Vec::new())))
    }

    const SHELL_DIFF: &str =
        "diff --git a/x.go b/x.go\n--- a/x.go\n+++ b/x.go\n@@ -1 +1 @@\n-old\n+new\n";

    fn shell() -> ShellExecution {
        ShellExecution {
            kind: "bash".to_owned(),
            ..ShellExecution::default()
        }
    }

    fn result(tool: Tool) -> Event {
        Event {
            kind: "tool_result".to_owned(),
            tool: Some(tool),
            ..Event::default()
        }
    }

    // Ported: markdiff_test.go TestMarkEmbeddedDiffs — the tagging table.
    #[test]
    fn tags_only_the_table_entries() {
        let cases: Vec<(&str, bool, Event, bool)> = vec![
            (
                "shell result that is a whole diff",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: SHELL_DIFF.to_owned(),
                    ..Tool::default()
                }),
                true,
            ),
            (
                "shell result that is prose",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: "ok\n".to_owned(),
                    ..Tool::default()
                }),
                false,
            ),
            (
                "non-shell result with diff output",
                true,
                result(Tool {
                    output: SHELL_DIFF.to_owned(),
                    ..Tool::default()
                }),
                false,
            ),
            (
                "shell result that failed",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: SHELL_DIFF.to_owned(),
                    err: "boom".to_owned(),
                    ..Tool::default()
                }),
                false,
            ),
            (
                "lossy bound is not tagged",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: SHELL_DIFF.to_owned(),
                    bound: Some(Bound {
                        kind: "truncated".to_owned(),
                        kept_bytes: 8,
                        ..Bound::default()
                    }),
                    ..Tool::default()
                }),
                false,
            ),
            (
                "spilled bound still tagged",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: SHELL_DIFF.to_owned(),
                    bound: Some(Bound {
                        kind: "spilled".to_owned(),
                        path: "/tmp/x".to_owned(),
                        ..Bound::default()
                    }),
                    ..Tool::default()
                }),
                true,
            ),
            (
                "dispatch is never tagged",
                true,
                Event {
                    kind: "tool_dispatch".to_owned(),
                    tool: Some(Tool {
                        execution: Some(shell()),
                        output: SHELL_DIFF.to_owned(),
                        ..Tool::default()
                    }),
                    ..Event::default()
                },
                false,
            ),
            (
                "already-tagged result is left alone",
                true,
                result(Tool {
                    execution: Some(shell()),
                    output: "not a diff".to_owned(),
                    output_diff: true,
                    ..Tool::default()
                }),
                true,
            ),
        ];
        for (name, enabled, ev, want) in cases {
            let rec = recorder();
            mark_embedded_diffs(rec.clone(), enabled).emit(ev);
            let got = rec.0.lock().unwrap_or_else(|e| e.into_inner());
            assert_eq!(got.len(), 1, "case {name:?}: emitted != 1");
            assert_eq!(
                got[0].tool.as_ref().unwrap().output_diff,
                want,
                "case {name:?}"
            );
        }
    }

    // Ported: markdiff_test.go — `enabled false` is a pass-through.
    #[test]
    fn disabled_is_a_pass_through() {
        let rec = recorder();
        let sink = mark_embedded_diffs(rec.clone(), false);
        let ev = result(Tool {
            execution: Some(shell()),
            output: SHELL_DIFF.to_owned(),
            ..Tool::default()
        });
        sink.emit(ev);
        let got = rec.0.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(got.len(), 1);
        assert!(!got[0].tool.as_ref().unwrap().output_diff);
    }

    // The tag is computed from the output the model received; a windowed
    // bound (still reachable) does not block it.
    #[test]
    fn windowed_bound_still_tagged() {
        let rec = recorder();
        let ev = result(Tool {
            execution: Some(shell()),
            output: SHELL_DIFF.to_owned(),
            bound: Some(Bound {
                kind: "windowed".to_owned(),
                lines: 6,
                ..Bound::default()
            }),
            ..Tool::default()
        });
        mark_embedded_diffs(rec.clone(), true).emit(ev);
        let got = rec.0.lock().unwrap_or_else(|e| e.into_inner());
        assert!(got[0].tool.as_ref().unwrap().output_diff);
    }
}
