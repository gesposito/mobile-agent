//! A fake device behind a fake tunnel, for the tests: an RSD endpoint that
//! answers the handshake with the services spike S9 recorded on the iPhone SE
//! (3rd generation) with iOS 26.6.2, each on a port of its own, and service
//! ports that accept connections, served by handlers the tests register.
//! The tunnel can drop, as jktcp's does when its adapter dies, and the
//! device can go away and come back.

use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use idevice::dvt::message::{Aux, AuxValue, Message, MessageHeader, PayloadHeader};
use idevice::provider::RsdProvider;
use idevice::xpc::{XPCMessage, XPCObject};
use idevice::{IdeviceError, ReadWrite};
use plist::{Dictionary, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::image::Mounter;
use super::tunnel::Opener;
use crate::discovery::tests::recorded_images;

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

/// The device behind the fake tunnel.
pub(crate) struct FakeDevice {
    /// Each service's name, port and recorded entry, in the recording's
    /// order.
    services: Vec<(String, u16, Dictionary)>,
    properties: Dictionary,
    state: Mutex<State>,
    handlers: Mutex<HashMap<u16, Handler>>,
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
            handlers: Mutex::new(HashMap::new()),
        })
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
        self.state().generation += 1;
    }

    /// Takes the device away: its tunnel drops and no new one opens.
    pub(crate) fn unplug(&self) {
        let mut state = self.state();
        state.generation += 1;
        state.gone = true;
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

/// A DVT service hub that says its capabilities and takes nothing more.
pub(crate) async fn dvt_hello(mut stream: DuplexStream) {
    if stream.write_all(&capabilities()).await.is_ok() {
        drain(stream).await;
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
            state.generation += 1;
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
        if port == RSD_PORT {
            let frame = (!silent).then(|| self.device.handshake_frame());
            tokio::spawn(async move {
                if let Some(frame) = frame {
                    let _ = server.write_all(&frame).await;
                }
                drain(server).await;
            });
        } else {
            let handler = self
                .device
                .handlers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(&port)
                .cloned();
            match handler {
                Some(handler) => {
                    tokio::spawn(handler(server));
                }
                None if Some(port) == self.device.port(crate::services::DVT) => {
                    tokio::spawn(dvt_hello(server));
                }
                None => {
                    tokio::spawn(drain(server));
                }
            }
        }
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
