//! CoreSimulator bindings: the service context, the default device set and
//! the device lifecycle, ported from spike S2. Classes are looked up by name
//! and every call is a dynamic message send inside the exception guard.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_foundation::{NSArray, NSDictionary, NSError, NSNumber, NSSize, NSString, NSURL, NSUUID};

use crate::error::{Error, Failure, Result};
use crate::gesture::Screen;
use crate::macos::objc::{self, SendBox, Shared, guarded, guarded_ns};
use crate::model::{DeviceInfo, DeviceState, DeviceTypeInfo, RuntimeInfo};

/// How long CoreSimulator gets to run a completion handler for calls that
/// normally answer in under a second.
const COMPLETION: Duration = Duration::from_secs(120);

/// `DEVELOPER_DIR` when set, otherwise the `xcode-select` choice.
pub fn developer_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("DEVELOPER_DIR").filter(|dir| !dir.is_empty()) {
        return PathBuf::from(dir);
    }
    std::fs::read_link("/var/db/xcode_select_link")
        .unwrap_or_else(|_| PathBuf::from("/Applications/Xcode.app/Contents/Developer"))
}

/// The connection to CoreSimulatorService for one developer directory.
pub struct ServiceContext {
    context: Shared,
}

impl ServiceContext {
    /// Loads CoreSimulator and connects to its service.
    pub fn connect() -> Result<ServiceContext> {
        objc::load_core_simulator()?;
        let class = objc::class(c"SimServiceContext")?;
        let dir = NSString::from_str(&developer_dir().to_string_lossy());
        let context: Retained<AnyObject> = guarded_ns(
            "+[SimServiceContext sharedServiceContextForDeveloperDir:error:]",
            // SAFETY: the selector takes an NSString and an NSError out
            // pointer and returns the shared context.
            || unsafe { msg_send![class, sharedServiceContextForDeveloperDir: &*dir, error: _] },
        )?;
        Ok(ServiceContext {
            context: Shared::new(context),
        })
    }

    pub fn default_set(&self) -> Result<DeviceSet> {
        let set: Retained<AnyObject> = guarded_ns(
            "-[SimServiceContext defaultDeviceSetWithError:]",
            // SAFETY: takes an NSError out pointer, returns the set.
            || unsafe { msg_send![&*self.context, defaultDeviceSetWithError: _] },
        )?;
        Ok(DeviceSet {
            set: Shared::new(set),
        })
    }

    pub fn runtimes(&self) -> Result<Vec<Runtime>> {
        let runtimes: Option<Retained<NSArray<AnyObject>>> =
            guarded("-[SimServiceContext supportedRuntimes]", || {
                // SAFETY: returns an NSArray of SimRuntime.
                unsafe { msg_send![&*self.context, supportedRuntimes] }
            })?;
        Ok(objects(runtimes).map(Runtime).collect())
    }

    pub fn device_types(&self) -> Result<Vec<DeviceType>> {
        let types: Option<Retained<NSArray<AnyObject>>> =
            guarded("-[SimServiceContext supportedDeviceTypes]", || {
                // SAFETY: returns an NSArray of SimDeviceType.
                unsafe { msg_send![&*self.context, supportedDeviceTypes] }
            })?;
        Ok(objects(types).map(DeviceType).collect())
    }
}

fn objects(array: Option<Retained<NSArray<AnyObject>>>) -> impl Iterator<Item = Shared> {
    array
        .map(|array| array.to_vec())
        .unwrap_or_default()
        .into_iter()
        .map(Shared::new)
}

/// A set of devices; the driver uses the default one, which `simctl` lists.
pub struct DeviceSet {
    set: Shared,
}

impl DeviceSet {
    pub fn devices(&self) -> Result<Vec<Device>> {
        let devices: Option<Retained<NSArray<AnyObject>>> =
            guarded("-[SimDeviceSet devices]", || {
                // SAFETY: returns an NSArray of SimDevice.
                unsafe { msg_send![&*self.set, devices] }
            })?;
        Ok(objects(devices).map(Device).collect())
    }

    pub fn create(
        &self,
        device_type: &DeviceType,
        runtime: &Runtime,
        name: &str,
    ) -> Result<Device> {
        const CALL: &str = "-[SimDeviceSet createDeviceAsyncWithType:runtime:name:completionQueue:completionHandler:]";
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |error: *mut NSError, device: *mut AnyObject| {
            // SAFETY: CoreSimulator passes nil or a live NSError and SimDevice.
            let result = unsafe { objc::completion_error(CALL, error) }.and_then(|()| {
                // SAFETY: as above; retained before the block returns.
                unsafe { Retained::retain(device) }
                    .map(SendBox)
                    .ok_or_else(|| Error::Framework {
                        call: CALL.into(),
                        failure: Failure::Nil,
                    })
            });
            let _ = tx.send(result);
        });
        let name = NSString::from_str(name);
        guarded(CALL, || {
            // SAFETY: the arguments match the selector; the block's signature
            // is `void (^)(NSError *, SimDevice *)`.
            unsafe {
                let _: () = msg_send![
                    &*self.set,
                    createDeviceAsyncWithType: &*device_type.0,
                    runtime: &*runtime.0,
                    name: &*name,
                    completionQueue: objc::completion_queue(),
                    completionHandler: &*handler
                ];
            }
        })?;
        let device = objc::wait(rx, CALL, COMPLETION)??;
        Ok(Device(Shared::new(device.0)))
    }

    pub fn delete(&self, device: &Device) -> Result<()> {
        const CALL: &str = "-[SimDeviceSet deleteDeviceAsync:completionQueue:completionHandler:]";
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: CoreSimulator passes nil or a live NSError.
            let _ = tx.send(unsafe { objc::completion_error(CALL, error) });
        });
        guarded(CALL, || {
            // SAFETY: the arguments match the selector.
            unsafe {
                let _: () = msg_send![
                    &*self.set,
                    deleteDeviceAsync: &*device.0,
                    completionQueue: objc::completion_queue(),
                    completionHandler: &*handler
                ];
            }
        })?;
        objc::wait(rx, CALL, COMPLETION)?
    }
}

/// An installed runtime.
#[derive(Clone)]
pub struct Runtime(Shared);

impl Runtime {
    pub fn info(&self) -> Result<RuntimeInfo> {
        guarded(
            "-[SimRuntime name/versionString/buildVersionString/identifier]",
            || {
                // SAFETY: each property returns an NSString or nil.
                unsafe {
                    RuntimeInfo {
                        name: objc::string(msg_send![&*self.0, name]),
                        version: objc::string(msg_send![&*self.0, versionString]),
                        build: objc::string(msg_send![&*self.0, buildVersionString]),
                        identifier: objc::string(msg_send![&*self.0, identifier]),
                    }
                }
            },
        )
    }
}

/// A device type, such as an iPhone model.
#[derive(Clone)]
pub struct DeviceType(Shared);

impl DeviceType {
    pub fn info(&self) -> Result<DeviceTypeInfo> {
        guarded("-[SimDeviceType name/identifier]", || {
            // SAFETY: each property returns an NSString or nil.
            unsafe {
                DeviceTypeInfo {
                    name: objc::string(msg_send![&*self.0, name]),
                    identifier: objc::string(msg_send![&*self.0, identifier]),
                }
            }
        })
    }

    /// The main screen: its native portrait size in pixels and its scale.
    pub fn main_screen(&self) -> Result<Screen> {
        guarded("-[SimDeviceType mainScreenSize/mainScreenScale]", || {
            // SAFETY: `mainScreenSize` returns an NSSize in pixels and
            // `mainScreenScale` a float.
            unsafe {
                let size: NSSize = msg_send![&*self.0, mainScreenSize];
                let scale: f32 = msg_send![&*self.0, mainScreenScale];
                Screen {
                    pixels: (size.width, size.height),
                    scale: f64::from(scale),
                }
            }
        })
    }
}

/// `-[SimDevice bootStatus]`, which turns terminal once the device is usable,
/// some time after its state reads Booted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BootStatus {
    pub status: u32,
    pub terminal: bool,
}

/// A Simulator.
#[derive(Clone)]
pub struct Device(Shared);

impl Device {
    /// The `SimDevice` itself, for the calls that take one.
    pub fn object(&self) -> &AnyObject {
        &self.0
    }

    pub fn udid(&self) -> Result<String> {
        guarded("-[SimDevice UDID]", || {
            // SAFETY: `UDID` returns an NSUUID.
            let uuid: Option<Retained<NSUUID>> = unsafe { msg_send![&*self.0, UDID] };
            uuid.map(|uuid| uuid.UUIDString().to_string())
                .unwrap_or_default()
        })
    }

    pub fn name(&self) -> Result<String> {
        guarded("-[SimDevice name]", || {
            // SAFETY: `name` returns an NSString.
            objc::string(unsafe { msg_send![&*self.0, name] })
        })
    }

    pub fn state(&self) -> Result<DeviceState> {
        let raw: u64 = guarded("-[SimDevice state]", || {
            // SAFETY: `state` returns an unsigned long long.
            unsafe { msg_send![&*self.0, state] }
        })?;
        Ok(DeviceState::from_raw(raw))
    }

    pub fn runtime(&self) -> Result<Option<Runtime>> {
        let runtime: Option<Retained<AnyObject>> = guarded("-[SimDevice runtime]", || {
            // SAFETY: returns a SimRuntime, or nil when it is not installed.
            unsafe { msg_send![&*self.0, runtime] }
        })?;
        Ok(runtime.map(|runtime| Runtime(Shared::new(runtime))))
    }

    pub fn device_type(&self) -> Result<Option<DeviceType>> {
        let device_type: Option<Retained<AnyObject>> = guarded("-[SimDevice deviceType]", || {
            // SAFETY: returns a SimDeviceType or nil.
            unsafe { msg_send![&*self.0, deviceType] }
        })?;
        Ok(device_type.map(|device_type| DeviceType(Shared::new(device_type))))
    }

    pub fn info(&self) -> Result<DeviceInfo> {
        Ok(DeviceInfo {
            udid: self.udid()?,
            name: self.name()?,
            state: self.state()?,
            runtime: self.runtime()?.map(|r| r.info()).transpose()?,
            device_type: self.device_type()?.map(|t| t.info()).transpose()?,
        })
    }

    /// `-[SimDevice io]`, the client for the device's IO ports.
    pub fn io(&self) -> Result<Shared> {
        const CALL: &str = "-[SimDevice io]";
        let io: Option<Retained<AnyObject>> = guarded(CALL, || {
            // SAFETY: `io` returns a SimDeviceIOClient or nil.
            unsafe { msg_send![&*self.0, io] }
        })?;
        io.map(Shared::new).ok_or_else(|| Error::Framework {
            call: CALL.into(),
            failure: Failure::Nil,
        })
    }

    /// The installed applications' bundle identifiers, each with whether
    /// it is a user application rather than part of the runtime.
    pub fn installed_apps(&self) -> Result<Vec<(String, bool)>> {
        const CALL: &str = "-[SimDevice installedAppsWithError:]";
        let apps: Retained<NSDictionary<NSString, AnyObject>> = guarded_ns(CALL, || {
            // SAFETY: takes an NSError out pointer and returns a dictionary
            // of bundle identifiers to information dictionaries.
            unsafe { msg_send![&*self.0, installedAppsWithError: _] }
        })?;
        guarded(CALL, || {
            let (keys, values) = apps.to_vecs();
            keys.iter()
                .zip(values)
                .map(|(bundle, info)| {
                    let user = objc::is_kind_of(&info, c"NSDictionary") && {
                        // SAFETY: just checked that it is a dictionary.
                        let info: &NSDictionary<NSString, AnyObject> =
                            unsafe { &*(Retained::as_ptr(&info).cast()) };
                        info.objectForKey(&NSString::from_str("ApplicationType"))
                            .is_some_and(|kind| {
                                objc::is_kind_of(&kind, c"NSString") && {
                                    // SAFETY: just checked that it is a string.
                                    unsafe { &*(Retained::as_ptr(&kind).cast::<NSString>()) }
                                        .to_string()
                                        == "User"
                                }
                            })
                    };
                    (bundle.to_string(), user)
                })
                .collect()
        })
    }

    /// Grants or revokes a TCC service for an application, as `simctl
    /// privacy` does; it takes TCC's own service names too.
    pub fn set_privacy(&self, service: &str, bundle_id: &str, granted: bool) -> Result<()> {
        let (service, bundle_id) = (NSString::from_str(service), NSString::from_str(bundle_id));
        guarded_ns(
            "-[SimDevice setPrivacyAccessForService:bundleID:granted:error:]",
            // SAFETY: two NSStrings, a BOOL and an NSError out pointer.
            || unsafe {
                msg_send![
                    &*self.0,
                    setPrivacyAccessForService: &*service,
                    bundleID: &*bundle_id,
                    granted: granted,
                    error: _
                ]
            },
        )
    }

    /// `-[SimDevice lookup:error:]`: the Mach port of `service` in the
    /// guest's bootstrap namespace. It returns a `mach_port_t` (S3).
    pub fn lookup(&self, service: &str) -> Result<u32> {
        const CALL: &str = "-[SimDevice lookup:error:]";
        let name = NSString::from_str(service);
        let (port, error) = guarded(CALL, || {
            let mut error: Option<Retained<NSError>> = None;
            // SAFETY: takes an NSString and an NSError out pointer and
            // returns a mach_port_t.
            let port: u32 = unsafe { msg_send![&*self.0, lookup: &*name, error: &mut error] };
            (port, error)
        })?;
        match (port, error) {
            (0, Some(error)) => Err(objc::ns_error(CALL, &error)),
            (0, None) => Err(Error::Framework {
                call: format!("{CALL} for {service}"),
                failure: Failure::Nil,
            }),
            (port, _) => Ok(port),
        }
    }

    /// The device's data directory on the host, which holds its home,
    /// preferences and installed applications.
    pub fn data_path(&self) -> Result<PathBuf> {
        const CALL: &str = "-[SimDevice dataPath]";
        let path: Option<Retained<AnyObject>> = guarded(CALL, || {
            // SAFETY: returns an NSString path, or an NSURL on some versions.
            unsafe { msg_send![&*self.0, dataPath] }
        })?;
        let path = path.ok_or_else(|| Error::Framework {
            call: CALL.into(),
            failure: Failure::Nil,
        })?;
        let text = if objc::is_kind_of(&path, c"NSURL") {
            guarded("-[NSURL path]", || {
                // SAFETY: `path` of an NSURL returns an NSString.
                objc::string(unsafe { msg_send![&*path, path] })
            })?
        } else if objc::is_kind_of(&path, c"NSString") {
            // SAFETY: just checked that it is an NSString.
            let string: &NSString = unsafe { &*(Retained::as_ptr(&path).cast::<NSString>()) };
            string.to_string()
        } else {
            return Err(Error::Framework {
                call: CALL.into(),
                failure: Failure::Exception {
                    name: "unexpected type".into(),
                    reason: "dataPath is neither an NSString nor an NSURL".into(),
                },
            });
        };
        Ok(PathBuf::from(text))
    }

    /// Boots the device. With `persist` it outlives this process, as with
    /// `simctl boot`; without it CoreSimulator shuts it down when this
    /// process exits. The completion arrives when the state reads Booted,
    /// well before the device is usable.
    pub fn boot(&self, persist: bool) -> Result<()> {
        const CALL: &str = "-[SimDevice bootAsyncWithOptions:completionQueue:completionHandler:]";
        let options: Retained<NSDictionary<NSString, NSNumber>> = NSDictionary::from_slices(
            &[&*NSString::from_str("persist")],
            &[&*NSNumber::new_bool(persist)],
        );
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: CoreSimulator passes nil or a live NSError.
            let _ = tx.send(unsafe { objc::completion_error(CALL, error) });
        });
        guarded(CALL, || {
            // SAFETY: the arguments match the selector.
            unsafe {
                let _: () = msg_send![
                    &*self.0,
                    bootAsyncWithOptions: &*options,
                    completionQueue: objc::completion_queue(),
                    completionHandler: &*handler
                ];
            }
        })?;
        objc::wait(rx, CALL, Duration::from_secs(300))?
    }

    pub fn boot_status(&self) -> Result<Option<BootStatus>> {
        guarded("-[SimDevice bootStatus]", || {
            // SAFETY: `bootStatus` returns a SimDeviceBootInfo or nil, whose
            // `status` is an unsigned int and `isTerminalStatus` a BOOL.
            unsafe {
                let info: Option<Retained<AnyObject>> = msg_send![&*self.0, bootStatus];
                info.map(|info| BootStatus {
                    status: msg_send![&*info, status],
                    terminal: msg_send![&*info, isTerminalStatus],
                })
            }
        })
    }

    /// Polls until the state is Booted and the boot status is terminal, which
    /// is the readiness `simctl bootstatus` and idb wait for.
    pub fn wait_until_ready(&self, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        loop {
            if self.state()? == DeviceState::Booted
                && self.boot_status()?.is_some_and(|status| status.terminal)
            {
                return Ok(());
            }
            if start.elapsed() > timeout {
                return Err(Error::Timeout {
                    what: "the device's boot status turning terminal".into(),
                    after: timeout,
                });
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// Polls until the state is `wanted`.
    pub fn wait_for_state(&self, wanted: DeviceState, timeout: Duration) -> Result<()> {
        let start = Instant::now();
        while self.state()? != wanted {
            if start.elapsed() > timeout {
                return Err(Error::Timeout {
                    what: format!("the device reaching the {wanted} state"),
                    after: timeout,
                });
            }
            thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    pub fn shutdown(&self) -> Result<()> {
        const CALL: &str = "-[SimDevice shutdownAsyncWithCompletionQueue:completionHandler:]";
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |error: *mut NSError| {
            // SAFETY: CoreSimulator passes nil or a live NSError.
            let _ = tx.send(unsafe { objc::completion_error(CALL, error) });
        });
        guarded(CALL, || {
            // SAFETY: the arguments match the selector.
            unsafe {
                let _: () = msg_send![
                    &*self.0,
                    shutdownAsyncWithCompletionQueue: objc::completion_queue(),
                    completionHandler: &*handler
                ];
            }
        })?;
        objc::wait(rx, CALL, COMPLETION)?
    }

    pub fn install(&self, app: &Path) -> Result<()> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&app.to_string_lossy()));
        let options: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::new();
        guarded_ns("-[SimDevice installApplication:withOptions:error:]", || {
            // SAFETY: takes a file URL, an options dictionary and an NSError
            // out pointer; returns BOOL.
            unsafe {
                msg_send![&*self.0, installApplication: &*url, withOptions: &*options, error: _]
            }
        })
    }

    /// Launches `bundle_id` with `arguments` and `environment` and returns
    /// its process identifier.
    pub fn launch(
        &self,
        bundle_id: &str,
        arguments: &[String],
        environment: &[(String, String)],
    ) -> Result<i32> {
        const CALL: &str =
            "-[SimDevice launchApplicationAsyncWithID:options:completionQueue:completionHandler:]";
        let arguments = strings(arguments);
        let environment = dictionary(environment);
        let arguments_object: &AnyObject = &arguments;
        let environment_object: &AnyObject = &environment;
        let options: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
            &[
                &*NSString::from_str("arguments"),
                &*NSString::from_str("environment"),
            ],
            &[arguments_object, environment_object],
        );
        let (tx, rx) = mpsc::channel();
        let handler = RcBlock::new(move |error: *mut NSError, pid: libc::pid_t| {
            // SAFETY: CoreSimulator passes nil or a live NSError.
            let _ = tx.send(unsafe { objc::completion_error(CALL, error) }.map(|()| pid));
        });
        let bundle_id = NSString::from_str(bundle_id);
        guarded(CALL, || {
            // SAFETY: the arguments match the selector; the block's signature
            // is `void (^)(NSError *, pid_t)`.
            unsafe {
                let _: () = msg_send![
                    &*self.0,
                    launchApplicationAsyncWithID: &*bundle_id,
                    options: &*options,
                    completionQueue: objc::completion_queue(),
                    completionHandler: &*handler
                ];
            }
        })?;
        objc::wait(rx, CALL, COMPLETION)?
    }

    pub fn terminate(&self, bundle_id: &str) -> Result<()> {
        let bundle_id = NSString::from_str(bundle_id);
        guarded_ns("-[SimDevice terminateApplicationWithID:error:]", || {
            // SAFETY: takes an NSString and an NSError out pointer.
            unsafe { msg_send![&*self.0, terminateApplicationWithID: &*bundle_id, error: _] }
        })
    }
}

fn strings(values: &[String]) -> Retained<NSArray<NSString>> {
    let strings: Vec<Retained<NSString>> = values.iter().map(|v| NSString::from_str(v)).collect();
    NSArray::from_retained_slice(&strings)
}

fn dictionary(pairs: &[(String, String)]) -> Retained<NSDictionary<NSString, NSString>> {
    let keys: Vec<Retained<NSString>> = pairs.iter().map(|(k, _)| NSString::from_str(k)).collect();
    let values: Vec<Retained<NSString>> =
        pairs.iter().map(|(_, v)| NSString::from_str(v)).collect();
    let keys: Vec<&NSString> = keys.iter().map(|k| &**k).collect();
    let values: Vec<&NSString> = values.iter().map(|v| &**v).collect();
    NSDictionary::from_slices(&keys, &values)
}
