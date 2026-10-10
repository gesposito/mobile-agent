//! The host side of discovery, through `idevice` (design D18): usbmuxd's
//! listing, and the lockdown probe of one device with the host's pairing
//! record, which usbmuxd hands over (`ReadPairRecord`), so nothing reads
//! `/var/db/lockdown`.

use std::future::Future;
use std::path::Path;
use std::time::Duration;

use idevice::installation_proxy::InstallationProxyClient;
use idevice::lockdown::LockdownClient;
use idevice::mobile_image_mounter::ImageMounter;
use idevice::provider::{IdeviceProvider, UsbmuxdProvider};
use idevice::usbmuxd::{Connection as MuxConnection, UsbmuxdAddr, UsbmuxdConnection};
use idevice::{IdeviceError, IdeviceService, ReadWrite};
use plist::{Dictionary, Value};
use tokio::time::timeout;

use crate::discovery::{
    self, BATTERY_DOMAIN, DESCRIPTION_KEYS, DEVELOPER_MODE_DOMAIN, DEVELOPER_MODE_KEY, LOCK_KEY,
    SCREEN_DOMAIN,
};
use crate::error::{Error, Result};
use crate::identity::IdentityCheck;
use crate::model::{Connection, DeviceInfo, MuxEntry, Probe, Screen, Trust};

use super::image::{DDI_DIRECTORY, Mounter, UsbMounter};
use super::runner::{TestManager, UsbTestManager};
use super::tunnel::{Opener, UsbTunnel};

/// How long usbmuxd gets to answer: 0.4 ms at p50 in spike S9.
const USBMUXD_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a device's probe gets: lockdown's session and values, and the
/// image mounter's listing, about 50 ms together on the spike's phone.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// The label lockdown and usbmuxd see the driver's connections under.
pub(crate) const LABEL: &str = "mobile-agent";

/// How long a lockdown session or an installation proxy lookup gets.
const LINK_TIMEOUT: Duration = Duration::from_secs(10);

/// How long an install gets: S10 installed its 3.7 MB runner in 1.1 s and
/// the Expo fixture in about 3 s.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(120);

/// An error and its sources on one line, since `idevice`'s errors say
/// little on their own; a cause that repeats the one before is left out.
pub(crate) fn describe(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut last = text.clone();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if message != last {
            text.push_str(": ");
            text.push_str(&message);
        }
        last = message;
        source = cause.source();
    }
    text
}

/// What the driver asks of one device over usbmuxd rather than the tunnel:
/// lockdown's lock, the installation proxy, whose uploads go faster through
/// usbmuxd than through jktcp's one segment at a time, and the runner's
/// socket, reached through usbmuxd's `Connect` for the same reason (D18).
/// Its futures are `Send`, so the runner's supervisor can await them on a
/// task of its own.
pub(crate) trait Link: Send + Sync + 'static {
    /// Lockdown's `PasswordProtected`, which follows the lock (S9).
    fn password_protected(&self) -> impl Future<Output = std::result::Result<bool, String>> + Send;

    /// Uploads the bundle at `app` to the device's `PublicStaging` through
    /// AFC and installs it through the installation proxy.
    fn install(&self, app: &Path) -> impl Future<Output = std::result::Result<(), String>> + Send;

    /// Whether the installation proxy lists `bundle_id`.
    fn installed(
        &self,
        bundle_id: &str,
    ) -> impl Future<Output = std::result::Result<bool, String>> + Send;

    /// A connection to the device's loopback `port` through usbmuxd's
    /// `Connect`; refused while nothing listens there.
    fn connect(
        &self,
        port: u16,
    ) -> impl Future<Output = std::result::Result<Box<dyn ReadWrite>, String>> + Send;

    /// The screen's pixels and scale, from lockdown's
    /// `com.apple.mobile.iTunes` domain.
    fn screen(&self) -> impl Future<Output = std::result::Result<Screen, String>> + Send;
}

/// What the driver needs from the host: the seam the tests replace.
pub(crate) trait Host: Send + Sync + 'static {
    /// Opens tunnels to one device.
    type Opener: Opener;
    /// Mounts the developer disk image on one device.
    type Mounter: Mounter;
    /// Lockdown and the installation proxy of one device.
    type Link: Link;
    /// Starts runners through the test manager of one device.
    type TestManager: TestManager;

    /// usbmuxd's listing, every entry as it comes.
    fn list(&self) -> impl Future<Output = Result<Vec<MuxEntry>>> + Send;

    /// What lockdown and the image mounter say about the device of `entry`:
    /// the values read, under their keys, and the probe.
    fn probe(&self, entry: &MuxEntry) -> impl Future<Output = (Dictionary, Probe)> + Send;

    /// The host's signing identity for the runner.
    fn identity(&self) -> IdentityCheck;

    /// The tunnels to the device of `entry`.
    fn opener(&self, entry: &MuxEntry) -> Self::Opener;

    /// The disk image mounts on `device`, the device of `entry`.
    fn mounter(&self, entry: &MuxEntry, device: &DeviceInfo) -> Self::Mounter;

    /// Lockdown and the installation proxy of the device of `entry`.
    fn link(&self, entry: &MuxEntry) -> Self::Link;

    /// The test manager of the device of `entry`.
    fn test_manager(&self, entry: &MuxEntry) -> Self::TestManager;
}

/// The real host: usbmuxd at its default socket, or the one
/// `USBMUXD_SOCKET_ADDRESS` names, and lockdown through it.
#[derive(Debug)]
pub(crate) struct UsbHost {
    addr: UsbmuxdAddr,
}

impl UsbHost {
    pub(crate) fn new() -> Result<UsbHost> {
        let addr = UsbmuxdAddr::from_env_var().map_err(|error| Error::Usbmuxd {
            reason: format!("USBMUXD_SOCKET_ADDRESS is not an address: {error}"),
        })?;
        Ok(UsbHost { addr })
    }

    /// The provider that reaches the device of `entry` through usbmuxd.
    pub(crate) fn provider(&self, entry: &MuxEntry) -> UsbmuxdProvider {
        UsbmuxdProvider {
            addr: self.addr.clone(),
            tag: entry.device_id,
            udid: entry.udid.clone(),
            device_id: entry.device_id,
            label: LABEL.to_owned(),
        }
    }
}

impl Host for UsbHost {
    type Opener = UsbTunnel;
    type Mounter = UsbMounter;
    type Link = UsbLink;
    type TestManager = UsbTestManager;

    async fn list(&self) -> Result<Vec<MuxEntry>> {
        let unavailable = |reason: String| Error::Usbmuxd { reason };
        let listing = async {
            let mux = self
                .addr
                .connect(0)
                .await
                .map_err(|error| unavailable(describe(&error)))?;
            list_devices(mux).await
        };
        timeout(USBMUXD_TIMEOUT, listing)
            .await
            .map_err(|_| unavailable(format!("no answer within {USBMUXD_TIMEOUT:?}")))?
    }

    async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe) {
        let provider = self.provider(entry);
        match timeout(PROBE_TIMEOUT, probe_device(&provider)).await {
            Ok(read) => read,
            Err(_) => (
                Dictionary::new(),
                Probe::untrusted(Trust::Unreachable(format!(
                    "lockdown did not answer within {PROBE_TIMEOUT:?}"
                ))),
            ),
        }
    }

    fn identity(&self) -> IdentityCheck {
        crate::identity::host_identity()
    }

    fn opener(&self, entry: &MuxEntry) -> UsbTunnel {
        UsbTunnel {
            provider: self.provider(entry),
        }
    }

    fn mounter(&self, entry: &MuxEntry, device: &DeviceInfo) -> UsbMounter {
        UsbMounter {
            provider: self.provider(entry),
            udid: entry.udid.clone(),
            product_type: device.model.clone(),
            directory: DDI_DIRECTORY.into(),
        }
    }

    fn link(&self, entry: &MuxEntry) -> UsbLink {
        UsbLink {
            provider: self.provider(entry),
            lockdown: tokio::sync::Mutex::new(None),
        }
    }

    fn test_manager(&self, entry: &MuxEntry) -> UsbTestManager {
        UsbTestManager {
            provider: self.provider(entry),
        }
    }
}

/// The real link: a lockdown session kept for the lock's reads, and the
/// installation proxy through usbmuxd.
pub(crate) struct UsbLink {
    provider: UsbmuxdProvider,
    /// The session the lock is read on: one `GetValue`, 0.84 ms at p50 in
    /// S9, where a new session costs 22 ms.
    lockdown: tokio::sync::Mutex<Option<LockdownClient>>,
}

impl Link for UsbLink {
    async fn password_protected(&self) -> std::result::Result<bool, String> {
        let mut session = self.lockdown.lock().await;
        let mut retried = false;
        loop {
            if session.is_none() {
                *session = Some(lockdown_session(&self.provider).await?);
            }
            let lockdown = session.as_mut().expect("a session");
            let failure =
                match timeout(LINK_TIMEOUT, lockdown.get_value(Some(LOCK_KEY), None)).await {
                    Ok(Ok(value)) => {
                        return value
                            .as_boolean()
                            .ok_or_else(|| format!("PasswordProtected is {value:?}"));
                    }
                    Ok(Err(error)) => describe(&error),
                    Err(_) => format!("no answer within {LINK_TIMEOUT:?}"),
                };
            // The device closes a session it held for a while; a new one
            // answers.
            *session = None;
            if retried {
                return Err(failure);
            }
            retried = true;
        }
    }

    async fn install(&self, app: &Path) -> std::result::Result<(), String> {
        timeout(
            INSTALL_TIMEOUT,
            idevice::utils::installation::install_package(&self.provider, app, None),
        )
        .await
        .map_err(|_| format!("the install did not finish within {INSTALL_TIMEOUT:?}"))?
        .map_err(|error| describe(&error))
    }

    async fn installed(&self, bundle_id: &str) -> std::result::Result<bool, String> {
        let lookup = async {
            let mut proxy = InstallationProxyClient::connect(&self.provider).await?;
            proxy.get_apps(None, Some(vec![bundle_id.to_owned()])).await
        };
        let apps = timeout(LINK_TIMEOUT, lookup)
            .await
            .map_err(|_| format!("the installation proxy did not answer within {LINK_TIMEOUT:?}"))?
            .map_err(|error| describe(&error))?;
        Ok(apps.contains_key(bundle_id))
    }

    async fn screen(&self) -> std::result::Result<Screen, String> {
        let mut session = self.lockdown.lock().await;
        if session.is_none() {
            *session = Some(lockdown_session(&self.provider).await?);
        }
        let lockdown = session.as_mut().expect("a session");
        let values = timeout(LINK_TIMEOUT, lockdown.get_value(None, Some(SCREEN_DOMAIN)))
            .await
            .map_err(|_| format!("lockdown did not answer within {LINK_TIMEOUT:?}"))?;
        let values = match values {
            Ok(values) => values,
            Err(error) => {
                // A session the device closed answers no more; the next read
                // starts another.
                *session = None;
                return Err(describe(&error));
            }
        };
        discovery::screen(&values).ok_or_else(|| {
            format!("{SCREEN_DOMAIN} gives no ScreenWidth, ScreenHeight and ScreenScaleFactor")
        })
    }

    async fn connect(&self, port: u16) -> std::result::Result<Box<dyn ReadWrite>, String> {
        let device = timeout(LINK_TIMEOUT, self.provider.connect(port))
            .await
            .map_err(|_| format!("usbmuxd did not connect port {port} within {LINK_TIMEOUT:?}"))?
            .map_err(|error| format!("usbmuxd's Connect to port {port}: {}", describe(&error)))?;
        device
            .get_socket()
            .ok_or_else(|| format!("usbmuxd gave no socket for port {port}"))
    }
}

/// A lockdown session with the pairing record usbmuxd holds.
async fn lockdown_session(
    provider: &UsbmuxdProvider,
) -> std::result::Result<LockdownClient, String> {
    let session = async {
        let mut lockdown = LockdownClient::connect(provider).await?;
        let pairing = provider.get_pairing_file().await?;
        lockdown.start_session(&pairing).await?;
        Ok::<_, IdeviceError>(lockdown)
    };
    timeout(LINK_TIMEOUT, session)
        .await
        .map_err(|_| format!("lockdown did not start a session within {LINK_TIMEOUT:?}"))?
        .map_err(|error| describe(&error))
}

/// Every entry of usbmuxd's listing over `mux`, with its connection.
pub(crate) async fn list_devices(mut mux: UsbmuxdConnection) -> Result<Vec<MuxEntry>> {
    let devices = mux.get_devices().await.map_err(|error| Error::Usbmuxd {
        reason: describe(&error),
    })?;
    Ok(devices
        .into_iter()
        .map(|device| MuxEntry {
            udid: device.udid,
            device_id: device.device_id,
            connection: match device.connection_type {
                MuxConnection::Usb => Connection::Usb,
                MuxConnection::Network(_) => Connection::Network,
                MuxConnection::Unknown(kind) => Connection::Other(kind),
            },
        })
        .collect())
}

/// Reads what readiness needs from the device: its description without a
/// session, then, inside a session with the pairing record usbmuxd holds,
/// Developer Mode, the lock and the battery, and with Developer Mode on the
/// image mounter's listing.
async fn probe_device(provider: &UsbmuxdProvider) -> (Dictionary, Probe) {
    let mut values = Dictionary::new();
    let mut lockdown = match LockdownClient::connect(provider).await {
        Ok(lockdown) => lockdown,
        Err(error) => {
            return (
                values,
                Probe::untrusted(Trust::Unreachable(describe(&error))),
            );
        }
    };
    for key in DESCRIPTION_KEYS {
        if let Ok(value) = lockdown.get_value(Some(key), None).await {
            values.insert(key.to_owned(), value);
        }
    }
    let trust = match provider.get_pairing_file().await {
        Err(error) => trust_after(&error),
        Ok(pairing) => match lockdown.start_session(&pairing).await {
            Ok(_) => Trust::Trusted,
            Err(error) => trust_after(&error),
        },
    };
    if trust != Trust::Trusted {
        let probe = discovery::probe(&values, trust, None);
        return (values, probe);
    }
    if let Ok(value) = lockdown
        .get_value(Some(DEVELOPER_MODE_KEY), Some(DEVELOPER_MODE_DOMAIN))
        .await
    {
        values.insert(DEVELOPER_MODE_KEY.to_owned(), value);
    }
    // The lock, from lockdown: CoreDevice's getlockstate answers that it is
    // not implemented on iOS 26.6.2 (spike S9), so it is never asked.
    if let Ok(value) = lockdown.get_value(Some(LOCK_KEY), None).await {
        values.insert(LOCK_KEY.to_owned(), value);
    }
    // The battery, which `devicectl` does not report, for the harness's
    // provenance.
    if let Ok(value) = lockdown.get_value(None, Some(BATTERY_DOMAIN)).await {
        values.insert(BATTERY_DOMAIN.to_owned(), value);
    }
    let developer_mode = values.get(DEVELOPER_MODE_KEY).and_then(Value::as_boolean);
    let images = if developer_mode == Some(true) {
        Some(mounted_images(provider).await)
    } else {
        None
    };
    let probe = discovery::probe(&values, trust, images);
    (values, probe)
}

/// The image mounter's `CopyDevices` entries.
pub(crate) async fn mounted_images(
    provider: &dyn IdeviceProvider,
) -> std::result::Result<Vec<Value>, String> {
    let mut mounter = ImageMounter::connect(provider)
        .await
        .map_err(|error| format!("the image mounter did not start: {}", describe(&error)))?;
    mounter
        .copy_devices()
        .await
        .map_err(|error| format!("CopyDevices failed: {}", describe(&error)))
}

/// The trust a failed `ReadPairRecord` or `StartSession` means.
pub(crate) fn trust_after(error: &IdeviceError) -> Trust {
    match error {
        IdeviceError::InvalidHostID => Trust::Refused,
        // usbmuxd answers ReadPairRecord without a record, which idevice
        // reports as a reply missing it.
        IdeviceError::UnexpectedResponse(message) if message.contains("PairRecordData") => {
            Trust::NotPaired
        }
        IdeviceError::UnknownErrorType(kind) if kind.contains("PasswordProtected") => {
            Trust::Unreachable(
                "the device is locked, and lockdown refuses a session until it is unlocked".into(),
            )
        }
        other => Trust::Unreachable(describe(other)),
    }
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::discovery::usb_entries;
    use crate::model::Connection;

    /// usbmuxd's reply to `ListDevices` as its socket carries it: the
    /// 16-byte header (length, version 1, plist message 8, tag) and the
    /// recorded listing.
    fn reply_packet() -> Vec<u8> {
        let body = include_bytes!("../../tests/fixtures/usbmuxd-list-devices.plist");
        let mut packet = Vec::new();
        packet.extend_from_slice(&(16 + body.len() as u32).to_le_bytes());
        packet.extend_from_slice(&1u32.to_le_bytes());
        packet.extend_from_slice(&8u32.to_le_bytes());
        packet.extend_from_slice(&0u32.to_le_bytes());
        packet.extend_from_slice(body);
        packet
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime")
    }

    #[test]
    fn the_recorded_listing_goes_through_usbmuxds_parser_to_the_phones_usb_entry() {
        let entries = runtime().block_on(async {
            let (client, mut server) = tokio::io::duplex(1 << 16);
            let usbmuxd = tokio::spawn(async move {
                // The request, then the reply; the request's bytes are not
                // checked, idevice writes them.
                let mut request = [0u8; 16];
                server
                    .read_exact(&mut request)
                    .await
                    .expect("a request header");
                let length = u32::from_le_bytes(request[..4].try_into().expect("4 bytes")) as usize;
                let mut body = vec![0u8; length - 16];
                server.read_exact(&mut body).await.expect("a request body");
                server.write_all(&reply_packet()).await.expect("the reply");
                body
            });
            let entries = list_devices(UsbmuxdConnection::new(Box::new(client), 0))
                .await
                .expect("the listing parses");
            let request = usbmuxd.await.expect("the fake usbmuxd ran");
            let request: Dictionary = plist::from_bytes(&request).expect("a plist request");
            assert_eq!(
                request.get("MessageType").and_then(|v| v.as_string()),
                Some("ListDevices")
            );
            entries
        });
        assert_eq!(
            entries,
            vec![
                MuxEntry {
                    udid: "placeholder-udid-phone".into(),
                    device_id: 22198,
                    connection: Connection::Network,
                },
                MuxEntry {
                    udid: "placeholder-udid-phone".into(),
                    device_id: 22197,
                    connection: Connection::Usb,
                },
                MuxEntry {
                    udid: "placeholder-udid-wifi-only".into(),
                    device_id: 22199,
                    connection: Connection::Network,
                },
            ]
        );
        assert_eq!(
            usb_entries(entries),
            vec![MuxEntry {
                udid: "placeholder-udid-phone".into(),
                device_id: 22197,
                connection: Connection::Usb,
            }]
        );
    }

    #[test]
    fn a_failed_pairing_record_or_session_says_what_stands_in_the_way() {
        assert_eq!(
            trust_after(&IdeviceError::UnexpectedResponse(
                "missing PairRecordData in pair record response".into()
            )),
            Trust::NotPaired
        );
        assert_eq!(trust_after(&IdeviceError::InvalidHostID), Trust::Refused);
        match trust_after(&IdeviceError::Socket(std::io::Error::new(
            std::io::ErrorKind::BrokenPipe,
            "the device went away",
        ))) {
            Trust::Unreachable(reason) => {
                assert!(reason.contains("the device went away"), "{reason}")
            }
            other => panic!("expected unreachable, got {other:?}"),
        }
        match trust_after(&IdeviceError::UnknownErrorType("PasswordProtected".into())) {
            Trust::Unreachable(reason) => assert!(reason.contains("locked"), "{reason}"),
            other => panic!("expected unreachable, got {other:?}"),
        }
    }
}
