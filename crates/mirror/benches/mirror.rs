//! Microbenchmarks of the replica, the listings, visibility, deltas, waits
//! and settle (design D14), run by `mobile-agent-bench micro`. Nothing is
//! measured yet: task 2.3 adds the benchmarks over the fixtures.

use criterion::{Criterion, criterion_group, criterion_main};

fn benches(_c: &mut Criterion) {}

criterion_group!(mirror, benches);
criterion_main!(mirror);
