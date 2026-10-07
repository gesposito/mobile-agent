//! Microbenchmarks of the codec and the framing (design D14), run by
//! `mobile-agent-bench micro`. Nothing is measured yet: task 2.2 adds the
//! benchmarks over the generated snapshots, events and requests.

use criterion::{Criterion, criterion_group, criterion_main};

#[path = "support/alloc.rs"]
mod alloc;

#[global_allocator]
static ALLOCATOR: alloc::Counting = alloc::Counting;

fn benches(_c: &mut Criterion) {
    let (len, counts) = alloc::counted(|| vec![0u8; 64].len());
    assert_eq!(len, 64);
    assert!(counts.allocations >= 1 && counts.bytes >= 64);
}

criterion_group!(codec, benches);
criterion_main!(codec);
