//! [`SimulatorDriver`]: the driver traits over CoreSimulator.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use protocol::message::SnapshotParams;
use protocol::snapshot::Snapshot;
use simulator_reader::codes;

use crate::cadence::Cadence;
use crate::compat::{self, Compatibility, Row, Status};
use crate::driver::{Frames, Input, Lifecycle, Tree};
use crate::error::{Error, Failure, Result, TreeFailure};
use crate::frames::FrameStream;
use crate::gesture;
use crate::lookup;
use crate::macos::coresim::{Device, DeviceSet, ServiceContext};
use crate::macos::dtuhid::Dtuhid;
use crate::macos::encode::Encoder;
use crate::macos::indigo::Indigo;
use crate::macos::input::{Target, Transport};
use crate::macos::reader::{self, Reader, Spawned};
use crate::macos::{frames, host, pasteboard, prefs, translator};
use crate::macos::{objc, xpc};
use crate::model::{
    BootReport, Button, DeviceInfo, DeviceState, DeviceTypeInfo, Diagnostics, Dispatch,
    DriverOptions, FrameOptions, Gesture, InputStatus, Key, LaunchOptions, Launched, Modifiers,
    Point, PreferencesOutcome, RuntimeInfo, Screenshot, ScreenshotOptions, Swipe, TargetStatus,
    TextEntry, TextMethod, TransportKind,
};
use crate::select::{self, HostVersions};
use crate::tree::{ForegroundTree, TreeBackend, TreeChoice, TreeOptions};

/// The first boot of a new device took 43 to 146 s to a terminal boot status
/// in spike S2, under load; later boots about 11 s.
const BOOT_TIMEOUT: Duration = Duration::from_secs(300);
/// A shutdown took about 3.4 s in S2.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);
/// The TCC service behind "Paste from Other Apps". Without it the first
/// paste of text that `devicectl` put on the pasteboard asks the user to
/// allow pasting from dtpasteboardd, and the text never arrives.
const PASTEBOARD_SERVICE: &str = "kTCCServicePasteboard";
/// The `v` usage, pasted with Command held.
const USAGE_V: u32 = 0x19;

/// installd answers IXErrorDomain 2 ("Failed to locate promise") right after
/// a boot; S2 saw a retry 3 s later succeed.
const INSTALL_ATTEMPTS: u32 = 5;
const INSTALL_RETRY: Duration = Duration::from_secs(2);

/// Drives the iOS Simulators of the default device set.
pub struct SimulatorDriver {
    context: ServiceContext,
    set: DeviceSet,
    options: DriverOptions,
    /// The host's CoreSimulator and Xcode, which choose the transport.
    host: HostVersions,
    /// Whether the selected Xcode is in the supported range; every command
    /// but `compatibility` refuses when it is not.
    gate: Result<()>,
    /// The devices input went to, by UDID, each with its transport.
    targets: Mutex<HashMap<String, Arc<Target>>>,
    /// Devices a failure in the private framework layer took out of
    /// service, by UDID, with why.
    unavailable: Mutex<HashMap<String, Diagnostics>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl SimulatorDriver {
    /// Loads CoreSimulator and connects to its service for the selected Xcode
    /// (`DEVELOPER_DIR`, else `xcode-select`).
    pub fn open() -> Result<SimulatorDriver> {
        SimulatorDriver::open_with(DriverOptions::default())
    }

    /// `open` with the input transport chosen by `options`.
    pub fn open_with(options: DriverOptions) -> Result<SimulatorDriver> {
        SimulatorDriver::with_host(options, host::detect())
    }

    /// `open_with` on a host whose versions are `host`.
    fn with_host(options: DriverOptions, host: HostVersions) -> Result<SimulatorDriver> {
        let context = ServiceContext::connect()?;
        let set = context.default_set()?;
        Ok(SimulatorDriver {
            context,
            set,
            options,
            gate: compat::check_xcode(host.xcode.as_deref()),
            host,
            targets: Mutex::new(HashMap::new()),
            unavailable: Mutex::new(HashMap::new()),
        })
    }

    /// The installed runtimes.
    pub fn runtimes(&self) -> Result<Vec<RuntimeInfo>> {
        self.gate.clone()?;
        self.context.runtimes()?.iter().map(|r| r.info()).collect()
    }

    /// The device types this Xcode knows.
    pub fn device_types(&self) -> Result<Vec<DeviceTypeInfo>> {
        self.gate.clone()?;
        self.context
            .device_types()?
            .iter()
            .map(|t| t.info())
            .collect()
    }

    /// Creates a device of `device_type` (a name or identifier) on `runtime`
    /// (a version, name or identifier; the newest iOS runtime when `None`),
    /// with its accessibility preferences written before its first boot.
    pub fn create_device(
        &self,
        name: &str,
        device_type: &str,
        runtime: Option<&str>,
    ) -> Result<DeviceInfo> {
        self.create_on(name, device_type, |runtimes| {
            lookup::runtime(runtime, runtimes)
        })
    }

    /// Creates a device of `device_type` on the runtime the device `like` (a
    /// UDID or name) runs, so that it runs the same iOS, with its
    /// accessibility preferences written before its first boot. The newest
    /// runtime installed may not take the device type: CoreSimulator refuses
    /// it with SimError 403, "Incompatible device".
    pub fn create_device_like(
        &self,
        name: &str,
        device_type: &str,
        like: &str,
    ) -> Result<DeviceInfo> {
        self.gate.clone()?;
        let devices = self.devices()?;
        self.create_on(name, device_type, |runtimes| {
            lookup::runtime_of(like, &devices, runtimes)
        })
    }

    /// Creates a device of `device_type` on the runtime `choose` picks from
    /// the installed ones.
    fn create_on(
        &self,
        name: &str,
        device_type: &str,
        choose: impl FnOnce(&[RuntimeInfo]) -> Result<usize>,
    ) -> Result<DeviceInfo> {
        self.gate.clone()?;
        let types = self.context.device_types()?;
        let type_infos: Vec<DeviceTypeInfo> =
            types.iter().map(|t| t.info()).collect::<Result<_>>()?;
        let device_type = &types[lookup::device_type(device_type, &type_infos)?];
        let runtimes = self.context.runtimes()?;
        let runtime_infos: Vec<RuntimeInfo> =
            runtimes.iter().map(|r| r.info()).collect::<Result<_>>()?;
        let runtime = &runtimes[choose(&runtime_infos)?];

        let device = self.set.create(device_type, runtime, name)?;
        prefs::write(&device.data_path()?)?;
        device.info()
    }

    /// Deletes a device, which must be shut down.
    pub fn delete_device(&self, target: &str) -> Result<()> {
        self.contain(target, || {
            let device = self.device(target)?;
            needs(target, &device, DeviceState::Shutdown, "shut down")?;
            self.set.delete(&device)
        })
    }

    /// Returns a device that a failure took out of service to use, dropping
    /// its sessions so the next request opens them anew.
    pub fn reset_target(&self, target: &str) -> Result<()> {
        let udid = self.udid_of(target)?;
        lock(&self.unavailable).remove(&udid);
        let dropped = lock(&self.targets).remove(&udid);
        drop(dropped);
        Ok(())
    }

    /// What the driver supports on this host, for `doctor`: the host's
    /// components and every private call with where it was validated. It
    /// answers outside the supported Xcode range too.
    pub fn compatibility(&self) -> Result<Compatibility> {
        let unknown = || "not found".to_string();
        let mut rows = Vec::new();

        let xcode = self.host.xcode.clone().unwrap_or_else(unknown);
        rows.push(Row {
            component: "Xcode".into(),
            detected: xcode.clone(),
            expected: compat::supported(),
            status: match (&self.gate, compat::validated_xcode(&xcode)) {
                (Err(_), _) => Status::Unsupported,
                (Ok(()), true) => Status::Validated,
                (Ok(()), false) => Status::Supported,
            },
            note: format!(
                "validated on Xcode {}; the CI matrix also builds on Xcode 26",
                compat::VALIDATED_XCODE
            ),
        });

        let plan = select::plan(&self.host, self.options.transport);
        let core_simulator = self.host.core_simulator.clone().unwrap_or_else(unknown);
        let dtuhidd = plan.transports.first() == Some(&TransportKind::Dtuhid);
        rows.push(Row {
            component: "CoreSimulator".into(),
            detected: core_simulator.clone(),
            expected: format!("{} or later for dtuhidd", select::DTUHIDD_SINCE),
            status: if core_simulator == compat::VALIDATED_CORE_SIMULATOR {
                Status::Validated
            } else if dtuhidd {
                Status::Supported
            } else {
                Status::Limited
            },
            note: if dtuhidd {
                "input through dtuhidd".into()
            } else {
                "input through Indigo: touch only, no keys, buttons or text".into()
            },
        });

        let kit = host::simulator_kit_version();
        rows.push(Row {
            component: "SimulatorKit".into(),
            detected: kit.clone().unwrap_or_else(unknown),
            expected: "present, for the Indigo fallback".into(),
            status: match kit.as_deref() {
                Some(compat::VALIDATED_SIMULATOR_KIT) => Status::Validated,
                Some(_) => Status::Supported,
                None if dtuhidd => Status::Limited,
                None => Status::Unsupported,
            },
            note: host::simulator_kit().display().to_string(),
        });

        let runtimes = self.context.runtimes()?;
        let names: Vec<String> = runtimes
            .iter()
            .map(|runtime| {
                runtime
                    .info()
                    .map(|info| format!("{} ({})", info.name, info.build))
            })
            .collect::<Result<_>>()?;
        rows.push(Row {
            component: "runtimes".into(),
            detected: if names.is_empty() {
                "none installed".into()
            } else {
                names.join(", ")
            },
            expected: format!("{} validated", compat::VALIDATED_RUNTIME),
            status: if names.is_empty() {
                Status::Unsupported
            } else if names
                .iter()
                .any(|name| name.starts_with(&format!("{} ", compat::VALIDATED_RUNTIME)))
            {
                Status::Validated
            } else {
                Status::Supported
            },
            note: "input and frames depend on the host, not the runtime".into(),
        });

        let transports: Vec<String> = plan.transports.iter().map(|t| t.to_string()).collect();
        rows.push(Row {
            component: "input transport".into(),
            detected: transports.join(", then "),
            expected: "dtuhidd from CoreSimulator 1155.4, Indigo before".into(),
            status: if self.gate.is_err() {
                Status::Unsupported
            } else if core_simulator == compat::VALIDATED_CORE_SIMULATOR {
                Status::Validated
            } else {
                Status::Supported
            },
            note: plan.reason,
        });

        let symbols = xpc::Sim4Symbols::resolve();
        rows.push(Row {
            component: "XPC symbols".into(),
            detected: match &symbols {
                Ok(_) => "the three _4sim functions resolve".into(),
                Err(reason) => reason.clone(),
            },
            expected: "resolvable with dlsym".into(),
            status: match (&symbols, dtuhidd) {
                (Ok(_), _) => Status::Validated,
                (Err(_), true) => Status::Unsupported,
                (Err(_), false) => Status::Limited,
            },
            note: "dtuhidd needs them".into(),
        });

        let reader = match &self.options.reader {
            Some(path) => Ok(path.clone()),
            None => reader::embedded(),
        };
        rows.push(Row {
            component: "tree reader".into(),
            detected: match &reader {
                Ok(path) => format!("{} at {}", simulator_reader::AGENT, path.display()),
                Err(why) => why.clone(),
            },
            expected: "built for aarch64-apple-ios-sim into this binary".into(),
            status: if reader.is_ok() {
                Status::Validated
            } else {
                Status::Limited
            },
            note: if reader.is_ok() {
                "spawned into each booted Simulator, nothing installed".into()
            } else {
                "tree reads use the translator; see Building in the README".into()
            },
        });

        let translator = translator::available();
        rows.push(Row {
            component: "tree translator".into(),
            detected: match &translator {
                Ok(()) => "AccessibilityPlatformTranslation loads".into(),
                Err(error) => error.to_string(),
            },
            expected: "present, for the fallback".into(),
            status: match (&translator, &reader) {
                (Ok(()), _) => Status::Validated,
                (Err(_), Ok(_)) => Status::Limited,
                (Err(_), Err(_)) => Status::Unsupported,
            },
            note: "reads when the reader cannot run; its snapshots are degraded".into(),
        });

        let encoder = Encoder::new(80).map(|encoder| encoder.name());
        rows.push(Row {
            component: "frame encoder".into(),
            detected: encoder.clone().unwrap_or("none").to_string(),
            expected: "turbojpeg, or jpeg-encoder without libjpeg-turbo".into(),
            status: if encoder.is_ok() {
                Status::Validated
            } else {
                Status::Unsupported
            },
            note: "chosen at build time; see Building in the README".into(),
        });

        Ok(Compatibility {
            rows,
            call_sites: compat::call_sites(),
        })
    }

    /// Makes the next private framework call made for `target` raise an
    /// Objective-C exception inside the driver's guard.
    #[cfg(feature = "fault-injection")]
    pub fn inject_fault(&self, target: &str) -> Result<()> {
        objc::fault::arm(&self.udid_of(target)?);
        Ok(())
    }

    /// The UDID `target` names, from the targets already seen when it is one
    /// of their UDIDs.
    fn udid_of(&self, target: &str) -> Result<String> {
        let key = target.to_ascii_uppercase();
        if lock(&self.targets).contains_key(&key) {
            return Ok(key);
        }
        self.device(target)?.udid()
    }

    /// Runs a request for one device. A device out of service fails it at
    /// once. An Objective-C exception or a panic inside the request, caught
    /// by the guard around each private call or here, takes the device out
    /// of service with diagnostics and fails the request with them; other
    /// devices are untouched.
    fn contain<R>(&self, target: &str, request: impl FnOnce() -> Result<R>) -> Result<R> {
        self.gate.clone()?;
        let udid = self.udid_of(target)?;
        if let Some(diagnostics) = lock(&self.unavailable).get(&udid).cloned() {
            return Err(Error::Unavailable {
                target: udid,
                diagnostics,
            });
        }
        let result = {
            #[cfg(feature = "fault-injection")]
            let _serving = objc::fault::serve(&udid);
            panic::catch_unwind(AssertUnwindSafe(request)).unwrap_or_else(|panic| {
                Err(Error::Framework {
                    call: "a request".into(),
                    failure: Failure::Panic(objc::panic_message(panic.as_ref())),
                })
            })
        };
        match result {
            Err(Error::Framework { call, failure })
                if matches!(failure, Failure::Exception { .. } | Failure::Panic(_)) =>
            {
                let diagnostics = Diagnostics {
                    call,
                    failure: failure.to_string(),
                    at: SystemTime::now(),
                    host: self.host.to_string(),
                };
                lock(&self.unavailable).insert(udid.clone(), diagnostics.clone());
                Err(Error::Unavailable {
                    target: udid,
                    diagnostics,
                })
            }
            other => other,
        }
    }

    /// Writes the accessibility preferences into a device that is shut down;
    /// `boot` does this itself.
    pub fn prepare_accessibility(&self, target: &str) -> Result<PreferencesOutcome> {
        self.contain(target, || {
            let device = self.device(target)?;
            needs(target, &device, DeviceState::Shutdown, "shut down")?;
            prefs::write(&device.data_path()?)
        })
    }

    /// The device `target` names: a UDID, or a name exactly one device has.
    fn device(&self, target: &str) -> Result<Device> {
        let mut devices = self.set.devices()?;
        let keys: Vec<(String, String)> = devices
            .iter()
            .map(|device| Ok((device.udid()?, device.name()?)))
            .collect::<Result<_>>()?;
        let index = lookup::device(target, &keys)?;
        Ok(devices.swap_remove(index))
    }

    /// The device `target` names, which must be booted.
    fn booted(&self, target: &str) -> Result<Device> {
        let device = self.device(target)?;
        needs(target, &device, DeviceState::Booted, "booted")?;
        Ok(device)
    }

    /// The booted target `target` names. A UDID seen before skips the device
    /// lookup and costs one state check.
    fn target(&self, target: &str) -> Result<Arc<Target>> {
        let known = lock(&self.targets)
            .get(&target.to_ascii_uppercase())
            .cloned();
        if let Some(known) = known {
            needs(target, &known.device, DeviceState::Booted, "booted")?;
            return Ok(known);
        }
        let device = self.booted(target)?;
        let udid = device.udid()?;
        let screen = device
            .device_type()?
            .ok_or_else(|| Error::Framework {
                call: "-[SimDevice deviceType]".into(),
                failure: Failure::Nil,
            })?
            .main_screen()?;
        let mut targets = lock(&self.targets);
        let entry = targets
            .entry(udid)
            .or_insert_with(|| Arc::new(Target::new(device, screen)));
        Ok(entry.clone())
    }

    /// Runs an input request on the target's transport, opened on first
    /// use. A dtuhidd session that never activated counts as failed on the
    /// target, and the request moves to the next transport of the plan;
    /// when none is left, the error names the versions and what was tried.
    fn with_input<R>(
        &self,
        target: &str,
        request: impl Fn(&Target, &dyn Transport) -> Result<R>,
    ) -> Result<R> {
        let target = self.target(target)?;
        loop {
            let transport = target.transport(|device| self.open_transport(&target, device))?;
            match request(&target, &*transport) {
                Err(Error::InputUnavailable {
                    transport: kind,
                    reason,
                }) if transport.activation().is_none() && kind == TransportKind::Dtuhid => {
                    target.failed(kind, reason);
                }
                result => return result,
            }
        }
    }

    /// Lets every user application on the target paste from other
    /// applications, so a paste raises no permission prompt, listing them
    /// again only once the device's applications changed. Granting leaves
    /// running applications running.
    fn allow_paste(&self, target: &Target) -> Result<()> {
        target.paste_access.allow(
            pasteboard::applications_changed(&target.device.data_path()?),
            || target.device.installed_apps(),
            |bundle_id| {
                target
                    .device
                    .set_privacy(PASTEBOARD_SERVICE, bundle_id, true)
            },
        )
    }

    /// Opens the first transport of the host's plan that works on `target`.
    fn open_transport(&self, target: &Target, device: &Device) -> Result<Arc<dyn Transport>> {
        let plan = select::plan(&self.host, self.options.transport);
        let failures = target.failures();
        let runtime = device
            .runtime()?
            .map(|runtime| runtime.info())
            .transpose()?
            .map_or_else(
                || "unknown".to_string(),
                |r| format!("{} ({})", r.name, r.build),
            );
        let transport = select::open_first(&plan, &failures, &self.host, &runtime, |kind| {
            let transport: Arc<dyn Transport> = match kind {
                TransportKind::Dtuhid => Arc::new(Dtuhid::open(device)?),
                TransportKind::Indigo => Arc::new(Indigo::open(device)?),
            };
            Ok(transport)
        })?;
        let chosen = transport.kind();
        target.set_reason(match plan.transports.first() {
            Some(&first) if first != chosen => format!(
                "{first} could not be used ({}), so {chosen} stands in; {}",
                failures
                    .iter()
                    .find(|(kind, _)| *kind == first)
                    .map_or("it failed to open", |(_, reason)| reason.as_str()),
                plan.reason
            ),
            _ => plan.reason,
        });
        Ok(transport)
    }
}

/// What the reader made of a read.
enum ReaderRead {
    Read(Snapshot),
    /// No reader can serve the device, and why.
    Unusable(String),
}

impl SimulatorDriver {
    /// Reads the tree through the target's reader, spawning it first when
    /// none runs. A reader whose connection ended is replaced once.
    fn read_with_reader(
        &self,
        target: &Target,
        params: &SnapshotParams,
        timeout: Duration,
    ) -> Result<ReaderRead> {
        for _ in 0..2 {
            let reader = match target.tree.reader(|| self.spawn_reader(target))? {
                Ok(reader) => reader,
                Err(why) => return Ok(ReaderRead::Unusable(why)),
            };
            match reader.snapshot(params.clone(), timeout) {
                Ok(snapshot) => return Ok(ReaderRead::Read(snapshot)),
                Err(error) if error.code == protocol::code::DISCONNECTED => {
                    target.tree.lost(&reader);
                }
                Err(error) => return Err(tree_error(error, timeout)),
            }
        }
        Ok(ReaderRead::Unusable(
            "the reader's connection ended during two reads in a row".into(),
        ))
    }

    /// Spawns a reader into the target: the one given in the driver's
    /// options, or the one built into this crate.
    fn spawn_reader(&self, target: &Target) -> Result<Spawned> {
        let program = match &self.options.reader {
            Some(program) => program.clone(),
            None => match reader::embedded() {
                Ok(program) => program,
                Err(why) => {
                    target.tree.give_up(why.clone());
                    return Ok(Spawned::Unusable(why));
                }
            },
        };
        let udid = target.device.udid()?;
        Reader::spawn(
            &target.device,
            &program,
            target.screen.scale,
            &reader::log_path(&udid),
        )
    }
}

/// The driver error for a failure the reader answered a read with.
fn tree_error(error: protocol::Error, timeout: Duration) -> Error {
    let failure = |kind| Error::Tree {
        kind,
        message: error.message.clone(),
    };
    match error.code.as_str() {
        codes::PRIVATE_API_EXCEPTION => {
            let (call, name, reason) = parse_exception(&error.message);
            Error::Framework {
                call: format!("{call}, in {}", reader::NAME),
                failure: Failure::Exception { name, reason },
            }
        }
        codes::NO_FOREGROUND => failure(TreeFailure::NoForeground),
        codes::SNAPSHOT_FAILED => failure(TreeFailure::SnapshotFailed),
        protocol::code::WINDOW_CHANGED => failure(TreeFailure::ForegroundChanged),
        protocol::code::BUSY => failure(TreeFailure::Busy),
        protocol::code::DEADLINE_EXCEEDED => Error::Timeout {
            what: "the tree read".into(),
            after: timeout,
        },
        _ => Error::Tree {
            kind: TreeFailure::Failed,
            message: error.to_string(),
        },
    }
}

/// The call, exception name and reason of the reader's
/// `<call> raised <name>: <reason>`.
fn parse_exception(message: &str) -> (String, String, String) {
    match message.split_once(" raised ") {
        Some((call, rest)) => {
            let (name, reason) = rest.split_once(": ").unwrap_or((rest, ""));
            (call.to_owned(), name.to_owned(), reason.to_owned())
        }
        None => (
            "a private call".to_owned(),
            "NSException".to_owned(),
            message.to_owned(),
        ),
    }
}

/// Fails unless the device is in `wanted`.
fn needs(target: &str, device: &Device, wanted: DeviceState, needed: &'static str) -> Result<()> {
    let state = device.state()?;
    if state == wanted {
        Ok(())
    } else {
        Err(Error::InvalidState {
            target: target.to_string(),
            state,
            needed,
        })
    }
}

/// Whether an error is CoreSimulator reporting that the application is not
/// running: `NSPOSIXErrorDomain` 3, `ESRCH`.
fn not_running(error: &Error) -> bool {
    matches!(
        error,
        Error::Framework {
            failure: Failure::NsError { domain, code: 3, .. },
            ..
        } if domain == "NSPOSIXErrorDomain"
    )
}

/// How long the driver waits before it launches an application again once
/// CoreSimulator refused the launch ([`refused_launch`]): `open --relaunch`
/// right after the app's termination and a press of Home was refused ten
/// times in two runs on an iOS 26.5 Simulator under load (pass-4-fixes,
/// design D2), and a launch 3 s later went through every time; nothing
/// shorter was measured. A freshly created Simulator on CI refused none of
/// ten such launches.
const LAUNCH_RETRY: Duration = Duration::from_secs(3);

/// Whether CoreSimulator refused to launch an application:
/// `NSPOSIXErrorDomain` 3, `ESRCH`, which the launch call answers as
/// "Simulator device failed to launch" with the bundle identifier.
fn refused_launch(error: &Error) -> bool {
    not_running(error)
}

/// Launches an application through `launch`, the device's launch call, and
/// once more after [`LAUNCH_RETRY`], which `wait` waits, when CoreSimulator
/// refused the first launch ([`refused_launch`]); a second refusal fails
/// as it came, and no other failure is launched again. Says how long it
/// waited before the second launch, if it made one.
fn launch_with<T>(
    mut launch: impl FnMut() -> Result<T>,
    wait: impl FnOnce(Duration),
) -> Result<(T, Option<Duration>)> {
    match launch() {
        Err(error) if refused_launch(&error) => {
            wait(LAUNCH_RETRY);
            launch().map(|launched| (launched, Some(LAUNCH_RETRY)))
        }
        result => result.map(|launched| (launched, None)),
    }
}

/// Whether an install failed with installd's transient IXErrorDomain 2.
fn transient_install_failure(error: &Error) -> bool {
    matches!(
        error,
        Error::Framework {
            failure: Failure::NsError { domain, code: 2, .. },
            ..
        } if domain == "IXErrorDomain"
    )
}

impl Lifecycle for SimulatorDriver {
    fn devices(&self) -> Result<Vec<DeviceInfo>> {
        self.gate.clone()?;
        self.set.devices()?.iter().map(Device::info).collect()
    }

    fn boot(&self, target: &str) -> Result<BootReport> {
        self.contain(target, || {
            let device = self.device(target)?;
            let start = Instant::now();
            match device.state()? {
                // Another boot is done or under way: wait for it to be usable.
                DeviceState::Booted | DeviceState::Booting => {
                    device.wait_until_ready(BOOT_TIMEOUT)?;
                    return Ok(BootReport {
                        already_booted: true,
                        elapsed: start.elapsed(),
                        accessibility: PreferencesOutcome::Skipped,
                    });
                }
                DeviceState::ShuttingDown | DeviceState::Creating => {
                    device.wait_for_state(DeviceState::Shutdown, SHUTDOWN_TIMEOUT)?;
                }
                DeviceState::Shutdown => {}
                state @ DeviceState::Other(_) => {
                    return Err(Error::InvalidState {
                        target: target.to_string(),
                        state,
                        needed: "shut down or booted",
                    });
                }
            }
            let accessibility = prefs::write(&device.data_path()?)?;
            device.boot(true)?;
            device.wait_until_ready(BOOT_TIMEOUT)?;
            Ok(BootReport {
                already_booted: false,
                elapsed: start.elapsed(),
                accessibility,
            })
        })
    }

    fn shutdown(&self, target: &str) -> Result<()> {
        self.contain(target, || {
            let device = self.device(target)?;
            match device.state()? {
                DeviceState::Shutdown => Ok(()),
                DeviceState::ShuttingDown => {
                    device.wait_for_state(DeviceState::Shutdown, SHUTDOWN_TIMEOUT)
                }
                _ => {
                    device.shutdown()?;
                    device.wait_for_state(DeviceState::Shutdown, SHUTDOWN_TIMEOUT)
                }
            }
        })
    }

    fn install(&self, target: &str, app: &Path) -> Result<()> {
        self.contain(target, || {
            let device = self.booted(target)?;
            let app = app.canonicalize().map_err(|error| Error::Io {
                what: format!("finding {}", app.display()),
                reason: error.to_string(),
            })?;
            let mut attempt = 1;
            loop {
                match device.install(&app) {
                    Err(error)
                        if transient_install_failure(&error) && attempt < INSTALL_ATTEMPTS =>
                    {
                        attempt += 1;
                        thread::sleep(INSTALL_RETRY);
                    }
                    result => return result,
                }
            }
        })
    }

    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched> {
        self.contain(target, || {
            let device = self.booted(target)?;
            if options.terminate_running {
                match device.terminate(bundle_id) {
                    Err(error) if !not_running(&error) => return Err(error),
                    _ => {}
                }
            }
            let start = Instant::now();
            let (pid, retried_after) = launch_with(
                || device.launch(bundle_id, &options.arguments, &options.environment),
                thread::sleep,
            )?;
            Ok(Launched {
                pid,
                elapsed: start.elapsed(),
                retried_after,
            })
        })
    }

    fn terminate(&self, target: &str, bundle_id: &str) -> Result<()> {
        self.contain(target, || {
            let device = self.booted(target)?;
            match device.terminate(bundle_id) {
                Err(error) if not_running(&error) => Ok(()),
                result => result,
            }
        })
    }

    fn status(&self, target: &str) -> Result<TargetStatus> {
        self.gate.clone()?;
        let device = self.device(target)?;
        let info = device.info()?;
        let known = lock(&self.targets).get(&info.udid).cloned();
        let unavailable = lock(&self.unavailable).get(&info.udid).cloned();
        let transport = known.as_ref().and_then(|target| target.current_transport());
        Ok(TargetStatus {
            device: info,
            input: transport.map(|transport| InputStatus {
                transport: transport.kind(),
                activation: transport.activation(),
                connections: transport.connections(),
                reason: known.as_ref().map(|t| t.reason()).unwrap_or_default(),
            }),
            tree: known.as_ref().and_then(|target| target.tree.status()),
            unavailable,
        })
    }
}

impl Frames for SimulatorDriver {
    fn frames(&self, target: &str, options: &FrameOptions) -> Result<FrameStream> {
        self.contain(target, || {
            let device = self.booted(target)?;
            let subscription = frames::Subscription::start(&device, options)?;
            Ok(FrameStream::new(Box::new(subscription)))
        })
    }

    fn screenshot(&self, target: &str, options: &ScreenshotOptions) -> Result<Screenshot> {
        self.contain(target, || {
            let device = self.booted(target)?;
            frames::screenshot(&device, options)
        })
    }
}

impl Input for SimulatorDriver {
    fn open_input(&self, target: &str, wait: Duration) -> Result<InputStatus> {
        self.contain(target, || {
            let target = self.target(target)?;
            let transport = target.transport(|device| self.open_transport(&target, device))?;
            // dtuhidd answers its activation probe on its own queue; a
            // transport without an activation, Indigo, is ready as it opens.
            if transport.kind() == TransportKind::Dtuhid {
                let deadline = Instant::now() + wait;
                while transport.activation().is_none()
                    && transport.usable()
                    && Instant::now() < deadline
                {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            Ok(InputStatus {
                transport: transport.kind(),
                activation: transport.activation(),
                connections: transport.connections(),
                reason: target.reason(),
            })
        })
    }

    fn tap(&self, target: &str, point: Point) -> Result<Dispatch> {
        self.press(target, point, Duration::ZERO)
    }

    fn press(&self, target: &str, point: Point, hold: Duration) -> Result<Dispatch> {
        self.contain(target, || {
            self.with_input(target, |target, transport| {
                target.touch(transport, &gesture::press(&target.screen, point, hold)?)
            })
        })
    }

    fn swipe(&self, target: &str, swipe: &Swipe) -> Result<Dispatch> {
        self.contain(target, || {
            self.with_input(target, |target, transport| {
                target.touch(transport, &gesture::swipe(&target.screen, swipe)?)
            })
        })
    }

    fn gesture(&self, target: &str, gesture: &Gesture) -> Result<Dispatch> {
        self.contain(target, || {
            self.with_input(target, |target, transport| {
                target.touch(transport, &gesture::gesture(&target.screen, gesture)?)
            })
        })
    }

    fn button(&self, target: &str, button: Button) -> Result<Dispatch> {
        self.contain(target, || {
            self.with_input(target, |target, transport| target.button(transport, button))
        })
    }

    fn key(&self, target: &str, key: Key) -> Result<Dispatch> {
        self.contain(target, || {
            self.with_input(target, |target, transport| target.key(transport, key))
        })
    }

    fn type_text_at(
        &self,
        target: &str,
        text: &str,
        method: TextMethod,
        cadence: Cadence,
    ) -> Result<TextEntry> {
        self.contain(target, || {
            let start = Instant::now();
            let characters = text.chars().count();
            let keys: Option<Vec<Key>> = text.chars().map(Key::character).collect();
            let method = method.resolve(text);
            if method == TextMethod::Keyboard {
                let keys = keys.ok_or_else(|| {
                    let missing = text
                        .chars()
                        .find(|&c| Key::character(c).is_none())
                        .unwrap_or_default();
                    Error::InvalidArgument(format!(
                        "{missing:?} has no key on a US keyboard; paste the text instead"
                    ))
                })?;
                self.with_input(target, |target, transport| {
                    for (index, &key) in keys.iter().enumerate() {
                        if index > 0 {
                            thread::sleep(cadence.gap);
                        }
                        target.key(transport, key)?;
                    }
                    Ok(())
                })?;
            } else {
                let device = self.target(target)?;
                self.allow_paste(&device)?;
                pasteboard::copy(&device.device, text)?;
                let paste = Key {
                    usage: USAGE_V,
                    modifiers: Modifiers {
                        command: true,
                        ..Modifiers::default()
                    },
                };
                self.with_input(target, |target, transport| target.key(transport, paste))?;
            }
            Ok(TextEntry {
                method,
                characters,
                elapsed: start.elapsed(),
            })
        })
    }
}

impl Tree for SimulatorDriver {
    fn snapshot(&self, target: &str, options: &TreeOptions) -> Result<ForegroundTree> {
        self.contain(target, || {
            let target = self.target(target)?;
            let start = Instant::now();
            let params = SnapshotParams {
                max_depth: Some(options.max_depth),
                max_nodes: Some(options.max_nodes),
                ..SnapshotParams::default()
            };
            let translate = || translator::read(&target.device, target.screen.scale, options);
            let (snapshot, backend, fallback) = match self.options.tree {
                TreeChoice::Only(TreeBackend::Translator) => (
                    translate()?,
                    TreeBackend::Translator,
                    Some("the driver was opened to read with the translator only".to_owned()),
                ),
                choice => match self.read_with_reader(&target, &params, options.timeout)? {
                    ReaderRead::Read(snapshot) => (snapshot, TreeBackend::Reader, None),
                    ReaderRead::Unusable(why) if choice == TreeChoice::Auto => (
                        translate()?,
                        TreeBackend::Translator,
                        Some(format!("the reader cannot run: {why}")),
                    ),
                    ReaderRead::Unusable(why) => {
                        return Err(Error::Tree {
                            kind: TreeFailure::Failed,
                            message: format!("the reader cannot run: {why}"),
                        });
                    }
                },
            };
            target.tree.used(
                backend,
                fallback
                    .clone()
                    .unwrap_or_else(|| format!("{} runs in the Simulator", reader::NAME)),
            );
            ForegroundTree::new(
                snapshot,
                target.screen.scale,
                backend,
                fallback,
                start.elapsed(),
            )
            .ok_or_else(|| Error::Tree {
                kind: TreeFailure::NoForeground,
                message: "the read named no process in front".into(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_driver_can_be_shared_between_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<SimulatorDriver>();
    }

    fn ns_error(domain: &str, code: i64) -> Error {
        Error::Framework {
            call: "a call".into(),
            failure: Failure::NsError {
                domain: domain.into(),
                code,
                description: String::new(),
            },
        }
    }

    /// A driver on this host's CoreSimulator that believes the selected
    /// Xcode is `xcode`.
    fn driver_on(xcode: &str) -> SimulatorDriver {
        SimulatorDriver::with_host(
            DriverOptions::default(),
            HostVersions {
                core_simulator: Some("1171.7".into()),
                xcode: Some(xcode.into()),
            },
        )
        .expect("the driver opens")
    }

    #[test]
    fn commands_refuse_an_xcode_outside_the_range() {
        let driver = driver_on("28.0 (28A5)");
        for error in [
            driver.devices().expect_err("listing refuses"),
            driver
                .boot("00000000-0000-0000-0000-000000000000")
                .expect_err("booting refuses"),
        ] {
            assert_eq!(error.code(), "unsupported_xcode", "{error}");
            let message = error.to_string();
            assert!(
                message.contains("28.0 (28A5)") && message.contains("26.0"),
                "{message}"
            );
        }
        let table = driver.compatibility().expect("doctor still reports");
        let xcode = table
            .rows
            .iter()
            .find(|row| row.component == "Xcode")
            .expect("an Xcode row");
        assert_eq!(xcode.status, crate::compat::Status::Unsupported);
        assert!(!table.usable());
    }

    #[test]
    fn this_host_is_within_the_range_and_the_table_says_why() {
        let driver = SimulatorDriver::open().expect("the driver opens");
        driver.devices().expect("commands work within the range");
        let table = driver.compatibility().expect("a table");
        let text = table.render();
        eprintln!("{text}");
        for component in [
            "Xcode",
            "CoreSimulator",
            "SimulatorKit",
            "input transport",
            "XPC symbols",
            "tree reader",
            "tree translator",
            "frame encoder",
        ] {
            assert!(
                table.rows.iter().any(|row| row.component == component),
                "{component} is missing from:\n{text}"
            );
        }
        assert!(table.usable(), "{text}");
        for call in [
            "lookup:error:",
            "spawnWithPath:",
            "userTestingSnapshotForElement:",
            "com.apple.coredevice.feature.pasteboard",
        ] {
            assert!(
                table.call_sites.iter().any(|site| site.call.contains(call)),
                "{call} is missing from:\n{text}"
            );
        }
    }

    #[test]
    fn reader_failures_become_driver_errors() {
        let error = |code: &str, message: &str| protocol::Error {
            code: code.into(),
            message: message.into(),
            retryable: false,
        };
        let thrown = tree_error(
            error(
                codes::PRIVATE_API_EXCEPTION,
                "-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:] raised NSInternalInconsistencyException: no server",
            ),
            Duration::from_secs(1),
        );
        assert_eq!(
            thrown,
            Error::Framework {
                call: "-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:], in simulator-reader".into(),
                failure: Failure::Exception {
                    name: "NSInternalInconsistencyException".into(),
                    reason: "no server".into(),
                },
            }
        );
        let early = tree_error(error(codes::NO_FOREGROUND, "starting"), Duration::ZERO);
        assert_eq!(early.code(), "no_foreground");
        assert!(early.retryable());
        let late = tree_error(
            error(protocol::code::DEADLINE_EXCEEDED, "late"),
            Duration::from_secs(3),
        );
        assert!(matches!(late, Error::Timeout { after, .. } if after == Duration::from_secs(3)));
        let odd = tree_error(error("something_else", "odd"), Duration::ZERO);
        assert_eq!(odd.code(), "tree_read_failed");
        assert!(!odd.retryable());
    }

    #[test]
    fn esrch_means_the_application_is_not_running() {
        assert!(not_running(&ns_error("NSPOSIXErrorDomain", 3)));
        assert!(!not_running(&ns_error("NSPOSIXErrorDomain", 1)));
        assert!(!not_running(&ns_error("IXErrorDomain", 3)));
    }

    /// Design D2 of pass-4-fixes: `open --relaunch` right after the app's
    /// termination and a press of Home was refused with `NSPOSIXErrorDomain`
    /// 3, "Simulator device failed to launch", ten times in two runs on an
    /// iOS 26.5 Simulator under load, and a launch 3 s later went through
    /// every time. A launch call that answers the refusal once is launched
    /// once more after the wait, which the launch reports; a second refusal
    /// fails as it came, and no other failure is launched again.
    #[test]
    fn a_launch_refused_with_esrch_is_launched_once_more_after_the_wait() {
        let refusal = || ns_error("NSPOSIXErrorDomain", 3);
        // A fake of the device's launch call: answers each launch with the
        // next of `answers`, counting the calls, and records the waits.
        let fake = |answers: Vec<Result<i32>>| {
            let calls = std::cell::Cell::new(0);
            let waits = std::cell::RefCell::new(Vec::new());
            let mut answers = answers.into_iter();
            let result = launch_with(
                || {
                    calls.set(calls.get() + 1);
                    answers.next().expect("no launch beyond the answers")
                },
                |wait| waits.borrow_mut().push(wait),
            );
            (result, calls.get(), waits.into_inner())
        };

        let (result, calls, waits) = fake(vec![Err(refusal()), Ok(42)]);
        assert_eq!(
            result.as_ref().ok(),
            Some(&(42, Some(Duration::from_secs(3)))),
            "{result:?}"
        );
        assert_eq!((calls, waits), (2, vec![Duration::from_secs(3)]));

        let (result, calls, waits) = fake(vec![Err(refusal()), Err(refusal())]);
        assert_eq!(result, Err(refusal()));
        assert_eq!((calls, waits), (2, vec![Duration::from_secs(3)]));

        for other in [
            ns_error("NSPOSIXErrorDomain", 1),
            ns_error("FBSOpenApplicationServiceErrorDomain", 3),
        ] {
            let (result, calls, waits) = fake(vec![Err(other.clone()), Ok(42)]);
            assert_eq!(result, Err(other));
            assert_eq!((calls, waits), (1, Vec::new()));
        }

        let (result, calls, waits) = fake(vec![Ok(42)]);
        assert_eq!(result, Ok((42, None)));
        assert_eq!((calls, waits), (1, Vec::new()));
    }

    #[test]
    fn only_installds_promise_error_is_retried() {
        assert!(transient_install_failure(&ns_error("IXErrorDomain", 2)));
        assert!(!transient_install_failure(&ns_error("IXErrorDomain", 1)));
        assert!(!transient_install_failure(&ns_error(
            "NSPOSIXErrorDomain",
            2
        )));
    }
}
