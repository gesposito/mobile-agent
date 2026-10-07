//! Microbenchmarks of the frame pipeline: downscale, hashes and the JPEG
//! encode (design D14), run by `mobile-agent-bench micro`. Nothing is
//! measured yet: task 2.5 adds the benchmarks over synthetic frames.

use criterion::{Criterion, criterion_group, criterion_main};

fn benches(_c: &mut Criterion) {}

criterion_group!(frames, benches);
criterion_main!(frames);
