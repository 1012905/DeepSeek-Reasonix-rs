//! OPT-011 evidence: the gate check hot path (host / origin / cookie,
//! constant-time compare). The naive shape constructs and clones per request;
//! the shipped shape validates the policy once and borrows through the check.

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use reasonix_serve::gate::{GateInput, LoopbackGate, constant_time_eq};

const ORIGIN: &str = "http://127.0.0.1:8080";
const TOKEN: &str = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn gate_input(method: &'static str) -> GateInput<'static> {
    GateInput {
        method,
        host: "127.0.0.1:8080",
        origin: Some(ORIGIN),
        token_cookie: Some(TOKEN),
    }
}

fn bench_gate(c: &mut Criterion) {
    let gate = LoopbackGate::new(ORIGIN, TOKEN);

    // The shipped shape: policy validated once, the check borrows.
    c.bench_function("gate_check_zero_alloc", |b| {
        b.iter(|| {
            let decision = gate.check_http(black_box(gate_input("POST")));
            black_box(decision)
        })
    });

    // The direct-translation shape a lazy port would land on: the gate is
    // rebuilt and the request headers cloned on every call.
    c.bench_function("gate_check_naive_rebuilt", |b| {
        b.iter(|| {
            let g = LoopbackGate::new(black_box(ORIGIN), black_box(TOKEN));
            let decision = g.check_http(GateInput {
                method: black_box("POST"),
                host: black_box("127.0.0.1:8080"),
                origin: Some(black_box(ORIGIN)),
                token_cookie: Some(black_box(TOKEN)),
            });
            black_box(decision)
        })
    });

    // The constant-time comparison on its own: equal and length-different.
    c.bench_function("constant_time_eq_equal", |b| {
        b.iter(|| constant_time_eq(black_box(TOKEN.as_bytes()), black_box(TOKEN.as_bytes())))
    });
    c.bench_function("constant_time_eq_length_diff", |b| {
        b.iter(|| {
            constant_time_eq(
                black_box(TOKEN.as_bytes()),
                black_box(&TOKEN.as_bytes()[..TOKEN.len() - 1]),
            )
        })
    });

    // The misconfigured path must cost the same shape as the allowed one.
    let broken = LoopbackGate::new("https://example.com:8080", TOKEN);
    c.bench_function("gate_check_misconfigured", |b| {
        b.iter(|| {
            let decision = broken.check_http(black_box(gate_input("POST")));
            black_box(decision)
        })
    });
}

criterion_group!(benches, bench_gate);
criterion_main!(benches);
