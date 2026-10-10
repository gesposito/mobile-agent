//! The WebDriverAgent baseline's helper process on macOS: it starts the
//! WebDriverAgent installed on a physical iPhone through the device's test
//! manager, with `idevice`'s xctest session, as `crates/ios` starts its own
//! runner, and forwards a localhost port to WebDriverAgent's HTTP port
//! through usbmuxd's `Connect`. Neither needs root, a RemoteXPC tunnel of
//! the host's or `xcodebuild`, whose test run on a device waits for the
//! Mac's DevToolsSecurity.
//!
//! `mobile-agent-bench wda-session --udid <udid> --bundle-id <runner>` runs
//! it: once WebDriverAgent answers its `/status`, the helper prints one line,
//! `wda http://127.0.0.1:<port>`, and keeps the session until its standard
//! input closes, when it ends the session, which ends WebDriverAgent's test
//! and with it the runner, and exits. A session that ends by itself, as when
//! the phone goes away, is an error on standard error and exit status 1.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use idevice::IdeviceService;
use idevice::dvt::xctest::{TestConfig, XCUITestService};
use idevice::installation_proxy::InstallationProxyClient;
use idevice::provider::UsbmuxdProvider;
use idevice::usbmuxd::{Connection as MuxConnection, UsbmuxdAddr};
use tokio::sync::oneshot;

/// The label the helper gives usbmuxd and lockdown.
const LABEL: &str = "mobile-agent-bench";

/// Starts WebDriverAgent, the runner `bundle_id` names, on the iPhone `udid`
/// on USB, waits up to `readiness` for its `/status`, prints its localhost
/// URL and serves until standard input closes.
pub fn serve(udid: &str, bundle_id: &str, readiness: Duration) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start the async runtime: {e}"))?;
    runtime.block_on(session(udid, bundle_id, readiness))
}

async fn session(udid: &str, bundle_id: &str, readiness: Duration) -> Result<(), String> {
    let addr = UsbmuxdAddr::from_env_var()
        .map_err(|e| format!("USBMUXD_SOCKET_ADDRESS is not an address: {e}"))?;
    let mut mux = addr
        .connect(0)
        .await
        .map_err(|e| format!("cannot reach usbmuxd: {e}"))?;
    let device = mux
        .get_devices()
        .await
        .map_err(|e| format!("usbmuxd did not list its devices: {e}"))?
        .into_iter()
        .find(|device| device.udid == udid && matches!(device.connection_type, MuxConnection::Usb))
        .ok_or_else(|| format!("usbmuxd lists no iPhone {udid} on USB"))?;
    let provider = UsbmuxdProvider {
        addr,
        tag: device.device_id,
        udid: device.udid.clone(),
        device_id: device.device_id,
        label: LABEL.to_owned(),
    };
    let mut proxy = InstallationProxyClient::connect(&provider)
        .await
        .map_err(|e| format!("cannot reach the installation proxy: {e}"))?;
    let config = TestConfig::from_installation_proxy(&mut proxy, bundle_id, None)
        .await
        .map_err(|e| format!("the installation proxy did not find the runner {bundle_id}: {e}"))?;
    drop(proxy);
    let handle = XCUITestService::new(Arc::new(provider))
        .run_until_wda_ready_with_bridge(config, readiness)
        .await
        .map_err(|e| format!("WebDriverAgent did not answer through the test manager: {e}"))?;
    let mut stdout = std::io::stdout();
    writeln!(stdout, "wda {}", handle.wda_url())
        .and_then(|()| stdout.flush())
        .map_err(|e| format!("cannot print WebDriverAgent's address: {e}"))?;

    // Standard input closes when the adapter lets go of the helper, or when
    // the adapter itself is gone. Returning drops the session and the bridge
    // with the runtime, which closes their connections to the phone.
    let (closed, stdin_closed) = oneshot::channel();
    std::thread::spawn(move || {
        let mut sink = [0_u8; 64];
        let mut stdin = std::io::stdin();
        while matches!(stdin.read(&mut sink), Ok(n) if n > 0) {}
        let _ = closed.send(());
    });
    tokio::select! {
        _ = stdin_closed => Ok(()),
        ended = handle.wait() => Err(match ended {
            Ok(()) => "WebDriverAgent's test ended".to_owned(),
            Err(e) => format!("WebDriverAgent's test manager session ended: {e}"),
        }),
    }
}
