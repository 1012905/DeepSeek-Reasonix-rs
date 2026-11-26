//! Process-local inbox telemetry counters.

use std::sync::atomic::{AtomicI64, Ordering};

use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MetricsSnapshot {
    #[serde(rename = "enqueue")]
    pub enqueue: i64,
    #[serde(rename = "enqueueBytes")]
    pub enqueue_bytes: i64,
    #[serde(rename = "recovered")]
    pub recovered: i64,
    #[serde(rename = "paused")]
    pub paused: i64,
    #[serde(rename = "steerAccepted")]
    pub steer_accepted: i64,
    #[serde(rename = "steerRejected")]
    pub steer_rejected: i64,
    #[serde(rename = "capacityReject")]
    pub capacity_reject: i64,
    #[serde(rename = "uncertain")]
    pub uncertain: i64,
    #[serde(rename = "txFail")]
    pub tx_fail: i64,
}

static METRIC_ENQUEUE: AtomicI64 = AtomicI64::new(0);
static METRIC_ENQUEUE_BYTES: AtomicI64 = AtomicI64::new(0);
static METRIC_RECOVERED: AtomicI64 = AtomicI64::new(0);
static METRIC_PAUSED: AtomicI64 = AtomicI64::new(0);
static METRIC_STEER_ACCEPTED: AtomicI64 = AtomicI64::new(0);
static METRIC_STEER_REJECTED: AtomicI64 = AtomicI64::new(0);
static METRIC_CAPACITY_REJECT: AtomicI64 = AtomicI64::new(0);
static METRIC_UNCERTAIN: AtomicI64 = AtomicI64::new(0);
static METRIC_TX_FAIL: AtomicI64 = AtomicI64::new(0);

pub fn metrics() -> MetricsSnapshot {
    MetricsSnapshot {
        enqueue: METRIC_ENQUEUE.load(Ordering::Relaxed),
        enqueue_bytes: METRIC_ENQUEUE_BYTES.load(Ordering::Relaxed),
        recovered: METRIC_RECOVERED.load(Ordering::Relaxed),
        paused: METRIC_PAUSED.load(Ordering::Relaxed),
        steer_accepted: METRIC_STEER_ACCEPTED.load(Ordering::Relaxed),
        steer_rejected: METRIC_STEER_REJECTED.load(Ordering::Relaxed),
        capacity_reject: METRIC_CAPACITY_REJECT.load(Ordering::Relaxed),
        uncertain: METRIC_UNCERTAIN.load(Ordering::Relaxed),
        tx_fail: METRIC_TX_FAIL.load(Ordering::Relaxed),
    }
}

pub fn note_enqueue(bytes: i64) {
    METRIC_ENQUEUE.fetch_add(1, Ordering::Relaxed);
    if bytes > 0 {
        METRIC_ENQUEUE_BYTES.fetch_add(bytes, Ordering::Relaxed);
    }
}

pub fn note_recovered(n: i64) {
    if n > 0 {
        METRIC_RECOVERED.fetch_add(n, Ordering::Relaxed);
    }
}

pub fn note_paused() {
    METRIC_PAUSED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_steer_accepted() {
    METRIC_STEER_ACCEPTED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_steer_rejected() {
    METRIC_STEER_REJECTED.fetch_add(1, Ordering::Relaxed);
}

pub fn note_capacity_reject() {
    METRIC_CAPACITY_REJECT.fetch_add(1, Ordering::Relaxed);
}

pub fn note_uncertain() {
    METRIC_UNCERTAIN.fetch_add(1, Ordering::Relaxed);
}

pub fn note_tx_fail() {
    METRIC_TX_FAIL.fetch_add(1, Ordering::Relaxed);
}
