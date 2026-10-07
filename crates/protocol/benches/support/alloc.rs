//! A global allocator that counts, for benches that report allocations per
//! iteration beside their time (design D14): criterion counts none. Every
//! allocation costs two relaxed atomic adds, which the timed iterations pay
//! too, so a bench counts over one extra call of its routine outside the
//! timed region and prints the count on a line of its own, which
//! `mobile-agent-bench micro` reads from the bench's output. The mirror's
//! and the daemon's benches include this file by path.

// Not every bench that includes this file uses all of it.
#![allow(dead_code)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// The system allocator, counting what it hands out.
pub struct Counting;

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static BYTES: AtomicU64 = AtomicU64::new(0);

// SAFETY: every method defers to `System` with the arguments it was given.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Relaxed);
        BYTES.fetch_add(layout.size() as u64, Relaxed);
        // SAFETY: the caller upholds `alloc`'s contract, which is `System`'s.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from `System` with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Relaxed);
        BYTES.fetch_add(new_size as u64, Relaxed);
        // SAFETY: `ptr` came from `System` with `layout`, and `new_size` is
        // the caller's, under `realloc`'s contract.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// What a call allocated: every allocation and reallocation, with the bytes
/// each asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub bytes: u64,
}

/// Runs `f` and returns its result with what it allocated. Other threads'
/// allocations count too, so it is called while nothing else runs.
pub fn counted<T>(f: impl FnOnce() -> T) -> (T, Counts) {
    let allocations = ALLOCATIONS.load(Relaxed);
    let bytes = BYTES.load(Relaxed);
    let out = f();
    let counts = Counts {
        allocations: ALLOCATIONS.load(Relaxed) - allocations,
        bytes: BYTES.load(Relaxed) - bytes,
    };
    (out, counts)
}

/// Prints the line `micro` reads for the benchmark `id`, criterion's
/// `group/function/parameter`: `allocations <id> <allocations> <bytes>`.
pub fn report(id: &str, counts: Counts) {
    println!("allocations {id} {} {}", counts.allocations, counts.bytes);
}
