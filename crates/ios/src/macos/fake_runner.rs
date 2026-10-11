//! The tests' fake runner and test manager.
//!
//! [`FakeTestManager`] stands for `idevice`'s `xctest` session: it refuses a
//! runner the fake device does not have installed, reports the session's
//! events as XCTest does, starts the runner's process on the fake device,
//! and lasts as long as that process, as the test manager's session ends
//! when its runner does. Scripted starts fail the test manager's handshake,
//! miss the disk image's test manager, or wait in "enabling automation
//! mode" for an approval. An aborted session leaves the runner running, the
//! case of S10's pulled cable, so the driver's own clean-up is what ends it.
//!
//! [`serve_runner`] is the runner's end of a connection to its port: the
//! protocol's `AgentSession`, saying hello with the installed version,
//! answering snapshots, input, text and screenshots, the snapshots and text
//! from the SwiftUI fixture when the fake device shows it, and closing when
//! the runner's process ends. A runner left from a lost session says hello
//! and closes the connection, as S10's did after its cable pull, and a
//! runner that fell silent keeps its connection and answers nothing on it.
//! A stuck runner keeps the next request it takes, as the real runner's
//! worker keeps a call XCTest never returns from, and answers the later
//! ones `busy`; a muted one answers pings and no request.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use protocol::message::{Call, InputAck, Reply, Screenshot, TextAck};
use protocol::message::{ImageFormat, Rotation, TextMethod};
use protocol::session::{AgentConfig, AgentEvent, AgentSession};
use protocol::snapshot::{Snapshot, Truncation};
use protocol::{Capabilities, code};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::fake::{FakeDevice, RunningRunner};
use super::runner::{SessionLog, StartError, TestManager, XcEvent, XcTask};
use crate::model::RUNNER_NAME;

/// What a start of the fake test manager does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Start {
    /// Starts the runner, as on the phone.
    Normal,
    /// The test manager does not answer its handshake: the session fails
    /// as `idevice`'s does after its 30 s wait, here at once.
    HandshakeUnanswered,
    /// RSD does not list the test manager.
    NoTestManager,
    /// XCTest waits in "enabling automation mode" for `wait`, then starts
    /// the runner when the owner approved, and fails otherwise.
    Approval { wait: Duration, approved: bool },
    /// Starts the runner after `wait`.
    Slow(Duration),
}

#[derive(Default)]
struct Script {
    starts: VecDeque<Start>,
    started: Vec<(String, Option<String>)>,
}

/// A test manager whose sessions start the fake device's runner. Clones
/// share what they were asked.
#[derive(Clone)]
pub(crate) struct FakeTestManager {
    device: Arc<FakeDevice>,
    script: Arc<Mutex<Script>>,
    aborted: Arc<AtomicU32>,
    /// The usbmuxd number it was made for, when it was made for an entry.
    usb_id: Option<u32>,
}

impl FakeTestManager {
    pub(crate) fn new(device: &Arc<FakeDevice>) -> FakeTestManager {
        FakeTestManager {
            device: Arc::clone(device),
            script: Arc::new(Mutex::new(Script::default())),
            aborted: Arc::new(AtomicU32::new(0)),
            usb_id: None,
        }
    }

    /// The same test manager, for the device as usbmuxd numbered it.
    pub(crate) fn for_entry(&self, usb_id: u32) -> FakeTestManager {
        FakeTestManager {
            usb_id: Some(usb_id),
            ..self.clone()
        }
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// What the next starts do, first first; later starts are normal.
    pub(crate) fn script_starts(&self, starts: &[Start]) {
        self.script().starts.extend(starts.iter().cloned());
    }

    /// Every start asked for: the runner and the application under test.
    pub(crate) fn started(&self) -> Vec<(String, Option<String>)> {
        self.script().started.clone()
    }

    /// Sessions the driver aborted while they ran.
    pub(crate) fn aborted(&self) -> u32 {
        self.aborted.load(Ordering::SeqCst)
    }
}

/// Counts a session dropped before it finished: an abort.
struct Aborted {
    count: Arc<AtomicU32>,
    finished: bool,
}

impl Drop for Aborted {
    fn drop(&mut self) {
        if !self.finished {
            self.count.fetch_add(1, Ordering::SeqCst);
        }
    }
}

impl TestManager for FakeTestManager {
    async fn start(
        &self,
        runner: &str,
        target_app: Option<&str>,
        log: Arc<SessionLog>,
    ) -> std::result::Result<XcTask, StartError> {
        self.device
            .check_usb_id(self.usb_id)
            .map_err(|error| StartError::Failed(format!("lockdown: {error}")))?;
        self.script()
            .started
            .push((runner.to_owned(), target_app.map(str::to_owned)));
        for app in std::iter::once(runner).chain(target_app) {
            if !self.device.is_installed(app) {
                return Err(StartError::NotInstalled(app.to_owned()));
            }
        }
        let start = self.script().starts.pop_front().unwrap_or(Start::Normal);
        let device = Arc::clone(&self.device);
        let guard = Aborted {
            count: Arc::clone(&self.aborted),
            finished: false,
        };
        Ok(tokio::spawn(async move {
            // The whole guard moves into the task, so dropping the task
            // before the session ends counts as an abort.
            let mut guard = guard;
            let outcome = session(device, start, log).await;
            guard.finished = true;
            outcome
        }))
    }
}

/// One fake session, as XCTest reports it.
async fn session(
    device: Arc<FakeDevice>,
    start: Start,
    log: Arc<SessionLog>,
) -> std::result::Result<(), String> {
    match start {
        Start::HandshakeUnanswered => {
            tokio::time::sleep(Duration::from_millis(10)).await;
            return Err("xctest session timed out after 30.0s".into());
        }
        Start::NoTestManager => return Err("service not found".into()),
        Start::Slow(wait) => tokio::time::sleep(wait).await,
        Start::Normal | Start::Approval { .. } => {}
    }
    log.push(XcEvent::RunnerReady);
    log.push(XcEvent::BundleReady);
    if let Start::Approval { wait, approved } = start {
        log.line("Enabling automation mode...");
        tokio::time::sleep(wait).await;
        if !approved {
            log.push(XcEvent::InitializationFailed(
                "Timed out while enabling automation mode.".into(),
            ));
            return Err("test runner disconnected before the test plan completed".into());
        }
    }
    log.push(XcEvent::InitializingForUiTesting);
    let mut deaths = device.runner_deaths.subscribe();
    deaths.borrow_and_update();
    let mut pulls = device.pulls.subscribe();
    pulls.borrow_and_update();
    let Some(pid) = device.launch_runner() else {
        return Err("the runner is not installed".into());
    };
    log.push(XcEvent::TestStarted);
    // The session lasts as long as its runner's process and the cable.
    while device.runner().is_some_and(|runner| runner.pid == pid) {
        tokio::select! {
            changed = deaths.changed() => {
                if changed.is_err() {
                    std::future::pending::<()>().await;
                }
            }
            _ = pulls.changed() => return Err("device socket io failed".into()),
        }
    }
    Err("test runner disconnected before the test plan completed".into())
}

/// What the fake runner answers `call` with: the device's SwiftUI fixture's
/// tree and its text entry when the device shows it, an empty tree and
/// text set otherwise.
fn answer(device: &FakeDevice, call: &Call) -> std::result::Result<Reply, protocol::Error> {
    match call {
        Call::Snapshot(_) => Ok(Reply::Snapshot(match device.fixture().as_mut() {
            Some(fixture) => fixture.read(),
            None => Snapshot {
                strings: Vec::new(),
                windows: Vec::new(),
                node_count: 0,
                truncated: Truncation::EMPTY,
                seq: 0,
            },
        })),
        Call::Tap(_) | Call::Swipe(_) | Call::Key(_) => {
            Ok(Reply::Input(InputAck { accepted: true }))
        }
        Call::TextSet(params) => match device.fixture().as_mut() {
            Some(fixture) => fixture
                .enter(&params.text, params.mode)
                .map(|method| Reply::Text(TextAck { method })),
            None => Ok(Reply::Text(TextAck {
                method: TextMethod::DIRECT,
            })),
        },
        Call::Screenshot(_) => Ok(Reply::Screenshot(Screenshot {
            format: ImageFormat::PNG,
            width: 750,
            height: 1334,
            rotation: Rotation::R0,
            data: b"\x89PNG placeholder".to_vec(),
        })),
        other => Err(protocol::Error::new(
            code::UNKNOWN_METHOD,
            format!("the fake runner does not serve {}", other.method()),
        )),
    }
}

/// The runner's end of one connection to its port, until the connection
/// closes or the runner's process ends.
pub(crate) async fn serve_runner(
    device: Arc<FakeDevice>,
    mut stream: DuplexStream,
    runner: RunningRunner,
) {
    let config = AgentConfig {
        agent: format!("{RUNNER_NAME} {}", runner.version),
        capabilities: Capabilities::TREE
            | Capabilities::INPUT
            | Capabilities::SCREENSHOT
            | Capabilities::TEXT_SET,
        ..AgentConfig::default()
    };
    let mut session = AgentSession::new(config, Instant::now());
    let mut deaths = device.runner_deaths.subscribe();
    deaths.borrow_and_update();
    let mut pulls = device.pulls.subscribe();
    pulls.borrow_and_update();
    let mut buffer = vec![0u8; 64 * 1024];
    // The request a stuck runner's worker keeps, as the real runner's
    // `busy` names it.
    let mut kept: Option<u32> = None;
    loop {
        if device.runner_state().silent {
            // A silent runner holds its connection and answers nothing
            // until its process ends or the cable is pulled.
            loop {
                tokio::select! {
                    changed = deaths.changed() => {
                        if changed.is_err() || device.runner().is_none_or(|now| now.pid != runner.pid) {
                            return;
                        }
                    }
                    _ = pulls.changed() => return,
                }
            }
        }
        while let Some(event) = session.poll_event() {
            match event {
                // A runner left from a lost session closes the connection
                // after its hello.
                AgentEvent::Ready(_) if runner.stale => {
                    flush(&mut session, &mut stream).await;
                    return;
                }
                AgentEvent::Ready(_) => {}
                AgentEvent::Request(request) => {
                    let mut state = device.runner_state();
                    state.served.push(request.call.method().to_string());
                    let (stuck, muted) = (state.stuck, state.muted);
                    drop(state);
                    match kept {
                        // A muted runner leaves every request unanswered.
                        _ if muted => {}
                        Some(running) => {
                            session.respond(
                                request.id,
                                Err(protocol::Error::new(
                                    code::BUSY,
                                    format!("the runner is still serving request {running}"),
                                )),
                            );
                        }
                        None if stuck => kept = Some(request.id),
                        None => {
                            session.respond(request.id, answer(&device, &request.call));
                        }
                    }
                }
                AgentEvent::Closed(_) => return,
            }
        }
        if !flush(&mut session, &mut stream).await {
            return;
        }
        let wake = session
            .poll_timeout()
            .map(tokio::time::Instant::from_std)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        // A muted runner's session answers no passed deadline; it still
        // answers the host's pings, which it reads.
        let timed = !device.runner_state().muted;
        tokio::select! {
            read = stream.read(&mut buffer) => match read {
                Ok(0) | Err(_) => session.receive_eof(Instant::now()),
                Ok(read) => session.receive(&buffer[..read], Instant::now()),
            },
            () = tokio::time::sleep_until(wake), if timed => session.handle_timeout(Instant::now()),
            changed = deaths.changed() => {
                if changed.is_err() || device.runner().is_none_or(|now| now.pid != runner.pid) {
                    return;
                }
            }
            // A pulled cable closes the socket; the process stays.
            _ = pulls.changed() => return,
        }
    }
}

/// Writes what the session queued; false once the connection is gone.
async fn flush(session: &mut AgentSession, stream: &mut DuplexStream) -> bool {
    while let Some(bytes) = session.poll_transmit() {
        if stream.write_all(&bytes).await.is_err() {
            return false;
        }
    }
    true
}
