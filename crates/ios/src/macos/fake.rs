//! A fake device behind a fake tunnel, for the tests: an RSD endpoint that
//! answers the handshake with the services spike S9 recorded on the iPhone SE
//! (3rd generation) with iOS 26.6.2, each on a port of its own, and service
//! ports that accept connections, served by handlers the tests register.
//! The tunnel can drop, as jktcp's does when its adapter dies, and the
//! device can go away and come back.
//!
//! Its DVT speaks DTX through `idevice`'s own message types: it opens
//! channels, launches the apps installed on it, signals and lists
//! processes, records every call with its arguments decoded, and refuses a
//! launch with an archived `NSError` as the phone does. A primitive pid in a
//! signal drops the connection, as `kill_app`'s did on iOS 26.6.2 (S9). Its
//! usbmuxd side, [`FakeLink`], reads the lock and installs bundles.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use idevice::dvt::message::{Aux, AuxValue, Message, MessageHeader, PayloadHeader};

use super::dvt::{
    DEVICE_INFO, IS_RUNNING_PID, LAUNCH, PROCESS_CONTROL, RUNNING_PROCESSES, SEND_SIGNAL,
};
use idevice::provider::RsdProvider;
use idevice::xpc::{XPCMessage, XPCObject};
use idevice::{IdeviceError, ReadWrite};
use plist::{Dictionary, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::host::Link;
use super::image::Mounter;
use super::tunnel::Opener;
use crate::discovery::tests::recorded_images;
use crate::services;

/// The port the fake RSD answers on.
const RSD_PORT: u16 = 58783;
/// The first service port.
const FIRST_PORT: u16 = 50000;

/// Serves one connection to a service port.
pub(crate) type Handler =
    Arc<dyn Fn(DuplexStream) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

#[derive(Default)]
struct State {
    /// The tunnel generation that serves; a provider of an older one fails
    /// as jktcp's does once its adapter is gone.
    generation: u32,
    gone: bool,
    rsd_silent: bool,
    opens: u32,
    closed: u32,
    connections: Vec<u16>,
}

/// A process of the fake device.
#[derive(Clone, Debug)]
struct FakeProcess {
    pid: u32,
    name: String,
    bundle_id: Option<String>,
    path: String,
}

/// What the fake DVT knows and was asked.
struct Dvt {
    /// The installed apps by bundle identifier, with their executables.
    apps: HashMap<String, String>,
    processes: Vec<FakeProcess>,
    next_pid: u32,
    calls: Vec<DvtCall>,
    refuse_launches: bool,
    silent: bool,
    closed: bool,
}

impl Default for Dvt {
    fn default() -> Dvt {
        Dvt {
            apps: HashMap::new(),
            processes: vec![
                FakeProcess {
                    pid: 1,
                    name: "launchd".into(),
                    bundle_id: None,
                    path: "/sbin/launchd".into(),
                },
                FakeProcess {
                    pid: 34,
                    name: "SpringBoard".into(),
                    bundle_id: Some("com.apple.springboard".into()),
                    path: "/System/Library/CoreServices/SpringBoard.app/SpringBoard".into(),
                },
            ],
            next_pid: 1000,
            calls: Vec::new(),
            refuse_launches: false,
            silent: false,
            closed: false,
        }
    }
}

/// What the fake usbmuxd side knows and was asked.
#[derive(Default)]
struct LinkState {
    locked: bool,
    lock_reads: u32,
    install_failure: Option<String>,
    installs: Vec<PathBuf>,
}

/// One argument of a DTX call, as the fake DVT decoded it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Arg {
    /// An archived object, unarchived.
    Archived(Value),
    /// A primitive integer, as `kill_app` sends a pid.
    Primitive(i64),
    Text(String),
    Other,
}

/// One call the fake DVT took.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DvtCall {
    /// The channel's identifier, such as DVT's process control.
    pub service: String,
    pub selector: String,
    pub args: Vec<Arg>,
}

/// The device behind the fake tunnel.
pub(crate) struct FakeDevice {
    /// Each service's name, port and recorded entry, in the recording's
    /// order.
    services: Vec<(String, u16, Dictionary)>,
    properties: Dictionary,
    state: Mutex<State>,
    /// The serving generation, for the connections open over a tunnel,
    /// which close when it drops.
    generations: tokio::sync::watch::Sender<u32>,
    handlers: Mutex<HashMap<u16, Handler>>,
    dvt: Mutex<Dvt>,
    link: Mutex<LinkState>,
}

impl fmt::Debug for FakeDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeDevice")
            .field("services", &self.services.len())
            .finish_non_exhaustive()
    }
}

impl FakeDevice {
    /// The device as spike S9 recorded its handshake: 82 services.
    pub(crate) fn recorded() -> Arc<FakeDevice> {
        FakeDevice::without(&[])
    }

    /// The recorded device without the services named.
    pub(crate) fn without(names: &[&str]) -> Arc<FakeDevice> {
        let recording: Dictionary =
            plist::from_bytes(include_bytes!("../../tests/fixtures/rsd-handshake.plist"))
                .expect("the recorded handshake parses");
        let entries = recording
            .get("Services")
            .and_then(Value::as_array)
            .expect("the recorded services");
        let mut services = Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            let entry = entry.as_dictionary().expect("a service entry").clone();
            let name = entry
                .get("Name")
                .and_then(Value::as_string)
                .expect("a name")
                .to_owned();
            if names.contains(&name.as_str()) {
                continue;
            }
            let port = FIRST_PORT + u16::try_from(index).expect("few services");
            services.push((name, port, entry));
        }
        Arc::new(FakeDevice {
            services,
            properties: recording
                .get("Properties")
                .and_then(Value::as_dictionary)
                .cloned()
                .unwrap_or_default(),
            state: Mutex::new(State::default()),
            generations: tokio::sync::watch::Sender::new(0),
            handlers: Mutex::new(HashMap::new()),
            dvt: Mutex::new(Dvt::default()),
            link: Mutex::new(LinkState::default()),
        })
    }

    fn dvt(&self) -> MutexGuard<'_, Dvt> {
        self.dvt
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn link(&self) -> MutexGuard<'_, LinkState> {
        self.link
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Installs an app, as the installation proxy would.
    pub(crate) fn install_app(&self, bundle_id: &str, executable: &str) {
        self.dvt()
            .apps
            .insert(bundle_id.to_owned(), executable.to_owned());
    }

    /// The pids of `bundle_id`'s processes.
    pub(crate) fn running(&self, bundle_id: &str) -> Vec<u32> {
        self.dvt()
            .processes
            .iter()
            .filter(|process| process.bundle_id.as_deref() == Some(bundle_id))
            .map(|process| process.pid)
            .collect()
    }

    /// Every call DVT took, in order.
    pub(crate) fn dvt_calls(&self) -> Vec<DvtCall> {
        self.dvt().calls.clone()
    }

    /// Makes DVT refuse every launch.
    pub(crate) fn refuse_launches(&self) {
        self.dvt().refuse_launches = true;
    }

    /// Makes DVT take calls and answer none.
    pub(crate) fn silence_dvt(&self) {
        self.dvt().silent = true;
    }

    /// Makes DVT close every connection at its next call.
    pub(crate) fn close_dvt(&self) {
        self.dvt().closed = true;
    }

    /// Locks or unlocks the device.
    pub(crate) fn set_locked(&self, locked: bool) {
        self.link().locked = locked;
    }

    /// How many times the lock was read.
    pub(crate) fn lock_reads(&self) -> u32 {
        self.link().lock_reads
    }

    /// Makes the installation proxy refuse every bundle with `reason`.
    pub(crate) fn fail_installs(&self, reason: &str) {
        self.link().install_failure = Some(reason.to_owned());
    }

    /// The bundles installed so far, by their paths on the host.
    pub(crate) fn installs(&self) -> Vec<PathBuf> {
        self.link().installs.clone()
    }

    /// What DVT answers `selector` on the channel `service` with, or `None`
    /// for no reply; an error closes the connection.
    fn answer(&self, service: &str, selector: &str, args: Vec<Arg>) -> Result<Option<Reply>, ()> {
        let mut dvt = self.dvt();
        dvt.calls.push(DvtCall {
            service: service.to_owned(),
            selector: selector.to_owned(),
            args: args.clone(),
        });
        if dvt.closed {
            return Err(());
        }
        if dvt.silent {
            return Ok(None);
        }
        let reply = match (service, selector) {
            (PROCESS_CONTROL, LAUNCH) => {
                let bundle = match args.get(1) {
                    Some(Arg::Archived(Value::String(bundle))) => bundle.clone(),
                    _ => String::new(),
                };
                let kill_existing = matches!(
                    args.get(4),
                    Some(Arg::Archived(Value::Dictionary(options)))
                        if options.get("KillExisting").and_then(Value::as_boolean) == Some(true)
                );
                let executable = dvt.apps.get(&bundle).cloned();
                match executable {
                    Some(executable) if !dvt.refuse_launches => {
                        let running = dvt
                            .processes
                            .iter()
                            .find(|process| process.bundle_id.as_deref() == Some(&bundle))
                            .map(|process| process.pid);
                        match running {
                            // A launch without KillExisting keeps a running
                            // instance and answers its pid, as DVT did on
                            // iOS 26.6.2.
                            Some(pid) if !kill_existing => Reply::Data(Some(pid_value(pid))),
                            _ => {
                                dvt.processes.retain(|process| {
                                    process.bundle_id.as_deref() != Some(&bundle)
                                });
                                let pid = dvt.next_pid;
                                dvt.next_pid += 1;
                                dvt.processes.push(FakeProcess {
                                    pid,
                                    name: executable.clone(),
                                    bundle_id: Some(bundle.clone()),
                                    path: format!(
                                        "/var/containers/Bundle/Application/PLACEHOLDER/{executable}.app/{executable}"
                                    ),
                                });
                                Reply::Data(Some(pid_value(pid)))
                            }
                        }
                    }
                    _ => Reply::Raw(ns_error_archive(
                        "FBSOpenApplicationServiceErrorDomain",
                        4,
                        &format!("The request to open \"{bundle}\" failed."),
                    )),
                }
            }
            (PROCESS_CONTROL, SEND_SIGNAL) => match args.get(1) {
                Some(Arg::Archived(Value::Integer(pid))) => {
                    let pid = pid.as_unsigned().unwrap_or_default();
                    dvt.processes
                        .retain(|process| u64::from(process.pid) != pid);
                    Reply::Data(None)
                }
                // A pid sent as a primitive stalled the phone's DVT service
                // and dropped its connections (S9).
                _ => return Err(()),
            },
            (DEVICE_INFO, RUNNING_PROCESSES) => Reply::Data(Some(Value::Array(
                dvt.processes.iter().map(process_value).collect(),
            ))),
            (DEVICE_INFO, IS_RUNNING_PID) => {
                let pid = match args.first() {
                    Some(Arg::Archived(Value::Integer(pid))) => pid.as_unsigned(),
                    _ => None,
                };
                Reply::Data(Some(Value::Boolean(
                    dvt.processes
                        .iter()
                        .any(|process| Some(u64::from(process.pid)) == pid),
                )))
            }
            _ => Reply::Data(None),
        };
        Ok(Some(reply))
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn rsd_port(&self) -> u16 {
        RSD_PORT
    }

    /// The port of the service named, when the device lists it.
    pub(crate) fn port(&self, name: &str) -> Option<u16> {
        self.services
            .iter()
            .find(|(service, _, _)| service == name)
            .map(|(_, port, _)| *port)
    }

    /// Serves every connection to the service named with `handler`.
    pub(crate) fn serve(&self, name: &str, handler: Handler) {
        let port = self.port(name).expect("a listed service");
        self.handlers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(port, handler);
    }

    /// The ports connected to so far, in order.
    pub(crate) fn connections(&self) -> Vec<u16> {
        self.state().connections.clone()
    }

    /// Tunnels opened so far.
    pub(crate) fn opens(&self) -> u32 {
        self.state().opens
    }

    /// Tunnels the driver closed so far.
    pub(crate) fn closed(&self) -> u32 {
        self.state().closed
    }

    /// Drops the serving tunnel, as jktcp's dies with its adapter; the
    /// device stays and a new tunnel serves.
    pub(crate) fn drop_tunnel(&self) {
        let mut state = self.state();
        self.next_generation(&mut state);
    }

    /// Takes the device away: its tunnel drops and no new one opens.
    pub(crate) fn unplug(&self) {
        let mut state = self.state();
        self.next_generation(&mut state);
        state.gone = true;
    }

    /// Ends the serving tunnel: new connections through it fail and the
    /// open ones close, as jktcp's do once its adapter is gone.
    fn next_generation(&self, state: &mut State) {
        state.generation += 1;
        self.generations.send_replace(state.generation);
    }

    /// Brings the device back.
    pub(crate) fn plug_in(&self) {
        self.state().gone = false;
    }

    /// Makes RSD accept connections and never answer them.
    pub(crate) fn silence_rsd(&self) {
        self.state().rsd_silent = true;
    }

    /// RSD's handshake as one HTTP/2 DATA frame on the root stream: an XPC
    /// message whose dictionary lists every service with its port.
    fn handshake_frame(&self) -> Vec<u8> {
        let mut services = Dictionary::new();
        for (name, port, entry) in &self.services {
            let mut properties = Dictionary::new();
            for key in ["UsesRemoteXPC", "Features", "ServiceVersion"] {
                if let Some(value) = entry.get(key) {
                    properties.insert(key.to_owned(), value.clone());
                }
            }
            let mut service = Dictionary::new();
            service.insert(
                "Entitlement".to_owned(),
                entry.get("Entitlement").cloned().expect("an entitlement"),
            );
            service.insert("Port".to_owned(), Value::String(port.to_string()));
            service.insert("Properties".to_owned(), Value::Dictionary(properties));
            services.insert(name.clone(), Value::Dictionary(service));
        }
        let mut root = Dictionary::new();
        root.insert("MessageType".into(), Value::String("Handshake".into()));
        root.insert("MessagingProtocolVersion".into(), Value::Integer(7.into()));
        root.insert("UUID".into(), Value::String("placeholder-rsd-uuid".into()));
        root.insert(
            "Properties".into(),
            Value::Dictionary(self.properties.clone()),
        );
        root.insert("Services".into(), Value::Dictionary(services));
        let body = XPCMessage::new(None, Some(XPCObject::from(Value::Dictionary(root))), None)
            .encode(0)
            .expect("the handshake encodes");
        let length = u32::try_from(body.len()).expect("a small handshake");
        let mut frame = length.to_be_bytes()[1..].to_vec();
        frame.extend_from_slice(&[0x00, 0x00]);
        frame.extend_from_slice(&1u32.to_be_bytes());
        frame.extend_from_slice(&body);
        frame
    }
}

/// DVT's first message on a new connection: the services it publishes, on
/// the control channel, which `idevice`'s client keeps for
/// `read_message(0)`.
pub(crate) fn capabilities() -> Vec<u8> {
    let mut services = Dictionary::new();
    for service in [
        "com.apple.instruments.server.services.processcontrol",
        "com.apple.instruments.server.services.deviceinfo",
        "com.apple.instruments.server.services.screenshot",
    ] {
        services.insert(service.to_owned(), Value::Integer(1.into()));
    }
    Message::new(
        MessageHeader::new(0, 1, 1, 0, 0, false),
        PayloadHeader::method_invocation(),
        Some(Aux::from_values(vec![AuxValue::archived_value(
            Value::Dictionary(services),
        )])),
        Some(Value::String("_notifyOfPublishedCapabilities:".into())),
    )
    .serialize()
}

enum Reply {
    /// Data to archive into the reply, or none for an empty one.
    Data(Option<Value>),
    /// An archive written as it is, such as an `NSError`.
    Raw(Vec<u8>),
}

fn pid_value(pid: u32) -> Value {
    Value::Integer(i64::from(pid).into())
}

fn process_value(process: &FakeProcess) -> Value {
    let mut entry = Dictionary::new();
    entry.insert("pid".into(), pid_value(process.pid));
    entry.insert("name".into(), Value::String(process.name.clone()));
    entry.insert("realAppName".into(), Value::String(process.path.clone()));
    entry.insert(
        "isApplication".into(),
        Value::Boolean(process.bundle_id.is_some()),
    );
    if let Some(bundle_id) = &process.bundle_id {
        entry.insert("bundleIdentifier".into(), Value::String(bundle_id.clone()));
    }
    Value::Dictionary(entry)
}

/// An `NSError` as `NSKeyedArchiver` archives it, with its domain, code and
/// a localized description in its user info.
pub(crate) fn ns_error_archive(domain: &str, code: i64, description: &str) -> Vec<u8> {
    let uid = |index: u64| Value::Uid(plist::Uid::new(index));
    let class = |name: &str| {
        let mut class = Dictionary::new();
        class.insert("$classname".into(), Value::String(name.into()));
        class.insert(
            "$classes".into(),
            Value::Array(vec![
                Value::String(name.into()),
                Value::String("NSObject".into()),
            ]),
        );
        Value::Dictionary(class)
    };
    let mut error = Dictionary::new();
    error.insert("$class".into(), uid(7));
    error.insert("NSCode".into(), Value::Integer(code.into()));
    error.insert("NSDomain".into(), uid(2));
    error.insert("NSUserInfo".into(), uid(3));
    let mut info = Dictionary::new();
    info.insert("$class".into(), uid(6));
    info.insert("NS.keys".into(), Value::Array(vec![uid(4)]));
    info.insert("NS.objects".into(), Value::Array(vec![uid(5)]));
    let mut top = Dictionary::new();
    top.insert("root".into(), uid(1));
    let mut archive = Dictionary::new();
    archive.insert("$archiver".into(), Value::String("NSKeyedArchiver".into()));
    archive.insert("$version".into(), Value::Integer(100000.into()));
    archive.insert("$top".into(), Value::Dictionary(top));
    archive.insert(
        "$objects".into(),
        Value::Array(vec![
            Value::String("$null".into()),
            Value::Dictionary(error),
            Value::String(domain.into()),
            Value::Dictionary(info),
            Value::String("NSLocalizedDescription".into()),
            Value::String(description.into()),
            class("NSDictionary"),
            class("NSError"),
        ]),
    );
    let mut bytes = Vec::new();
    plist::to_writer_binary(&mut bytes, &Value::Dictionary(archive)).expect("archives");
    bytes
}

/// A DTX reply to the call `identifier` on `channel`, with its archived
/// data, empty for an acknowledgement.
fn reply_bytes(identifier: u32, channel: i32, archived: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(48 + archived.len());
    out.extend_from_slice(&0x1F3D_5B79u32.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(16 + archived.len() as u32).to_le_bytes());
    out.extend_from_slice(&identifier.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&channel.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(if archived.is_empty() { 0 } else { 3 });
    out.extend_from_slice(&[0, 0, 0]);
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(archived.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(archived);
    out
}

fn arg(value: &AuxValue) -> Arg {
    match value {
        AuxValue::Array(bytes) => ns_keyed_archive::decode::from_bytes(bytes)
            .map(Arg::Archived)
            .unwrap_or(Arg::Other),
        AuxValue::U32(value) => Arg::Primitive(i64::from(*value)),
        AuxValue::I64(value) => Arg::Primitive(*value),
        AuxValue::String(text) => Arg::Text(text.clone()),
        _ => Arg::Other,
    }
}

/// Serves one DVT connection: its capabilities first, then channels and
/// the calls on them, until the driver closes it or the device does.
pub(crate) async fn serve_dvt(device: Arc<FakeDevice>, stream: DuplexStream) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    if writer.write_all(&capabilities()).await.is_err() {
        return;
    }
    let mut channels: HashMap<i32, String> = HashMap::new();
    loop {
        let Ok(message) = Message::from_reader(&mut reader).await else {
            return;
        };
        let header = message.message_header.serialize();
        let identifier = u32::from_le_bytes(header[16..20].try_into().expect("4 bytes"));
        // A call comes on its channel's code, which the reader negates.
        let code = -message.message_header.channel;
        let selector = message
            .data
            .as_ref()
            .and_then(Value::as_string)
            .unwrap_or_default()
            .to_owned();
        let args: Vec<Arg> = message
            .aux
            .as_ref()
            .map(|aux| aux.values.iter().map(arg).collect())
            .unwrap_or_default();
        let reply = if code == 0 && selector == "_requestChannelWithCode:identifier:" {
            if let (Some(Arg::Primitive(channel)), Some(Arg::Archived(Value::String(name)))) =
                (args.first(), args.get(1))
            {
                channels.insert(*channel as i32, name.clone());
            }
            Some(Reply::Data(None))
        } else {
            let service = channels.get(&code).cloned().unwrap_or_default();
            match device.answer(&service, &selector, args) {
                Ok(reply) => reply,
                Err(()) => return,
            }
        };
        let archived = match reply {
            None => continue,
            Some(Reply::Data(None)) => Vec::new(),
            Some(Reply::Data(Some(data))) => {
                ns_keyed_archive::encode::encode_to_bytes(data).expect("the reply archives")
            }
            Some(Reply::Raw(raw)) => raw,
        };
        if writer
            .write_all(&reply_bytes(identifier, code, &archived))
            .await
            .is_err()
        {
            return;
        }
    }
}

/// Reads and drops what the other end writes until it closes.
pub(crate) async fn drain(mut stream: DuplexStream) {
    let mut buffer = [0u8; 4096];
    while matches!(stream.read(&mut buffer).await, Ok(read) if read > 0) {}
}

/// Opens fake tunnels to a [`FakeDevice`].
pub(crate) struct FakeOpener {
    device: Arc<FakeDevice>,
}

impl FakeOpener {
    pub(crate) fn new(device: &Arc<FakeDevice>) -> FakeOpener {
        FakeOpener {
            device: Arc::clone(device),
        }
    }
}

impl Opener for FakeOpener {
    type Provider = FakeProvider;

    async fn open(&self) -> std::result::Result<(FakeProvider, u16), String> {
        let mut state = self.device.state();
        if state.gone {
            return Err("CoreDeviceProxy: the device is not in usbmuxd's listing".into());
        }
        state.opens += 1;
        Ok((
            FakeProvider {
                device: Arc::clone(&self.device),
                generation: state.generation,
            },
            RSD_PORT,
        ))
    }

    async fn close(&self, provider: &mut FakeProvider) {
        let mut state = self.device.state();
        state.closed += 1;
        if provider.generation == state.generation {
            self.device.next_generation(&mut state);
        }
    }
}

/// Connections to the fake device's ports through one fake tunnel.
#[derive(Clone)]
pub(crate) struct FakeProvider {
    device: Arc<FakeDevice>,
    generation: u32,
}

impl fmt::Debug for FakeProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FakeProvider")
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl RsdProvider for FakeProvider {
    async fn connect_to_service_port(
        &mut self,
        port: u16,
    ) -> std::result::Result<Box<dyn ReadWrite>, IdeviceError> {
        let silent = {
            let mut state = self.device.state();
            if state.gone || state.generation != self.generation {
                return Err(IdeviceError::Socket(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "adapter closed",
                )));
            }
            state.connections.push(port);
            state.rsd_silent
        };
        let (client, mut server) = tokio::io::duplex(1 << 20);
        let server: Pin<Box<dyn Future<Output = ()> + Send>> = if port == RSD_PORT {
            let frame = (!silent).then(|| self.device.handshake_frame());
            Box::pin(async move {
                if let Some(frame) = frame {
                    let _ = server.write_all(&frame).await;
                }
                drain(server).await;
            })
        } else {
            let handler = self
                .device
                .handlers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&port)
                .cloned();
            match handler {
                Some(handler) => handler(server),
                None if Some(port) == self.device.port(services::DVT) => {
                    Box::pin(serve_dvt(Arc::clone(&self.device), server))
                }
                None => Box::pin(drain(server)),
            }
        };
        // The connection lives as long as its tunnel does.
        let mut generations = self.device.generations.subscribe();
        let generation = self.generation;
        tokio::spawn(async move {
            tokio::select! {
                () = server => {}
                () = async {
                    while *generations.borrow_and_update() == generation {
                        if generations.changed().await.is_err() {
                            std::future::pending::<()>().await;
                        }
                    }
                } => {}
            }
        });
        Ok(Box::new(client))
    }
}

/// A mounter whose ways of mounting do what the test says, and whose image
/// mounter lists the image once one of them mounted it. Clones share the
/// image's state, so a fake host can list what a fake mount did.
#[derive(Clone)]
pub(crate) struct FakeMounter {
    mounted: Arc<Mutex<bool>>,
    pub(crate) idevice: std::result::Result<(), String>,
    pub(crate) idevice_hangs: bool,
    /// Whether idevice's success mounts the image; a hollow one leaves the
    /// image mounter listing none.
    pub(crate) idevice_mounts: bool,
    pub(crate) devicectl: std::result::Result<(), String>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

impl FakeMounter {
    pub(crate) fn new(
        mounted: bool,
        idevice: std::result::Result<(), String>,
        devicectl: std::result::Result<(), String>,
    ) -> FakeMounter {
        FakeMounter {
            mounted: Arc::new(Mutex::new(mounted)),
            idevice,
            idevice_hangs: false,
            idevice_mounts: true,
            devicectl,
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// What was asked, in order: images, idevice, devicectl.
    pub(crate) fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// The image mounter's listing as it is now.
    pub(crate) fn listing(&self) -> Vec<Value> {
        if *self
            .mounted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
        {
            recorded_images()
        } else {
            Vec::new()
        }
    }

    fn call(&self, what: &'static str) {
        self.calls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(what);
    }

    fn mount(&self) {
        *self
            .mounted
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    }
}

impl Mounter for FakeMounter {
    async fn images(&self) -> std::result::Result<Vec<Value>, String> {
        self.call("images");
        Ok(self.listing())
    }

    async fn mount_with_idevice(&self) -> std::result::Result<(), String> {
        self.call("idevice");
        if self.idevice_hangs {
            std::future::pending::<()>().await;
        }
        if self.idevice.is_ok() && self.idevice_mounts {
            self.mount();
        }
        self.idevice.clone()
    }

    async fn mount_with_devicectl(&self) -> std::result::Result<(), String> {
        self.call("devicectl");
        if self.devicectl.is_ok() {
            self.mount();
        }
        self.devicectl.clone()
    }
}

/// The fake device's usbmuxd side: lockdown's lock and the installation
/// proxy.
pub(crate) struct FakeLink {
    device: Arc<FakeDevice>,
}

impl FakeLink {
    pub(crate) fn new(device: &Arc<FakeDevice>) -> FakeLink {
        FakeLink {
            device: Arc::clone(device),
        }
    }
}

impl Link for FakeLink {
    async fn password_protected(&self) -> std::result::Result<bool, String> {
        let mut link = self.device.link();
        link.lock_reads += 1;
        Ok(link.locked)
    }

    async fn install(&self, app: &Path) -> std::result::Result<(), String> {
        let failure = self.device.link().install_failure.clone();
        if let Some(reason) = failure {
            return Err(reason);
        }
        let bundle = crate::bundle::inspect(app)?;
        self.device
            .install_app(&bundle.bundle_id, &bundle.executable);
        self.device.link().installs.push(app.to_owned());
        Ok(())
    }

    async fn installed(&self, bundle_id: &str) -> std::result::Result<bool, String> {
        Ok(self.device.dvt().apps.contains_key(bundle_id))
    }
}
