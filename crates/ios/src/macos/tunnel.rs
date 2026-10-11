//! The CoreDevice tunnel in this process (design D18): lockdown's
//! `CoreDeviceProxy` over usbmuxd carries the tunnel, `idevice`'s jktcp stack
//! ends it inside the process, so there is no TUN interface and no root, and
//! RSD's handshake over it lists the developer services by name and port.
//! The tunnel is opened once per target and kept; a service connection that
//! finds it dropped opens a new one, once, and a device that is gone is
//! reported disconnected.

use std::future::Future;
use std::time::{Duration, Instant};

use idevice::core_device_proxy::CoreDeviceProxy;
use idevice::provider::{RsdProvider, UsbmuxdProvider};
use idevice::rsd::RsdHandshake;
use idevice::tcp::handle::AdapterHandle;
use idevice::{IdeviceError, IdeviceService, RsdService};

use tokio::time::timeout;

use crate::error::{Error, Result};
use crate::model::{Disconnect, TunnelStatus};
use crate::services;

use super::host::describe;

/// Opens tunnels to one device: the seam the tests replace with a fake
/// endpoint. Its futures are `Send`, so a task of its own can reopen one.
pub(crate) trait Opener: Send + Sync + 'static {
    /// Reaches the device's service ports once the tunnel is open.
    type Provider: RsdProvider + Send + Sync + 'static;

    /// Opens a tunnel: a provider of connections to the device's ports, and
    /// the port RSD answers on.
    fn open(
        &self,
    ) -> impl Future<Output = std::result::Result<(Self::Provider, u16), String>> + Send;

    /// Closes the tunnel `provider` reaches.
    fn close(&self, provider: &mut Self::Provider) -> impl Future<Output = ()> + Send;
}

/// How long the tunnel's steps get.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// `CoreDeviceProxy`, the TCP connection to RSD and its handshake: 60 ms
    /// at p50 in spike S9.
    pub open: Duration,
    /// One service connection: 25 to 37 ms for DVT's in spike S9.
    pub connect: Duration,
    /// One reply on a service connection: a launch's came in 46 ms at p95
    /// in spike S9.
    pub reply: Duration,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            open: Duration::from_secs(10),
            connect: Duration::from_secs(10),
            reply: Duration::from_secs(10),
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
        if self.link.is_some() {
            return Ok(());
        }
        let started = Instant::now();
        let opened = timeout(self.limits.open, self.opener.open()).await;
        let (mut provider, rsd_port) = match opened {
            Ok(Ok(opened)) => opened,
            Ok(Err(cause)) => {
                return Err(Error::Disconnected {
                    kind: Disconnect::Lost,
                    target: self.target.clone(),
                    cause,
                });
            }
            Err(_) => {
                return Err(Error::Timeout {
                    what: format!("the CoreDevice tunnel to {}", self.target),
                    after: self.limits.open,
                });
            }
        };
        let left = self.limits.open.saturating_sub(started.elapsed());
        let handshake = timeout(left, rsd_handshake(&mut provider, rsd_port)).await;
        let error = match handshake {
            Ok(Ok(handshake)) => {
                self.opens += 1;
                self.opened_in = started.elapsed();
                self.link = Some(Link {
                    provider,
                    handshake,
                });
                return Ok(());
            }
            Ok(Err(error)) if tunnel_gone(&error) => Error::Disconnected {
                kind: Disconnect::Lost,
                target: self.target.clone(),
                cause: format!("RSD's handshake: {}", describe(&error)),
            },
            Ok(Err(error)) => Error::Device {
                call: "RSD's handshake".into(),
                reason: describe(&error),
            },
            Err(_) => Error::Timeout {
                what: format!("RSD's handshake with {}", self.target),
                after: self.limits.open,
            },
        };
        self.opener.close(&mut provider).await;
        Err(error)
    }

    /// A connection to the service `S` through the tunnel, opening it first
    /// when it is not open. A tunnel found dropped is opened again, once; a
    /// device that is gone fails as disconnected, and a service the
    /// handshake does not list as missing.
    pub(crate) async fn connect<S: RsdService>(&mut self) -> Result<S> {
        let name = S::rsd_service_name().into_owned();
        let mut reopened = false;
        loop {
            self.open().await?;
            let link = self.link.as_mut().expect("an open tunnel");
            if !link.handshake.services.contains_key(&name) {
                return Err(Error::ServiceMissing {
                    target: self.target.clone(),
                    service: name,
                    listed: link.handshake.services.len(),
                });
            }
            let connection = timeout(
                self.limits.connect,
                link.handshake.connect::<S>(&mut link.provider),
            )
            .await;
            match connection {
                Ok(Ok(service)) => return Ok(service),
                Ok(Err(error)) if tunnel_gone(&error) => {
                    self.close().await;
                    if reopened {
                        return Err(Error::Disconnected {
                            kind: Disconnect::Lost,
                            target: self.target.clone(),
                            cause: format!("connecting to {name}: {}", describe(&error)),
                        });
                    }
                    reopened = true;
                }
                Ok(Err(error)) => {
                    return Err(Error::Device {
                        call: format!("connecting to {name}"),
                        reason: describe(&error),
                    });
                }
                Err(_) => {
                    self.close().await;
                    return Err(Error::Timeout {
                        what: format!("a connection to {name} on {}", self.target),
                        after: self.limits.connect,
                    });
                }
            }
        }
    }

    /// Whether a tunnel is open: one opened and not closed since, though it
    /// may have dropped.
    pub(crate) fn is_open(&self) -> bool {
        self.link.is_some()
    }

    /// What the open tunnel's handshake listed, `None` before one opened.
    pub(crate) fn status(&self) -> Option<TunnelStatus> {
        let link = self.link.as_ref()?;
        let mut services: Vec<String> = link.handshake.services.keys().cloned().collect();
        services.sort();
        let missing = services::missing(services.iter().map(String::as_str));
        Some(TunnelStatus {
            opens: self.opens,
            opened_in: self.opened_in,
            services,
            missing,
        })
    }

    /// Closes the tunnel; the next connection opens a new one.
    pub(crate) async fn close(&mut self) {
        if let Some(mut link) = self.link.take() {
            self.opener.close(&mut link.provider).await;
        }
    }
}

/// RSD's handshake on `port` through `provider`. A function of its own
/// rather than a block, since a block borrowing the port would keep the
/// tunnel's futures from being `Send` for every lifetime.
async fn rsd_handshake<P: RsdProvider>(
    provider: &mut P,
    port: u16,
) -> std::result::Result<RsdHandshake, IdeviceError> {
    let stream = provider.connect_to_service_port(port).await?;
    RsdHandshake::new(stream).await
}

/// Whether `error` says the tunnel itself is gone, so that a new one may
/// serve: jktcp's stack answers a connection on a dead tunnel with "adapter
/// closed".
pub(crate) fn tunnel_gone(error: &IdeviceError) -> bool {
    use std::io::ErrorKind;
    matches!(
        error,
        IdeviceError::Socket(io) if matches!(
            io.kind(),
            ErrorKind::BrokenPipe
                | ErrorKind::NetworkUnreachable
                | ErrorKind::ConnectionReset
                | ErrorKind::ConnectionAborted
                | ErrorKind::NotConnected
                | ErrorKind::UnexpectedEof
                | ErrorKind::TimedOut
        )
    )
}

/// The real opener: `CoreDeviceProxy` through lockdown, ended by jktcp.
pub(crate) struct UsbTunnel {
    pub(crate) provider: UsbmuxdProvider,
}

impl Opener for UsbTunnel {
    type Provider = AdapterHandle;

    async fn open(&self) -> std::result::Result<(AdapterHandle, u16), String> {
        let proxy = CoreDeviceProxy::connect(&self.provider)
            .await
            .map_err(|error| format!("CoreDeviceProxy: {}", describe(&error)))?;
        let rsd_port = proxy.tunnel_info().server_rsd_port;
        let adapter = proxy
            .create_software_tunnel()
            .map_err(|error| format!("jktcp's tunnel: {}", describe(&error)))?;
        Ok((adapter.to_async_handle(), rsd_port))
    }

    async fn close(&self, provider: &mut AdapterHandle) {
        let _ = provider.close().await;
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::sync::Arc;

    use idevice::ReadWrite;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;
    use crate::macos::fake::{FakeDevice, FakeOpener, drain};
    use crate::services;

    /// A connection to DVT's service hub, as raw as it comes.
    struct Dvt(Box<dyn ReadWrite>);

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
        reply: Duration::from_millis(500),
    };

    #[test]
    fn the_handshake_lists_the_recorded_services_and_a_service_is_found_by_name() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.serve(
                services::DVT,
                Arc::new(|mut stream| {
                    Box::pin(async move {
                        let _ = stream.write_all(b"dvt").await;
                        drain(stream).await;
                    })
                }),
            );
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
            let Dvt(mut stream) = tunnel.connect::<Dvt>().await.expect("DVT is listed");
            let mut greeting = [0u8; 3];
            stream
                .read_exact(&mut greeting)
                .await
                .expect("DVT's server answers");
            assert_eq!(&greeting, b"dvt", "the connection reaches DVT's own server");
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
