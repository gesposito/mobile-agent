//! `IosDriver`: the driver traits over `idevice`, on a tokio runtime the
//! driver owns, as the Android driver's emulator controller owns one
//! (`crates/android/src/session/controller.rs`). The traits are synchronous;
//! each call blocks on the runtime, from a thread outside it.
//!
//! The logic lives in [`Core`], generic over the host so that the tests run
//! it on recorded listings and values and a fake endpoint instead of a
//! device. What the driver opens for a device, its tunnel, DVT's channels
//! over it and a lockdown session for the lock, is kept per target until the
//! target is reset; a command on a device it has not connected connects it
//! first.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use idevice::ReadWrite;
use idevice::dvt::message::Message;
use idevice::dvt::remote_server::RemoteServerClient;
use tokio::time::timeout;

use crate::bundle;
use crate::discovery::{self, usb_entries};
use crate::driver::{Devices, Lifecycle};
use crate::error::{Error, Result};
use crate::model::{
    Connected, DeviceInfo, DiskImage, Installed, LaunchOptions, Launched, MountReport, MuxEntry,
    TargetStatus, Terminated,
};
use crate::readiness::{self, Need, Readiness};

use super::dvt::{self, CallError, DvtChannel};
use super::host::{Host, Link, UsbHost, describe};
use super::image::{self, MountLimits};
use super::tunnel::{Limits, Opener, Tunnel};

/// How long `terminate` waits for the processes to leave DVT's list: 22 ms
/// after the request in spike S9.
const TERMINATE_WAIT: Duration = Duration::from_secs(2);
/// The pause between two reads of the process list while it waits.
const TERMINATE_POLL: Duration = Duration::from_millis(10);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Drives physical iOS devices attached over USB.
pub struct IosDriver {
    core: Core<UsbHost>,
}

impl IosDriver {
    /// Starts the driver's runtime; nothing is asked of usbmuxd or a device
    /// until a method needs it.
    pub fn open() -> Result<IosDriver> {
        Ok(IosDriver {
            core: Core::new(UsbHost::new()?)?,
        })
    }

    /// Closes what the driver opened for `target`, its tunnel and the
    /// service connections over it; the next command opens them anew.
    pub fn reset_target(&self, target: &str) -> Result<()> {
        self.core.reset_target(target)
    }
}

impl Devices for IosDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.core.devices()
    }

    fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        self.core.require(target, need)
    }

    fn connect(&self, target: &str) -> Result<Connected> {
        self.core.connect(target)
    }

    fn status(&self, target: &str) -> Result<TargetStatus> {
        self.core.status(target)
    }
}

impl Lifecycle for IosDriver {
    fn install(&self, target: &str, app: &Path) -> Result<Installed> {
        self.core.install(target, app)
    }

    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched> {
        self.core.launch(target, bundle_id, options)
    }

    fn terminate(&self, target: &str, bundle_id: &str) -> Result<Terminated> {
        self.core.terminate(target, bundle_id)
    }

    fn running(&self, target: &str, bundle_id: &str) -> Result<Option<u32>> {
        self.core.running(target, bundle_id)
    }
}

/// What the driver keeps for one device.
pub(crate) struct Target<O: Opener, L: Link> {
    target: String,
    tunnel: Tunnel<O>,
    link: L,
    disk_image: Option<MountReport>,
    /// DVT's channels by identifier, each on a connection of its own, kept
    /// for the target's lifetime: a new connection costs 25 to 37 ms (S9).
    channels: BTreeMap<&'static str, DvtChannel>,
}

impl<O: Opener, L: Link> Target<O, L> {
    /// What `status` reports of the target.
    fn snapshot(&self) -> TargetStatus {
        TargetStatus {
            tunnel: self.tunnel.status(),
            disk_image: self.disk_image.clone(),
            services_open: self.channels.keys().copied().collect(),
        }
    }

    /// The channel `identifier`, opened on a DVT connection of its own unless
    /// it serves. DVT sends the services it publishes first, which are read
    /// before the channel opens, as `idevice`'s own process-control tool
    /// reads them.
    async fn channel(&mut self, identifier: &'static str, limits: Limits) -> Result<&DvtChannel> {
        if !self.channels.get(identifier).is_some_and(DvtChannel::alive) {
            self.channels.remove(identifier);
            let mut client: RemoteServerClient<Box<dyn ReadWrite>> = self.tunnel.connect().await?;
            match timeout(limits.reply, client.read_message(0)).await {
                Ok(Ok(_capabilities)) => {}
                Ok(Err(error)) => {
                    return Err(Error::Device {
                        call: "DVT's capabilities".into(),
                        reason: describe(&error),
                    });
                }
                Err(_) => {
                    return Err(Error::Timeout {
                        what: "DVT's capabilities".into(),
                        after: limits.reply,
                    });
                }
            }
            let channel = DvtChannel::open(client, identifier, limits.reply)
                .await
                .map_err(|reason| Error::Device {
                    call: format!("opening DVT's {identifier}"),
                    reason,
                })?;
            self.channels.insert(identifier, channel);
        }
        Ok(self
            .channels
            .get(identifier)
            .expect("the channel was just opened"))
    }

    /// Sends `call` on the channel `identifier` and returns its reply. A
    /// call whose connection was gone before it was written goes again on a
    /// new one, once, through a tunnel opened again if it dropped; so does a
    /// `repeatable` call whose connection closed before its reply.
    async fn call(
        &mut self,
        identifier: &'static str,
        call: dvt::Call,
        repeatable: bool,
        limits: Limits,
    ) -> Result<Message> {
        let selector = call.selector;
        let mut retried = false;
        loop {
            let channel = self.channel(identifier, limits).await?;
            let error = match channel.call(call.clone(), limits.reply).await {
                Ok(reply) => return Ok(reply),
                Err(error) => error,
            };
            self.channels.remove(identifier);
            let again = match &error {
                CallError::NotSent(_) => true,
                CallError::Closed(_) => repeatable,
                CallError::TimedOut(_) => false,
            };
            if again && !retried {
                retried = true;
                continue;
            }
            return Err(match error {
                CallError::TimedOut(after) => Error::Timeout {
                    what: format!("DVT's reply to {selector}"),
                    after,
                },
                CallError::NotSent(cause) | CallError::Closed(cause) => Error::Disconnected {
                    target: self.target.clone(),
                    cause: format!("DVT's {selector}: {cause}"),
                },
            });
        }
    }

    /// DVT's process list.
    async fn processes(&mut self, limits: Limits) -> Result<Vec<dvt::Process>> {
        let reply = self
            .call(dvt::DEVICE_INFO, dvt::running_processes(), true, limits)
            .await?;
        dvt::processes(&reply).map_err(|reason| Error::Device {
            call: "DVT's runningProcesses".into(),
            reason,
        })
    }

    /// The pids of `bundle_id`'s processes: an app's own, which DVT lists
    /// with its bundle identifier on iOS 26.6.2, and not its extensions',
    /// which have identifiers of their own.
    async fn pids(&mut self, bundle_id: &str, limits: Limits) -> Result<Vec<u32>> {
        Ok(self
            .processes(limits)
            .await?
            .into_iter()
            .filter(|process| process.bundle_id.as_deref() == Some(bundle_id))
            .map(|process| process.pid)
            .collect())
    }

    /// Ends `bundle_id`'s processes with SIGKILL and waits until they left
    /// the process list; returns the pids signalled.
    async fn terminate(&mut self, bundle_id: &str, limits: Limits) -> Result<Vec<u32>> {
        let pids = self.pids(bundle_id, limits).await?;
        for pid in &pids {
            // A signal is not sent twice: the pid could belong to another
            // process by then.
            self.call(
                dvt::PROCESS_CONTROL,
                dvt::send_signal(*pid, dvt::SIGKILL),
                false,
                limits,
            )
            .await?;
        }
        let deadline = Instant::now() + TERMINATE_WAIT;
        while !pids.is_empty() {
            let left: Vec<u32> = self
                .pids(bundle_id, limits)
                .await?
                .into_iter()
                .filter(|pid| pids.contains(pid))
                .collect();
            if left.is_empty() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout {
                    what: format!("the end of {bundle_id}'s processes {left:?}"),
                    after: TERMINATE_WAIT,
                });
            }
            tokio::time::sleep(TERMINATE_POLL).await;
        }
        Ok(pids)
    }
}

/// The slots of a host's devices.
type Slots<H> = HashMap<String, Arc<Slot<<H as Host>::Opener, <H as Host>::Link>>>;

/// A device's state, behind the lock its commands take, and what `status`
/// reports, which it reads without that lock while a command runs.
pub(crate) struct Slot<O: Opener, L: Link> {
    state: tokio::sync::Mutex<Target<O, L>>,
    snapshot: Mutex<TargetStatus>,
}

/// The driver's state and logic over a host.
pub(crate) struct Core<H: Host> {
    /// Declared before the runtime, so that what the targets keep open is
    /// dropped while the tasks serving it can still end on their own.
    targets: Mutex<Slots<H>>,
    host: H,
    runtime: tokio::runtime::Runtime,
    limits: Limits,
    mount_limits: MountLimits,
}

impl<H: Host> Core<H> {
    pub(crate) fn new(host: H) -> Result<Core<H>> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ios-driver")
            .enable_all()
            .build()
            .map_err(|error| Error::Io {
                what: "starting the iOS driver's runtime".into(),
                reason: error.to_string(),
            })?;
        Ok(Core {
            targets: Mutex::new(HashMap::new()),
            host,
            runtime,
            limits: Limits::default(),
            mount_limits: MountLimits::default(),
        })
    }

    /// Shorter waits, for the tests.
    #[cfg(test)]
    pub(crate) fn with_limits(mut self, limits: Limits, mount_limits: MountLimits) -> Core<H> {
        self.limits = limits;
        self.mount_limits = mount_limits;
        self
    }

    pub(crate) fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.runtime.block_on(async {
            let entries = usb_entries(self.host.list().await?);
            let identity = self.host.identity();
            let mut devices = Vec::with_capacity(entries.len());
            for entry in &entries {
                let (values, probe) = self.host.probe(entry).await;
                devices.push(discovery::device_info(
                    &entry.udid,
                    &values,
                    probe,
                    &identity,
                ));
            }
            Ok(devices)
        })
    }

    pub(crate) fn require(&self, target: &str, need: Need) -> Result<DeviceInfo> {
        self.runtime.block_on(async {
            let entry = self.entry(target).await?;
            let (values, probe) = self.host.probe(&entry).await;
            let identity = self.host.identity();
            let info = discovery::device_info(target, &values, probe, &identity);
            readiness::assess(info.ios_version.as_deref(), &info.probe, &identity, need).map_err(
                |readiness| Error::NotReady {
                    target: target.to_owned(),
                    readiness,
                },
            )?;
            Ok(info)
        })
    }

    /// Gets the device ready for its developer services: fails on what
    /// stands in the way, mounts the developer disk image when it is
    /// missing, then opens the tunnel and DVT's process control unless they
    /// are open.
    pub(crate) fn connect(&self, target: &str) -> Result<Connected> {
        self.runtime
            .block_on(self.connect_target(target))
            .map(|(_, connected)| connected)
    }

    async fn connect_target(
        &self,
        target: &str,
    ) -> Result<(Arc<Slot<H::Opener, H::Link>>, Connected)> {
        let entry = self.entry(target).await?;
        let identity = self.host.identity();
        let not_ready = |readiness| Error::NotReady {
            target: target.to_owned(),
            readiness,
        };
        let (values, probe) = self.host.probe(&entry).await;
        let mut device = discovery::device_info(target, &values, probe, &identity);
        let assess = |device: &DeviceInfo| {
            readiness::assess(
                device.ios_version.as_deref(),
                &device.probe,
                &identity,
                Need::DiskImage,
            )
        };
        let disk_image = match assess(&device) {
            Ok(()) => MountReport {
                image: device
                    .probe
                    .disk_image
                    .clone()
                    .unwrap_or(DiskImage::NotMounted),
                attempts: Vec::new(),
            },
            Err(Readiness::DiskImageUnmounted) => {
                let mounter = self.host.mounter(&entry, &device);
                let report = image::ensure_mounted(&mounter, self.mount_limits, target)
                    .await
                    .map_err(|failure| Error::DiskImage {
                        target: target.to_owned(),
                        attempts: failure.attempts,
                        offline: failure.offline,
                    })?;
                let (values, probe) = self.host.probe(&entry).await;
                device = discovery::device_info(target, &values, probe, &identity);
                assess(&device).map_err(not_ready)?;
                report
            }
            Err(readiness) => return Err(not_ready(readiness)),
        };
        let slot = self.slot(&entry);
        let mut state = slot.state.lock().await;
        state.disk_image = Some(disk_image.clone());
        let opened = match state.tunnel.open().await {
            Ok(()) => state
                .channel(dvt::PROCESS_CONTROL, self.limits)
                .await
                .map(|_| ()),
            Err(error) => Err(error),
        };
        *lock(&slot.snapshot) = state.snapshot();
        opened?;
        let tunnel = state.tunnel.status().expect("an open tunnel");
        drop(state);
        Ok((
            slot,
            Connected {
                device,
                tunnel,
                disk_image,
            },
        ))
    }

    /// The state of `target`, connecting it first when the driver holds
    /// none for it.
    async fn connected(&self, target: &str) -> Result<Arc<Slot<H::Opener, H::Link>>> {
        let held = lock(&self.targets).get(target).cloned();
        match held {
            Some(slot) => Ok(slot),
            None => Ok(self.connect_target(target).await?.0),
        }
    }

    pub(crate) fn install(&self, target: &str, app: &Path) -> Result<Installed> {
        let bundle = bundle::inspect(app).map_err(|reason| Error::InvalidBundle {
            path: app.display().to_string(),
            reason,
        })?;
        self.runtime.block_on(async {
            let slot = self.connected(target).await?;
            let state = slot.state.lock().await;
            let started = Instant::now();
            state
                .link
                .install(app)
                .await
                .map_err(|reason| Error::InstallFailed {
                    bundle_id: bundle.bundle_id.clone(),
                    reason,
                })?;
            Ok(Installed {
                bundle_id: bundle.bundle_id,
                elapsed: started.elapsed(),
            })
        })
    }

    pub(crate) fn launch(
        &self,
        target: &str,
        bundle_id: &str,
        options: &LaunchOptions,
    ) -> Result<Launched> {
        self.runtime.block_on(async {
            let started = Instant::now();
            let slot = self.connected(target).await?;
            let mut state = slot.state.lock().await;
            // The lock first (design D19): on a locked phone DVT refuses with
            // an error it does not explain, and CoreDevice's app service
            // refused at some moments and launched behind the lock screen at
            // others (S9), so a launch is not tried.
            match state.link.password_protected().await {
                Ok(false) => {}
                Ok(true) => {
                    return Err(Error::NotReady {
                        target: target.to_owned(),
                        readiness: Readiness::Locked,
                    });
                }
                Err(cause) => {
                    return Err(Error::Disconnected {
                        target: target.to_owned(),
                        cause: format!("reading lockdown's PasswordProtected: {cause}"),
                    });
                }
            }
            let reply = state
                .call(
                    dvt::PROCESS_CONTROL,
                    dvt::launch(bundle_id, options),
                    false,
                    self.limits,
                )
                .await;
            *lock(&slot.snapshot) = state.snapshot();
            match dvt::launched_pid(&reply?) {
                Ok(pid) => Ok(Launched {
                    pid,
                    elapsed: started.elapsed(),
                }),
                // DVT's refusal does not say whether the app is installed;
                // the installation proxy does.
                Err(reason) => match state.link.installed(bundle_id).await {
                    Ok(false) => Err(Error::AppNotInstalled {
                        target: target.to_owned(),
                        bundle_id: bundle_id.to_owned(),
                    }),
                    _ => Err(Error::LaunchFailed {
                        bundle_id: bundle_id.to_owned(),
                        reason,
                    }),
                },
            }
        })
    }

    pub(crate) fn terminate(&self, target: &str, bundle_id: &str) -> Result<Terminated> {
        self.runtime.block_on(async {
            let started = Instant::now();
            let slot = self.connected(target).await?;
            let mut state = slot.state.lock().await;
            let pids = state.terminate(bundle_id, self.limits).await;
            *lock(&slot.snapshot) = state.snapshot();
            Ok(Terminated {
                pids: pids?,
                elapsed: started.elapsed(),
            })
        })
    }

    pub(crate) fn running(&self, target: &str, bundle_id: &str) -> Result<Option<u32>> {
        self.runtime.block_on(async {
            let slot = self.connected(target).await?;
            let mut state = slot.state.lock().await;
            let pids = state.pids(bundle_id, self.limits).await;
            *lock(&slot.snapshot) = state.snapshot();
            Ok(pids?.first().copied())
        })
    }

    /// What the driver holds for `target`; nothing for a device it has not
    /// connected to. It reads a snapshot, so it answers while a command on
    /// the target runs.
    pub(crate) fn status(&self, target: &str) -> Result<TargetStatus> {
        let slot = lock(&self.targets).get(target).cloned();
        Ok(slot.map_or_else(TargetStatus::default, |slot| lock(&slot.snapshot).clone()))
    }

    /// Closes what the driver opened for `target`.
    pub(crate) fn reset_target(&self, target: &str) -> Result<()> {
        let Some(slot) = lock(&self.targets).remove(target) else {
            return Ok(());
        };
        self.runtime.block_on(async {
            let mut state = slot.state.lock().await;
            state.channels.clear();
            state.tunnel.close().await;
        });
        Ok(())
    }

    /// The state the driver keeps for the device of `entry`, made on first
    /// use.
    fn slot(&self, entry: &MuxEntry) -> Arc<Slot<H::Opener, H::Link>> {
        let mut targets = lock(&self.targets);
        let slot = targets.entry(entry.udid.clone()).or_insert_with(|| {
            Arc::new(Slot {
                state: tokio::sync::Mutex::new(Target {
                    target: entry.udid.clone(),
                    tunnel: Tunnel::new(&entry.udid, self.host.opener(entry), self.limits),
                    link: self.host.link(entry),
                    disk_image: None,
                    channels: BTreeMap::new(),
                }),
                snapshot: Mutex::new(TargetStatus::default()),
            })
        });
        Arc::clone(slot)
    }

    /// The USB entry of the device whose identifier is `target`.
    async fn entry(&self, target: &str) -> Result<MuxEntry> {
        usb_entries(self.host.list().await?)
            .into_iter()
            .find(|entry| entry.udid == target)
            .ok_or_else(|| Error::NotFound {
                target: target.to_owned(),
            })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use plist::{Dictionary, Value};

    use super::*;
    use crate::discovery::tests::{identity, recorded_images, recorded_values};
    use crate::discovery::{DEVELOPER_MODE_KEY, LOCK_KEY, probe as probe_from};
    use crate::identity::IdentityCheck;
    use crate::macos::dvt::{LAUNCH, PROCESS_CONTROL, SEND_SIGNAL};
    use crate::macos::fake::{Arg, FakeDevice, FakeLink, FakeMounter, FakeOpener};
    use crate::model::{Connection, DiskImage, MountPath, Probe, Trust};
    use crate::readiness::Readiness;
    use crate::services;

    pub(crate) const PHONE: &str = "placeholder-udid-phone";

    /// What the fake host's image mounter lists.
    #[derive(Clone)]
    pub(crate) enum Images {
        /// What the probe's image mounter answered, or `None` when it was
        /// not asked.
        Fixed(Option<std::result::Result<Vec<Value>, String>>),
        /// What the fake mounter has mounted so far.
        Mounter,
    }

    /// A host that answers with recorded values instead of usbmuxd and
    /// lockdown, and whose phone is a fake device behind a fake tunnel.
    pub(crate) struct FakeHost {
        pub(crate) listing: std::result::Result<Vec<MuxEntry>, String>,
        pub(crate) values: Dictionary,
        pub(crate) trust: Trust,
        pub(crate) images: Images,
        pub(crate) identity: IdentityCheck,
        pub(crate) device: Arc<FakeDevice>,
        pub(crate) mounter: FakeMounter,
        pub(crate) probed: Mutex<Vec<MuxEntry>>,
    }

    impl FakeHost {
        /// The recorded phone, ready, listed on the network and on USB as
        /// Wi-Fi sync lists it.
        pub(crate) fn ready() -> FakeHost {
            FakeHost {
                listing: Ok(vec![
                    MuxEntry {
                        udid: PHONE.into(),
                        device_id: 2,
                        connection: Connection::Network,
                    },
                    MuxEntry {
                        udid: PHONE.into(),
                        device_id: 1,
                        connection: Connection::Usb,
                    },
                ]),
                values: recorded_values(),
                trust: Trust::Trusted,
                images: Images::Fixed(Some(Ok(recorded_images()))),
                identity: identity(),
                device: FakeDevice::recorded(),
                mounter: FakeMounter::new(true, Err("not called".into()), Err("not called".into())),
                probed: Mutex::new(Vec::new()),
            }
        }

        /// The recorded phone with `values` changed, `trust` and `images`.
        pub(crate) fn with(
            changes: &[(&str, Value)],
            trust: Trust,
            images: Option<std::result::Result<Vec<Value>, String>>,
        ) -> FakeHost {
            let mut values = recorded_values();
            for (key, value) in changes {
                values.insert((*key).to_owned(), value.clone());
            }
            FakeHost {
                values,
                trust,
                images: Images::Fixed(images),
                ..FakeHost::ready()
            }
        }

        /// The recorded phone without the disk image, which `mounter`
        /// mounts as it says.
        pub(crate) fn unmounted(mounter: FakeMounter) -> FakeHost {
            FakeHost {
                images: Images::Mounter,
                mounter,
                ..FakeHost::ready()
            }
        }
    }

    impl Host for FakeHost {
        type Opener = FakeOpener;
        type Mounter = FakeMounter;
        type Link = FakeLink;

        async fn list(&self) -> Result<Vec<MuxEntry>> {
            self.listing
                .clone()
                .map_err(|reason| Error::Usbmuxd { reason })
        }

        async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe) {
            self.probed.lock().expect("probed").push(entry.clone());
            if entry.udid != PHONE {
                return (
                    Dictionary::new(),
                    Probe::untrusted(Trust::Unreachable("unknown".into())),
                );
            }
            let images = match &self.images {
                Images::Fixed(images) => images.clone(),
                Images::Mounter => Some(Ok(self.mounter.listing())),
            };
            let probe = probe_from(&self.values, self.trust.clone(), images);
            (self.values.clone(), probe)
        }

        fn identity(&self) -> IdentityCheck {
            self.identity.clone()
        }

        fn opener(&self, _entry: &MuxEntry) -> FakeOpener {
            FakeOpener::new(&self.device)
        }

        fn mounter(&self, _entry: &MuxEntry, _device: &DeviceInfo) -> FakeMounter {
            self.mounter.clone()
        }

        fn link(&self, _entry: &MuxEntry) -> FakeLink {
            FakeLink::new(&self.device)
        }
    }

    pub(crate) const SHORT: Limits = Limits {
        open: Duration::from_millis(500),
        connect: Duration::from_millis(500),
        reply: Duration::from_millis(500),
    };

    pub(crate) const SHORT_MOUNTS: MountLimits = MountLimits {
        idevice: Duration::from_millis(200),
        devicectl: Duration::from_millis(200),
    };

    pub(crate) fn core(host: FakeHost) -> Core<FakeHost> {
        Core::new(host)
            .expect("the runtime starts")
            .with_limits(SHORT, SHORT_MOUNTS)
    }

    #[test]
    fn a_ready_phone_is_listed_once_with_its_identifier_name_model_and_version() {
        let core = core(FakeHost::ready());
        let devices = core.devices().expect("devices");
        assert_eq!(devices.len(), 1, "{devices:?}");
        let phone = &devices[0];
        assert_eq!(phone.udid, PHONE);
        assert_eq!(phone.name.as_deref(), Some("placeholder-iphone"));
        assert_eq!(phone.model.as_deref(), Some("iPhone14,6"));
        assert_eq!(phone.ios_version.as_deref(), Some("26.6.2"));
        assert_eq!(phone.readiness, Readiness::Ready);
        let probed = core.host.probed.lock().expect("probed").clone();
        assert_eq!(probed.len(), 1, "probed once: {probed:?}");
        assert_eq!(
            probed[0].connection,
            Connection::Usb,
            "through its USB entry"
        );
        assert_eq!(probed[0].device_id, 1);
    }

    /// Each unready phone, with the code a command on it fails with.
    fn unready() -> Vec<(FakeHost, &'static str)> {
        vec![
            (FakeHost::with(&[], Trust::NotPaired, None), "not_trusted"),
            (FakeHost::with(&[], Trust::Refused, None), "not_trusted"),
            (
                FakeHost::with(
                    &[(DEVELOPER_MODE_KEY, Value::Boolean(false))],
                    Trust::Trusted,
                    None,
                ),
                "developer_mode_off",
            ),
            (
                FakeHost::with(&[], Trust::Trusted, Some(Ok(Vec::new()))),
                "disk_image_unmounted",
            ),
            (
                FakeHost::with(
                    &[(LOCK_KEY, Value::Boolean(true))],
                    Trust::Trusted,
                    Some(Ok(recorded_images())),
                ),
                "device_locked",
            ),
            (
                FakeHost {
                    identity: IdentityCheck::NoTeam,
                    ..FakeHost::ready()
                },
                "no_signing_identity",
            ),
            (
                FakeHost::with(
                    &[("ProductVersion", Value::String("16.7.10".into()))],
                    Trust::Trusted,
                    Some(Ok(recorded_images())),
                ),
                "unsupported_ios",
            ),
        ]
    }

    #[test]
    fn an_unready_phone_is_listed_in_its_state_with_the_step_to_take() {
        for (host, code) in unready() {
            let devices = core(host).devices().expect("devices");
            assert_eq!(devices.len(), 1, "{code}");
            let state = &devices[0].readiness;
            assert_eq!(state.code(), code);
            assert!(!state.message().is_empty(), "{code}");
        }
    }

    #[test]
    fn a_command_on_an_unready_phone_fails_with_a_code_naming_the_state() {
        for (host, code) in unready() {
            let error = core(host).require(PHONE, Need::Session).expect_err(code);
            assert_eq!(error.code(), code, "{error}");
            assert!(error.to_string().contains(PHONE), "{error}");
        }
        let ready = core(FakeHost::ready())
            .require(PHONE, Need::Session)
            .expect("a ready phone");
        assert_eq!(ready.readiness, Readiness::Ready);
    }

    #[test]
    fn a_command_needs_only_what_its_operation_needs() {
        let locked = FakeHost::with(
            &[(LOCK_KEY, Value::Boolean(true))],
            Trust::Trusted,
            Some(Ok(recorded_images())),
        );
        let core = core(locked);
        let info = core
            .require(PHONE, Need::DiskImage)
            .expect("the tunnel's services need no unlock");
        assert_eq!(info.readiness, Readiness::Locked, "still listed as locked");
        assert_eq!(
            core.require(PHONE, Need::Unlocked)
                .expect_err("a launch needs it unlocked")
                .code(),
            "device_locked"
        );
    }

    #[test]
    fn an_identifier_no_usb_device_has_is_not_found_and_names_are_not_identifiers() {
        let core = core(FakeHost::ready());
        for target in ["placeholder-udid-other", "placeholder-iphone", ""] {
            let error = core.require(target, Need::Trust).expect_err(target);
            assert_eq!(error.code(), "not_found", "{target}: {error}");
        }
    }

    #[test]
    fn usbmuxd_that_cannot_be_asked_is_reported() {
        let host = FakeHost {
            listing: Err("connecting to /var/run/usbmuxd: no such file".into()),
            ..FakeHost::ready()
        };
        let error = core(host).devices().expect_err("no usbmuxd");
        assert_eq!(error.code(), "usbmuxd_unavailable");
        assert!(error.to_string().contains("no such file"), "{error}");
    }

    #[test]
    fn connecting_a_ready_phone_opens_the_tunnel_and_lists_its_services() {
        let core = core(FakeHost::ready());
        assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        let connected = core.connect(PHONE).expect("connects");
        assert_eq!(connected.device.udid, PHONE);
        assert!(connected.disk_image.attempts.is_empty(), "already mounted");
        assert!(matches!(
            connected.disk_image.image,
            DiskImage::Mounted { .. }
        ));
        assert_eq!(connected.tunnel.opens, 1);
        assert_eq!(connected.tunnel.services.len(), 82);
        assert!(connected.tunnel.missing.is_empty());
        let status = core.status(PHONE).expect("status");
        assert_eq!(status.tunnel.as_ref(), Some(&connected.tunnel));
        assert_eq!(status.disk_image.as_ref(), Some(&connected.disk_image));
        assert_eq!(
            status.services_open,
            vec![PROCESS_CONTROL],
            "DVT's process control is opened and kept"
        );
        let dvt = core.host.device.port(services::DVT).expect("DVT's port");
        assert_eq!(
            core.host.device.connections(),
            vec![core.host.device.rsd_port(), dvt]
        );
        let again = core.connect(PHONE).expect("connects again");
        assert_eq!(again.tunnel.opens, 1, "the tunnel is kept");
        assert_eq!(core.host.device.opens(), 1);
        assert_eq!(
            core.host.device.connections(),
            vec![core.host.device.rsd_port(), dvt],
            "and DVT's connection with it"
        );
    }

    #[test]
    fn connecting_a_locked_phone_opens_the_tunnel_since_its_services_need_no_unlock() {
        let core = core(FakeHost::with(
            &[(LOCK_KEY, Value::Boolean(true))],
            Trust::Trusted,
            Some(Ok(recorded_images())),
        ));
        let connected = core.connect(PHONE).expect("connects");
        assert_eq!(connected.device.readiness, Readiness::Locked);
        assert_eq!(connected.tunnel.opens, 1);
    }

    #[test]
    fn connecting_a_phone_that_is_not_ready_opens_nothing_and_names_its_state() {
        for (host, code) in [
            (FakeHost::with(&[], Trust::NotPaired, None), "not_trusted"),
            (
                FakeHost::with(
                    &[(DEVELOPER_MODE_KEY, Value::Boolean(false))],
                    Trust::Trusted,
                    None,
                ),
                "developer_mode_off",
            ),
        ] {
            let core = core(host);
            let error = core.connect(PHONE).expect_err(code);
            assert_eq!(error.code(), code, "{error}");
            assert_eq!(core.host.device.opens(), 0, "{code}: no tunnel");
            assert_eq!(
                core.host.mounter.calls(),
                Vec::<&str>::new(),
                "{code}: no mount"
            );
            assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        }
    }

    #[test]
    fn a_phone_without_the_disk_image_has_it_mounted_before_the_tunnel_opens() {
        let core = core(FakeHost::unmounted(FakeMounter::new(
            false,
            Ok(()),
            Err("not called".into()),
        )));
        let connected = core.connect(PHONE).expect("mounted and connected");
        assert_eq!(connected.disk_image.attempts.len(), 1);
        assert_eq!(connected.disk_image.attempts[0].path, MountPath::Idevice);
        assert!(matches!(
            connected.disk_image.image,
            DiskImage::Mounted { .. }
        ));
        assert_eq!(connected.device.readiness, Readiness::Ready);
        assert_eq!(connected.tunnel.opens, 1);
        assert_eq!(
            core.status(PHONE).expect("status").disk_image,
            Some(connected.disk_image)
        );
    }

    #[test]
    fn a_mount_that_fails_offline_fails_naming_the_signing_server_and_opens_nothing() {
        let core = core(FakeHost::unmounted(FakeMounter::new(
            false,
            Err("HTTP request failed: dns error".into()),
            Err("devicectl exited with 1: The Internet connection appears to be offline.".into()),
        )));
        let error = core.connect(PHONE).expect_err("not mounted");
        assert_eq!(error.code(), "disk_image_unmounted");
        assert!(error.to_string().contains("signing server"), "{error}");
        assert_eq!(core.host.device.opens(), 0);
    }

    #[test]
    fn missing_services_are_reported_with_the_tunnel() {
        let host = FakeHost {
            device: FakeDevice::without(&[services::HID_INDIGO, services::HID_UNIVERSAL]),
            ..FakeHost::ready()
        };
        let connected = core(host).connect(PHONE).expect("connects");
        assert_eq!(
            connected.tunnel.missing,
            vec![services::HID_INDIGO, services::HID_UNIVERSAL]
        );
    }

    #[test]
    fn reset_closes_the_tunnel_and_the_next_connect_opens_a_new_one() {
        let core = core(FakeHost::ready());
        core.connect(PHONE).expect("connects");
        core.reset_target(PHONE).expect("resets");
        assert_eq!(core.status(PHONE).expect("status"), TargetStatus::default());
        assert_eq!(core.host.device.closed(), 1);
        assert_eq!(core.host.device.connections().len(), 2, "RSD's and DVT's");
        let connected = core.connect(PHONE).expect("connects again");
        assert_eq!(connected.tunnel.opens, 1, "a new target's first tunnel");
        assert_eq!(core.host.device.opens(), 2);
        core.reset_target("placeholder-udid-other")
            .expect("resetting what was never opened does nothing");
    }

    #[test]
    fn connecting_an_unknown_identifier_is_not_found() {
        let error = core(FakeHost::ready())
            .connect("placeholder-udid-other")
            .expect_err("no such device");
        assert_eq!(error.code(), "not_found");
    }

    const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";

    /// A bundle laid out as a signed device build of the SwiftUI fixture,
    /// removed when dropped.
    struct App {
        root: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    impl App {
        fn new(test: &str, platform: &str) -> App {
            let root =
                std::env::temp_dir().join(format!("ios-driver-{test}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&root);
            let path = root.join("FixtureSwiftUI.app");
            std::fs::create_dir_all(path.join("_CodeSignature")).expect("the bundle");
            let mut info = Dictionary::new();
            info.insert("CFBundleIdentifier".into(), Value::String(SWIFTUI.into()));
            info.insert(
                "CFBundleExecutable".into(),
                Value::String("FixtureSwiftUI".into()),
            );
            info.insert(
                "CFBundleSupportedPlatforms".into(),
                Value::Array(vec![Value::String(platform.into())]),
            );
            plist::to_file_binary(path.join("Info.plist"), &info).expect("Info.plist");
            std::fs::write(path.join("FixtureSwiftUI"), b"\xca\xfe").expect("the executable");
            std::fs::write(path.join("_CodeSignature/CodeResources"), b"<plist/>")
                .expect("the signature");
            std::fs::write(path.join("embedded.mobileprovision"), b"profile").expect("the profile");
            App { root, path }
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    /// The selectors DVT's process control took, in order.
    fn control_calls(device: &FakeDevice) -> Vec<String> {
        device
            .dvt_calls()
            .into_iter()
            .filter(|call| call.service == PROCESS_CONTROL)
            .map(|call| call.selector)
            .collect()
    }

    #[test]
    fn install_checks_the_bundle_uploads_it_and_reports_its_identifier_and_time() {
        let core = core(FakeHost::ready());
        let app = App::new("install", "iPhoneOS");
        let installed = core.install(PHONE, &app.path).expect("installs");
        assert_eq!(installed.bundle_id, SWIFTUI);
        assert_eq!(core.host.device.installs(), vec![app.path.clone()]);
        assert_eq!(core.running(PHONE, SWIFTUI).expect("running"), None);
    }

    #[test]
    fn a_bundle_a_device_does_not_take_is_refused_before_anything_is_asked_of_it() {
        let core = core(FakeHost::ready());
        let app = App::new("simulator-build", "iPhoneSimulator");
        let error = core
            .install(PHONE, &app.path)
            .expect_err("a Simulator build");
        assert_eq!(error.code(), "invalid_bundle", "{error}");
        assert!(error.to_string().contains("Simulator"), "{error}");
        assert!(core.host.device.installs().is_empty());
        assert_eq!(
            core.host.device.opens(),
            0,
            "nothing was asked of the device"
        );
    }

    #[test]
    fn an_install_the_installation_proxy_refuses_says_why() {
        let core = core(FakeHost::ready());
        core.host
            .device
            .fail_installs("ApplicationVerificationFailed: the profile does not list this device");
        let app = App::new("refused", "iPhoneOS");
        let error = core.install(PHONE, &app.path).expect_err("refused");
        assert_eq!(error.code(), "install_failed");
        assert!(
            error.to_string().contains("does not list this device"),
            "{error}"
        );
    }

    #[test]
    fn a_launch_reads_the_lock_then_launches_through_process_control_and_returns_the_pid() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        let options = LaunchOptions::new()
            .env("FIXTURE_INITIAL_TAB", "Big")
            .terminating_running();
        let launched = core.launch(PHONE, SWIFTUI, &options).expect("launches");
        assert_eq!(core.host.device.lock_reads(), 1, "the lock is read first");
        assert_eq!(core.host.device.running(SWIFTUI), vec![launched.pid]);
        assert_eq!(control_calls(&core.host.device), vec![LAUNCH]);
        assert_eq!(
            core.running(PHONE, SWIFTUI).expect("running"),
            Some(launched.pid)
        );
        let again = core
            .launch(PHONE, SWIFTUI, &options)
            .expect("launches again");
        assert_eq!(core.host.device.lock_reads(), 2, "before every launch");
        assert_ne!(again.pid, launched.pid, "KillExisting starts a new process");
        assert_eq!(core.host.device.running(SWIFTUI), vec![again.pid]);
    }

    #[test]
    fn a_launch_on_a_locked_phone_is_refused_without_being_tried() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        core.host.device.set_locked(true);
        let error = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect_err("locked");
        assert_eq!(error.code(), "device_locked", "{error}");
        assert!(
            control_calls(&core.host.device).is_empty(),
            "DVT was not asked"
        );
        core.host.device.set_locked(false);
        core.launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect("unlocked, it launches");
    }

    #[test]
    fn launching_an_app_that_is_not_installed_fails_as_not_installed() {
        let core = core(FakeHost::ready());
        let error = core
            .launch(
                PHONE,
                "dev.mobileagent.placeholder.missing",
                &LaunchOptions::new(),
            )
            .expect_err("not installed");
        assert_eq!(error.code(), "app_not_installed", "{error}");
        assert!(
            error
                .to_string()
                .contains("dev.mobileagent.placeholder.missing"),
            "{error}"
        );
    }

    #[test]
    fn a_refused_launch_of_an_installed_app_says_why() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        core.host.device.refuse_launches();
        let error = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect_err("refused");
        assert_eq!(error.code(), "launch_failed", "{error}");
        assert!(
            error
                .to_string()
                .contains("FBSOpenApplicationServiceErrorDomain"),
            "{error}"
        );
    }

    #[test]
    fn terminate_signals_the_apps_process_and_returns_once_it_is_gone() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        let launched = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect("launches");
        let terminated = core.terminate(PHONE, SWIFTUI).expect("terminates");
        assert_eq!(terminated.pids, vec![launched.pid]);
        assert!(core.host.device.running(SWIFTUI).is_empty());
        assert_eq!(core.running(PHONE, SWIFTUI).expect("running"), None);
        let signal = core
            .host
            .device
            .dvt_calls()
            .into_iter()
            .find(|call| call.selector == SEND_SIGNAL)
            .expect("a signal");
        assert_eq!(
            signal.args,
            vec![
                Arg::Archived(Value::Integer(9.into())),
                Arg::Archived(Value::Integer(i64::from(launched.pid).into())),
            ]
        );
    }

    #[test]
    fn terminating_an_app_that_is_not_running_signals_nothing() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        let terminated = core.terminate(PHONE, SWIFTUI).expect("nothing to do");
        assert!(terminated.pids.is_empty());
        assert!(!control_calls(&core.host.device).contains(&SEND_SIGNAL.to_owned()));
    }

    #[test]
    fn a_call_after_the_tunnel_dropped_goes_through_a_new_tunnel() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        let launched = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect("launches");
        core.host.device.drop_tunnel();
        assert_eq!(
            core.running(PHONE, SWIFTUI).expect("served again"),
            Some(launched.pid)
        );
        core.terminate(PHONE, SWIFTUI)
            .expect("process control is served again too");
        assert_eq!(core.host.device.opens(), 2);
        let status = core.status(PHONE).expect("status");
        assert_eq!(status.tunnel.map(|tunnel| tunnel.opens), Some(2));
    }

    #[test]
    fn a_launch_whose_dvt_never_answers_times_out() {
        let core = core(FakeHost::ready());
        core.host.device.install_app(SWIFTUI, "FixtureSwiftUI");
        core.connect(PHONE).expect("connects");
        core.host.device.silence_dvt();
        let error = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect_err("no reply");
        assert_eq!(error.code(), "timeout", "{error}");
    }

    #[test]
    fn a_command_on_a_phone_that_is_not_ready_fails_with_its_state() {
        let host = FakeHost::with(
            &[(DEVELOPER_MODE_KEY, Value::Boolean(false))],
            Trust::Trusted,
            None,
        );
        let core = core(host);
        let error = core
            .launch(PHONE, SWIFTUI, &LaunchOptions::new())
            .expect_err("developer mode is off");
        assert_eq!(error.code(), "developer_mode_off");
        let error = core.running(PHONE, SWIFTUI).expect_err("still off");
        assert_eq!(error.code(), "developer_mode_off");
    }
}
