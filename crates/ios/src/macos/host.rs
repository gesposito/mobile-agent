//! The host side of discovery, through `idevice` (design D18): usbmuxd's
//! listing, and the lockdown probe of one device with the host's pairing
//! record, which usbmuxd hands over (`ReadPairRecord`), so nothing reads
//! `/var/db/lockdown`.

use idevice::IdeviceError;
use idevice::usbmuxd::UsbmuxdConnection;
use plist::Dictionary;

use crate::error::Result;
use crate::identity::IdentityCheck;
use crate::model::{MuxEntry, Probe, Trust};

/// What discovery needs from the host: the seam the tests replace.
pub(crate) trait Host: Send + Sync + 'static {
    /// usbmuxd's listing, every entry as it comes.
    async fn list(&self) -> Result<Vec<MuxEntry>>;

    /// What lockdown and the image mounter say about the device of `entry`:
    /// the values read, under their keys, and the probe.
    async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe);

    /// The host's signing identity for the runner.
    fn identity(&self) -> IdentityCheck;
}

/// The real host: usbmuxd at its default socket, lockdown through it.
#[derive(Debug, Default)]
pub(crate) struct UsbHost;

impl Host for UsbHost {
    async fn list(&self) -> Result<Vec<MuxEntry>> {
        todo!()
    }

    async fn probe(&self, entry: &MuxEntry) -> (Dictionary, Probe) {
        let _ = entry;
        todo!()
    }

    fn identity(&self) -> IdentityCheck {
        crate::identity::host_identity()
    }
}

/// Every entry of usbmuxd's listing over `mux`, with its connection.
pub(crate) async fn list_devices(mux: UsbmuxdConnection) -> Result<Vec<MuxEntry>> {
    let _ = mux;
    todo!()
}

/// The trust a failed `ReadPairRecord` or `StartSession` means.
pub(crate) fn trust_after(error: &IdeviceError) -> Trust {
    let _ = error;
    todo!()
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
