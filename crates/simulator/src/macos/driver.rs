//! [`SimulatorDriver`]: the driver traits over CoreSimulator.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::driver::{Frames, Input, Lifecycle};
use crate::error::{Error, Failure, Result};
use crate::frames::FrameStream;
use crate::gesture;
use crate::lookup;
use crate::macos::coresim::{Device, DeviceSet, ServiceContext};
use crate::macos::dtuhid::Dtuhid;
use crate::macos::indigo::Indigo;
use crate::macos::input::{Target, Transport};
use crate::macos::{frames, prefs};
use crate::model::{
    BootReport, Button, DeviceInfo, DeviceState, DeviceTypeInfo, Dispatch, DriverOptions,
    FrameOptions, Gesture, InputStatus, Key, LaunchOptions, Launched, Point, PreferencesOutcome,
    RuntimeInfo, Screenshot, ScreenshotOptions, Swipe, TargetStatus, TransportChoice,
    TransportKind,
};

/// The first boot of a new device took 43 to 146 s to a terminal boot status
/// in spike S2, under load; later boots about 11 s.
const BOOT_TIMEOUT: Duration = Duration::from_secs(300);
/// A shutdown took about 3.4 s in S2.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);
/// installd answers IXErrorDomain 2 ("Failed to locate promise") right after
/// a boot; S2 saw a retry 3 s later succeed.
const INSTALL_ATTEMPTS: u32 = 5;
const INSTALL_RETRY: Duration = Duration::from_secs(2);

/// Drives the iOS Simulators of the default device set.
pub struct SimulatorDriver {
    context: ServiceContext,
    set: DeviceSet,
    options: DriverOptions,
    /// The devices input went to, by UDID, each with its transport.
    targets: Mutex<HashMap<String, Arc<Target>>>,
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
        let context = ServiceContext::connect()?;
        let set = context.default_set()?;
        Ok(SimulatorDriver {
            context,
            set,
            options,
            targets: Mutex::new(HashMap::new()),
        })
    }

    /// The installed runtimes.
    pub fn runtimes(&self) -> Result<Vec<RuntimeInfo>> {
        self.context.runtimes()?.iter().map(|r| r.info()).collect()
    }

    /// The device types this Xcode knows.
    pub fn device_types(&self) -> Result<Vec<DeviceTypeInfo>> {
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
        let types = self.context.device_types()?;
        let type_infos: Vec<DeviceTypeInfo> =
            types.iter().map(|t| t.info()).collect::<Result<_>>()?;
        let device_type = &types[lookup::device_type(device_type, &type_infos)?];
        let runtimes = self.context.runtimes()?;
        let runtime_infos: Vec<RuntimeInfo> =
            runtimes.iter().map(|r| r.info()).collect::<Result<_>>()?;
        let runtime = &runtimes[lookup::runtime(runtime, &runtime_infos)?];

        let device = self.set.create(device_type, runtime, name)?;
        prefs::write(&device.data_path()?)?;
        device.info()
    }

    /// Deletes a device, which must be shut down.
    pub fn delete_device(&self, target: &str) -> Result<()> {
        let device = self.device(target)?;
        needs(target, &device, DeviceState::Shutdown, "shut down")?;
        self.set.delete(&device)
    }

    /// Writes the accessibility preferences into a device that is shut down;
    /// `boot` does this itself.
    pub fn prepare_accessibility(&self, target: &str) -> Result<PreferencesOutcome> {
        let device = self.device(target)?;
        needs(target, &device, DeviceState::Shutdown, "shut down")?;
        prefs::write(&device.data_path()?)
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

    /// The target and its input transport, opened on first use.
    fn input(&self, target: &str) -> Result<(Arc<Target>, Arc<dyn Transport>)> {
        let target = self.target(target)?;
        let transport = target.transport(|device| self.open_transport(device))?;
        Ok((target, transport))
    }

    fn open_transport(&self, device: &Device) -> Result<Arc<dyn Transport>> {
        match self.options.transport {
            TransportChoice::Auto | TransportChoice::Only(TransportKind::Dtuhid) => {
                Ok(Arc::new(Dtuhid::open(device)?))
            }
            TransportChoice::Only(TransportKind::Indigo) => Ok(Arc::new(Indigo::open(device)?)),
        }
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
        self.set.devices()?.iter().map(Device::info).collect()
    }

    fn boot(&self, target: &str) -> Result<BootReport> {
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
    }

    fn shutdown(&self, target: &str) -> Result<()> {
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
    }

    fn install(&self, target: &str, app: &Path) -> Result<()> {
        let device = self.booted(target)?;
        let app = app.canonicalize().map_err(|error| Error::Io {
            what: format!("finding {}", app.display()),
            reason: error.to_string(),
        })?;
        let mut attempt = 1;
        loop {
            match device.install(&app) {
                Err(error) if transient_install_failure(&error) && attempt < INSTALL_ATTEMPTS => {
                    attempt += 1;
                    thread::sleep(INSTALL_RETRY);
                }
                result => return result,
            }
        }
    }

    fn launch(&self, target: &str, bundle_id: &str, options: &LaunchOptions) -> Result<Launched> {
        let device = self.booted(target)?;
        if options.terminate_running {
            match device.terminate(bundle_id) {
                Err(error) if !not_running(&error) => return Err(error),
                _ => {}
            }
        }
        let start = Instant::now();
        let pid = device.launch(bundle_id, &options.arguments, &options.environment)?;
        Ok(Launched {
            pid,
            elapsed: start.elapsed(),
        })
    }

    fn terminate(&self, target: &str, bundle_id: &str) -> Result<()> {
        let device = self.booted(target)?;
        match device.terminate(bundle_id) {
            Err(error) if not_running(&error) => Ok(()),
            result => result,
        }
    }

    fn status(&self, target: &str) -> Result<TargetStatus> {
        let device = self.device(target)?;
        let info = device.info()?;
        let transport = lock(&self.targets)
            .get(&info.udid)
            .and_then(|target| target.current_transport());
        Ok(TargetStatus {
            device: info,
            input: transport.map(|transport| InputStatus {
                transport: transport.kind(),
                activation: transport.activation(),
                connections: transport.connections(),
            }),
        })
    }
}

impl Frames for SimulatorDriver {
    fn frames(&self, target: &str, options: &FrameOptions) -> Result<FrameStream> {
        let device = self.booted(target)?;
        let subscription = frames::Subscription::start(&device, options)?;
        Ok(FrameStream::new(Box::new(subscription)))
    }

    fn screenshot(&self, target: &str, options: &ScreenshotOptions) -> Result<Screenshot> {
        let device = self.booted(target)?;
        frames::screenshot(&device, options)
    }
}

impl Input for SimulatorDriver {
    fn tap(&self, target: &str, point: Point) -> Result<Dispatch> {
        self.press(target, point, Duration::ZERO)
    }

    fn press(&self, target: &str, point: Point, hold: Duration) -> Result<Dispatch> {
        let (target, transport) = self.input(target)?;
        let plan = gesture::press(&target.screen, point, hold)?;
        target.touch(&*transport, &plan)
    }

    fn swipe(&self, target: &str, swipe: &Swipe) -> Result<Dispatch> {
        let (target, transport) = self.input(target)?;
        let plan = gesture::swipe(&target.screen, swipe)?;
        target.touch(&*transport, &plan)
    }

    fn gesture(&self, target: &str, gesture: &Gesture) -> Result<Dispatch> {
        let (target, transport) = self.input(target)?;
        let plan = gesture::gesture(&target.screen, gesture)?;
        target.touch(&*transport, &plan)
    }

    fn button(&self, target: &str, button: Button) -> Result<Dispatch> {
        let (target, transport) = self.input(target)?;
        target.button(&*transport, button)
    }

    fn key(&self, target: &str, key: Key) -> Result<Dispatch> {
        let (target, transport) = self.input(target)?;
        target.key(&*transport, key)
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

    #[test]
    fn esrch_means_the_application_is_not_running() {
        assert!(not_running(&ns_error("NSPOSIXErrorDomain", 3)));
        assert!(!not_running(&ns_error("NSPOSIXErrorDomain", 1)));
        assert!(!not_running(&ns_error("IXErrorDomain", 3)));
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
