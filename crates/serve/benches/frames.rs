//! OPT-013 evidence: SSE write-out. The contract freezes per-frame flush, so
//! the optimization is within one frame: one buffer, one write, never merged
//! across frames. The direct-translation shape builds the id line and data
//! line as separate allocations (and would cross a syscall boundary for it).

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::eventwire::tool::Tool;
use reasonix_serve::broadcaster::Broadcaster;
use reasonix_serve::broadcaster::SubscriptionMode;
use reasonix_serve::eventstream::frame_bytes;

const FRAME_JSON: &str = r#"{"kind":"tool_result","tool":{"id":"t1","name":"bash","output":"line one\nline two\n"},"seq":42}"#;

/// The direct-translation renderer: per-line allocations, the shape a
/// literal port of Go's two Fprintf calls would take.
fn frame_bytes_direct(seq: i64, data: &str) -> Vec<u8> {
    if seq > 0 {
        let id = format!("id: {seq}\n");
        let mut out = id.into_bytes();
        out.extend_from_slice(format!("data: {data}\n\n").as_bytes());
        out
    } else {
        format!("data: {data}\n\n").into_bytes()
    }
}

fn bench_frames(c: &mut Criterion) {
    c.bench_function("frame_render_single_buffer", |b| {
        b.iter(|| black_box(frame_bytes(black_box(42), black_box(FRAME_JSON.as_bytes()))))
    });
    c.bench_function("frame_render_direct_allocs", |b| {
        b.iter(|| black_box(frame_bytes_direct(black_box(42), black_box(FRAME_JSON))))
    });

    // Unnumbered frames skip the id line entirely.
    c.bench_function("frame_render_unnumbered", |b| {
        b.iter(|| black_box(frame_bytes(black_box(0), black_box(FRAME_JSON.as_bytes()))))
    });

    // 1000 frames out the door: the same bytes a stream task would hand the
    // socket, one buffer per frame, no cross-frame merging.
    c.bench_function("sse_write_1000_frames", |b| {
        b.iter(|| {
            let mut total = 0usize;
            for seq in 1..=1000i64 {
                let frame = frame_bytes(seq, FRAME_JSON.as_bytes());
                total += frame.len();
            }
            black_box(total)
        })
    });

    // The wire a subscriber sees: emit through the broadcaster and drain the
    // rendered frames, measuring the queue-to-render path end to end.
    c.bench_function("sse_pipeline_100_numbered_frames", |b| {
        b.iter(|| {
            let bc = Broadcaster::new();
            let mut sub = bc.subscribe(SubscriptionMode::From(0));
            for i in 1..=100i64 {
                bc.emit(Event {
                    kind: "tool_result".to_owned(),
                    tool: Some(Tool {
                        id: format!("t{i}"),
                        name: "bash".to_owned(),
                        ..Tool::default()
                    }),
                    ..Default::default()
                });
                let frame = sub.try_recv();
                black_box(frame);
            }
        })
    });
}

criterion_group!(benches, bench_frames);
criterion_main!(benches);
