//! OPT-016 evidence: the three-layer load (built-in defaults + user file +
//! project file) against fixed fixtures. The strategy pair runs from
//! pre-read TOML text so the filesystem does not dominate the measurement:
//! the shipped direct per-layer decode (benches/direct.rs) against the
//! plan's Value-tree merge (benches/merge.rs). load_e2e is the shipped
//! Config::load including the real file reads. The direct decode measured
//! faster; src/load.rs records the reasoning.

use std::fs;
use std::path::{Path, PathBuf};

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use reasonix_config::Config;

mod direct;
mod merge;

const USER_TOML: &str = r#"
config_version = 12
language = "zh"

[desktop]
theme = "dark"
telemetry = false
tray = "off"

[serve]
auth_mode = "token"
token = "0123456789abcdef0123456789abcdef"
behind_proxy = true
share_port = 8090

[storage]
state = "D:/data/reasonix-state"
"#;

const PROJECT_TOML: &str = r#"
language = "en"

[desktop]
metrics = false

[agent]
compact_ratio = 0.85
"#;

fn write_fixture(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).expect("bench fixture write");
    path
}

fn bench_load(c: &mut Criterion) {
    let base = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/test-tmp");
    std::fs::create_dir_all(&base).expect("create test-tmp base");
    let dir = tempfile::Builder::new()
        .prefix(".tmpbench")
        .tempdir_in(&base)
        .expect("bench fixture dir");
    let user = write_fixture(dir.path(), "user.toml", USER_TOML);
    let project = write_fixture(dir.path(), "project.toml", PROJECT_TOML);

    let mut group = c.benchmark_group("config_load");
    // The OPT-016 strategy comparison, isolated from filesystem noise: the
    // shipped direct per-layer decode against the plan's Value-tree merge.
    group.bench_function("strategy_direct_per_layer", |b| {
        b.iter(|| black_box(direct::decode_direct_layers(&[USER_TOML, PROJECT_TOML])))
    });
    group.bench_function("strategy_value_merge", |b| {
        b.iter(|| black_box(merge::decode_merged_layers(&[USER_TOML, PROJECT_TOML])))
    });
    // The shipped Config::load end to end, including the real file reads.
    group.bench_function("load_e2e", |b| {
        b.iter(|| black_box(Config::load(&[user.as_path(), project.as_path()])))
    });
    group.finish();
}

criterion_group!(benches, bench_load);
criterion_main!(benches);
