//! The runner's session (design D20; spec `ios-device-driver`, "One
//! resident runner per session"): the runner started through the test
//! manager with `idevice`'s `xctest` session, kept on a task for the
//! target's lifetime, and reached over usbmuxd's `Connect` to its loopback
//! port, where the protocol's `HostSession` waits for its hello.
//!
//! Before a start the runner processes left from a lost session are ended,
//! since one outlived a pulled cable in S10 and answered the next attach
//! from the stale process. A start the test manager does not answer is
//! repeated within the attach's time; a runner that is missing or says
//! hello with another version is replaced by the bundle the caller gives.
//! Once it serves, a supervisor task watches the connection and the test
//! manager's session, starts the runner again once when either ends, and
//! reports the restart in the target's status.

use std::future::Future;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::time::{Duration, Instant};

use idevice::dvt::xctest::listener::{XCTestCaseResult, XCUITestListener};
use idevice::dvt::xctest::{TestConfig, XCUITestService};
use idevice::installation_proxy::InstallationProxyClient;
use idevice::provider::{IdeviceProvider, UsbmuxdProvider};
use idevice::{IdeviceError, IdeviceService};
use protocol::message::{Call, Reply};
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::connection::{CallFailure, RunnerConnection};
use super::driver::{Slot, Target};
use super::host::{Host, Link, describe};
use super::tunnel::Limits;
use crate::error::{Error, Result};
use crate::model::{AttachOptions, RunnerState, RunnerStatus};
use crate::readiness::Readiness;
use crate::runner::{self, StartFailure};
use crate::services;

/// How long the installation proxy gets to look the runner up.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(10);

/// The longest the session log keeps; XCTest logs a few dozen lines a
/// start.
const LOG_CAPACITY: usize = 400;

/// How long the steps of a runner's start get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SessionLimits {
    /// The pause between two connections to the runner's port while it
    /// starts: S10 tried every 25 ms.
    pub poll: Duration,
    /// How long a connection waits for the runner's hello, which it sends
    /// as it accepts.
    pub hello: Duration,
    /// How long after XCTest asked for automation mode without the runner's
    /// hello the start reports that it waits for the approval: the hello
    /// came 430 ms after the request in S10.
    pub approval_grace: Duration,
    /// The pause before a start the test manager did not answer is made
    /// again.
    pub retry_pause: Duration,
}

impl Default for SessionLimits {
    fn default() -> SessionLimits {
        SessionLimits {
            poll: Duration::from_millis(25),
            hello: Duration::from_secs(2),
            approval_grace: Duration::from_secs(3),
            retry_pause: Duration::from_secs(2),
        }
    }
}

/// What the test manager's session reported.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum XcEvent {
    /// `_XCT_testRunnerReadyWithCapabilities:`: the runner process runs.
    RunnerReady,
    /// The test bundle loaded and said its protocol version.
    BundleReady,
    /// XCTest began initializing for UI testing.
    InitializingForUiTesting,
    /// XCTest logged "enabling automation mode": it asks the device owner
    /// to approve "Enable UI Automation" unless that was approved since the
    /// phone started, and waits 60 s for it (S10).
    AutomationRequested,
    InitializationFailed(String),
    BootstrapFailed(String),
    TestStarted,
    TestFailed(String),
    PlanFinished,
    Log(String),
}

/// What one test manager session reported, with when, from its start.
#[derive(Debug)]
pub(crate) struct SessionLog {
    started: Instant,
    events: Mutex<Vec<(Duration, XcEvent)>>,
}

impl SessionLog {
    pub(crate) fn new() -> Arc<SessionLog> {
        Arc::new(SessionLog {
            started: Instant::now(),
            events: Mutex::new(Vec::new()),
        })
    }

    pub(crate) fn push(&self, event: XcEvent) {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        if events.len() < LOG_CAPACITY {
            events.push((self.started.elapsed(), event));
        }
    }

    /// A log line, and the automation request it may announce.
    pub(crate) fn line(&self, line: &str) {
        if line
            .to_ascii_lowercase()
            .contains("enabling automation mode")
        {
            self.push(XcEvent::AutomationRequested);
        }
        self.push(XcEvent::Log(line.to_owned()));
    }

    /// When the first event `matching` came, from the session's start.
    pub(crate) fn first(&self, matching: impl Fn(&XcEvent) -> bool) -> Option<Duration> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .find(|(_, event)| matching(event))
            .map(|(at, _)| *at)
    }
}

/// Why the test manager could not start a session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum StartError {
    /// The installation proxy does not list this app: the runner, or the
    /// application under test.
    NotInstalled(String),
    Failed(String),
}

/// The test manager's session for one runner, on a task of its own: it
/// ends with the session, and aborting it closes the test manager's
/// connections.
pub(crate) type XcTask = JoinHandle<std::result::Result<(), String>>;

/// Starts runners through the test manager of one device: the seam the
/// tests replace with a fake session. Its future is `Send`, so the runner's
/// supervisor can start one on a task of its own.
pub(crate) trait TestManager: Send + Sync + 'static {
    /// Starts the xctest session of the runner `runner` with `target_app`
    /// as the application under test, telling `log` what happens. Returns
    /// once the session's task runs.
    fn start(
        &self,
        runner: &str,
        target_app: Option<&str>,
        log: Arc<SessionLog>,
    ) -> impl Future<Output = std::result::Result<XcTask, StartError>> + Send;
}

/// How many times a session starts its runner again after it died: once,
/// so a runner that dies again fails the session, and a new attach starts
/// over.
pub(crate) const RESTARTS: u32 = 1;

/// A runner that said hello: the connection to it and the test manager's
/// session that started it.
pub(crate) struct Live {
    pub(crate) connection: Arc<RunnerConnection>,
    pub(crate) xc: XcTask,
    /// From the start's request to the hello.
    pub(crate) started_in: Duration,
}

/// A target's runner session: what `status` reports of it, the connection
/// that serves, and the supervisor that starts the runner again.
pub(crate) struct RunnerSession {
    pub(crate) target: String,
    pub(crate) options: AttachOptions,
    shared: Mutex<Shared>,
    /// Bumped at every change, for the calls waiting on a restart.
    changes: watch::Sender<u64>,
    /// Set by `release`; the supervisor ends at it.
    stopping: watch::Sender<bool>,
}

struct Shared {
    status: RunnerStatus,
    connection: Option<Arc<RunnerConnection>>,
    /// What calls fail with once the session failed.
    failure: Option<Error>,
    supervisor: Option<std::thread::JoinHandle<()>>,
}

impl RunnerSession {
    pub(crate) fn new(target: &str, options: &AttachOptions) -> Arc<RunnerSession> {
        Arc::new(RunnerSession {
            target: target.to_owned(),
            options: options.clone(),
            shared: Mutex::new(Shared {
                status: RunnerStatus::starting("checking the device"),
                connection: None,
                failure: None,
                supervisor: None,
            }),
            changes: watch::Sender::new(0),
            stopping: watch::Sender::new(false),
        })
    }

    fn shared(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Changes the session and tells the calls waiting on it.
    fn update(&self, change: impl FnOnce(&mut Shared)) {
        change(&mut self.shared());
        self.changes.send_modify(|changes| *changes += 1);
    }

    /// What the target's status reports of the session.
    pub(crate) fn status(&self) -> RunnerStatus {
        self.shared().status.clone()
    }

    /// Whether the session serves or is on its way to: neither failed nor
    /// released.
    pub(crate) fn serving(&self) -> bool {
        !matches!(
            self.shared().status.state,
            RunnerState::Failed { .. } | RunnerState::Stopped
        )
    }

    /// What a start waits for, while the session is starting.
    fn step(&self, step: &str) {
        self.update(|shared| {
            if matches!(shared.status.state, RunnerState::Starting { .. }) {
                shared.status.state = RunnerState::Starting {
                    step: step.to_owned(),
                };
            }
        });
    }

    fn awaiting_approval(&self) {
        self.update(|shared| shared.status.state = RunnerState::AwaitingApproval);
    }

    /// The runner said hello with the expected version.
    pub(crate) fn ready(&self, live: &Live) {
        self.update(|shared| {
            shared.status.state = RunnerState::Ready;
            shared.status.agent = Some(live.connection.agent().to_owned());
            shared.status.capabilities = live.connection.capabilities();
            shared.status.started_in = Some(live.started_in);
            shared.connection = Some(Arc::clone(&live.connection));
        });
    }

    /// The runner died; the supervisor starts it again.
    fn restarting(&self, cause: &str) {
        self.update(|shared| {
            shared.status.state = RunnerState::Restarting {
                cause: cause.to_owned(),
            };
            shared.status.last_death = Some(cause.to_owned());
            shared.connection = None;
        });
    }

    /// The runner said hello again after its restart.
    fn restarted(&self, live: &Live) {
        self.update(|shared| shared.status.restarts += 1);
        self.ready(live);
    }

    /// The session cannot serve until a new attach.
    pub(crate) fn fail(&self, error: &Error) {
        self.update(|shared| {
            shared.status.state = RunnerState::Failed {
                code: error.code(),
                message: error.to_string(),
            };
            shared.connection = None;
            shared.failure = Some(error.clone());
        });
    }

    fn stopped(&self) {
        self.update(|shared| {
            shared.status.state = RunnerState::Stopped;
            shared.connection = None;
        });
    }

    pub(crate) fn set_supervisor(&self, supervisor: std::thread::JoinHandle<()>) {
        self.shared().supervisor = Some(supervisor);
    }

    /// Asks the supervisor to end, closing the connection and the test
    /// manager's session, and returns its thread to be joined.
    pub(crate) fn stop(&self) -> Option<std::thread::JoinHandle<()>> {
        self.stopping.send_replace(true);
        self.shared().supervisor.take()
    }

    fn current(&self) -> (RunnerState, Option<Arc<RunnerConnection>>) {
        let shared = self.shared();
        (shared.status.state.clone(), shared.connection.clone())
    }

    fn failure(&self) -> Error {
        self.shared()
            .failure
            .clone()
            .unwrap_or_else(|| Error::NotAttached {
                target: self.target.clone(),
            })
    }
}

/// Starts the runner on `target` for `session`: checks that RSD lists the
/// test manager, ends the runner processes left from a lost session, then
/// starts the runner through the test manager until its hello, within
/// `deadline`. A start the test manager does not answer is made again; a
/// runner that is missing or says hello with another version is replaced
/// by the bundle the session names, once.
pub(crate) async fn start<H: Host>(
    target: &mut Target<H>,
    session: &RunnerSession,
    limits: SessionLimits,
    dvt: Limits,
    deadline: Instant,
) -> Result<Live> {
    let spec = &session.options.runner;
    let name = target.target.clone();
    target.tunnel.open().await?;
    let listed = target.tunnel.status().map(|status| {
        let found = status
            .services
            .iter()
            .any(|service| service == services::TEST_MANAGER);
        (found, status.services.len())
    });
    if let Some((false, listed)) = listed {
        return Err(Error::TestManagerMissing {
            target: name,
            listed,
        });
    }
    session.step("ending runner processes left from an earlier session");
    let leftovers = target.terminate(&spec.bundle_id, dvt).await?;
    if !leftovers.is_empty() {
        session.update(|shared| shared.status.leftovers.extend(&leftovers));
    }
    let mut installed = false;
    loop {
        if Instant::now() >= deadline {
            return Err(Error::Timeout {
                what: format!("the runner's start on {name}"),
                after: session.options.timeout,
            });
        }
        session.step("starting the runner through the test manager");
        let log = SessionLog::new();
        let requested = Instant::now();
        let target_app = session.options.target_app.as_deref();
        let xc = match target
            .test_manager
            .start(&spec.bundle_id, target_app, Arc::clone(&log))
            .await
        {
            Ok(xc) => xc,
            Err(StartError::NotInstalled(app)) if app == spec.bundle_id => {
                let Some(bundle) = spec.bundle.as_deref().filter(|_| !installed) else {
                    return Err(Error::RunnerNotInstalled {
                        target: name,
                        bundle_id: app,
                    });
                };
                install(target, session, bundle).await?;
                installed = true;
                continue;
            }
            Err(StartError::NotInstalled(app)) => {
                return Err(Error::AppNotInstalled {
                    target: name,
                    bundle_id: app,
                });
            }
            Err(StartError::Failed(reason)) => {
                return Err(Error::RunnerStart {
                    target: name,
                    reason,
                });
            }
        };
        session.step("waiting for the runner's hello");
        match wait_hello(&target.link, spec.port, xc, &log, session, limits, deadline).await {
            Hello::Ready(connection, xc) => {
                if runner::is_expected(spec, connection.agent()) {
                    return Ok(Live {
                        connection: Arc::new(connection),
                        xc,
                        started_in: requested.elapsed(),
                    });
                }
                let reported = connection.agent().to_owned();
                connection.close().await;
                xc.abort();
                target.terminate(&spec.bundle_id, dvt).await?;
                let Some(bundle) = spec.bundle.as_deref().filter(|_| !installed) else {
                    return Err(Error::RunnerVersion {
                        target: name,
                        reported,
                        expected: spec.agent(),
                    });
                };
                install(target, session, bundle).await?;
                installed = true;
            }
            Hello::Ended(reason) => match StartFailure::classify(&reason) {
                StartFailure::NoTestManager => {
                    return Err(Error::TestManagerMissing {
                        target: name,
                        listed: 0,
                    });
                }
                StartFailure::HandshakeUnanswered(_)
                    if Instant::now() + limits.retry_pause < deadline =>
                {
                    session.update(|shared| shared.status.handshake_retries += 1);
                    tokio::time::sleep(limits.retry_pause).await;
                }
                failure => {
                    return Err(Error::RunnerStart {
                        target: name,
                        reason: failure.describe(),
                    });
                }
            },
            Hello::NotApproved => {
                return Err(Error::NotReady {
                    target: name,
                    readiness: Readiness::AutomationNotApproved,
                });
            }
            Hello::TimedOut(xc) => {
                xc.abort();
                return Err(Error::Timeout {
                    what: format!("the runner's hello on {name}"),
                    after: session.options.timeout,
                });
            }
        }
    }
}

/// Installs the runner's bundle at `bundle`, checked on the host first.
async fn install<H: Host>(
    target: &Target<H>,
    session: &RunnerSession,
    bundle: &Path,
) -> Result<()> {
    session.step("installing the runner's bundle");
    let checked = crate::bundle::inspect(bundle).map_err(|reason| Error::InvalidBundle {
        path: bundle.display().to_string(),
        reason,
    })?;
    target
        .link
        .install(bundle)
        .await
        .map_err(|reason| Error::InstallFailed {
            bundle_id: checked.bundle_id,
            reason,
        })?;
    session.update(|shared| shared.status.replaced = true);
    Ok(())
}

/// How a wait for the runner's hello ended.
enum Hello {
    Ready(RunnerConnection, XcTask),
    /// The test manager's session ended first; why.
    Ended(String),
    /// The session asked for automation mode and ended without the hello:
    /// the owner did not approve "Enable UI Automation" in XCTest's 60 s.
    NotApproved,
    TimedOut(XcTask),
}

/// Connects to the runner's port until its hello comes, the test manager's
/// session ends or `deadline` passes, reporting the wait for an approval of
/// "Enable UI Automation" once XCTest asked for automation mode and no hello
/// followed within the grace.
async fn wait_hello<L: Link>(
    link: &L,
    port: u16,
    mut xc: XcTask,
    log: &SessionLog,
    session: &RunnerSession,
    limits: SessionLimits,
    deadline: Instant,
) -> Hello {
    let mut awaiting = false;
    loop {
        if xc.is_finished() {
            let reason = match (&mut xc).await {
                Ok(Ok(())) => "the test plan finished before the runner said hello".to_owned(),
                Ok(Err(reason)) => reason,
                Err(error) => error.to_string(),
            };
            if log
                .first(|event| *event == XcEvent::AutomationRequested)
                .is_some()
            {
                return Hello::NotApproved;
            }
            return Hello::Ended(reason);
        }
        if !awaiting
            && let Some(asked) = log.first(|event| *event == XcEvent::AutomationRequested)
            && log.started.elapsed() >= asked + limits.approval_grace
        {
            session.awaiting_approval();
            awaiting = true;
        }
        if let Ok(stream) = link.connect(port).await
            && let Ok(connection) = RunnerConnection::open(stream, limits.hello).await
        {
            return Hello::Ready(connection, xc);
        }
        if Instant::now() >= deadline {
            return Hello::TimedOut(xc);
        }
        tokio::time::sleep(limits.poll).await;
    }
}

/// Watches the runner of `session` until it is released: when its
/// connection closes or the test manager's session ends, the runner died,
/// and it is started again once; a runner that dies again, or whose restart
/// fails, fails the session. It runs on a thread of its own, driven by the
/// runtime's handle, since rustc cannot show the generic restart's future
/// `Send` for every lifetime it borrows.
pub(crate) async fn supervise<H: Host>(
    slot: Weak<Slot<H>>,
    session: Arc<RunnerSession>,
    mut live: Live,
    limits: SessionLimits,
    dvt: Limits,
) {
    let mut stopping = session.stopping.subscribe();
    loop {
        let cause = tokio::select! {
            reason = live.connection.closed() => format!("its connection closed: {reason}"),
            outcome = &mut live.xc => format!(
                "the test manager's session ended: {}",
                match outcome {
                    Ok(Ok(())) => "the test plan finished".to_owned(),
                    Ok(Err(reason)) => reason,
                    Err(error) => error.to_string(),
                }
            ),
            () = released(&mut stopping) => {
                live.connection.close().await;
                live.xc.abort();
                return;
            }
        };
        live.connection.close().await;
        live.xc.abort();
        if session.status().restarts >= RESTARTS {
            session.fail(&Error::RunnerDied {
                target: session.target.clone(),
                cause: format!("{cause}, after it was started again once"),
            });
            return;
        }
        session.restarting(&cause);
        let Some(slot) = slot.upgrade() else {
            return;
        };
        let deadline = Instant::now() + session.options.timeout;
        let started = {
            let mut target = slot.state.lock().await;
            tokio::select! {
                started = start(&mut target, &session, limits, dvt, deadline) => started,
                () = released(&mut stopping) => return,
            }
        };
        match started {
            Ok(restarted) => {
                session.restarted(&restarted);
                live = restarted;
            }
            Err(error) => {
                session.fail(&restart_failure(&session.target, &cause, error));
                return;
            }
        }
    }
}

/// Waits until the session is released.
async fn released(stopping: &mut watch::Receiver<bool>) {
    // The guard `wait_for` returns is dropped here, before the caller
    // awaits anything else, since it may not cross threads.
    let _ = stopping.wait_for(|stop| *stop).await;
}

/// What a session whose runner died and could not start again fails with.
fn restart_failure(target: &str, cause: &str, error: Error) -> Error {
    match error {
        Error::Disconnected { .. } => error,
        Error::NotFound { .. } => Error::Disconnected {
            target: target.to_owned(),
            cause: format!("{cause}, and the device left usbmuxd's listing"),
        },
        other => Error::RunnerDied {
            target: target.to_owned(),
            cause: format!("{cause}; starting it again failed: {other}"),
        },
    }
}

/// Sends `call` to the runner of `session` and waits for its answer within
/// `timeout`, waiting meanwhile for a runner that is starting again.
pub(crate) async fn call(session: &RunnerSession, call: Call, timeout: Duration) -> Result<Reply> {
    let deadline = Instant::now() + timeout;
    let method = call.method().to_string();
    let mut changes = session.changes.subscribe();
    loop {
        changes.borrow_and_update();
        let (state, connection) = session.current();
        match (state, connection) {
            (RunnerState::Ready, Some(connection)) => {
                let left = deadline.saturating_duration_since(Instant::now());
                match connection.call(call.clone(), left).await {
                    Ok(reply) => return Ok(reply),
                    Err(CallFailure::Refused(error)) => {
                        return Err(Error::Runner { method, error });
                    }
                    Err(CallFailure::Lost(cause)) => {
                        return Err(Error::Disconnected {
                            target: session.target.clone(),
                            cause: format!(
                                "the runner's connection closed before it answered {method} \
                                 ({cause}); the runner is started again"
                            ),
                        });
                    }
                    // The connection was gone before the call was written:
                    // the supervisor is starting the runner again.
                    Err(CallFailure::NotSent(_)) => {}
                }
            }
            (RunnerState::Failed { .. }, _) => return Err(session.failure()),
            (RunnerState::Stopped, _) => {
                return Err(Error::NotAttached {
                    target: session.target.clone(),
                });
            }
            _ => {}
        }
        match tokio::time::timeout_at(deadline.into(), changes.changed()).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                return Err(Error::NotAttached {
                    target: session.target.clone(),
                });
            }
            Err(_) => {
                return Err(Error::Timeout {
                    what: format!("the runner's answer to {method} while it starts"),
                    after: timeout,
                });
            }
        }
    }
}

/// Ends `session` once its supervisor ended, which closed the connection
/// and the test manager's session: signals the runner processes still
/// running and returns their pids.
pub(crate) async fn stop<H: Host>(
    slot: &Slot<H>,
    session: &RunnerSession,
    dvt: Limits,
) -> Result<Vec<u32>> {
    let mut target = slot.state.lock().await;
    let ended = target
        .terminate(&session.options.runner.bundle_id, dvt)
        .await;
    session.stopped();
    ended
}

/// The real test manager: `idevice`'s `XCUITestService` over its own
/// CoreDevice tunnel, as spike S10 drove it.
pub(crate) struct UsbTestManager {
    pub(crate) provider: UsbmuxdProvider,
}

impl TestManager for UsbTestManager {
    async fn start(
        &self,
        runner: &str,
        target_app: Option<&str>,
        log: Arc<SessionLog>,
    ) -> std::result::Result<XcTask, StartError> {
        let lookup = async {
            let mut proxy = InstallationProxyClient::connect(&self.provider).await?;
            // The runner first, so that a missing one is told apart from a
            // missing application under test.
            let runners = proxy.get_apps(None, Some(vec![runner.to_owned()])).await?;
            if !runners.contains_key(runner) {
                return Ok(Err(StartError::NotInstalled(runner.to_owned())));
            }
            match TestConfig::from_installation_proxy(&mut proxy, runner, target_app).await {
                Ok(config) => Ok(Ok(config)),
                Err(IdeviceError::AppNotInstalled) => Ok(Err(StartError::NotInstalled(
                    target_app.unwrap_or(runner).to_owned(),
                ))),
                Err(error) => Err(error),
            }
        };
        let config = timeout(LOOKUP_TIMEOUT, lookup)
            .await
            .map_err(|_| {
                StartError::Failed(format!(
                    "the installation proxy did not look the runner up within {LOOKUP_TIMEOUT:?}"
                ))
            })?
            .map_err(|error| {
                StartError::Failed(format!(
                    "the installation proxy's lookup of the runner: {}",
                    describe(&error)
                ))
            })??;
        let provider: Arc<dyn IdeviceProvider> = Arc::new(self.provider.clone());
        Ok(tokio::spawn(async move {
            let mut listener = Listener { log };
            XCUITestService::new(provider)
                .run(config, &mut listener, None)
                .await
                .map_err(|error| describe(&error))
        }))
    }
}

/// Writes what the session reports to its log.
struct Listener {
    log: Arc<SessionLog>,
}

impl XCUITestListener for Listener {
    async fn did_begin_executing_test_plan(&mut self) -> Result<(), IdeviceError> {
        Ok(())
    }

    async fn did_finish_executing_test_plan(&mut self) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::PlanFinished);
        Ok(())
    }

    async fn test_runner_ready_with_capabilities(&mut self) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::RunnerReady);
        Ok(())
    }

    async fn test_bundle_ready_with_protocol_version(
        &mut self,
        _protocol: u64,
        _minimum: u64,
    ) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::BundleReady);
        Ok(())
    }

    async fn test_case_did_start(
        &mut self,
        _class: &str,
        _method: &str,
    ) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::TestStarted);
        Ok(())
    }

    async fn test_case_did_finish(&mut self, result: XCTestCaseResult) -> Result<(), IdeviceError> {
        self.log
            .push(XcEvent::Log(format!("test finished: {}", result.status)));
        Ok(())
    }

    async fn test_case_did_fail(
        &mut self,
        _class: &str,
        _method: &str,
        message: &str,
        _file: &str,
        _line: u64,
    ) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::TestFailed(message.to_owned()));
        Ok(())
    }

    async fn log_message(&mut self, message: &str) -> Result<(), IdeviceError> {
        self.log.line(message);
        Ok(())
    }

    async fn log_debug_message(&mut self, message: &str) -> Result<(), IdeviceError> {
        self.log.line(message);
        Ok(())
    }

    async fn did_begin_initializing_for_ui_testing(&mut self) -> Result<(), IdeviceError> {
        self.log.push(XcEvent::InitializingForUiTesting);
        Ok(())
    }

    async fn initialization_for_ui_testing_did_fail(
        &mut self,
        description: &str,
    ) -> Result<(), IdeviceError> {
        self.log
            .push(XcEvent::InitializationFailed(description.to_owned()));
        Ok(())
    }

    async fn did_fail_to_bootstrap(&mut self, description: &str) -> Result<(), IdeviceError> {
        self.log
            .push(XcEvent::BootstrapFailed(description.to_owned()));
        Err(IdeviceError::UnexpectedResponse(format!(
            "the runner failed to bootstrap: {description}"
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use plist::{Dictionary, Value};
    use protocol::Capabilities;
    use protocol::message::{Call, Reply, ScreenshotParams, SnapshotParams, TapParams};

    use super::*;
    use crate::macos::driver::tests::{FakeHost, PHONE, core};
    use crate::macos::dvt::{PROCESS_CONTROL, SEND_SIGNAL};
    use crate::macos::fake::{Arg, FakeDevice, RUNNER_EXECUTABLE};
    use crate::macos::fake_runner::Start;
    use crate::model::{RUNNER_BUNDLE_ID, RunnerSpec, RunnerState};
    use crate::services;

    const VERSION: &str = "0.1.0";
    const WAIT: Duration = Duration::from_secs(2);

    fn options() -> AttachOptions {
        AttachOptions {
            runner: RunnerSpec::new(VERSION),
            target_app: None,
            timeout: Duration::from_secs(5),
        }
    }

    /// The recorded phone with the runner of `version` installed.
    fn with_runner(version: &str) -> FakeHost {
        let host = FakeHost::ready();
        host.device.install_runner(version);
        host
    }

    /// A runner bundle of `version`, laid out as a signed device build,
    /// removed when dropped.
    struct RunnerBundle {
        root: PathBuf,
        path: PathBuf,
    }

    impl RunnerBundle {
        fn new(test: &str, version: &str) -> RunnerBundle {
            let root =
                std::env::temp_dir().join(format!("ios-runner-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let path = root.join(format!("{RUNNER_EXECUTABLE}.app"));
            std::fs::create_dir_all(path.join("_CodeSignature")).expect("the bundle");
            let mut info = Dictionary::new();
            info.insert(
                "CFBundleIdentifier".into(),
                Value::String(RUNNER_BUNDLE_ID.into()),
            );
            info.insert(
                "CFBundleExecutable".into(),
                Value::String(RUNNER_EXECUTABLE.into()),
            );
            info.insert(
                "CFBundleSupportedPlatforms".into(),
                Value::Array(vec![Value::String("iPhoneOS".into())]),
            );
            info.insert(
                "CFBundleShortVersionString".into(),
                Value::String(version.into()),
            );
            plist::to_file_binary(path.join("Info.plist"), &info).expect("Info.plist");
            std::fs::write(path.join(RUNNER_EXECUTABLE), b"\xca\xfe").expect("the executable");
            std::fs::write(path.join("_CodeSignature/CodeResources"), b"<plist/>")
                .expect("the signature");
            std::fs::write(path.join("embedded.mobileprovision"), b"profile").expect("the profile");
            RunnerBundle { root, path }
        }
    }

    impl Drop for RunnerBundle {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn snapshot() -> Call {
        Call::Snapshot(SnapshotParams::default())
    }

    /// The pids DVT's process control signalled, in order.
    fn signalled(device: &FakeDevice) -> Vec<u32> {
        device
            .dvt_calls()
            .into_iter()
            .filter(|call| call.service == PROCESS_CONTROL && call.selector == SEND_SIGNAL)
            .filter_map(|call| match call.args.get(1) {
                Some(Arg::Archived(Value::Integer(pid))) => {
                    pid.as_unsigned().and_then(|pid| u32::try_from(pid).ok())
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_runner_starts_through_the_test_manager_and_says_hello_with_its_version() {
        let core = core(with_runner(VERSION));
        let attached = core.attach(PHONE, &options()).expect("attaches");
        assert_eq!(attached.runner.state, RunnerState::Ready);
        assert_eq!(
            attached.runner.agent.as_deref(),
            Some("mobile-agent-ios-runner 0.1.0")
        );
        assert!(
            attached
                .runner
                .capabilities
                .contains(Capabilities::TREE | Capabilities::INPUT)
        );
        assert!(attached.runner.started_in.is_some());
        assert_eq!(
            core.host.test_manager.started(),
            vec![(RUNNER_BUNDLE_ID.to_owned(), None)],
            "started once, through the test manager"
        );
        let reply = core.call(PHONE, snapshot(), WAIT).expect("a snapshot");
        assert!(matches!(reply, Reply::Snapshot(_)), "{reply:?}");
        assert_eq!(core.host.device.runner_served(), vec!["snapshot"]);
        let status = core
            .status(PHONE)
            .expect("status")
            .runner
            .expect("a runner");
        assert_eq!(status.state, RunnerState::Ready);
        assert_eq!(status.restarts, 0);
        let again = core.attach(PHONE, &options()).expect("attached already");
        assert_eq!(again.runner.state, RunnerState::Ready);
        assert_eq!(
            core.host.test_manager.started().len(),
            1,
            "a session that serves is kept"
        );
    }

    #[test]
    fn fifty_commands_are_served_by_one_runner_with_no_restart() {
        let core = core(with_runner(VERSION));
        core.attach(PHONE, &options()).expect("attaches");
        let pid = core.host.device.runner().expect("a runner").pid;
        for index in 0..50 {
            let call = match index % 3 {
                0 => snapshot(),
                1 => Call::Screenshot(ScreenshotParams::default()),
                _ => Call::Tap(TapParams {
                    x: 10,
                    y: 10,
                    duration_ms: None,
                }),
            };
            core.call(PHONE, call, WAIT)
                .unwrap_or_else(|error| panic!("command {index}: {error}"));
        }
        assert_eq!(core.host.device.runner_served().len(), 50);
        assert_eq!(
            core.host.device.runner().map(|runner| runner.pid),
            Some(pid),
            "the same runner process"
        );
        assert_eq!(core.host.device.runner_state().connections, 1);
        let status = core
            .status(PHONE)
            .expect("status")
            .runner
            .expect("a runner");
        assert_eq!(status.restarts, 0);
        assert_eq!(core.host.test_manager.started().len(), 1);
    }

    #[test]
    fn a_runner_of_another_version_is_replaced_by_the_bundle_given() {
        let core = core(with_runner("0.0.9"));
        let bundle = RunnerBundle::new("replaced", VERSION);
        let mut options = options();
        options.runner = RunnerSpec::new(VERSION).with_bundle(&bundle.path);
        let attached = core.attach(PHONE, &options).expect("attaches");
        assert_eq!(
            attached.runner.agent.as_deref(),
            Some("mobile-agent-ios-runner 0.1.0")
        );
        assert!(attached.runner.replaced);
        assert_eq!(core.host.device.installs(), vec![bundle.path.clone()]);
        assert_eq!(
            core.host.test_manager.started().len(),
            2,
            "the old runner said hello, then the new one"
        );
        assert_eq!(
            signalled(&core.host.device).len(),
            1,
            "the old runner was ended"
        );
        core.call(PHONE, snapshot(), WAIT)
            .expect("the new runner serves");
    }

    #[test]
    fn a_runner_of_another_version_without_a_bundle_fails_naming_both_versions() {
        let core = core(with_runner("0.0.9"));
        let error = core.attach(PHONE, &options()).expect_err("another version");
        assert_eq!(error.code(), "runner_version_mismatch", "{error}");
        let message = error.to_string();
        assert!(
            message.contains("0.0.9") && message.contains("0.1.0"),
            "{message}"
        );
        assert!(
            core.host.device.runner().is_none(),
            "the runner of the other version is not left running"
        );
        let status = core
            .status(PHONE)
            .expect("status")
            .runner
            .expect("a runner");
        assert!(
            matches!(
                status.state,
                RunnerState::Failed {
                    code: "runner_version_mismatch",
                    ..
                }
            ),
            "{status:?}"
        );
    }

    #[test]
    fn a_missing_runner_is_installed_from_the_bundle_or_reported() {
        let core = core(FakeHost::ready());
        let error = core.attach(PHONE, &options()).expect_err("no runner");
        assert_eq!(error.code(), "runner_not_installed", "{error}");
        let bundle = RunnerBundle::new("missing", VERSION);
        let mut options = options();
        options.runner = RunnerSpec::new(VERSION).with_bundle(&bundle.path);
        let attached = core
            .attach(PHONE, &options)
            .expect("installed, then attached");
        assert!(attached.runner.replaced);
        assert_eq!(core.host.device.installs(), vec![bundle.path.clone()]);
    }

    /// The runner's status once `done` says so, within two seconds.
    fn runner_once(
        core: &crate::macos::driver::Core<FakeHost>,
        done: impl Fn(&RunnerStatus) -> bool,
    ) -> RunnerStatus {
        let deadline = Instant::now() + WAIT;
        loop {
            let status = core
                .status(PHONE)
                .expect("status")
                .runner
                .expect("a runner");
            if done(&status) || Instant::now() >= deadline {
                return status;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_runner_that_dies_is_started_again_once_and_the_restart_is_reported() {
        let core = core(with_runner(VERSION));
        core.attach(PHONE, &options()).expect("attaches");
        let first = core.host.device.runner().expect("a runner").pid;
        core.host.device.kill_runner();
        // The supervisor starts it again without waiting for a command.
        let status = runner_once(&core, |status| {
            status.restarts == 1 && status.state == RunnerState::Ready
        });
        assert_eq!(status.state, RunnerState::Ready, "{status:?}");
        assert_eq!(status.restarts, 1);
        assert!(status.last_death.is_some(), "{status:?}");
        let second = core.host.device.runner().expect("a runner again").pid;
        assert_ne!(second, first, "a new runner process");
        assert_eq!(core.host.test_manager.started().len(), 2);
        core.call(PHONE, snapshot(), WAIT)
            .expect("served by the restarted runner");

        core.host.device.kill_runner();
        let status = runner_once(&core, |status| {
            matches!(status.state, RunnerState::Failed { .. })
        });
        assert_eq!(status.restarts, 1, "{status:?}");
        let error = core
            .call(PHONE, snapshot(), WAIT)
            .expect_err("restarted once only");
        assert_eq!(error.code(), "runner_died", "{error}");
        let status = core
            .status(PHONE)
            .expect("status")
            .runner
            .expect("a runner");
        assert_eq!(status.restarts, 1);
        assert!(
            matches!(
                status.state,
                RunnerState::Failed {
                    code: "runner_died",
                    ..
                }
            ),
            "{status:?}"
        );
    }

    #[test]
    fn release_stops_the_runner_and_its_session() {
        let core = core(with_runner(VERSION));
        core.attach(PHONE, &options()).expect("attaches");
        let pid = core.host.device.runner().expect("a runner").pid;
        let released = core.release(PHONE).expect("releases");
        assert_eq!(
            released.ended,
            vec![pid],
            "the runner outlived its session and was signalled"
        );
        assert!(core.host.device.runner().is_none());
        assert_eq!(
            core.host.test_manager.aborted(),
            1,
            "the test manager's session ended"
        );
        assert_eq!(core.status(PHONE).expect("status").runner, None);
        let error = core.call(PHONE, snapshot(), WAIT).expect_err("released");
        assert_eq!(error.code(), "not_attached", "{error}");
        assert_eq!(
            core.release(PHONE).expect_err("nothing attached").code(),
            "not_attached"
        );
    }

    #[test]
    fn a_runner_left_from_a_lost_session_is_ended_before_the_start() {
        let host = with_runner(VERSION);
        let stale = host.device.leave_runner(VERSION);
        let core = core(host);
        let attached = core.attach(PHONE, &options()).expect("attaches");
        assert_eq!(attached.runner.leftovers, vec![stale]);
        assert_eq!(signalled(&core.host.device), vec![stale]);
        let runner = core.host.device.runner().expect("a new runner");
        assert_ne!(runner.pid, stale);
        assert!(!runner.stale);
        core.call(PHONE, snapshot(), WAIT)
            .expect("the new runner serves, not the stale one");
    }

    #[test]
    fn a_start_the_test_manager_does_not_answer_is_made_again() {
        let host = with_runner(VERSION);
        host.test_manager
            .script_starts(&[Start::HandshakeUnanswered, Start::HandshakeUnanswered]);
        let core = core(host);
        let attached = core
            .attach(PHONE, &options())
            .expect("attaches on the third start");
        assert_eq!(attached.runner.state, RunnerState::Ready);
        assert_eq!(attached.runner.handshake_retries, 2);
        assert_eq!(core.host.test_manager.started().len(), 3);
    }

    #[test]
    fn a_start_waiting_for_the_automation_approval_is_reported_as_that_state() {
        let host = with_runner(VERSION);
        host.test_manager.script_starts(&[Start::Approval {
            wait: Duration::from_millis(800),
            approved: true,
        }]);
        let core = core(host);
        let seen = std::thread::scope(|scope| {
            let attach = scope.spawn(|| core.attach(PHONE, &options()));
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut seen = None;
            while Instant::now() < deadline && seen.is_none() {
                if let Some(runner) = core.status(PHONE).expect("status").runner
                    && runner.state == RunnerState::AwaitingApproval
                {
                    seen = Some(runner);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            let attached = attach
                .join()
                .expect("the attach ran")
                .expect("approved, attached");
            assert_eq!(attached.runner.state, RunnerState::Ready);
            seen
        });
        assert!(seen.is_some(), "the wait for the approval was reported");
        let readiness = crate::readiness::Readiness::AutomationNotApproved;
        assert_eq!(readiness.code(), "automation_not_enabled");
        assert!(readiness.message().contains("Enable UI Automation"));
    }

    #[test]
    fn a_start_whose_approval_never_comes_fails_naming_the_step() {
        let host = with_runner(VERSION);
        host.test_manager.script_starts(&[Start::Approval {
            wait: Duration::from_millis(200),
            approved: false,
        }]);
        let core = core(host);
        let error = core.attach(PHONE, &options()).expect_err("not approved");
        assert_eq!(error.code(), "automation_not_enabled", "{error}");
        assert!(
            error.to_string().contains("Enable UI Automation"),
            "{error}"
        );
    }

    #[test]
    fn a_device_whose_rsd_lists_no_test_manager_fails_as_the_disk_image() {
        let device = FakeDevice::without(&[services::TEST_MANAGER]);
        device.install_runner(VERSION);
        let core = core(FakeHost::with_device(device));
        let error = core.attach(PHONE, &options()).expect_err("no test manager");
        assert_eq!(error.code(), "disk_image_unmounted", "{error}");
        assert!(error.to_string().contains("testmanagerd"), "{error}");
        assert!(
            core.host.test_manager.started().is_empty(),
            "no start was tried"
        );
    }

    #[test]
    fn a_session_the_test_manager_ends_for_want_of_its_service_is_the_disk_image() {
        let host = with_runner(VERSION);
        host.test_manager.script_starts(&[Start::NoTestManager]);
        let core = core(host);
        let error = core
            .attach(PHONE, &options())
            .expect_err("service not found");
        assert_eq!(error.code(), "disk_image_unmounted", "{error}");
    }

    #[test]
    fn a_call_before_attaching_fails_as_not_attached() {
        let core = core(with_runner(VERSION));
        let error = core
            .call(PHONE, snapshot(), WAIT)
            .expect_err("nothing attached");
        assert_eq!(error.code(), "not_attached", "{error}");
    }

    #[test]
    fn a_locked_phone_does_not_attach() {
        let host = with_runner(VERSION);
        let core = core(FakeHost::with(
            &[(crate::discovery::LOCK_KEY, Value::Boolean(true))],
            crate::model::Trust::Trusted,
            Some(Ok(crate::discovery::tests::recorded_images())),
        ));
        drop(host);
        let error = core.attach(PHONE, &options()).expect_err("locked");
        assert_eq!(error.code(), "device_locked", "{error}");
    }
}
