//! The target registry: the targets the drivers find, a worker per target a
//! command has used, which session holds which target, and each session's
//! settings.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::driver::{CompatibilityRow, Driver, Platform, Report, TargetInfo};
use crate::wire::{Failure, code};
use crate::worker::{Handle, Status, Worker};

/// How long a listing of targets is reused before the drivers are asked
/// again; `devices` always asks.
const DISCOVERY_TTL: Duration = Duration::from_secs(2);

/// How long the daemon waits for its workers to detach their targets when
/// it exits: long enough for the Android agent's `shutdown` and a kill.
pub(crate) const STOP_WAIT: Duration = Duration::from_secs(5);

/// Which target a command asks for: `--device` and `--platform`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Selection {
    /// A target id, or a name that one target alone has.
    pub device: Option<String>,
    pub platform: Option<Platform>,
}

impl Selection {
    fn admits(&self, target: &TargetInfo) -> bool {
        self.platform
            .is_none_or(|platform| target.platform == platform)
    }

    /// The targets `device` names: the one with that id, else those whose
    /// name it is, ignoring case.
    fn named<'a>(&self, targets: &[&'a TargetInfo]) -> Vec<&'a TargetInfo> {
        let Some(device) = &self.device else {
            return targets.to_vec();
        };
        if let Some(target) = targets.iter().find(|t| t.id == *device) {
            return vec![target];
        }
        targets
            .iter()
            .filter(|t| t.name.eq_ignore_ascii_case(device))
            .copied()
            .collect()
    }

    fn describe(&self) -> String {
        match (&self.device, self.platform) {
            (Some(device), Some(platform)) => format!("{} target {device:?}", platform.id()),
            (Some(device), None) => format!("target {device:?}"),
            (None, Some(platform)) => format!("{} target", platform.id()),
            (None, None) => "target".to_owned(),
        }
    }
}

/// Targets as an error's details and lines.
fn candidates(targets: &[&TargetInfo]) -> (serde_json::Value, String) {
    let data = targets
        .iter()
        .map(|t| json!({"id": t.id, "platform": t.platform.id(), "class": t.class.id(), "name": t.name}))
        .collect::<Vec<_>>();
    let width = targets.iter().map(|t| t.id.len()).max().unwrap_or(0);
    let lines = targets
        .iter()
        .map(|t| format!("\n  {:width$}  {:7}  {}", t.id, t.platform.id(), t.name))
        .collect::<String>();
    (json!({"candidates": data}), lines)
}

/// What a session sets for itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SessionSettings {
    /// The timeout of waiting commands that give none.
    pub default_timeout: Option<Duration>,
}

pub(crate) struct Registry {
    drivers: Vec<Arc<dyn Driver>>,
    known: Vec<TargetInfo>,
    listed: Option<Instant>,
    /// Drivers whose last listing failed, with the reason.
    failures: Vec<String>,
    workers: HashMap<String, Worker>,
    /// Target id to the session holding it.
    claims: HashMap<String, String>,
    sessions: BTreeMap<String, SessionSettings>,
    /// Set once the workers were stopped, as the daemon exits.
    stopped: bool,
}

impl Registry {
    pub fn new(drivers: Vec<Arc<dyn Driver>>) -> Registry {
        Registry {
            drivers,
            known: Vec::new(),
            listed: None,
            failures: Vec::new(),
            workers: HashMap::new(),
            claims: HashMap::new(),
            sessions: BTreeMap::new(),
            stopped: false,
        }
    }

    /// The rows every driver contributes to `doctor`'s compatibility table.
    pub fn compatibility(&self) -> Vec<CompatibilityRow> {
        self.drivers
            .iter()
            .flat_map(|d| d.compatibility())
            .collect()
    }

    /// The fields every driver adds to `doctor`'s reply.
    pub fn doctor(&self) -> serde_json::Map<String, serde_json::Value> {
        self.drivers.iter().flat_map(|d| d.doctor()).collect()
    }

    pub fn driver_names(&self) -> Vec<String> {
        self.drivers.iter().map(|d| d.name().to_owned()).collect()
    }

    /// The targets connected now, asking the drivers again when `fresh` is
    /// set or the last listing is older than a couple of seconds.
    pub fn targets(&mut self, fresh: bool) -> Vec<TargetInfo> {
        let stale = self.listed.is_none_or(|at| at.elapsed() >= DISCOVERY_TTL);
        if fresh || stale {
            let mut known = Vec::new();
            let mut failures = Vec::new();
            for driver in &self.drivers {
                match driver.discover() {
                    Ok(targets) => known.extend(targets),
                    Err(error) => failures.push(format!("{}: {}", driver.name(), error.message)),
                }
            }
            self.known = known;
            self.failures = failures;
            self.listed = Some(Instant::now());
        }
        self.known.clone()
    }

    /// The targets connected now, as `devices` lists them: every driver
    /// forgets what it keeps between listings, then lists afresh.
    pub fn devices(&mut self) -> Vec<TargetInfo> {
        self.targets(true)
    }

    /// Why drivers failed to list their targets the last time.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// The target a command of `session` addresses: the one `selection`
    /// names; else the only connected target of the selected platform; else
    /// the one target of that platform the session holds. Several without a
    /// choice fail listing them.
    pub fn resolve(&mut self, session: &str, selection: &Selection) -> Result<TargetInfo, Failure> {
        let mut targets = self.targets(false);
        let known = |targets: &[TargetInfo]| {
            let admitted: Vec<&TargetInfo> =
                targets.iter().filter(|t| selection.admits(t)).collect();
            !selection.named(&admitted).is_empty()
        };
        // A target missing from the cached listing may have just connected.
        if targets.is_empty() || !known(&targets) {
            targets = self.targets(true);
        }
        if targets.is_empty() {
            return Err(Failure::new(
                code::NO_TARGET,
                "no target is connected; start an emulator or boot a Simulator, then run devices",
            ));
        }
        let all: Vec<&TargetInfo> = targets.iter().collect();
        let admitted: Vec<&TargetInfo> = all
            .iter()
            .copied()
            .filter(|t| selection.admits(t))
            .collect();
        let named = selection.named(&admitted);
        if selection.device.is_some() && named.is_empty() {
            let (data, lines) = candidates(&all);
            return Err(Failure::new(
                code::TARGET_NOT_FOUND,
                format!(
                    "no {} is connected; connected targets:{lines}",
                    selection.describe()
                ),
            )
            .with_data(data));
        }
        match named.as_slice() {
            [] => {
                let (data, lines) = candidates(&all);
                Err(Failure::new(
                    code::NO_TARGET,
                    format!(
                        "no {} is connected; connected targets:{lines}",
                        selection.describe()
                    ),
                )
                .with_data(data))
            }
            [target] => Ok((*target).clone()),
            several => {
                let held: Vec<&&TargetInfo> = several
                    .iter()
                    .filter(|t| {
                        self.claims
                            .get(&t.id)
                            .is_some_and(|holder| holder == session)
                    })
                    .collect();
                if let [target] = held.as_slice() {
                    return Ok((**target).clone());
                }
                let (data, lines) = candidates(several);
                Err(Failure::new(
                    code::AMBIGUOUS_TARGET,
                    format!(
                        "{} targets are connected; choose one with --device (or narrow with --platform):{lines}",
                        several.len()
                    ),
                )
                .with_data(data))
            }
        }
    }

    /// Gives `target` to `session`, unless another session holds it and
    /// `force` is unset.
    pub fn claim(
        &mut self,
        target: &TargetInfo,
        session: &str,
        force: bool,
    ) -> Result<(), Failure> {
        if let Some(holder) = self.claims.get(&target.id)
            && holder != session
            && !force
        {
            return Err(Failure::new(
                code::TARGET_CLAIMED,
                format!(
                    "{} is claimed by session \"{holder}\"; run the command with --session {holder}, or with --force to take the target over",
                    target.id
                ),
            )
            .with_data(json!({"holder": holder, "target": target.id})));
        }
        self.claims.insert(target.id.clone(), session.to_owned());
        self.sessions.entry(session.to_owned()).or_default();
        Ok(())
    }

    /// Releases `target` if `session` holds it.
    pub fn release(&mut self, target: &str, session: &str) {
        if self
            .claims
            .get(target)
            .is_some_and(|holder| holder == session)
        {
            self.claims.remove(target);
        }
    }

    /// The session holding `target`.
    pub fn holder(&self, target: &str) -> Option<&str> {
        self.claims.get(target).map(String::as_str)
    }

    /// The targets `session` holds.
    pub fn held_by(&self, session: &str) -> Vec<String> {
        let mut held: Vec<String> = self
            .claims
            .iter()
            .filter(|(_, holder)| *holder == session)
            .map(|(target, _)| target.clone())
            .collect();
        held.sort();
        held
    }

    /// Sets `session`'s default timeout for waiting commands.
    pub fn set_default_timeout(&mut self, session: &str, timeout: Duration) {
        self.sessions
            .entry(session.to_owned())
            .or_default()
            .default_timeout = Some(timeout);
    }

    /// `session`'s default timeout, if it set one.
    pub fn default_timeout(&self, session: &str) -> Option<Duration> {
        self.sessions.get(session)?.default_timeout
    }

    /// The sessions that hold a target or set something, by name.
    pub fn sessions(&self) -> &BTreeMap<String, SessionSettings> {
        &self.sessions
    }

    /// The worker of `target`, started when none runs. None starts once the
    /// workers were stopped, so a request that came in as the daemon began
    /// to exit attaches no target it would leave behind.
    pub fn handle(&mut self, target: &TargetInfo) -> Result<Handle, Failure> {
        if let Some(worker) = self.workers.get(&target.id) {
            return Ok(worker.handle());
        }
        if self.stopped {
            return Err(Failure::new(
                code::DAEMON_STOPPING,
                "the daemon is shutting down; run the command again to start a new one",
            ));
        }
        let driver = self
            .drivers
            .iter()
            .find(|driver| driver.name() == target.driver)
            .cloned()
            .ok_or_else(|| Failure::internal(format!("no driver named {}", target.driver)))?;
        let worker = Worker::spawn(target.clone(), driver);
        let handle = worker.handle();
        self.workers.insert(target.id.clone(), worker);
        Ok(handle)
    }

    pub fn worker_status(&self, target: &str) -> Option<Status> {
        self.workers.get(target).map(Worker::status)
    }

    /// What the driver reports about `target`, while it is attached.
    pub fn worker_report(&self, target: &str) -> Option<Arc<dyn Report>> {
        self.workers.get(target).and_then(Worker::report)
    }

    /// Stops every worker, detaching every target: all at once, then waits
    /// for them together, up to [`STOP_WAIT`]. Returns the targets whose
    /// worker had not finished detaching by then, by id. No worker starts
    /// afterwards.
    pub fn stop(&mut self) -> Vec<String> {
        self.stop_within(STOP_WAIT)
    }

    fn stop_within(&mut self, wait: Duration) -> Vec<String> {
        self.stopped = true;
        let workers: Vec<(String, Worker)> = self.workers.drain().collect();
        for (_, worker) in &workers {
            worker.signal_stop();
        }
        let deadline = Instant::now() + wait;
        let mut unstopped: Vec<String> = workers
            .into_iter()
            .filter_map(|(target, worker)| (!worker.join(deadline)).then_some(target))
            .collect();
        unstopped.sort();
        unstopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::AttachOptions;
    use crate::fake::FakeDriver;
    use crate::wire::Success;

    /// A registry of the fake targets `spec`, each taking `stop` to detach,
    /// with every one attached.
    fn attached(spec: &str, stop: Duration) -> Registry {
        let driver: Arc<dyn Driver> = Arc::new(FakeDriver::parse(spec).unwrap().with_stop(stop));
        let mut registry = Registry::new(vec![driver]);
        for target in registry.targets(true) {
            registry
                .handle(&target)
                .unwrap()
                .run(AttachOptions::default(), |_| Ok(Success::new(json!({}))))
                .unwrap();
        }
        registry
    }

    /// Task 8.16: stopping waits for every target to detach, up to the wait,
    /// and names the targets still detaching once it is over.
    #[test]
    fn stopping_names_the_targets_still_detaching_when_the_wait_is_over() {
        let mut quick = attached("android,ios", Duration::from_millis(100));
        let started = Instant::now();
        assert_eq!(
            quick.stop_within(Duration::from_secs(2)),
            Vec::<String>::new()
        );
        let took = started.elapsed();
        assert!(
            (Duration::from_millis(100)..Duration::from_secs(1)).contains(&took),
            "{took:?}"
        );

        let mut slow = attached("android,ios", Duration::from_secs(1));
        let started = Instant::now();
        assert_eq!(
            slow.stop_within(Duration::from_millis(200)),
            ["FAKE-IOS-1", "emulator-5554"]
        );
        let took = started.elapsed();
        assert!(
            (Duration::from_millis(200)..Duration::from_millis(800)).contains(&took),
            "{took:?}"
        );
    }

    /// The fake targets of a spec, counting the listings asked of them and
    /// the times they were told to forget what they keep.
    struct Counting {
        fake: FakeDriver,
        listings: std::sync::atomic::AtomicU32,
        forgets: std::sync::atomic::AtomicU32,
    }

    impl Counting {
        fn new(spec: &str) -> Counting {
            Counting {
                fake: FakeDriver::parse(spec).unwrap(),
                listings: std::sync::atomic::AtomicU32::new(0),
                forgets: std::sync::atomic::AtomicU32::new(0),
            }
        }

        fn listings(&self) -> u32 {
            self.listings.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn forgets(&self) -> u32 {
            self.forgets.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Driver for Counting {
        fn name(&self) -> &str {
            "fake"
        }

        fn discover(&self) -> Result<Vec<TargetInfo>, crate::driver::DriverError> {
            self.listings
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.fake.discover()
        }

        fn attach(
            &self,
            target: &TargetInfo,
            options: &AttachOptions,
        ) -> Result<crate::driver::Attachment, crate::driver::DriverError> {
            self.fake.attach(target, options)
        }

        fn forget(&self) {
            self.forgets
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// Physical-ios, task 9.2 (design D26): a command on a target the
    /// daemon holds attached is resolved from its attachment, by its id or
    /// as the one target its session holds, however old the listing, with
    /// no driver asked: an iPhone's listing probes lockdown and runs
    /// `security` twice, 150 to 220 ms before each command after a pause. A
    /// target that is not attached is still listed, and `devices` lists
    /// afresh, every driver told first to forget what it keeps.
    #[test]
    fn an_attached_target_is_resolved_without_a_listing_and_devices_lists_afresh() {
        let driver = Arc::new(Counting::new("ios-device,ios"));
        let mut registry = Registry::new(vec![Arc::clone(&driver) as Arc<dyn Driver>]);
        let phone = registry
            .targets(true)
            .into_iter()
            .find(|target| target.id == "FAKE-IOS-DEVICE-1")
            .unwrap();
        registry.claim(&phone, "agent", false).unwrap();
        registry
            .handle(&phone)
            .unwrap()
            .run(AttachOptions::default(), |_| Ok(Success::new(json!({}))))
            .unwrap();
        assert_eq!(driver.listings(), 1);
        registry.listed = Instant::now().checked_sub(DISCOVERY_TTL * 2);
        let by_id = Selection {
            device: Some(phone.id.clone()),
            platform: None,
        };
        assert_eq!(registry.resolve("agent", &by_id).unwrap().id, phone.id);
        assert_eq!(
            registry.resolve("agent", &Selection::default()).unwrap().id,
            phone.id,
            "the one target the session holds"
        );
        assert_eq!(
            driver.listings(),
            1,
            "an attached target asks no driver to list again"
        );
        let simulator = Selection {
            device: Some("FAKE-IOS-1".into()),
            platform: None,
        };
        assert_eq!(
            registry.resolve("agent", &simulator).unwrap().id,
            "FAKE-IOS-1"
        );
        assert_eq!(driver.listings(), 2, "a target not attached is listed");
        let listed = registry.devices();
        assert_eq!(listed.len(), 2);
        assert_eq!(driver.listings(), 3, "devices lists afresh");
        assert_eq!(driver.forgets(), 1, "after telling the drivers to forget");
        registry.stop();
    }

    /// Task 8.16: once stopped, the registry starts no worker, so a request
    /// that raced the shutdown attaches no target the exiting daemon would
    /// leave attached.
    #[test]
    fn a_stopped_registry_starts_no_worker() {
        let mut registry = attached("android", Duration::ZERO);
        assert_eq!(registry.stop(), Vec::<String>::new());
        let target = registry.targets(false).remove(0);
        let refused = registry.handle(&target).err().expect("no worker starts");
        assert_eq!(refused.code, code::DAEMON_STOPPING);
        assert!(registry.worker_status(&target.id).is_none());
    }
}
