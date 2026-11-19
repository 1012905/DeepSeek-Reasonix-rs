//! OPT-012 evidence: the event envelope hot path — one wire serialization,
//! one replay-log append for numbered frames, none for droppable ones (the
//! zero-cost droppable decision runs before anything else).

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use reasonix_contract::eventwire::event::Event;
use reasonix_contract::eventwire::tool::Tool;
use reasonix_serve::broadcaster::{Broadcaster, SubscriptionMode};

fn tool_result(id: &str) -> Event {
    Event {
        kind: "tool_result".to_owned(),
        tool: Some(Tool {
            id: id.to_owned(),
            name: "bash".to_owned(),
            ..Tool::default()
        }),
        ..Event::default()
    }
}

fn text_delta() -> Event {
    Event {
        kind: "text".to_owned(),
        text: "a streaming delta of the answer text".to_owned(),
        ..Event::default()
    }
}

fn bench_emit(c: &mut Criterion) {
    // Numbered frame, no subscribers: serialization + numbering + replay log.
    let solo = Broadcaster::new();
    c.bench_function("emit_numbered_no_subs", |b| {
        b.iter(|| {
            solo.emit(black_box(tool_result("t1")));
        })
    });

    // Droppable frame, no subscribers: serialization only, no replay entry.
    c.bench_function("emit_droppable_no_subs", |b| {
        b.iter(|| {
            solo.emit(black_box(text_delta()));
        })
    });

    // The stalled-subscriber worst case: the queue is over budget, so every
    // numbered emit pays one eviction. Emit must still never block.
    let stalled = Broadcaster::new();
    let _sub = stalled.subscribe(SubscriptionMode::From(0));
    for i in 0..4000 {
        stalled.emit(tool_result(&format!("warm-{i}")));
    }
    c.bench_function("emit_numbered_stalled_subscriber", |b| {
        b.iter(|| {
            stalled.emit(black_box(tool_result("t1")));
        })
    });

    // Droppable against a full live window: shed before any queue work.
    c.bench_function("emit_droppable_stalled_subscriber", |b| {
        b.iter(|| {
            stalled.emit(black_box(text_delta()));
        })
    });
}

criterion_group!(benches, bench_emit);
criterion_main!(benches);
