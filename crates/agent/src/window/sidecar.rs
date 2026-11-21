//! Projection sidecar bind, revalidate and reuse reporting (Go:
//! `preflight.go`'s compaction half — `LoadProjectionSidecar`,
//! `BindSessionPath`, `InvalidateProjection`, `RevalidateProjection`,
//! `ProjectionDropped`, `FoldNotReusedNotice`, `SetCacheState`).
//!
//! State the window cannot read or use is dropped in memory only, so the
//! next request rebuilds from canonical and the file stays with whichever
//! release wrote it. Whoever first owns the coverage judgement owns the
//! revalidate call: a caller that rewrote history asks here instead of
//! deciding for itself.

use std::path::Path;

use reasonix_state::sessionstore::compaction_state::{
    COMPACTION_STATE_SCHEMA_CURRENT, CompactionState, ContextProjection, load_compaction_state,
    remove_compaction_state,
};

use reasonix_contract::eventwire::event::Event;

use super::Window;
use super::cache_state;
use super::{lineage_key_compatible, projection_content_valid, projection_valid};

/// Why a saved fold was not reused when its session was bound (Go:
/// `ProjectionDrop`). The host is the only party that saw which it was.
pub mod drop_reason {
    pub const NONE: &str = "";
    /// The fold's lineage matched but the history behind it changed.
    pub const HISTORY: &str = "dropped_history";
    /// Another model or workspace made it and the history does not match either.
    pub const LINEAGE: &str = "dropped_lineage";
}

/// What the person is told when a saved fold is not used and the whole
/// history goes to the model instead (Go: `FoldNotReusedNotice`); both the
/// session bind and a later request say it in these words.
pub fn fold_not_reused_notice(drop: &str) -> Option<(&'static str, &'static str)> {
    match drop {
        drop_reason::HISTORY => Some((
            crate::sink::notice_code::FOLD_NOT_REUSED_HISTORY,
            "A saved compaction of this conversation was not reused: the history it summarised has changed since. The full history is sent instead.",
        )),
        drop_reason::LINEAGE => Some((
            crate::sink::notice_code::FOLD_NOT_REUSED_LINEAGE,
            "A saved compaction of this conversation was made under another model or workspace and does not match its history. The full history is sent instead.",
        )),
        _ => None,
    }
}

impl Window {
    /// Loads the context sidecar into the window and binds persistence to
    /// `session_path` (Go: `LoadProjectionSidecar`). A sidecar whose
    /// lineage key no longer matches is rebound to the current key only
    /// when its projection body still matches the canonical covered
    /// prefix; anything else is dropped in memory and the file stays put.
    pub fn load_projection_sidecar(&self, session_path: &str) {
        self.set_session_path(session_path);
        self.reset_compaction_state();
        if session_path.is_empty() {
            return;
        }
        let st = match load_compaction_state(Path::new(session_path)) {
            Ok(Some(st)) => st,
            Ok(None) => return,
            // Corrupt or unsupported state: rebuild from canonical and
            // leave the file with whichever release wrote it.
            Err(_) => return,
        };
        self.with_core(|core| {
            let key = self.prompt_cache_key();
            let (mut normalized, mut key_ok) =
                match lineage_key_compatible(&st.prompt_cache_key, &key) {
                    Some(k) => (k, true),
                    None => (String::new(), false),
                };
            // Keep receipt-only blocked/failed sidecars (no projection body)
            // and legacy top-level blocked hashes, so generation-scoped
            // suppressions survive restart.
            let has_maintenance_signal = !st.projection.covered_prefix_hash.is_empty()
                || !st.blocked_input_hash.is_empty()
                || st
                    .last_receipt
                    .as_ref()
                    .is_some_and(|r| matches!(r.status.as_str(), "blocked" | "failed" | "applied"));
            let msgs = self.session().snapshot();
            if !key.is_empty() && !key_ok {
                // Lineage key changed (upgrade, model/workspace switch).
                // Rebind when the projection body still matches the
                // canonical covered prefix.
                if projection_content_valid(&st, &msgs, None) {
                    normalized = key.clone();
                    key_ok = true;
                }
            }
            if (!key.is_empty() && !key_ok) || !has_maintenance_signal {
                core.compaction_state = CompactionState::default();
                core.checkpoint_state = "none".to_owned();
                if !key.is_empty() && !key_ok && !st.projection.messages.is_empty() {
                    core.checkpoint_state = drop_reason::LINEAGE.to_owned();
                }
                return;
            }
            // Only rewrite legacy native-editing lineage keys; exact
            // matches and another release line's file stay pure-read.
            let mut st = st;
            let mut needs_normalization = false;
            if key_ok && !key.is_empty() && normalized != st.prompt_cache_key {
                st.prompt_cache_key = normalized;
                needs_normalization = !st.foreign();
            }
            let valid =
                !st.projection.messages.is_empty() && projection_valid(&st, &msgs, &key, None);
            let dropped = !valid && !st.projection.messages.is_empty();
            if dropped {
                // Keep blocked receipts / telemetry; drop the unusable
                // projection body.
                st.projection = ContextProjection::default();
            }
            core.compaction_state = st;
            if dropped {
                core.checkpoint_state = drop_reason::HISTORY.to_owned();
            } else if valid {
                core.checkpoint_state = "restored".to_owned();
                if needs_normalization
                    && self
                        .persist_compaction_state(&core.compaction_state)
                        .is_err()
                {
                    // The normalized key is in memory; the next commit
                    // republishes it.
                }
            } else {
                core.checkpoint_state = "none".to_owned();
            }
        });
    }

    /// Rebinds projection persistence to `path` (Go: `BindSessionPath`).
    /// With `load_sidecar` the existing sidecar is loaded (resume/switch);
    /// otherwise the in-memory projection is cleared without deleting
    /// another session's sidecar file.
    pub fn bind_session_path(&self, path: &str, load_sidecar: bool) {
        if load_sidecar {
            self.load_projection_sidecar(path);
            return;
        }
        self.set_session_path(path);
        self.with_core(|core| {
            core.compaction_state = CompactionState::default();
            core.checkpoint_state = "none".to_owned();
            core.cache_state = cache_state::UNKNOWN.to_owned();
            core.restart();
        });
    }

    /// Drops the in-memory and on-disk projection after lineage-changing
    /// operations (Go: `InvalidateProjection` — rewind, branch, fork,
    /// system/model change).
    pub fn invalidate_projection(&self) {
        let path = self.with_core(|core| {
            core.compaction_state = CompactionState::default();
            core.compaction_stuck = false;
            core.last_reported = super::maintenance::MaintenanceReport::default();
            self.session_path()
        });
        if path.is_empty() {
            return;
        }
        // Go skips removal for a foreign release's log (Go:
        // `IsForeignSessionLog`); the session-DAG wave owns that checker,
        // so the removal runs for every bound path here.
        let _ = remove_compaction_state(Path::new(&path));
    }

    /// Drops the installed projection only when it no longer covers the
    /// current canonical transcript (Go: `RevalidateProjection`). A caller
    /// that rewrote history asks this instead of deciding for itself.
    pub fn revalidate_projection(&self) {
        let st = self.with_core(|core| core.compaction_state.clone());
        if st.projection.messages.is_empty() {
            return;
        }
        let msgs = self.session().snapshot();
        if projection_valid(&st, &msgs, &self.prompt_cache_key(), None) {
            return;
        }
        self.invalidate_projection();
    }

    /// Why the fold saved beside the bound session was not reused (Go:
    /// `ProjectionDropped`), or none when there was none or it was.
    pub fn projection_dropped(&self) -> &'static str {
        let state = self.with_core(|core| core.checkpoint_state.clone());
        match state.as_str() {
            drop_reason::HISTORY => drop_reason::HISTORY,
            drop_reason::LINEAGE => drop_reason::LINEAGE,
            _ => drop_reason::NONE,
        }
    }

    /// Says why a fold that was installed stopped being used (Go:
    /// `announceWidenedView`): the request that widens the view to the
    /// whole history is the one that carries the reason, once per cause.
    pub(crate) fn announce_widened_view(&self, st: &CompactionState, key: &str) {
        if st.projection.messages.is_empty() {
            return;
        }
        let drop = if super::projection_lineage_ok(st, key) {
            drop_reason::HISTORY
        } else {
            drop_reason::LINEAGE
        };
        let already = self.with_core(|core| {
            let same = core.checkpoint_state == drop;
            core.checkpoint_state = drop.to_owned();
            same
        });
        if already {
            return;
        }
        if let Some((code, text)) = fold_not_reused_notice(drop) {
            self.sink().emit(Event {
                kind: crate::sink::kind::NOTICE.to_owned(),
                level: crate::sink::level::WARN.to_owned(),
                code: code.to_owned(),
                text: text.to_owned(),
                ..Event::default()
            });
        }
    }

    /// Records the resume-time cache estimate without rewriting history
    /// (Go: `SetCacheState`).
    pub fn set_cache_state(&self, state: &str) {
        let state = match state {
            cache_state::WARM | cache_state::COLD | cache_state::UNKNOWN => state,
            _ => cache_state::UNKNOWN,
        };
        self.with_core(|core| {
            core.cache_state = state.to_owned();
            let st = &mut core.compaction_state;
            if st.schema_version == 0 && st.projection.messages.is_empty() {
                st.schema_version = COMPACTION_STATE_SCHEMA_CURRENT;
            }
            st.last_cache_state = state.to_owned();
        });
    }

    /// The last estimated cache warm/cold/unknown label (Go: `CacheState`).
    pub fn cache_state(&self) -> String {
        let state = self.with_core(|core| core.cache_state.clone());
        if state.is_empty() {
            return cache_state::UNKNOWN.to_owned();
        }
        state
    }

    /// Clears the in-memory compaction state and its restored label (Go:
    /// `resetCompactionState`).
    fn reset_compaction_state(&self) {
        self.with_core(|core| {
            core.compaction_state = CompactionState::default();
            core.checkpoint_state = "none".to_owned();
        });
    }
}

#[cfg(test)]
#[path = "sidecar_tests.rs"]
mod tests;
