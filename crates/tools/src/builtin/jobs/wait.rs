//! TOOL-047 wait/watch decision tables (Go `internal/tools/jobs/watch.go`,
//! `progress.go`).
//!
//! Done here: the four wait outcomes, the watch carry window and seed/fire
//! rules, the produced/step snapshot and its non-negative delta, and the
//! target resolution selection. The merged hit channel, once-wakeup, timer
//! and cancellation propagation are runtime WIP.

#![forbid(unsafe_code)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use regex::Regex;

/// `watchCarry`: bytes kept from the previous write so a match split across
/// two writes stays visible; a stream without newlines cannot grow forever
/// (`watch.go:13`).
pub const WATCH_CARRY: usize = 4 << 10;

/// Byte-stream echo closure handed to a watch.
type EchoSink = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// `WaitOutcome`: matched-while-still-running and timed-out-while-still-running
/// are different facts the caller cannot read off status alone (`watch.go:31-38`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Finished,
    Matched,
    TimedOut,
    Cancelled,
}

impl WaitOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Matched => "matched",
            Self::TimedOut => "timeout",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(raw: &str) -> Option<Self> {
        Some(match raw {
            "finished" => Self::Finished,
            "matched" => Self::Matched,
            "timeout" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

/// Wait call options. `timeout = None` sets no timer; `matched` is supplied
/// by the caller, the host does not choose an interruption word; `echo`
/// streams whole lines outside the job lock (`watch.go:17-27`).
#[derive(Default)]
pub struct WaitOptions {
    pub timeout: Option<Duration>,
    pub matched: Option<Regex>,
    pub echo: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

/// Copy at most the final `n` bytes, so the carry does not alias the buffer
/// still being written (`lastBytes`).
pub fn last_bytes(b: &[u8], n: usize) -> Vec<u8> {
    if n == 0 || b.is_empty() {
        Vec::new()
    } else if b.len() <= n {
        b.to_vec()
    } else {
        b[b.len() - n..].to_vec()
    }
}

/// One registered watch on a job's stream.
pub struct OutputWatch {
    echo: Option<EchoSink>,
    re: Option<Regex>,
    hit: Mutex<bool>,
    carry: Mutex<Vec<u8>>,
    fired: Mutex<bool>,
}

impl OutputWatch {
    /// Register a watch, seeding the carry from the existing tail. A match
    /// already present in the tail fires immediately: a late caller does not
    /// wait out the full timeout for a condition that is already true.
    pub fn new(re: Option<Regex>, echo: Option<EchoSink>, tail: &[u8]) -> (Arc<Self>, bool) {
        let mut already = false;
        let carry = match &re {
            Some(re) if re.is_match(String::from_utf8_lossy(tail).as_ref()) => {
                already = true;
                Vec::new()
            }
            _ => last_bytes(tail, WATCH_CARRY),
        };
        let watch = Arc::new(Self {
            echo,
            re,
            hit: Mutex::new(already),
            carry: Mutex::new(carry),
            fired: Mutex::new(already),
        });
        (watch, already)
    }

    /// Whether this watch has already fired.
    pub fn fired(&self) -> bool {
        *self.fired.lock().expect("watch fired lock")
    }

    /// `notifyWatches`: one write routed to this watch (`watch.go:50-67`).
    /// Returns true when the match fired on this write.
    pub fn notify(&self, p: &[u8]) -> bool {
        if let Some(echo) = &self.echo {
            echo(p);
            return false;
        }
        let mut fired = self.fired.lock().expect("watch fired lock");
        if *fired {
            return false;
        }
        let re = match &self.re {
            Some(re) => re,
            None => return false,
        };
        let mut carry = self.carry.lock().expect("watch carry lock");
        let mut window = Vec::with_capacity(carry.len() + p.len());
        window.extend_from_slice(&carry);
        window.extend_from_slice(p);
        let matched = re.is_match(String::from_utf8_lossy(&window).as_ref());
        if matched {
            *fired = true;
            *self.hit.lock().expect("watch hit lock") = true;
            return true;
        }
        *carry = last_bytes(&window, WATCH_CARRY);
        false
    }
}

/// Non-negative between-wait delta (`progress.go:28-59`).
pub fn delta(after: i64, before: i64) -> i64 {
    (after - before).max(0)
}

/// A job reference the resolver selects over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveRef {
    pub id: String,
    pub session: String,
    pub running: bool,
}

/// `resolve`: empty `ids` lists every running job in the session in insertion
/// order; named ids match scoped by key, unscoped by id (`jobs.go:825-850`).
pub fn resolve<'a>(jobs: &'a [ResolveRef], session: &str, ids: &[String]) -> Vec<&'a ResolveRef> {
    let session = session.trim();
    if ids.is_empty() {
        return jobs
            .iter()
            .filter(|job| job.running && session_matches(session, &job.session))
            .collect();
    }
    ids.iter()
        .filter_map(|want| {
            jobs.iter().find(|job| {
                if session.is_empty() {
                    job.id == *want
                } else {
                    job.session == session && job.id == *want
                }
            })
        })
        .collect()
}

/// Active-session emit gate shared with completion dispatch.
pub fn session_matches(active: &str, session: &str) -> bool {
    active.is_empty() || session.is_empty() || active == session
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_render_the_four_wait_states() {
        assert_eq!(WaitOutcome::Finished.as_str(), "finished");
        assert_eq!(WaitOutcome::Matched.as_str(), "matched");
        assert_eq!(WaitOutcome::TimedOut.as_str(), "timeout");
        assert_eq!(WaitOutcome::Cancelled.as_str(), "cancelled");
        assert_eq!(
            WaitOutcome::from_str("timeout"),
            Some(WaitOutcome::TimedOut)
        );
        assert!(WaitOutcome::from_str("nope").is_none());
    }

    #[test]
    fn a_match_already_in_the_tail_fires_on_registration() {
        let re = Regex::new("needle").unwrap();
        let (_watch, fired) = OutputWatch::new(Some(re), None, b"the needle here");
        assert!(fired);
    }

    #[test]
    fn carry_bridges_a_match_split_across_two_writes() {
        let re = Regex::new("needle").unwrap();
        let (watch, fired) = OutputWatch::new(Some(re), None, b"");
        assert!(!fired);
        assert!(!watch.notify(b"nee"));
        assert!(watch.notify(b"dle"), "split match crosses the carry");
        assert!(watch.fired());
    }

    #[test]
    fn a_fired_watch_ignores_later_writes() {
        let re = Regex::new("ab").unwrap();
        let (watch, _) = OutputWatch::new(Some(re), None, b"");
        assert!(watch.notify(b"ab"));
        assert!(!watch.notify(b"ab"), "one match per watch");
    }

    #[test]
    fn echo_only_watches_never_hit() {
        let seen: Arc<Mutex<Vec<Vec<u8>>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_clone = seen.clone();
        let echo: EchoSink = Arc::new(move |p| {
            seen_clone.lock().expect("seen lock").push(p.to_vec());
        });
        let (watch, fired) = OutputWatch::new(None, Some(echo), b"seed");
        assert!(!fired);
        assert!(!watch.notify(b"hello"));
        assert_eq!(seen.lock().expect("seen lock").len(), 1);
    }

    #[test]
    fn delta_clamps_negative_differences_to_zero() {
        assert_eq!(delta(120, 90), 30);
        assert_eq!(delta(90, 120), 0);
        assert_eq!(delta(120, 120), 0);
    }

    #[test]
    fn resolve_lists_running_in_session_or_matches_named_ids() {
        let jobs = vec![
            ResolveRef {
                id: "a".into(),
                session: "s1".into(),
                running: true,
            },
            ResolveRef {
                id: "b".into(),
                session: "s1".into(),
                running: false,
            },
            ResolveRef {
                id: "c".into(),
                session: "s2".into(),
                running: true,
            },
            ResolveRef {
                id: "d".into(),
                session: String::new(),
                running: true,
            },
        ];
        let all_s1 = resolve(&jobs, "s1", &[]);
        assert_eq!(
            all_s1.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "d"]
        );
        let named = resolve(&jobs, "s1", &["b".into()]);
        assert_eq!(named.len(), 1);
        let unscoped = resolve(&jobs, "", &["d".into()]);
        assert_eq!(unscoped.len(), 1);
        let unscoped_all = resolve(&jobs, "", &[]);
        assert_eq!(unscoped_all.len(), 3);
    }
}
