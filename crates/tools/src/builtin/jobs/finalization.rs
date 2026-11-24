//! TOOL-048 job finalizers, typed process exits and the uninvoked-job
//! completion path (Go `internal/tools/jobs/finalization.go`, `exit_error.go`).
//!
//! Done here: the `ErrFinalization` sentinel and its one-wrap level, the
//! once-only finalizer set with ordered error join, the `ExitError` three-part
//! record and its `errors.As`-style chain extraction, and the panic-cause
//! wording. Real handle/lease/temp-file release held by finalizers is host
//! resource WIP; the once semantics and wrapping hierarchy are locked here.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;
use std::sync::Mutex;

/// `ErrFinalization` — the sentinel every finalizer failure wraps
/// (`finalization.go:10`).
pub const ERR_FINALIZATION: &str = "job resource finalization failed";

/// Prefix Go assigns to a panic captured inside a job's own finalizer
/// (`finalization.go:41`). Distinct from the job-body panic prefix below.
pub const PANIC_PREFIX: &str = "panic";
/// Prefix Go assigns to a panic recovered from the job run itself
/// (`panic_recovery.go:16`).
pub const INTERNAL_PANIC_PREFIX: &str = "internal error: panic";

/// What finalization was asked to do for a job. `invoked = false` only when
/// the manager can prove the closure will never be called.
#[derive(Default)]
pub struct Finalization {
    pub invoked: bool,
    pub err: Option<Box<dyn Error + Send + Sync>>,
}

/// A finalizer callback: releases whatever host resource the job held. It
/// must not block the job pipeline; errors are joined into the job failure.
pub type FinalizerCallback =
    Box<dyn FnOnce(&Finalization) -> Result<(), Box<dyn Error + Send + Sync>> + Send + Sync>;

/// The outer wrapper around a finalizer failure (`finalization.go:39-47`):
/// `ErrFinalization: <panic: cause>` or `ErrFinalization: <inner>`.
#[derive(Debug)]
pub struct FinalizationError {
    pub inner: Box<dyn Error + Send + Sync>,
}

impl FinalizationError {
    /// Wraps a finalizer's own returned error at the `ErrFinalization` level.
    pub fn wrap(inner: Box<dyn Error + Send + Sync>) -> Self {
        Self { inner }
    }

    /// Wraps a captured panic cause: `ErrFinalization: panic: <cause>`.
    pub fn from_panic(cause: &str) -> Self {
        let message = format!("{PANIC_PREFIX}: {cause}");
        Self {
            inner: Box::new(std::io::Error::other(message)),
        }
    }
}

impl fmt::Display for FinalizationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{ERR_FINALIZATION}: {}", self.inner)
    }
}

impl Error for FinalizationError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.inner.as_ref())
    }
}

/// Whether the error chain carries the `ErrFinalization` wrapper
/// (`errors.Is`).
pub fn is_finalization(err: &(dyn Error + Send + Sync + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(candidate) = current {
        if candidate.downcast_ref::<FinalizationError>().is_some() {
            return true;
        }
        if candidate.to_string().starts_with(ERR_FINALIZATION) {
            return true;
        }
        current = candidate.source();
    }
    false
}

/// `panicCause`: format a captured panic value with the given prefix.
pub fn panic_cause(prefix: &str, value: &str) -> String {
    format!("{prefix}: {value}")
}

/// One job's finalizer set, executed exactly once (`jobFinalizers.once`).
pub struct JobFinalizers {
    ran: Mutex<bool>,
    callbacks: Mutex<Vec<FinalizerCallback>>,
    err_text: Mutex<Option<String>>,
}

impl Default for JobFinalizers {
    fn default() -> Self {
        Self::new()
    }
}

impl JobFinalizers {
    pub fn new() -> Self {
        Self {
            ran: Mutex::new(false),
            callbacks: Mutex::new(Vec::new()),
            err_text: Mutex::new(None),
        }
    }

    /// Registers a finalizer. Called before the job runs.
    pub fn push(&self, callback: FinalizerCallback) {
        self.callbacks
            .lock()
            .expect("finalizers lock")
            .push(callback);
    }

    /// Runs every registered finalizer in order, joining their errors.
    /// Subsequent calls return the stored accumulated error without rerunning
    /// the callbacks.
    pub fn finalize(&self, state: &Finalization) -> Result<(), FinalizationError> {
        let mut ran = self.ran.lock().expect("finalizers once lock");
        if *ran {
            return match self.err_text.lock().expect("finalizers err lock").clone() {
                Some(text) => Err(FinalizationError::wrap(Box::new(std::io::Error::other(
                    text,
                )))),
                None => Ok(()),
            };
        }
        *ran = true;
        drop(ran);

        let mut guard = self.callbacks.lock().expect("finalizers list lock");
        let callbacks: Vec<FinalizerCallback> = guard.drain(..).collect();
        drop(guard);
        let mut joined: Option<Box<dyn Error + Send + Sync>> = None;
        for callback in callbacks {
            if let Err(inner) = callback(state) {
                joined = Some(match joined {
                    Some(previous) => Box::new(JoinedErrors { previous, inner }),
                    None => inner,
                });
            }
        }

        let stored_text = joined.as_ref().map(|err| err.to_string());
        *self.err_text.lock().expect("finalizers err lock") = stored_text;
        match joined {
            Some(err) => Err(FinalizationError::wrap(err)),
            None => Ok(()),
        }
    }
}

#[derive(Debug)]
struct JoinedErrors {
    previous: Box<dyn Error + Send + Sync>,
    inner: Box<dyn Error + Send + Sync>,
}

impl fmt::Display for JoinedErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}; {}", self.previous, self.inner)
    }
}

impl Error for JoinedErrors {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.inner.as_ref())
    }
}

/// `ExitError`: a typed process exit the producer knows how to classify; the
/// reader does not re-parse the error text (`exit_error.go:9-13`).
#[derive(Debug)]
pub struct ExitError {
    /// Exit status; `0` when the signal terminated it.
    pub code: i32,
    /// Terminating signal; `0` when the process exited on its own.
    pub signal: i32,
    pub err: Box<dyn Error + Send + Sync>,
}

impl ExitError {
    pub fn new(code: i32, signal: i32, err: Box<dyn Error + Send + Sync>) -> Self {
        Self { code, signal, err }
    }
}

impl fmt::Display for ExitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.err.fmt(f)
    }
}

impl Error for ExitError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.err.as_ref())
    }
}

/// The typed exit details worth surfacing on a `job_failed` Notice; the
/// original error text stays on the [`ExitError`] itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitInfo {
    pub code: i32,
    pub signal: i32,
}

/// `exitOf`: walk the error chain and extract the typed exit, if any
/// (`errors.As`).
pub fn exit_of(err: &(dyn Error + Send + Sync + 'static)) -> Option<ExitInfo> {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(candidate) = current {
        if let Some(ex) = candidate.downcast_ref::<ExitError>() {
            return Some(ExitInfo {
                code: ex.code,
                signal: ex.signal,
            });
        }
        current = candidate.source();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(text: &str) -> Box<dyn Error + Send + Sync> {
        Box::new(std::io::Error::other(text.to_owned()))
    }

    #[test]
    fn exit_error_keeps_the_inner_text_and_unwraps_to_it() {
        let ex = ExitError::new(-1, 9, msg("signal: killed"));
        assert_eq!(ex.to_string(), "signal: killed");
        assert!(ex.source().is_some());
    }

    #[test]
    fn exit_of_extracts_the_typed_exit_from_a_wrapped_chain() {
        let ex = ExitError::new(-1, 9, msg("signal: killed"));
        let wrapped: Box<dyn Error + Send + Sync> = Box::new(FinalizationError::wrap(Box::new(ex)));
        let info = exit_of(wrapped.as_ref()).expect("exit in chain");
        assert_eq!(info.code, -1);
        assert_eq!(info.signal, 9);
    }

    #[test]
    fn exit_of_returns_none_for_a_plain_error() {
        assert!(exit_of(msg("boom").as_ref()).is_none());
    }

    #[test]
    fn finalizers_run_exactly_once_and_join_errors_in_order() {
        let calls = std::sync::Mutex::new(Vec::new());
        let finalizers = JobFinalizers::new();
        let calls_a = std::sync::Arc::new(calls);
        let calls_b = calls_a.clone();
        finalizers.push(Box::new(move |_state: &Finalization| {
            calls_a.lock().expect("calls lock").push("a");
            Err(msg("first cleanup failed"))
        }));
        let calls_c = calls_b.clone();
        finalizers.push(Box::new(move |_state: &Finalization| {
            calls_c.lock().expect("calls lock").push("b");
            Err(msg("second cleanup failed"))
        }));

        let state = Finalization::default();
        let first = finalizers.finalize(&state).expect_err("two failures");
        assert!(is_finalization(&first));
        assert!(first.to_string().contains("first cleanup failed"));
        assert!(first.to_string().contains("second cleanup failed"));

        let second = finalizers.finalize(&state).expect_err("stored error");
        assert!(is_finalization(&second));
        assert_eq!(*calls_b.lock().expect("calls lock"), vec!["a", "b"]);
    }

    #[test]
    fn a_clean_finalize_is_ok_and_a_rerun_stays_ok() {
        let finalizers = JobFinalizers::new();
        finalizers.push(Box::new(|_state: &Finalization| Ok(())));
        finalizers
            .finalize(&Finalization::default())
            .expect("clean");
        finalizers
            .finalize(&Finalization::default())
            .expect("rerun does not fail");
    }

    #[test]
    fn panic_finalizer_wraps_at_the_errfinalization_level() {
        let err = FinalizationError::from_panic("boom");
        assert_eq!(
            err.to_string(),
            "job resource finalization failed: panic: boom"
        );
        assert!(is_finalization(&err));
        assert_eq!(
            panic_cause(INTERNAL_PANIC_PREFIX, "boom"),
            "internal error: panic: boom"
        );
    }
}
