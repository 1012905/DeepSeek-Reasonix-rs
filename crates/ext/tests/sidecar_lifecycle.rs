//! Sidecar lifecycle over a real process boundary (contract §14).
//!
//! The peer is this test binary re-executed through the shared support
//! harness, so the handshake, shutdown, and teardown run against an actual
//! spawned process rather than an in-memory double.

mod support;

use std::time::Duration;

use reasonix_ext::sidecar::{ClientOptions, Manager};

/// Re-exec peer entry: when the helper marker is set the harness serves the
/// protocol loop and exits there; under the normal test run the marker is
/// absent and this is a no-op.
#[test]
fn helper_peer() {
    let _ = support::run_peer_from_env();
}

#[test]
fn spawned_sidecar_handshakes_and_shuts_down_cleanly() {
    let home = support::Home::new();
    let init = support::init_result("alpha", "");
    let fixture = home.install("alpha", &init, &[]);
    assert_eq!(fixture.name, "alpha");
    assert!(fixture.root.exists());

    let mut outcome = Manager::start_packages(
        home.path(),
        support::session(&home),
        &ClientOptions::default(),
    )
    .expect("the enabled helper runtime starts");

    assert_eq!(outcome.manager.len(), 1);
    let client = outcome.manager.client("alpha").expect("alpha client");
    assert_eq!(client.plugin_id(), "alpha");
    let initialized = client.handshake_result().expect("handshake completed");
    assert_eq!(initialized.name, "alpha");
    assert!(!client.crashed());

    outcome.manager.close();
    support::wait_until("sidecar process exits", Duration::from_secs(5), || {
        client.exited()
    });
    assert!(client.exited());
}
