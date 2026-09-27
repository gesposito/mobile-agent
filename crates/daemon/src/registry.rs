//! The target registry: the targets the drivers find, a worker per target a
//! command has used, which session holds which target, and each session's
//! settings.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use crate::driver::{Driver, TargetInfo};
use crate::wire::{Failure, code};
use crate::worker::{Handle, Status, Worker};

/// How long a listing of targets is reused before the drivers are asked
/// again; `devices` always asks.
const DISCOVERY_TTL: Duration = Duration::from_secs(2);

/// How long the daemon waits for each worker to detach when it exits.
const STOP_WAIT: Duration = Duration::from_secs(2);

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
        }
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

    /// Why drivers failed to list their targets the last time.
    pub fn failures(&self) -> &[String] {
        &self.failures
    }

    /// The target a command of `session` addresses.
    pub fn resolve(&mut self, _session: &str) -> Result<TargetInfo, Failure> {
        let mut targets = self.targets(false);
        if targets.is_empty() {
            targets = self.targets(true);
        }
        match targets.len() {
            0 => Err(Failure::new(
                code::NO_TARGET,
                "no target is connected; start an emulator or boot a Simulator, then run devices",
            )),
            1 => Ok(targets.remove(0)),
            n => Err(Failure::new(
                code::AMBIGUOUS_TARGET,
                format!("{n} targets are connected; choose one"),
            )),
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

    /// The sessions that hold a target or set something, by name.
    pub fn sessions(&self) -> &BTreeMap<String, SessionSettings> {
        &self.sessions
    }

    /// The worker of `target`, started when none runs.
    pub fn handle(&mut self, target: &TargetInfo) -> Result<Handle, Failure> {
        if let Some(worker) = self.workers.get(&target.id) {
            return Ok(worker.handle());
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

    /// Stops every worker, detaching every target.
    pub fn stop(&mut self) {
        for (_, worker) in self.workers.drain() {
            worker.stop(STOP_WAIT);
        }
    }
}
