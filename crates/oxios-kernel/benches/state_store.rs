//! Performance benchmarks for oxios-kernel.
//!
//! Run with: `cargo bench --bench kernel_bench`
#![allow(missing_docs)] // criterion_group macro items lack docs (benches have no public API)
#![allow(clippy::unwrap_used)] // bench setup uses `.unwrap()` (idiomatic for benchmarks)

use criterion::{Criterion, black_box, criterion_group, criterion_main};
use tempfile::TempDir;

fn bench_state_save(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dir = TempDir::new().unwrap();
    // StateStore::new is sync
    let store = oxios_kernel::state_store::StateStore::new(dir.path().into()).unwrap();

    let data = serde_json::json!({
        "key": "value",
        "list": [1, 2, 3, 4, 5],
        "nested": { "a": "b", "c": "d" }
    });

    c.bench_function("state_save_json", |b| {
        b.iter(|| {
            rt.block_on(store.save_json("bench", "item", &data))
                .unwrap();
        });
    });
}

fn bench_git_commit(c: &mut Criterion) {
    let dir = TempDir::new().unwrap();
    let git = oxios_kernel::git_layer::GitLayer::new(dir.path().into(), true).unwrap();
    std::fs::write(dir.path().join("bench.txt"), "hello").unwrap();

    c.bench_function("git_commit_file", |b| {
        b.iter(|| {
            black_box(git.commit_file("bench.txt", "bench commit").unwrap());
        });
    });
}

fn bench_audit_append(c: &mut Criterion) {
    use oxicode_sdk::observability::audit_trail::AuditAction;
    let audit = oxicode_sdk::observability::audit_trail::AuditTrail::new(100_000);

    c.bench_function("audit_append", |b| {
        let mut i = 0u64;
        b.iter(|| {
            i += 1;
            black_box(audit.append(
                format!("agent-{i}"),
                AuditAction::Other {
                    detail: format!("test-{i}"),
                },
                format!("resource-{i}"),
            ));
        });
    });
}

fn bench_resource_snapshot(c: &mut Criterion) {
    let monitor = oxios_kernel::resource_monitor::ResourceMonitor::new(60, 10);

    c.bench_function("resource_snapshot", |b| {
        b.iter(|| {
            black_box(monitor.snapshot());
        });
    });
}

fn bench_kernel_build(c: &mut Criterion) {
    c.bench_function("kernel_build", |b| {
        b.iter(|| {
            // Benchmark state store creation
            let dir = TempDir::new().unwrap();
            black_box(oxios_kernel::state_store::StateStore::new(dir.path().into()).unwrap());
        });
    });
}

criterion_group!(
    benches,
    bench_state_save,
    bench_git_commit,
    bench_audit_append,
    bench_resource_snapshot,
    bench_kernel_build
);
criterion_main!(benches);
