//! The parent lease (docs/contracts/host.md §1.5): stdin held open by the
//! parent is the watchdog; its EOF ends the host cleanly.

#[derive(Debug, PartialEq, Eq)]
pub enum Lease {
    /// A pipe or socket held open by the parent: watch it for EOF.
    Supervised,
    /// A terminal would never end; the host runs unsupervised.
    Unsupervised,
}

/// Classifies stdin the way `parentLease` does. `std` cannot name the handle
/// type without FFI and this tree forbids unsafe, so the terminal is the only
/// unsupervised case spelled here; a redirected regular file reads as
/// supervised (its EOF then ends the host) — recorded in OPEN_QUESTIONS.md,
/// and unreachable from the shell, which always hands the child a pipe.
pub fn parent_lease(is_terminal: bool) -> Lease {
    if is_terminal {
        Lease::Unsupervised
    } else {
        Lease::Supervised
    }
}

/// Drains the lease to EOF; the parent closing its end is the whole message,
/// identically on every platform.
pub async fn watch_lease(cancel: tokio_util::sync::CancellationToken) {
    let mut stdin = tokio::io::stdin();
    let mut sink = tokio::io::sink();
    let _ = tokio::io::copy(&mut stdin, &mut sink).await;
    cancel.cancel();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_is_not_a_lease() {
        assert_eq!(parent_lease(true), Lease::Unsupervised);
    }
}
