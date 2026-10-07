//! Microbenchmarks of the JSON wire and of a snapshot through the socket of
//! a daemon on a fake target (design D14), run by `mobile-agent-bench
//! micro`. Nothing is measured yet: task 2.4 adds the benchmarks.

use criterion::{Criterion, criterion_group, criterion_main};

fn benches(_c: &mut Criterion) {}

criterion_group!(daemon, benches);
criterion_main!(daemon);
