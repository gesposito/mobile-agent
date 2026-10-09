//! The fake device's RemoteXPC services, reached by `idevice`'s own clients
//! over the fake tunnel: HTTP/2 frames carrying XPC messages, as the phone's
//! `remoted` serves them. The client's handshake needs no answer; requests
//! that want one are answered on the reply stream.
//!
//! `dtuhidd`'s two services record what they take with when it arrived:
//! the universal HID service decodes `mainTouchscreen` reports and the
//! virtual keyboard's reports, registers and removes that keyboard, and
//! lists the surfaces the phone listed in S11 beside the keyboards
//! registered; the Indigo service decodes button and keyboard events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use idevice::xpc::{XPCMessage, XPCObject};
use plist::{Dictionary, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

use super::fake::FakeDevice;

/// The service a fake RemoteXPC connection reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum XpcService {
    HidUniversal,
    HidIndigo,
}

/// What `dtuhidd` took.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HidEvent {
    /// A `mainTouchscreen` report: its first contact's state byte (0xC2
    /// touching, 0x02 lifted) and coordinates.
    Touch {
        state: u8,
        x: u16,
        y: u16,
    },
    /// A report of a registered virtual keyboard: the usages it holds down.
    KeyboardReport {
        pressed: Vec<u16>,
    },
    KeyboardCreated(u64),
    KeyboardRemoved(u64),
    /// An Indigo button event.
    Button {
        page: u64,
        usage: u64,
        down: bool,
    },
    /// An Indigo keyboard event.
    IndigoKey {
        usage: u64,
        down: bool,
    },
    /// A report to any other surface.
    Report {
        service: u64,
        bytes: Vec<u8>,
    },
}

/// The main touchscreen's surface identifier.
const TOUCHSCREEN: u64 = 257;

/// HTTP/2's frame types this endpoint writes or reads.
const DATA: u8 = 0x0;
const SETTINGS: u8 = 0x4;
const WINDOW_UPDATE: u8 = 0x8;
/// The stream `idevice`'s RemoteXPC client reads replies on.
const REPLY_STREAM: u32 = 3;
/// The largest flow-control window HTTP/2 allows, granted up front, as a
/// peer that keeps reading would replenish it.
const WINDOW: u32 = 0x7FFF_FFFF;

fn frame(kind: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).expect("a small frame");
    let mut out = length.to_be_bytes()[1..].to_vec();
    out.push(kind);
    out.push(0);
    out.extend_from_slice(&(stream & 0x7FFF_FFFF).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// The frames that grant the client the widest windows: the initial window
/// of every stream, and the connection's.
fn windows() -> Vec<u8> {
    let mut setting = 0x4u16.to_be_bytes().to_vec();
    setting.extend_from_slice(&WINDOW.to_be_bytes());
    let mut out = frame(SETTINGS, 0, &setting);
    out.extend(frame(WINDOW_UPDATE, 0, &(WINDOW - 65_535).to_be_bytes()));
    out
}

/// Serves one connection to `service` until the client closes it.
pub(crate) async fn serve_xpc(device: Arc<FakeDevice>, stream: DuplexStream, service: XpcService) {
    let (mut reader, mut writer) = tokio::io::split(stream);
    if writer.write_all(&windows()).await.is_err() {
        return;
    }
    let mut magic = [0u8; 24];
    if reader.read_exact(&mut magic).await.is_err() {
        return;
    }
    let mut streams: HashMap<u32, Vec<u8>> = HashMap::new();
    loop {
        let mut header = [0u8; 9];
        if reader.read_exact(&mut header).await.is_err() {
            return;
        }
        let length = u32::from_be_bytes([0, header[0], header[1], header[2]]) as usize;
        let stream = u32::from_be_bytes([header[5], header[6], header[7], header[8]]) & 0x7FFF_FFFF;
        let mut payload = vec![0u8; length];
        if reader.read_exact(&mut payload).await.is_err() {
            return;
        }
        if header[3] != DATA {
            continue;
        }
        let buffer = streams.entry(stream).or_default();
        buffer.extend_from_slice(&payload);
        while buffer.len() >= 24 {
            let body = u64::from_le_bytes(buffer[8..16].try_into().expect("8 bytes")) as usize;
            if buffer.len() < 24 + body {
                break;
            }
            let message: Vec<u8> = buffer.drain(..24 + body).collect();
            let Some(request) = XPCMessage::decode(&message)
                .ok()
                .and_then(|message| message.message)
                .map(|object| object.to_plist())
            else {
                continue;
            };
            let reply = match service {
                XpcService::HidUniversal => universal(&device, &request),
                XpcService::HidIndigo => {
                    indigo(&device, &request);
                    None
                }
            };
            if let Some(reply) = reply {
                let bytes =
                    XPCMessage::new(None, Some(XPCObject::from(Value::Dictionary(reply))), None)
                        .encode(1)
                        .expect("the reply encodes");
                if writer
                    .write_all(&frame(DATA, REPLY_STREAM, &bytes))
                    .await
                    .is_err()
                {
                    return;
                }
            }
        }
    }
}

fn payload(request: &Value) -> Option<&Dictionary> {
    request.as_dictionary()?.get("payload")?.as_dictionary()
}

/// A request to the universal HID service, and its answer when it wants
/// one.
fn universal(device: &FakeDevice, request: &Value) -> Option<Dictionary> {
    let payload = payload(request)?;
    if let Some(send) = payload.get("send").and_then(Value::as_dictionary) {
        let report = send.get("_0").and_then(Value::as_data)?;
        let service = send.get("_1").and_then(Value::as_unsigned_integer)?;
        let event = if service == TOUCHSCREEN && report.len() == 58 && report[0] == 0x09 {
            HidEvent::Touch {
                state: report[3],
                x: u16::from_le_bytes([report[4], report[5]]),
                y: u16::from_le_bytes([report[6], report[7]]),
            }
        } else if device.keyboard(service).is_some() && report.len() == 39 && report[0] == 0x01 {
            let pressed = report[1..30]
                .iter()
                .enumerate()
                .flat_map(|(byte, bits)| {
                    (0..8u16)
                        .filter(move |bit| bits & (1 << bit) != 0)
                        .map(move |bit| byte as u16 * 8 + bit)
                })
                .collect();
            HidEvent::KeyboardReport { pressed }
        } else {
            HidEvent::Report {
                service,
                bytes: report.to_vec(),
            }
        };
        device.record_hid(event);
        return None;
    }
    if payload.contains_key("connectedServices") {
        let mut reply = Dictionary::new();
        reply.insert(
            "connectedServices".into(),
            Value::Array(device.hid_surfaces()),
        );
        return Some(reply);
    }
    if let Some(create) = payload
        .get("createService")
        .and_then(Value::as_dictionary)
        .and_then(|create| create.get("_0"))
        .and_then(Value::as_dictionary)
    {
        let id = create
            .get("_ServiceID")
            .and_then(Value::as_unsigned_integer)?;
        device.register_keyboard(id, create.clone());
        let mut reply = Dictionary::new();
        reply.insert("serviceID".into(), Value::Integer(id.into()));
        return Some(reply);
    }
    if let Some(id) = payload
        .get("removeService")
        .and_then(Value::as_dictionary)
        .and_then(|remove| remove.get("_0"))
        .and_then(Value::as_unsigned_integer)
    {
        device.remove_keyboard(id);
    }
    None
}

/// An Indigo event.
fn indigo(device: &FakeDevice, request: &Value) {
    let Some(kind) = request
        .as_dictionary()
        .and_then(|request| request.get("messageType"))
        .and_then(Value::as_string)
    else {
        return;
    };
    let Some(payload) = payload(request) else {
        return;
    };
    let number = |key: &str| payload.get(key).and_then(Value::as_unsigned_integer);
    let down = number("state") == Some(1);
    let event = match kind {
        "IndigoButtonEvent" => HidEvent::Button {
            page: number("usagePage").unwrap_or_default(),
            usage: number("usageCode").unwrap_or_default(),
            down,
        },
        "IndigoKeyboardButtonEvent" => HidEvent::IndigoKey {
            usage: number("usageCode").unwrap_or_default(),
            down,
        },
        _ => return,
    };
    device.record_hid(event);
}

/// A surface as the universal HID service lists it.
pub(crate) fn surface(id: u64, product: &str, page: u64, usage: u64) -> Value {
    let mut surface = Dictionary::new();
    surface.insert("_ServiceID".into(), Value::Integer(id.into()));
    surface.insert("Product".into(), Value::String(product.into()));
    surface.insert("PrimaryUsagePage".into(), Value::Integer(page.into()));
    surface.insert("PrimaryUsage".into(), Value::Integer(usage.into()));
    Value::Dictionary(surface)
}

/// The surfaces the phone listed in S11.
pub(crate) fn recorded_surfaces() -> Vec<Value> {
    vec![
        surface(TOUCHSCREEN, "CoreDevice touchscreen(nil)", 13, 4),
        surface(512, "CoreDevice keyboard", 0, 0),
        surface(1026, "CoreDevice mainScreenButtons", 11, 1),
        surface(1280, "CoreDevice avpCustom", 65377, 91),
    ]
}

/// A registered virtual keyboard as the service lists it: its usages and
/// the codable storage it was created with, which carries its identity.
pub(crate) fn keyboard_surface(id: u64, created: &Dictionary) -> Value {
    let mut surface = Dictionary::new();
    surface.insert("_ServiceID".into(), Value::Integer(id.into()));
    let product = created
        .get("Product")
        .and_then(Value::as_string)
        .unwrap_or("placeholder keyboard");
    surface.insert("Product".into(), Value::String(product.into()));
    surface.insert("PrimaryUsagePage".into(), Value::Integer(1.into()));
    surface.insert("PrimaryUsage".into(), Value::Integer(6.into()));
    if let Some(storage) = created.get("_CoreDevice_codablePropertyStorage") {
        surface.insert("_CoreDevice_codablePropertyStorage".into(), storage.clone());
    }
    Value::Dictionary(surface)
}

/// One event `dtuhidd` took and when.
#[derive(Clone, Debug)]
pub(crate) struct TimedHid {
    pub(crate) at: Instant,
    pub(crate) event: HidEvent,
}
