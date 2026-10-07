//! Microbenchmarks of the emulator frame hash (design D14), run by
//! `mobile-agent-bench micro`. Nothing is measured yet: task 2.5 adds the
//! benchmark over a synthetic frame.

use criterion::{Criterion, criterion_group, criterion_main};

fn benches(_c: &mut Criterion) {}

criterion_group!(frames, benches);
criterion_main!(frames);
