//! Support for the host-side tests of the Android agent.
//!
//! Every test drives the agent on the device named by
//! `MOBILE_AGENT_ANDROID_SERIAL` through `crates/protocol`, the way the daemon
//! will: it starts the agent with `app_process` as the shell user and opens
//! its abstract socket as an adb server service stream. Without that variable
//! each test prints why it skips and passes, so `cargo test` runs anywhere.
//!
//! The tests share one emulator with other users, so they run one at a time
//! ([`device`] holds a process-wide lock), hold UI automation only while an
//! agent runs, and kill only processes they started, by pid.

pub mod adb;
pub mod agent;
pub mod client;
pub mod fixture;
pub mod holder;
pub mod tree;

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub use adb::Device;
pub use agent::{Agent, Launch};
pub use client::Client;

/// The environment variable naming the device the tests run against.
pub const SERIAL_VAR: &str = "MOBILE_AGENT_ANDROID_SERIAL";

static DEVICE_LOCK: Mutex<()> = Mutex::new(());

/// A device the tests may use, held exclusively by this test while it lives.
pub struct TestDevice {
    pub device: Device,
    _lock: MutexGuard<'static, ()>,
}

impl std::ops::Deref for TestDevice {
    type Target = Device;
    fn deref(&self) -> &Device {
        &self.device
    }
}

/// The device named by [`SERIAL_VAR`], or `None` after printing why the
/// calling test skips.
pub fn device(test: &str) -> Option<TestDevice> {
    let Ok(serial) = std::env::var(SERIAL_VAR) else {
        println!("skipping {test}: {SERIAL_VAR} is not set");
        return None;
    };
    // A test that panicked while holding the lock leaves nothing to protect.
    let lock = DEVICE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let device = Device::new(serial);
    device.check_ready().unwrap_or_else(|e| panic!("{e}"));
    Some(TestDevice {
        device,
        _lock: lock,
    })
}

/// Milliseconds as a float, for measurements.
pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Polls `f` every `step` until it returns `Some` or `timeout` passes.
pub fn poll<T>(timeout: Duration, step: Duration, mut f: impl FnMut() -> Option<T>) -> Option<T> {
    let end = Instant::now() + timeout;
    loop {
        if let Some(v) = f() {
            return Some(v);
        }
        if Instant::now() >= end {
            return None;
        }
        std::thread::sleep(step);
    }
}

/// Nearest-rank percentile of `values`, which must not be empty.
pub fn percentile(values: &[f64], p: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

/// A random lowercase hex string of `bytes` bytes from `/dev/urandom`.
pub fn random_hex(bytes: usize) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; bytes];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut buf))
        .expect("read /dev/urandom");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}
