//! The CoreDevice tunnel in this process (design D18): lockdown's
//! `CoreDeviceProxy` over usbmuxd carries the tunnel, `idevice`'s jktcp stack
//! ends it inside the process, so there is no TUN interface and no root, and
//! RSD's handshake over it lists the developer services by name and port.
//! The tunnel is opened once per target and kept; a service connection that
//! finds it dropped opens a new one, once, and a device that is gone is
//! reported disconnected.

use std::time::{Duration, Instant};

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::provider::{RsdProvider, UsbmuxdProvider};
use idevice::rsd::RsdHandshake;
use idevice::tcp::handle::AdapterHandle;
use idevice::{IdeviceError, IdeviceService, RsdService};

use crate::error::{Error, Result};
use crate::model::TunnelStatus;

use super::host::describe;

/// Opens tunnels to one device: the seam the tests replace with a fake
/// endpoint.
pub(crate) trait Opener: Send + Sync + 'static {
    /// Reaches the device's service ports once the tunnel is open.
    type Provider: RsdProvider + Send + Sync + 'static;

    /// Opens a tunnel: a provider of connections to the device's ports, and
    /// the port RSD answers on.
    async fn open(&self) -> std::result::Result<(Self::Provider, u16), String>;

    /// Closes the tunnel `provider` reaches.
    async fn close(&self, provider: &mut Self::Provider);
}

/// How long the tunnel's steps get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// `CoreDeviceProxy`, the TCP connection to RSD and its handshake: 60 ms
    /// at p50 in spike S9.
    pub open: Duration,
    /// One service connection: 25 to 37 ms for DVT's in spike S9.
    pub connect: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            open: Duration::from_secs(10),
            connect: Duration::from_secs(10),
        }
    }
}

/// An open tunnel and what its handshake listed.
struct Link<P> {
    provider: P,
    handshake: RsdHandshake,
}

/// The tunnel to one device, opened on first use and kept.
pub(crate) struct Tunnel<O: Opener> {
    target: String,
    opener: O,
    limits: Limits,
    link: Option<Link<O::Provider>>,
    opens: u32,
    opened_in: Duration,
}

impl<O: Opener> Tunnel<O> {
    pub(crate) fn new(target: &str, opener: O, limits: Limits) -> Tunnel<O> {
        Tunnel {
            target: target.to_owned(),
            opener,
            limits,
            link: None,
            opens: 0,
            opened_in: Duration::ZERO,
        }
    }

    /// Opens the tunnel and RSD's handshake, unless they are open.
    pub(crate) async fn open(&mut self) -> Result<()> {
        let _ = (Instant::now, &self.opener, self.limits, describe);
        todo!()
    }

    /// A connection to the service `S` through the tunnel, opening it first
    /// when it is not open. A tunnel found dropped is opened again, once; a
    /// device that is gone fails as disconnected, and a service the
    /// handshake does not list as missing.
    pub(crate) async fn connect<S: RsdService>(&mut self) -> Result<S> {
        todo!()
    }

    /// What the open tunnel's handshake listed, `None` before one opened.
    pub(crate) fn status(&self) -> Option<TunnelStatus> {
        todo!()
    }

    /// Closes the tunnel; the next connection opens a new one.
    pub(crate) async fn close(&mut self) {
        todo!()
    }
}

/// Whether `error` says the tunnel itself is gone, so that a new one may
/// serve: jktcp's stack answers a connection on a dead tunnel with "adapter
/// closed".
pub(crate) fn tunnel_gone(error: &IdeviceError) -> bool {
    let _ = error;
    todo!()
}

/// The real opener: `CoreDeviceProxy` through lockdown, ended by jktcp.
pub(crate) struct UsbTunnel {
    pub(crate) provider: UsbmuxdProvider,
}

impl Opener for UsbTunnel {
    type Provider = AdapterHandle;

    async fn open(&self) -> std::result::Result<(AdapterHandle, u16), String> {
        let _ = CoreDeviceProxy::connect;
        todo!()
    }

    async fn close(&self, provider: &mut AdapterHandle) {
        let _ = provider.close().await;
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use idevice::ReadWrite;

    use super::*;
    use crate::macos::fake::{FakeDevice, FakeOpener};
    use crate::services;

    /// A connection to DVT's service hub, as raw as it comes.
    struct Dvt(#[allow(dead_code)] Box<dyn ReadWrite>);

    impl RsdService for Dvt {
        fn rsd_service_name() -> Cow<'static, str> {
            Cow::Borrowed(services::DVT)
        }

        async fn from_stream(stream: Box<dyn ReadWrite>) -> std::result::Result<Dvt, IdeviceError> {
            Ok(Dvt(stream))
        }
    }

    /// A service no device lists.
    struct Unlisted;

    impl RsdService for Unlisted {
        fn rsd_service_name() -> Cow<'static, str> {
            Cow::Borrowed("dev.mobileagent.placeholder.unlisted")
        }

        async fn from_stream(_: Box<dyn ReadWrite>) -> std::result::Result<Unlisted, IdeviceError> {
            Ok(Unlisted)
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime")
    }

    const SHORT: Limits = Limits {
        open: Duration::from_millis(500),
        connect: Duration::from_millis(500),
    };

    #[test]
    fn the_handshake_lists_the_recorded_services_and_a_service_is_found_by_name() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            assert_eq!(
                tunnel.status(),
                None,
                "nothing is open before the first use"
            );
            tunnel.open().await.expect("the tunnel opens");
            let status = tunnel.status().expect("an open tunnel");
            assert_eq!(status.opens, 1);
            assert_eq!(status.services.len(), 82);
            assert!(
                status.services.windows(2).all(|pair| pair[0] < pair[1]),
                "sorted"
            );
            assert!(status.missing.is_empty(), "{:?}", status.missing);
            tunnel.connect::<Dvt>().await.expect("DVT is listed");
            assert_eq!(
                device.connections(),
                vec![
                    device.rsd_port(),
                    device.port(services::DVT).expect("a port")
                ],
                "the handshake on RSD's port, then DVT on its own"
            );
            tunnel.open().await.expect("still open");
            assert_eq!(tunnel.status().expect("open").opens, 1, "opened once");
        });
    }

    #[test]
    fn the_services_a_device_without_the_disk_image_lacks_are_named_missing() {
        runtime().block_on(async {
            let device = FakeDevice::without(&[
                services::TEST_MANAGER,
                services::HID_INDIGO,
                services::HID_UNIVERSAL,
            ]);
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            tunnel.open().await.expect("the tunnel opens");
            let status = tunnel.status().expect("an open tunnel");
            assert_eq!(status.services.len(), 79);
            assert_eq!(
                status.missing,
                vec![
                    services::TEST_MANAGER,
                    services::HID_INDIGO,
                    services::HID_UNIVERSAL
                ]
            );
        });
    }

    #[test]
    fn a_service_the_handshake_does_not_list_is_missing_and_the_tunnel_stays() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            let error = match tunnel.connect::<Unlisted>().await {
                Ok(_) => panic!("an unlisted service connected"),
                Err(error) => error,
            };
            assert_eq!(error.code(), "service_missing", "{error}");
            assert!(
                error
                    .to_string()
                    .contains("dev.mobileagent.placeholder.unlisted"),
                "{error}"
            );
            assert_eq!(
                device.opens(),
                1,
                "a missing service is no reason to reopen"
            );
        });
    }

    #[test]
    fn a_tunnel_that_dropped_is_opened_again_once() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            tunnel.connect::<Dvt>().await.expect("the first connection");
            device.drop_tunnel();
            tunnel.connect::<Dvt>().await.expect("a new tunnel serves");
            assert_eq!(device.opens(), 2);
            assert_eq!(tunnel.status().expect("open").opens, 2);
        });
    }

    #[test]
    fn a_device_that_is_gone_is_disconnected_with_the_cause() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            tunnel.connect::<Dvt>().await.expect("the first connection");
            device.unplug();
            let error = match tunnel.connect::<Dvt>().await {
                Ok(_) => panic!("a gone device served"),
                Err(error) => error,
            };
            assert_eq!(error.code(), "disconnected", "{error}");
            assert!(
                error.to_string().contains("placeholder-udid-phone"),
                "{error}"
            );
            assert_eq!(tunnel.status(), None, "nothing stays open");
            device.plug_in();
            tunnel
                .connect::<Dvt>()
                .await
                .expect("plugged in again, it serves");
        });
    }

    #[test]
    fn a_handshake_that_never_comes_times_out_and_leaves_nothing_open() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.silence_rsd();
            let mut tunnel = Tunnel::new("placeholder-udid-phone", FakeOpener::new(&device), SHORT);
            let error = tunnel.open().await.expect_err("no handshake");
            assert_eq!(error.code(), "timeout", "{error}");
            assert_eq!(tunnel.status(), None);
            assert!(device.closed() >= 1, "the half-open tunnel is closed");
        });
    }

    #[test]
    fn a_dead_tunnel_is_told_by_its_stacks_errors() {
        use std::io::{Error as IoError, ErrorKind};
        for kind in [
            ErrorKind::BrokenPipe,
            ErrorKind::NetworkUnreachable,
            ErrorKind::ConnectionReset,
            ErrorKind::UnexpectedEof,
            ErrorKind::TimedOut,
        ] {
            assert!(
                tunnel_gone(&IdeviceError::Socket(IoError::new(kind, "adapter closed"))),
                "{kind:?}"
            );
        }
        assert!(!tunnel_gone(&IdeviceError::ServiceNotFound));
        assert!(!tunnel_gone(&IdeviceError::UnexpectedResponse("no".into())));
    }
}
