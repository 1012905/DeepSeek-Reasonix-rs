use std::collections::BTreeSet;

use super::*;

#[test]
fn metrics_use_process_global_deltas_and_the_exact_go_json_names() {
    let before = metrics();
    note_enqueue(7);
    note_enqueue(0);
    note_enqueue(-3);
    let after_enqueue = metrics();
    assert_eq!(after_enqueue.enqueue - before.enqueue, 3);
    assert_eq!(after_enqueue.enqueue_bytes - before.enqueue_bytes, 7);

    let before_recovered = after_enqueue;
    note_recovered(4);
    note_recovered(0);
    note_recovered(-2);
    let after_recovered = metrics();
    assert_eq!(after_recovered.recovered - before_recovered.recovered, 4);

    let before_single = after_recovered;
    note_paused();
    note_paused();
    note_steer_accepted();
    note_steer_accepted();
    note_steer_rejected();
    note_steer_rejected();
    note_capacity_reject();
    note_capacity_reject();
    note_uncertain();
    note_uncertain();
    note_tx_fail();
    note_tx_fail();
    let after = metrics();
    assert_eq!(after.paused - before_single.paused, 2);
    assert_eq!(after.steer_accepted - before_single.steer_accepted, 2);
    assert_eq!(after.steer_rejected - before_single.steer_rejected, 2);
    assert_eq!(after.capacity_reject - before_single.capacity_reject, 2);
    assert_eq!(after.uncertain - before_single.uncertain, 2);
    assert_eq!(after.tx_fail - before_single.tx_fail, 2);

    let value = serde_json::to_value(after).unwrap();
    let object = value.as_object().unwrap();
    let names: BTreeSet<&str> = object.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "enqueue",
            "enqueueBytes",
            "recovered",
            "paused",
            "steerAccepted",
            "steerRejected",
            "capacityReject",
            "uncertain",
            "txFail",
        ])
    );
    for name in names {
        assert!(object[name].is_i64(), "{name}");
    }
}
