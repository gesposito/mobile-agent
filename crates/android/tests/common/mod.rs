//! Helpers shared by the integration tests, which run against a live device
//! only when [`SERIAL_VAR`] names one.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

/// Environment variable naming the device the integration tests drive.
pub const SERIAL_VAR: &str = "MOBILE_AGENT_ANDROID_SERIAL";

/// The serial under test, or `None` after printing why `test` is skipped.
pub fn serial_or_skip(test: &str) -> Option<String> {
    match std::env::var(SERIAL_VAR) {
        Ok(serial) if !serial.trim().is_empty() => Some(serial.trim().to_owned()),
        _ => {
            eprintln!("skipped {test}: set {SERIAL_VAR} to a connected device's serial to run it");
            None
        }
    }
}

/// A name no other test in any run on this host uses: a tag, the process id
/// and a counter. Safe as a file name and an abstract socket name.
pub fn unique_name(tag: &str) -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "mobile-agent-it-{tag}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// Nearest-rank percentile `p` (0 to 100) of samples; sorts them in place.
pub fn percentile(samples: &mut [f64], p: f64) -> f64 {
    assert!(!samples.is_empty(), "no samples");
    samples.sort_by(f64::total_cmp);
    let rank = ((p / 100.0) * samples.len() as f64).ceil() as usize;
    samples[rank.clamp(1, samples.len()) - 1]
}

/// Deterministic bytes that no transport compression shrinks (xorshift64*).
pub fn pattern(len: usize, seed: u64) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        out.extend_from_slice(&state.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
    }
    out.truncate(len);
    out
}

/// One test at a time drives the device's single UI automation role.
static DEVICE: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Holds the device for the calling test; a test that panicked leaves
/// nothing to protect, so a poisoned lock is taken anyway.
pub fn device_lock() -> std::sync::MutexGuard<'static, ()> {
    DEVICE.lock().unwrap_or_else(|e| e.into_inner())
}

/// The agent jar `agent-android/build.sh` wrote, or `$MOBILE_AGENT_JAR`.
pub fn agent_jar() -> android::agent::AgentJar {
    android::agent::AgentJar::locate().unwrap_or_else(|e| panic!("{e}"))
}

/// `classes.dex` of a jar `jar --create` wrote: found through the central
/// directory, since the local header of a deflated entry carries no sizes.
pub fn dex_of(jar: &[u8]) -> Vec<u8> {
    let u16_at = |at: usize| u16::from_le_bytes([jar[at], jar[at + 1]]) as usize;
    let u32_at = |at: usize| u32::from_le_bytes(jar[at..at + 4].try_into().unwrap()) as usize;
    let end = (0..=jar.len() - 22)
        .rev()
        .find(|&at| jar[at..at + 4] == *b"PK\x05\x06")
        .expect("an end of central directory");
    let mut entry = u32_at(end + 16);
    for _ in 0..u16_at(end + 10) {
        assert_eq!(
            &jar[entry..entry + 4],
            b"PK\x01\x02",
            "a central directory entry"
        );
        let name_len = u16_at(entry + 28);
        let name = &jar[entry + 46..entry + 46 + name_len];
        if name == b"classes.dex" {
            let method = u16_at(entry + 10);
            let size = u32_at(entry + 20);
            let local = u32_at(entry + 42);
            let data = local + 30 + u16_at(local + 26) + u16_at(local + 28);
            let raw = &jar[data..data + size];
            return match method {
                0 => raw.to_vec(),
                8 => miniz_oxide::inflate::decompress_to_vec(raw).expect("inflate classes.dex"),
                other => panic!("compression method {other}"),
            };
        }
        entry += 46 + name_len + u16_at(entry + 30) + u16_at(entry + 32);
    }
    panic!("no classes.dex in the jar");
}

/// The current agent with every `mobile-agent-android <current>` string
/// replaced by `version` (same length), as a bare dex, which `app_process`
/// loads like a jar. The dex checksum is recomputed, as ART verifies it.
pub fn agent_reporting(version: &str) -> android::agent::AgentJar {
    let current = format!("mobile-agent-android {}", android::agent::AGENT_VERSION);
    let other = format!("mobile-agent-android {version}");
    assert_eq!(
        current.len(),
        other.len(),
        "the version must keep its length"
    );
    let jar = agent_jar();
    let mut dex = dex_of(jar.bytes());
    let mut replaced = 0;
    let mut at = 0;
    while let Some(found) = dex[at..]
        .windows(current.len())
        .position(|w| w == current.as_bytes())
    {
        let start = at + found;
        dex[start..start + other.len()].copy_from_slice(other.as_bytes());
        replaced += 1;
        at = start + other.len();
    }
    assert!(replaced >= 2, "the banner and the hello name the version");
    let checksum = miniz_oxide::mz_adler32_oxide(1, &dex[12..]);
    dex[8..12].copy_from_slice(&checksum.to_le_bytes());
    android::agent::AgentJar::from_bytes(dex, version)
}

/// Kills, when dropped, the agents a test started that are still there,
/// by pid and only while `/proc/<pid>/cmdline` still names their socket.
pub struct Leftovers {
    device: android::adb::Device,
    agents: Vec<(u32, String)>,
}

impl Leftovers {
    pub fn new(device: &android::adb::Device) -> Leftovers {
        Leftovers {
            device: device.clone(),
            agents: Vec::new(),
        }
    }

    pub fn track(&mut self, pid: u32, socket: &str) {
        self.agents.push((pid, socket.to_owned()));
    }
}

impl Drop for Leftovers {
    fn drop(&mut self) {
        for (pid, socket) in &self.agents {
            let _ = self.device.shell(&format!(
                "grep -q {socket} /proc/{pid}/cmdline 2>/dev/null && kill -9 {pid}"
            ));
        }
    }
}

/// Whether a process whose command line names `socket` runs.
pub fn agent_alive(device: &android::adb::Device, pid: u32, socket: &str) -> bool {
    device
        .shell(&format!("grep -q {socket} /proc/{pid}/cmdline"))
        .map(|out| out.success())
        .unwrap_or(false)
}
